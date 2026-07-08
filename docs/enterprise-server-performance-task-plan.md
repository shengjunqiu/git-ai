# Enterprise Server 性能优化任务清单

本文档把 [Enterprise Server 性能优化执行计划](./enterprise-server-performance-optimization-plan.md) 拆成可以逐步执行、逐步验收、逐步提交的工程任务。

执行原则：

1. 每次只处理一个任务块。
2. 每个任务块单独提交，便于回滚和压测对比。
3. 先做 P0 低风险改动，再做 P1 写入吞吐，最后做 P2 dashboard 大数据量优化。
4. 每个任务块必须包含代码、测试、必要的迁移和压测记录。
5. 修改生产行为前先保留默认兼容配置，避免一次性改变太多变量。

## 阶段 0: 准备和基线记录

### 0.1 确认本地 enterprise-server 依赖可用

目标：确认本地能运行服务端测试、容器依赖和基础压测。

步骤：

- [x] 进入服务端目录：

```bash
cd enterprise-server
```

- [x] 启动依赖：

```bash
docker compose up -d postgres redis minio
```

- [x] 确认容器状态：

```bash
docker compose ps
```

- [x] 确认 Postgres 就绪：

```bash
docker compose exec postgres pg_isready -U gitai -d gitai_enterprise
```

- [x] 跑服务端测试：

```bash
cargo test
```

验收标准：

- [x] Postgres、Redis、MinIO 都处于 running/healthy。
- [x] `cargo test` 通过。
- [ ] 如果 Redis/Postgres 不可用，记录原因，不继续做性能改动。

### 0.2 记录当前基础压测基线

目标：为后续优化提供可对比数据。

步骤：

- [x] 启动 API：

```bash
docker compose up -d --build api
```

- [x] 确认健康检查：

```bash
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/ready
```

- [x] 跑基础压测：

```bash
ab -n 100 -c 20 http://127.0.0.1:8080/health
ab -n 100 -c 20 http://127.0.0.1:8080/ready
```

- [x] 记录容器资源：

```bash
docker stats --no-stream enterprise-server-api-1 enterprise-server-postgres-1 enterprise-server-redis-1 enterprise-server-minio-1
```

记录项：

- [x] `/health` RPS、p50、p95、p99、失败数。
- [x] `/ready` RPS、p50、p95、p99、失败数。
- [x] API/Postgres/Redis/MinIO CPU 和内存。
- [x] 当前数据库数据量：

```bash
docker compose exec -T postgres psql -U gitai -d gitai_enterprise -c "
SELECT 'metrics_events' AS table_name, COUNT(*) FROM metrics_events
UNION ALL SELECT 'commit_stats', COUNT(*) FROM commit_stats
UNION ALL SELECT 'cas_objects', COUNT(*) FROM cas_objects
UNION ALL SELECT 'report_uploads', COUNT(*) FROM report_uploads
UNION ALL SELECT 'users', COUNT(*) FROM users;
"
```

验收标准：

- [x] 基线结果写入 PR 描述或单独记录文件。
- [x] 后续每个阶段都能与这组数据对比。

### 阶段 0 执行记录

执行日期：2026-07-08

环境状态：

| 项目 | 结果 |
| --- | --- |
| `docker compose up -d postgres redis minio` | Postgres、Redis、MinIO 均已运行 |
| `docker compose ps` | API healthy，Postgres healthy，Redis healthy，MinIO running |
| `pg_isready` | `/var/run/postgresql:5432 - accepting connections` |
| `cargo test` | 76 passed, 0 failed |
| `docker compose up -d --build api` | 成功；release build 用时约 8m29s，API 容器重建后 healthy |
| `/health` | `{"service":"git-ai-enterprise-server","status":"ok","version":"0.1.0"}` |
| `/ready` | `{"checks":{"database":"ok"},"status":"ready"}` |

基础压测：

| 端点 | 请求/并发 | RPS | p50 | p95 | p99 | 失败数 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `/health` | 100 / 20 | 466.06 | 40ms | 48ms | 50ms | 0 |
| `/ready` | 100 / 20 | 451.78 | 39ms | 61ms | 65ms | 0 |

容器资源快照：

| 容器 | CPU | 内存 |
| --- | ---: | ---: |
| `enterprise-server-api-1` | 0.00% | 12.13MiB / 7.655GiB |
| `enterprise-server-postgres-1` | 0.00% | 199.2MiB / 7.655GiB |
| `enterprise-server-redis-1` | 0.65% | 10.26MiB / 7.655GiB |
| `enterprise-server-minio-1` | 0.07% | 164.8MiB / 7.655GiB |

数据库数据量：

| 表 | 行数 |
| --- | ---: |
| `metrics_events` | 44 |
| `commit_stats` | 1 |
| `cas_objects` | 32 |
| `report_uploads` | 1 |
| `users` | 3 |

## 阶段 1: P0 低风险立即优化

### 1.1 健康检查和 readiness 跳过业务限流

目标：`/health` 和 `/ready` 不消耗默认 rate limit，避免负载均衡器、容器健康检查或监控探测被 429。

涉及文件：

- `enterprise-server/src/services/rate_limit.rs`

实现步骤：

- [x] 打开 `rate_limit_middleware`。
- [x] 在读取 path 后、执行限流检查前增加 bypass：

```rust
if path == "/health" || path == "/ready" {
    return Ok(next.run(request).await);
}
```

- [x] 保持 worker、admin、dashboard API 的限流逻辑不变。
- [x] 增加单元测试或 middleware 测试，覆盖 `/health` 和 `/ready` 连续请求不被限制。
- [x] 增加测试确认普通路径仍会被限制。

测试命令：

```bash
cd enterprise-server
cargo test rate_limit
cargo test
```

手动压测：

```bash
ab -n 1000 -c 50 http://127.0.0.1:8080/health
ab -n 1000 -c 50 http://127.0.0.1:8080/ready
```

验收标准：

- [x] `/health` 1000 请求无 429。
- [x] `/ready` 1000 请求无 429。
- [x] 受保护业务路径仍会被 rate limit。

提交建议：

```bash
git add enterprise-server/src/services/rate_limit.rs
git commit -m "Bypass rate limits for health checks"
```

### 1.2 Redis rate limiter 复用连接管理器

目标：避免每个请求都从 `redis::Client` 获取连接，降低高 QPS 下的 Redis 连接获取开销。

涉及文件：

- `enterprise-server/src/services/rate_limit.rs`
- `enterprise-server/src/main.rs`
- 相关测试文件内的 `RateLimiter::with_redis` 调用点

实现步骤：

- [x] 把 `RateLimiter` 的 Redis 字段从 `Option<redis::Client>` 改成 `Option<redis::aio::ConnectionManager>`。
- [x] 将 `with_redis` 改成 async 构造函数：

```rust
pub async fn with_redis(redis: redis::Client) -> Self
```

- [x] 在构造函数中调用：

```rust
let manager = tokio::time::timeout(Duration::from_secs(1), redis.get_connection_manager()).await;
```

- [x] 在 `check_redis` 中 clone manager 并执行 Lua 脚本。
- [x] 更新 `main.rs`：

```rust
let rate_limiter = services::rate_limit::RateLimiter::with_redis(redis_client.clone()).await;
```

- [x] 更新测试中的构造调用。
- [x] 保留 Redis 失败时 fallback 到内存计数的当前语义。

测试命令：

```bash
cd enterprise-server
cargo test rate_limit
cargo test
```

手动验证：

- [x] Redis 正常时，两个 `RateLimiter` 实例共享计数。
- [x] Redis 不可用时，仍 fallback 到内存计数并打印 warn。

验收标准：

- [x] `services::rate_limit::tests::redis_limiter_shares_counts_across_instances` 通过。
- [x] `services::rate_limit::tests::redis_failure_falls_back_to_in_memory_limit` 通过。
- [x] `/health`、`/ready` 基础压测延迟不变差。

提交建议：

```bash
git add enterprise-server/src/services/rate_limit.rs enterprise-server/src/main.rs
git commit -m "Reuse Redis connection manager for rate limits"
```

### 1.3 数据库连接池参数配置化

目标：允许不同部署环境通过环境变量调整 DB pool，而不是固定 20 个连接。

涉及文件：

- `enterprise-server/src/config.rs`
- `enterprise-server/src/main.rs`
- `enterprise-server/.env.example`
- `enterprise-server/deploy/.env.example`
- 如有需要，同步更新 `docs/enterprise-server-deployment.md`

实现步骤：

- [x] 在 `AppConfig` 增加：

```rust
pub database_max_connections: u32,
pub database_min_connections: u32,
pub database_acquire_timeout_seconds: u64,
```

- [x] 在 `EnvConfig` 增加对应可选字段：

```rust
pub database_max_connections: Option<u32>,
pub database_min_connections: Option<u32>,
pub database_acquire_timeout_seconds: Option<u64>,
```

- [x] 设置默认值：

```rust
database_max_connections: env.database_max_connections.unwrap_or(20),
database_min_connections: env.database_min_connections.unwrap_or(1),
database_acquire_timeout_seconds: env.database_acquire_timeout_seconds.unwrap_or(5),
```

- [x] 在 `main.rs` 使用配置值：

```rust
let db_pool = sqlx::postgres::PgPoolOptions::new()
    .max_connections(config.database_max_connections)
    .min_connections(config.database_min_connections)
    .acquire_timeout(std::time::Duration::from_secs(config.database_acquire_timeout_seconds))
    .connect(&config.database_url)
    .await?;
```

- [x] 更新 `.env.example`：

```env
DATABASE_MAX_CONNECTIONS=20
DATABASE_MIN_CONNECTIONS=1
DATABASE_ACQUIRE_TIMEOUT_SECONDS=5
```

测试命令：

```bash
cd enterprise-server
cargo test
DATABASE_MAX_CONNECTIONS=3 DATABASE_MIN_CONNECTIONS=1 DATABASE_ACQUIRE_TIMEOUT_SECONDS=5 cargo test config
```

手动验证：

```bash
DATABASE_MAX_CONNECTIONS=30 cargo run
```

验收标准：

- [x] 不设置环境变量时默认行为与当前一致。
- [x] 设置环境变量后服务能正常启动。
- [x] 配置文档说明每实例连接数如何按 Postgres 容量计算。

提交建议：

```bash
git add enterprise-server/src/config.rs enterprise-server/src/main.rs enterprise-server/.env.example enterprise-server/deploy/.env.example docs/enterprise-server-deployment.md
git commit -m "Make enterprise database pool configurable"
```

### 1.4 增加上传 batch 大小限制

目标：避免单请求携带过大的 metrics/CAS/report payload，占用过多内存、DB 连接和对象存储带宽。

涉及文件：

- `enterprise-server/src/handlers/metrics.rs`
- `enterprise-server/src/handlers/cas.rs`
- 可选：`enterprise-server/src/config.rs`

实现步骤：

- [x] 为 metrics batch 添加限制，建议初始为 500：

```rust
if batch.events.len() > 500 {
    return Err(AppError::BadRequest("Maximum 500 events per batch".into()));
}
```

- [x] 为 CAS batch 添加限制，建议初始为 100：

```rust
if req.objects.len() > 100 {
    return Err(AppError::BadRequest("Maximum 100 CAS objects per batch".into()));
}
```

- [x] 本阶段暂不配置化 batch 上限；后续如需动态调整，再增加：

```env
METRICS_MAX_BATCH_EVENTS=500
CAS_MAX_BATCH_OBJECTS=100
```

- [x] 增加超限测试。
- [x] 确认合法 batch 行为不变。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test cas
cargo test
```

验收标准：

- [x] 超过 metrics 限制返回 400。
- [x] 超过 CAS 限制返回 400。
- [x] 现有正常上传测试不受影响。

提交建议：

```bash
git add enterprise-server/src/handlers/metrics.rs enterprise-server/src/handlers/cas.rs
git commit -m "Limit enterprise upload batch sizes"
```

### 阶段 1 执行记录

执行日期：2026-07-08

实现结果：

| 任务 | 结果 |
| --- | --- |
| 1.1 健康检查和 readiness 跳过业务限流 | 已实现；`/health`、`/ready` 在 middleware 中直接 bypass，业务路径仍走原限流分层 |
| 1.2 Redis rate limiter 复用连接管理器 | 已实现；`RateLimiter` 持有 `redis::aio::ConnectionManager`，请求内 clone manager 执行 Lua；初始化失败或 1 秒超时会降级到内存计数 |
| 1.3 数据库连接池参数配置化 | 已实现；新增 `DATABASE_MAX_CONNECTIONS`、`DATABASE_MIN_CONNECTIONS`、`DATABASE_ACQUIRE_TIMEOUT_SECONDS`，默认值保持 `20/1/5` |
| 1.4 上传 batch 大小限制 | 已实现；metrics 单 batch 最大 500 events，CAS 单 batch 最大 100 objects |

验证命令：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过；仅有既有 unused/dead_code warning |
| `cargo test rate_limit` | 4 passed, 0 failed |
| `cargo test metrics` | 8 passed, 0 failed |
| `cargo test cas` | 8 passed, 0 failed |
| `cargo test` | 79 passed, 0 failed |
| `DATABASE_MAX_CONNECTIONS=3 DATABASE_MIN_CONNECTIONS=1 DATABASE_ACQUIRE_TIMEOUT_SECONDS=5 cargo test config` | 通过；0 tests matched, 编译和测试 harness 正常 |
| `docker compose up -d --build api` | 成功；release build 用时约 8m16s，API 容器重建后 healthy |
| `curl -sS http://127.0.0.1:8080/health` | `{"service":"git-ai-enterprise-server","status":"ok","version":"0.1.0"}` |
| `curl -sS http://127.0.0.1:8080/ready` | `{"checks":{"database":"ok"},"status":"ready"}` |

健康接口压测：

| 端点 | 请求/并发 | RPS | p50 | p95 | p99 | 失败数 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `/health` | 1000 / 50 | 872.51 | 53ms | 77ms | 81ms | 0 |
| `/ready` | 1000 / 50 | 614.75 | 81ms | 94ms | 97ms | 0 |

## 阶段 2: P1 写入吞吐优化

### 2.1 metrics 上传复用 auth org scope

目标：去掉 metrics batch 中每条事件一次的 org scope 查询。

涉及文件：

- `enterprise-server/src/handlers/metrics.rs`
- `enterprise-server/src/services/metrics.rs`

实现步骤：

- [ ] 修改 `process_metrics_batch` 签名，增加 `org_id: Option<Uuid>`。
- [ ] 在 `upload_metrics` 中传入 `auth.0.org_id`。
- [ ] 修改 `store_event` 签名，接收 `org_id`。
- [ ] 删除 `store_event` 内部的 `preferred_org_scope(pool, uid)` 调用。
- [ ] 确认落库时使用传入的 org_id。
- [ ] 增加测试覆盖：
  - 有 org_id 时 metrics_events.org_id 正确。
  - 无 org_id 时 org_id 为 null。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test
```

验收标准：

- [ ] metrics 上传不再每事件查询 org scope。
- [ ] 现有 metrics 聚合字段测试通过。
- [ ] 100 条 event batch 的 DB 往返明显减少。

提交建议：

```bash
git add enterprise-server/src/handlers/metrics.rs enterprise-server/src/services/metrics.rs
git commit -m "Reuse auth org scope for metrics uploads"
```

### 2.2 metrics 批量 insert

目标：把 metrics batch 的 N 次 insert 降为按 chunk 的批量 insert。

涉及文件：

- `enterprise-server/src/services/metrics.rs`

实现步骤：

- [ ] 定义内部结构 `PreparedMetricRow`，存放 decode 和聚合后的字段。
- [ ] 第一阶段遍历 events，只做 decode 和字段聚合，收集：
  - `Vec<PreparedMetricRow>`
  - `Vec<MetricUploadError>`
- [ ] 使用 `sqlx::QueryBuilder` 实现 `insert_metrics_chunk`。
- [ ] 初始 chunk size 设置为 500。
- [ ] 对成功 decode 的 rows 按 chunk insert。
- [ ] 如果 chunk insert 失败，首版可以将 chunk 内所有事件记为 storage error。
- [ ] 可选 fallback：chunk 失败时逐条 insert，用于定位坏行。
- [ ] 保持接口 partial success 语义。

测试步骤：

- [ ] 全部成功：batch N 条，落库 N 条。
- [ ] 部分 decode 失败：成功行落库，失败行出现在 errors。
- [ ] 模拟 DB insert 失败：返回 storage errors。
- [ ] 原有 aggregate rollup 单元测试继续通过。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test
```

压测建议：

```text
batch sizes: 1, 10, 100, 500
concurrency: 1, 5, 20
```

记录：

- [ ] rows/s
- [ ] p50/p95/p99
- [ ] DB CPU
- [ ] DB active connection 数

验收标准：

- [ ] 100/500 条 batch 的总耗时明显低于逐条 insert。
- [ ] partial success 行为不回退。
- [ ] DB 错误不会被静默吞掉。

提交建议：

```bash
git add enterprise-server/src/services/metrics.rs
git commit -m "Bulk insert enterprise metrics events"
```

### 2.3 client status last_seen 更新降频

目标：降低高频 metrics 上传导致的 `developer_client_status` upsert/update 频率。

涉及文件：

- `enterprise-server/src/services/client_status.rs`
- 可选：相关 dashboard status 测试

实现步骤：

- [ ] 在 `touch_last_seen` 的 upsert 中加入 60 秒节流语义。
- [ ] 当现有 `last_seen_at` 小于 `now() - interval '60 seconds'` 时才更新。
- [ ] 如果当前状态不是 `logged_in`，仍应更新为 `logged_in`。
- [ ] 不要影响显式 login/logout 的 `record_status`。
- [ ] 增加测试：
  - 连续两次 `touch_last_seen`，第二次不更新 `last_seen_at`。
  - 超过节流窗口后会更新。
  - logout 后 metrics touch 会恢复 logged_in。

测试命令：

```bash
cd enterprise-server
cargo test client_status
cargo test
```

验收标准：

- [ ] 高频 metrics 上传不再每次更新 status 行。
- [ ] dashboard 当前用户 CLI 状态显示仍正确。

提交建议：

```bash
git add enterprise-server/src/services/client_status.rs
git commit -m "Throttle client last seen updates"
```

### 2.4 CAS batch 有界并发

目标：减少 CAS 单请求内多个对象串行处理导致的延迟，同时避免无限并发压垮 S3/DB。

涉及文件：

- `enterprise-server/src/handlers/cas.rs`
- `enterprise-server/Cargo.toml`，如果需要引入 `futures`
- 可选：`enterprise-server/src/config.rs`

实现步骤：

- [ ] 先在 handler 开头校验 batch 大小。
- [ ] 预校验所有对象的 hash 格式和 hash/content 匹配；如果有 bad request，直接返回 400。
- [ ] 将真正写入流程改成有界并发，初始并发度 8。
- [ ] 如果引入配置，增加：

```env
CAS_UPLOAD_CONCURRENCY=8
```

- [ ] 单对象内部流程保持不变：
  - secrets scan
  - S3 put
  - DB transaction
  - ownership upsert
- [ ] 并发处理结果聚合成原有 response 格式。
- [ ] 保持部分对象失败时返回 partial result 的语义。

测试步骤：

- [ ] 同 hash 同内容并发上传仍幂等。
- [ ] 同 hash 不同内容仍返回 400 或只允许正确内容成功。
- [ ] S3 put 失败时 DB 不留下记录。
- [ ] batch 中一部分对象失败时 success/failure count 正确。

测试命令：

```bash
cd enterprise-server
cargo test cas
cargo test
```

压测建议：

```text
CAS objects per batch: 1, 10, 50, 100
concurrency: 1, 5, 20
CAS_UPLOAD_CONCURRENCY: 1, 4, 8
```

验收标准：

- [ ] 并发度为 1 时行为等同当前串行逻辑。
- [ ] 并发度为 4/8 时 batch latency 下降。
- [ ] API 内存、MinIO/S3 错误率没有明显上升。

提交建议：

```bash
git add enterprise-server/src/handlers/cas.rs enterprise-server/Cargo.toml enterprise-server/Cargo.lock
git commit -m "Process CAS uploads with bounded concurrency"
```

### 2.5 report stats 批量 upsert

目标：减少大型 report 上传时 transaction 内逐条 SQL 的耗时。

涉及文件：

- `enterprise-server/src/handlers/report.rs`

实现步骤：

- [ ] 保留整个 `upload_report` 的 transaction。
- [ ] 将 `commit_stats` 写入改成按 chunk 批量 upsert。
- [ ] 将 `tool_model_stats` 写入改成批量 upsert。
- [ ] 保持 `ON CONFLICT ... DO UPDATE SET` 更新全部字段。
- [ ] 如果 inserted/updated 计数难以精确保留，可以先增加 `processed_commits`，但不要误导性地返回错误计数。
- [ ] 更新测试断言。

测试步骤：

- [ ] 事务回滚测试继续通过。
- [ ] 重复上传同 commit 后数据更新为第二次结果。
- [ ] 大 report 上传不会超时。

测试命令：

```bash
cd enterprise-server
cargo test report
cargo test
```

压测建议：

```text
commits per report: 10, 100, 1000, 5000
tool_model rows: 1, 10, 100
```

验收标准：

- [ ] 1000 commit report 上传耗时明显下降。
- [ ] transaction 语义不变。

提交建议：

```bash
git add enterprise-server/src/handlers/report.rs
git commit -m "Bulk upsert enterprise report stats"
```

## 阶段 3: P2 dashboard 查询优化

### 3.1 时间过滤改成 epoch 秒比较

目标：避免 dashboard 查询对 `metrics_events.timestamp` 调用 `to_timestamp()`，提高索引利用率。

涉及文件：

- `enterprise-server/src/handlers/dashboard.rs`
- 可能涉及 `enterprise-server/src/handlers/lifecycle.rs`

实现步骤：

- [ ] 搜索所有 `to_timestamp(timestamp)`。
- [ ] 在 Rust 查询处理处将 `since/until` 转成 epoch 秒：

```rust
let since_ts = query.since.map(|dt| dt.timestamp());
let until_ts = query.until.map(|dt| dt.timestamp());
```

- [ ] SQL 改成：

```sql
AND ($3::bigint IS NULL OR timestamp >= $3)
AND ($4::bigint IS NULL OR timestamp <= $4)
```

- [ ] 保留 `DATE_TRUNC` 分组场景中必要的 timestamp 转换，但 WHERE 过滤优先用 bigint。
- [ ] 增加测试确认 since/until 过滤结果不变。

测试命令：

```bash
cd enterprise-server
cargo test dashboard
cargo test
```

手动验证：

```sql
EXPLAIN (ANALYZE, BUFFERS)
SELECT COUNT(*)
FROM metrics_events
WHERE event_type = 1
  AND timestamp >= 1780000000;
```

验收标准：

- [ ] 查询结果与改前一致。
- [ ] 大数据量下 WHERE 条件能使用 timestamp 相关索引。

提交建议：

```bash
git add enterprise-server/src/handlers/dashboard.rs
git commit -m "Use epoch filters for dashboard metrics queries"
```

### 3.2 增加 metrics/dashboard 组合索引

目标：为 dashboard 高频组合查询增加索引。

涉及文件：

- `enterprise-server/migrations/014_metrics_query_indexes.sql`
- `enterprise-server/deploy/migrations/014_metrics_query_indexes.sql`
- `enterprise-server/src/db/migrations.rs`

实现步骤：

- [ ] 新增 migration 文件：

```sql
CREATE INDEX IF NOT EXISTS idx_metrics_event_org_time
    ON metrics_events(event_type, org_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_event_user_time
    ON metrics_events(event_type, user_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_event_commit
    ON metrics_events(event_type, commit_sha);

CREATE INDEX IF NOT EXISTS idx_commit_stats_project_author_time
    ON commit_stats(project_id, author_time);

CREATE INDEX IF NOT EXISTS idx_projects_org_user
    ON projects(org_id, user_id);
```

- [ ] 同步复制到 deploy migrations。
- [ ] 在 `src/db/migrations.rs` 的 `MIGRATIONS` 中注册 `014_metrics_query_indexes`。
- [ ] 如果生产表已经很大，评估是否需要单独使用 `CREATE INDEX CONCURRENTLY` 的运维迁移，而不是应用启动时创建。

测试命令：

```bash
cd enterprise-server
cargo test db::migrations
cargo test
```

手动验证：

```bash
cargo run -- --migrate
```

验收标准：

- [ ] migration 可重复执行。
- [ ] 新索引存在。
- [ ] 写入压测没有明显恶化。
- [ ] dashboard 查询 `EXPLAIN` 开始使用组合索引。

提交建议：

```bash
git add enterprise-server/migrations/014_metrics_query_indexes.sql enterprise-server/deploy/migrations/014_metrics_query_indexes.sql enterprise-server/src/db/migrations.rs
git commit -m "Add dashboard metrics query indexes"
```

### 3.3 增加 author_time_at 结构化时间字段

目标：避免 `commit_stats.author_time` 作为 text 导致 report 聚合难以走时间索引。

涉及文件：

- `enterprise-server/migrations/015_commit_stats_author_time_at.sql`
- `enterprise-server/deploy/migrations/015_commit_stats_author_time_at.sql`
- `enterprise-server/src/db/migrations.rs`
- `enterprise-server/src/handlers/report.rs`
- `enterprise-server/src/handlers/dashboard.rs`

实现步骤：

- [ ] 新增字段：

```sql
ALTER TABLE commit_stats
ADD COLUMN IF NOT EXISTS author_time_at TIMESTAMPTZ;
```

- [ ] 回填可解析的历史数据：

```sql
UPDATE commit_stats
SET author_time_at = NULLIF(author_time, '')::timestamptz
WHERE author_time_at IS NULL
  AND author_time IS NOT NULL
  AND author_time != '';
```

- [ ] 增加索引：

```sql
CREATE INDEX IF NOT EXISTS idx_commit_stats_project_author_time_at
    ON commit_stats(project_id, author_time_at);
```

- [ ] report 上传时写入 `author_time_at`。
- [ ] dashboard report 聚合使用 `author_time_at` 过滤和分组。
- [ ] 对无法解析的 author_time 保持 null，不让上传失败，除非数据模型已明确要求 ISO 时间。

测试命令：

```bash
cd enterprise-server
cargo test report
cargo test dashboard
cargo test
```

验收标准：

- [ ] 新上传 report 的 `author_time_at` 正确。
- [ ] 历史数据可回填。
- [ ] dashboard report 时间过滤结果与原来一致。

提交建议：

```bash
git add enterprise-server/migrations/015_commit_stats_author_time_at.sql enterprise-server/deploy/migrations/015_commit_stats_author_time_at.sql enterprise-server/src/db/migrations.rs enterprise-server/src/handlers/report.rs enterprise-server/src/handlers/dashboard.rs
git commit -m "Store structured commit author times"
```

### 3.4 设计并落地 metrics daily rollup 表

目标：dashboard summary/trends/tool comparison 默认查 rollup，避免每次扫明细。

涉及文件：

- `enterprise-server/migrations/016_metrics_daily_rollups.sql`
- `enterprise-server/deploy/migrations/016_metrics_daily_rollups.sql`
- `enterprise-server/src/db/migrations.rs`
- `enterprise-server/src/services/metrics.rs`
- `enterprise-server/src/handlers/dashboard.rs`
- 可选：`enterprise-server/src/config.rs`

实现步骤：

- [ ] 新增 rollup 表：

```sql
CREATE TABLE IF NOT EXISTS metrics_daily_rollups (
    day DATE NOT NULL,
    org_id UUID,
    user_id UUID,
    repo_url TEXT NOT NULL DEFAULT '',
    tool_model TEXT NOT NULL DEFAULT '',
    commits BIGINT NOT NULL DEFAULT 0,
    total_lines BIGINT NOT NULL DEFAULT 0,
    ai_lines BIGINT NOT NULL DEFAULT 0,
    human_lines BIGINT NOT NULL DEFAULT 0,
    mixed_lines BIGINT NOT NULL DEFAULT 0,
    ai_accepted BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (day, org_id, user_id, repo_url, tool_model)
);
```

- [ ] 增加索引：

```sql
CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_org_day
    ON metrics_daily_rollups(org_id, day);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_user_day
    ON metrics_daily_rollups(user_id, day);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_tool_day
    ON metrics_daily_rollups(tool_model, day);
```

- [ ] metrics batch 成功写入明细后，在内存中按 day/org/user/repo/tool 聚合。
- [ ] 使用批量 upsert 更新 rollup。
- [ ] 增加配置开关：

```env
DASHBOARD_USE_ROLLUPS=false
METRICS_WRITE_ROLLUPS=true
```

- [ ] 首版先只写 rollup，不默认切 dashboard。
- [ ] 增加回填 SQL 或管理脚本。
- [ ] 增加一致性测试：rollup 聚合结果与明细查询结果一致。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test dashboard
cargo test
```

验收标准：

- [ ] metrics 写入后 rollup 行正确增加。
- [ ] 重复 batch 不会产生非预期重复，前提是当前 metrics 明细语义允许重复上传。
- [ ] 回填结果与明细聚合一致。
- [ ] rollup 写入对 metrics upload 延迟影响可接受。

提交建议：

```bash
git add enterprise-server/migrations/016_metrics_daily_rollups.sql enterprise-server/deploy/migrations/016_metrics_daily_rollups.sql enterprise-server/src/db/migrations.rs enterprise-server/src/services/metrics.rs enterprise-server/src/handlers/dashboard.rs enterprise-server/src/config.rs
git commit -m "Write daily metrics rollups"
```

### 3.5 dashboard summary/trends 切到 rollup 查询

目标：让 dashboard 常用聚合接口使用 rollup 表。

涉及文件：

- `enterprise-server/src/handlers/dashboard.rs`
- `enterprise-server/src/config.rs`

实现步骤：

- [ ] 在 `AppConfig` 增加 `dashboard_use_rollups: bool`。
- [ ] 默认先设为 `false`，生产验证后再切为 `true`。
- [ ] 为 `aggregate_summary` 增加 rollup 查询路径。
- [ ] 为 `aggregate_trends` 增加 rollup 查询路径。
- [ ] 为 `aggregate_tools` 或 `aggregate_agent_comparison` 增加 rollup 查询路径。
- [ ] 保留明细查询 fallback。
- [ ] 增加测试对比：
  - rollup enabled 的结果
  - 明细查询结果
  - 两者一致

测试命令：

```bash
cd enterprise-server
cargo test dashboard
cargo test
```

压测建议：

```text
数据量: 10万、100万、500万 metrics_events
接口: /api/v1/aggregate/summary, /api/v1/aggregate/trends, /api/v1/aggregate/tools
对比: DASHBOARD_USE_ROLLUPS=false/true
```

验收标准：

- [ ] rollup enabled 时 dashboard 常用接口 p95 明显下降。
- [ ] 查询结果与明细路径一致。
- [ ] 可通过配置快速回退到明细查询。

提交建议：

```bash
git add enterprise-server/src/handlers/dashboard.rs enterprise-server/src/config.rs enterprise-server/.env.example enterprise-server/deploy/.env.example
git commit -m "Read dashboard aggregates from rollups"
```

### 3.6 预计算 tool_model 明细表

目标：避免 dashboard 高频查询中反复展开 `raw_values` JSONB。

涉及文件：

- `enterprise-server/migrations/017_metrics_tool_model_events.sql`
- `enterprise-server/deploy/migrations/017_metrics_tool_model_events.sql`
- `enterprise-server/src/db/migrations.rs`
- `enterprise-server/src/services/metrics.rs`
- `enterprise-server/src/handlers/dashboard.rs`

实现步骤：

- [ ] 新增结构化表：

```sql
CREATE TABLE IF NOT EXISTS metrics_tool_model_events (
    metric_event_id BIGINT NOT NULL REFERENCES metrics_events(id) ON DELETE CASCADE,
    org_id UUID,
    user_id UUID,
    timestamp BIGINT NOT NULL,
    tool_model TEXT NOT NULL,
    ai_additions INTEGER NOT NULL DEFAULT 0,
    mixed_additions INTEGER NOT NULL DEFAULT 0,
    ai_accepted INTEGER NOT NULL DEFAULT 0,
    total_ai_additions INTEGER NOT NULL DEFAULT 0,
    total_ai_deletions INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (metric_event_id, tool_model)
);
```

- [ ] 增加索引：

```sql
CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_org_time
    ON metrics_tool_model_events(org_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_user_time
    ON metrics_tool_model_events(user_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_tool_time
    ON metrics_tool_model_events(tool_model, timestamp);
```

- [ ] metrics insert 后获取 inserted event id。
- [ ] 展开 tool_model_pairs/raw_values 到结构化 rows。
- [ ] 批量 insert 到 `metrics_tool_model_events`。
- [ ] dashboard tool comparison 改查结构化表或 rollup。
- [ ] 增加回填脚本。

注意：

- 如果 metrics bulk insert 不返回每行 id，需要使用 `RETURNING id` 并维护 row 顺序。
- 该任务复杂度高，建议放在 daily rollup 后执行。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test dashboard
cargo test
```

验收标准：

- [ ] tool_model 结构化结果与当前 JSONB lateral 查询一致。
- [ ] tool comparison 大数据量查询明显加快。

提交建议：

```bash
git add enterprise-server/migrations/017_metrics_tool_model_events.sql enterprise-server/deploy/migrations/017_metrics_tool_model_events.sql enterprise-server/src/db/migrations.rs enterprise-server/src/services/metrics.rs enterprise-server/src/handlers/dashboard.rs
git commit -m "Store structured metrics tool model rows"
```

## 阶段 4: 压测脚本和容量验证

### 4.1 增加 enterprise 压测脚本目录

目标：把手工压测变成可重复执行的脚本。

涉及文件：

- `scripts/benchmarks/enterprise/`
- 可选：`docs/enterprise-server-performance-task-plan.md`

实现步骤：

- [ ] 新建目录：

```bash
mkdir -p scripts/benchmarks/enterprise
```

- [ ] 增加 README，说明依赖、环境变量、运行方式。
- [ ] 增加 `seed_metrics` 脚本，支持生成大规模 metrics 数据。
- [ ] 增加 `bench_health_ready` 脚本，包装 `/health`、`/ready` 压测。
- [ ] 增加 `bench_metrics_upload` 脚本，构造认证请求和不同 batch size。
- [ ] 增加 `bench_dashboard` 脚本，测试 summary/trends/tools。

建议环境变量：

```env
ENTERPRISE_BASE_URL=http://127.0.0.1:8080
ENTERPRISE_API_KEY=...
BENCH_CONCURRENCY=20
BENCH_REQUESTS=1000
```

验收标准：

- [ ] 新机器上按 README 可以跑通基础压测。
- [ ] 脚本输出 RPS、p95、p99、错误率。
- [ ] 失败时退出码非 0。

提交建议：

```bash
git add scripts/benchmarks/enterprise
git commit -m "Add enterprise server benchmark scripts"
```

### 4.2 增加大数据量 dashboard 验证

目标：验证 dashboard 在 10万、100万、500万 metrics 数据下的行为。

步骤：

- [ ] 使用造数脚本生成 10 万 metrics。
- [ ] 运行 dashboard 压测。
- [ ] 记录 p95/p99 和慢查询。
- [ ] 生成 100 万 metrics。
- [ ] 重复压测。
- [ ] 如果机器允许，生成 500 万 metrics。
- [ ] 对比 rollup enabled/disabled。

验收标准：

- [ ] 30 天 summary p95 小于目标值。
- [ ] trends p95 小于目标值。
- [ ] tools comparison p95 小于目标值。
- [ ] 没有 DB acquire timeout。
- [ ] Postgres CPU 没有长期打满。

### 4.3 增加 Postgres 慢查询观测

目标：让性能问题可定位。

步骤：

- [ ] 在本地/测试环境启用 `pg_stat_statements`。
- [ ] 为生产部署文档增加启用说明。
- [ ] 增加慢查询查看 SQL：

```sql
SELECT
    query,
    calls,
    mean_exec_time,
    max_exec_time,
    rows
FROM pg_stat_statements
ORDER BY mean_exec_time DESC
LIMIT 20;
```

- [ ] 增加连接状态查看 SQL：

```sql
SELECT state, COUNT(*)
FROM pg_stat_activity
WHERE datname = 'gitai_enterprise'
GROUP BY state;
```

验收标准：

- [ ] 压测后能看到最慢查询。
- [ ] PR 或发布记录中包含慢查询截图/文本摘要。

## 阶段 5: 上线和回滚

### 5.1 分阶段上线

目标：减少性能优化上线风险。

步骤：

- [ ] P0 任务上线后观察 24 小时。
- [ ] P1 metrics 优化先在测试环境跑真实客户端上传。
- [ ] P1 CAS 并发先使用低并发度，例如 4。
- [ ] P2 rollup 先只写不读。
- [ ] rollup 回填完成并校验后，再打开 `DASHBOARD_USE_ROLLUPS=true`。
- [ ] 保留明细查询 fallback 至少一个版本。

验收标准：

- [ ] 错误率没有上升。
- [ ] p95/p99 没有回退。
- [ ] DB CPU 和连接数在预期范围。
- [ ] 可通过配置回退关键行为。

### 5.2 回滚方案

| 任务 | 快速回滚方式 |
| --- | --- |
| 健康检查跳过限流 | 回滚代码 |
| Redis connection manager | 回滚到 Client 获取连接 |
| DB pool 配置化 | 环境变量调回默认值 |
| batch size 限制 | 临时调大限制或回滚 |
| metrics bulk insert | 使用逐条 insert fallback |
| client status 节流 | 调小/关闭节流窗口 |
| CAS 有界并发 | 并发度设为 1 |
| report bulk upsert | 回滚到逐条 SQL |
| epoch 时间过滤 | 回滚 SQL 过滤方式 |
| 新索引 | 通常保留，必要时单独 drop |
| rollup 写入 | 关闭 `METRICS_WRITE_ROLLUPS` |
| dashboard rollup 读取 | 关闭 `DASHBOARD_USE_ROLLUPS` |

## 总体验收命令

完成任意阶段后至少运行：

```bash
cd enterprise-server
cargo test
```

完成涉及根项目的改动后运行：

```bash
task build
task test
```

完成格式化和 lint：

```bash
task format
task lint
```

如果只改 `enterprise-server/` 且根项目测试耗时过长，PR 里必须明确说明实际运行的命令和未运行的命令。

## 推荐执行顺序

优先顺序：

1. 1.1 健康检查和 readiness 跳过业务限流。
2. 1.2 Redis rate limiter 复用连接管理器。
3. 1.3 数据库连接池参数配置化。
4. 2.1 metrics 上传复用 auth org scope。
5. 2.2 metrics 批量 insert。
6. 2.4 CAS batch 有界并发。
7. 3.1 时间过滤改成 epoch 秒比较。
8. 3.2 增加 metrics/dashboard 组合索引。
9. 3.4 增加 daily rollup 表。
10. 3.5 dashboard summary/trends 切到 rollup 查询。

不建议一开始就做 rollup 或分区。先把低风险链路和写入吞吐优化完成，再用压测数据决定 P2/P3 的投入。
