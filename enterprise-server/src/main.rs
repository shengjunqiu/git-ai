use clap::Parser;
use tracing_subscriber::EnvFilter;

mod auth;
mod config;
mod db;
mod error;
mod handlers;
mod models;
mod pos_encoded;
mod routes;
mod services;

// test
// 测试

/// git-ai Enterprise Server
#[derive(Parser, Debug)]
#[command(name = "git-ai-enterprise-server", version, about)]
struct Args {
    /// Run database migrations and exit
    #[arg(long)]
    migrate: bool,

    /// Listen address
    #[arg(long, env = "LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    listen_addr: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("git_ai_enterprise_server=debug,tower_http=debug")),
        )
        .init();

    let args = Args::parse();
    let config = config::AppConfig::from_env()?;

    // Initialize database pool
    let db_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&config.database_url)
        .await?;

    if args.migrate {
        tracing::info!("Running database migrations...");
        db::run_migrations(&db_pool).await?;
        tracing::info!("Migrations completed successfully.");
        return Ok(());
    }

    // Initialize Redis
    let redis_client = redis::Client::open(config.redis_url.clone())?;

    // Initialize CAS store
    let cas_store = services::cas::CasStore::new(&config)?;

    // Initialize rate limiter
    let rate_limiter = services::rate_limit::RateLimiter::new();

    // Build application state
    let state = crate::routes::AppState {
        db: db_pool.clone(),
        redis: redis_client,
        config: config.clone(),
        cas_store,
        rate_limiter,
    };

    // Run migrations on startup (auto-migrate)
    db::run_migrations(&db_pool).await?;

    // Build router
    let app = routes::build_router(state);

    // Start server
    let listener = tokio::net::TcpListener::bind(&args.listen_addr).await?;
    tracing::info!("git-ai Enterprise Server listening on {}", args.listen_addr);
    tracing::info!("Dashboard available at {}/me", config.base_url);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C handler");
    tracing::info!("Shutdown signal received, gracefully stopping...");
}
