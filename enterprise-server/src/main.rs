// 引入命令行参数解析库 clap，用于定义和解析启动参数
use clap::Parser;
// 引入标准库的时间Duration类型，用于设置各类超时与间隔
use std::time::Duration;
// 引入 tracing 的日志过滤工具，用于根据环境变量控制日志级别
use tracing_subscriber::EnvFilter;

// 以下 `mod` 声明引入本服务各功能模块；模块源码位于同目录下的对应 .rs 文件
mod auth; // 认证相关（JWT、OAuth、登录等）
mod config; // 配置加载（环境变量 / .env）
mod db; // 数据库连接与迁移
mod error; // 统一错误类型
mod handlers; // HTTP 请求处理函数（按路由分组）
mod models; // 数据模型 / 结构体定义
mod pos_encoded; // 游标分页（position-encoded）相关的编解码工具
mod routes; // 路由定义与共享状态（AppState）
mod services; // 业务逻辑层（CAS、限流、指标汇总等）

/// git-ai 企业服务端（Enterprise Server）的命令行参数定义
/// 使用 clap 的派生宏，从命令行或环境变量中解析参数。
#[derive(Parser, Debug)]
#[command(name = "git-ai-enterprise-server", version, about)]
struct Args {
    /// 仅执行数据库迁移后退出，不启动 HTTP 服务
    /// 用法示例：`git-ai-enterprise-server --migrate`
    #[arg(long)]
    migrate: bool,

    /// 服务监听地址（host:port）
    /// 可通过命令行 `--listen-addr` 或环境变量 `LISTEN_ADDR` 指定，
    /// 默认监听在所有网卡的 8080 端口。
    #[arg(long, env = "LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    listen_addr: String,
}

/// 程序异步入口。使用 tokio 运行时（多线程）驱动整个异步服务。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 加载项目根目录下的 .env 文件到环境变量。
    // `.ok()` 表示即使不存在 .env 文件也不报错（生产环境通常用真实环境变量）。
    dotenvy::dotenv().ok();

    // 初始化日志（tracing）订阅器：
    // 1. 优先使用环境变量 `RUST_LOG`（try_from_default_env）控制日志级别；
    // 2. 若未设置，则回退到默认过滤规则：本服务 crate 与 tower_http 均输出 debug 级别日志。
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new("git_ai_enterprise_server=debug,tower_http=debug")
            }),
        )
        .init();

    // 解析命令行参数（含来自环境变量的 listen_addr）
    let args = Args::parse();
    // 从环境变量 / .env 加载应用配置（数据库连接、Redis、限流策略等）
    let config = config::AppConfig::from_env()?;

    // 初始化 PostgreSQL 连接池：
    // - max_connections：连接池上限；
    // - min_connections：空闲时保持的最小连接数，避免冷启动开销；
    // - acquire_timeout：获取连接的最长等待时间，超时则返回错误。
    let db_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .min_connections(config.database_min_connections)
        .acquire_timeout(Duration::from_secs(config.database_acquire_timeout_seconds))
        .connect(&config.database_url)
        .await?;

    // 若传入 `--migrate`，则只执行一次数据库迁移并正常退出，
    // 用于独立的“迁移容器 / 部署前置步骤”场景。
    if args.migrate {
        tracing::info!("Running database migrations...");
        db::run_migrations(&db_pool).await?;
        tracing::info!("Migrations completed successfully.");
        return Ok(());
    }

    // 初始化 Redis 客户端（用于限流计数、缓存、会话等）。
    // 这里只是创建客户端，真正的连接在使用时惰性建立。
    let redis_client = redis::Client::open(config.redis_url.clone())?;

    // 初始化 CAS（Content-Addressable Storage，内容寻址存储）存储。
    // 通常用于按内容哈希去重存储上报的较大负载（如快照/产物）。
    let cas_store = services::cas::CasStore::new(&config)?;

    // 初始化全局限流器（基于 Redis，实现跨进程/跨实例的共享计数）。
    // 若 Redis 不可用会返回 None，调用方需做降级处理。
    let rate_limiter = services::rate_limit::RateLimiter::with_redis(redis_client.clone()).await;
    // 初始化“密码登录”专用限流器（防止暴力破解），参数来自配置。
    let auth_password_limiter = crate::routes::auth_password_limiter(&config);

    // 组装共享的应用状态 AppState，交给后续所有路由处理函数按需使用。
    // 注意：db、redis、config 等通过 clone 共享同一份连接池 / 客户端（内部为 Arc）。
    let state = crate::routes::AppState {
        db: db_pool.clone(),
        redis: redis_client,
        config: config.clone(),
        cas_store,
        rate_limiter,
        auth_password_limiter,
    };

    // 服务启动时自动执行数据库迁移（auto-migrate），
    // 确保表结构与代码版本一致；失败则直接返回错误并终止启动。
    db::run_migrations(&db_pool).await?;

    // 根据配置决定是否启动“指标汇总（rollup）后台 worker”：
    // 该 worker 周期性地将原始 metrics 事件聚合到汇总表，提升大盘查询性能。
    if config.metrics_rollup_worker_enabled {
        tracing::info!(
            interval_seconds = config.metrics_rollup_worker_interval_seconds,
            batch_size = config.metrics_rollup_worker_batch_size,
            "starting metrics rollup worker"
        );
        services::metrics::spawn_metrics_rollup_worker(
            db_pool.clone(),
            Duration::from_secs(config.metrics_rollup_worker_interval_seconds),
            config.metrics_rollup_worker_batch_size,
        );
    } else if matches!(
        config.metrics_rollup_write_mode,
        config::MetricsRollupWriteMode::DirtyAsync
    ) {
        // 配置冲突告警：要求异步脏数据汇总，却没有启用 worker，
        // 会导致脏数据永远不会被汇总，这里仅记录 warning 提示运维。
        tracing::warn!(
            "METRICS_ROLLUP_WRITE_MODE=dirty_async is set but METRICS_ROLLUP_WORKER_ENABLED=false"
        );
    }

    // 构建 axum 路由（将 AppState 注入各 handler）
    let app = routes::build_router(state);

    // 绑定 TCP 监听地址，准备接收 HTTP 请求
    let listener = tokio::net::TcpListener::bind(&args.listen_addr).await?;
    tracing::info!("git-ai Enterprise Server listening on {}", args.listen_addr);
    // 提示 Dashboard 访问入口（基于配置中的 base_url）
    tracing::info!("Dashboard available at {}/me", config.base_url);

    // 启动 HTTP 服务，并注册优雅关闭逻辑：
    // 收到 SIGINT（Ctrl+C）等信号时，先处理完在途请求再退出。
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// 优雅关闭信号处理器。
/// 阻塞等待 Ctrl+C（SIGINT）信号；收到后记录日志并返回，
/// 使 axum 的 `with_graceful_shutdown` 触发平滑停机。
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C handler");
    tracing::info!("Shutdown signal received, gracefully stopping...");
}
