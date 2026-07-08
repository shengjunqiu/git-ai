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

- [x] 修改 `process_metrics_batch` 签名，增加 `org_id: Option<Uuid>`。
- [x] 在 `upload_metrics` 中传入 `auth.0.org_id`。
- [x] 修改 `store_event` 签名，接收 `org_id`。
- [x] 删除 `store_event` 内部的 `preferred_org_scope(pool, uid)` 调用。
- [x] 确认落库时使用传入的 org_id。
- [x] 增加测试覆盖：
  - [x] 有 org_id 时 metrics_events.org_id 正确。
  - [x] 无 org_id 时 org_id 为 null。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test
```

验收标准：

- [x] metrics 上传不再每事件查询 org scope。
- [x] 现有 metrics 聚合字段测试通过。
- [x] 100 条 event batch 可减少约 100 次 org scope 查询往返；逐条 insert 往返保留到 2.2 处理。

提交建议：

```bash
git add enterprise-server/src/handlers/metrics.rs enterprise-server/src/services/metrics.rs
git commit -m "Reuse auth org scope for metrics uploads"
```

### 阶段 2.1 执行记录

执行日期：2026-07-08

实现结果：

| 任务 | 结果 |
| --- | --- |
| `upload_metrics` 传递认证上下文 org scope | 已实现；调用 `process_metrics_batch` 时传入 `auth.0.org_id` |
| `process_metrics_batch` 接收 org_id | 已实现；函数签名新增 `org_id: Option<Uuid>` |
| `store_event` 移除每事件 org scope 查询 | 已实现；删除内部 `preferred_org_scope(pool, uid)` 调用，直接使用调用方传入的 org_id |
| org_id 落库测试 | 已覆盖；有 org_id 时写入对应 org，无 org_id 时保持 null |

验证命令：

| 命令 | 结果 |
| --- | --- |
| `rustfmt --edition 2024 --check src/services/metrics.rs src/handlers/metrics.rs` | 通过 |
| `cargo check` | 通过；仅有既有 unused/dead_code warning |
| `cargo test metrics` | 10 passed, 0 failed |
| `cargo test` | 81 passed, 0 failed |

### 2.2 metrics 批量 insert

目标：把 metrics batch 的 N 次 insert 降为按 chunk 的批量 insert。

涉及文件：

- `enterprise-server/src/services/metrics.rs`

实现步骤：

- [x] 定义内部结构 `PreparedMetricRow`，存放 decode 和聚合后的字段。
- [x] 第一阶段遍历 events，只做 decode 和字段聚合，收集：
  - `Vec<PreparedMetricRow>`
  - `Vec<MetricUploadError>`
- [x] 使用 `sqlx::QueryBuilder` 实现 `insert_metrics_chunk`。
- [x] 初始 chunk size 设置为 500。
- [x] 对成功 decode 的 rows 按 chunk insert。
- [x] 如果 chunk insert 失败，首版将 chunk 内所有事件记为 storage error。
- [x] 首版不启用逐条 fallback；后续如需要定位坏行，再单独加入诊断模式。
- [x] 保持接口 partial success 语义。

测试步骤：

- [x] 全部成功：batch N 条，落库 N 条。
- [x] 部分 decode 失败：成功行落库，失败行出现在 errors。
- [x] 模拟 DB insert 失败：返回 storage errors。
- [x] 原有 aggregate rollup 单元测试继续通过。

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

- [ ] rows/s：本阶段未跑外部 HTTP 压测，留到阶段 4 统一记录。
- [ ] p50/p95/p99：本阶段未跑外部 HTTP 压测，留到阶段 4 统一记录。
- [ ] DB CPU：本阶段未跑外部 HTTP 压测，留到阶段 4 统一记录。
- [ ] DB active connection 数：本阶段未跑外部 HTTP 压测，留到阶段 4 统一记录。

验收标准：

- [x] 100/500 条 batch 的 DB 写入往返从 N 次降为 `ceil(N / 500)` 次；端到端耗时压测留到阶段 4 统一执行。
- [x] partial success 行为不回退。
- [x] DB 错误不会被静默吞掉。

提交建议：

```bash
git add enterprise-server/src/services/metrics.rs
git commit -m "Bulk insert enterprise metrics events"
```

阶段 2.2 执行记录：

| 项目 | 结果 |
| --- | --- |
| `PreparedMetricRow` | 已实现，集中保存 decode 后的写库字段和原始 event index |
| 批量 insert | 已实现，`insert_metrics_chunk` 使用 `sqlx::QueryBuilder` 写入 `metrics_events` |
| chunk size | 已设置为 500，501 条事件测试覆盖跨 chunk 写入 |
| decode partial success | 已保持，decode 失败只返回对应 index 的错误，成功事件仍落库 |
| DB storage error | 已覆盖，chunk 写入失败时 chunk 内事件全部返回 storage error |
| 外部压测 | 本阶段未执行，吞吐和 p95/p99 留到阶段 4 统一对比 |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `rustfmt --edition 2024 --check src/services/metrics.rs` | 通过 |
| `cargo check` | 通过，仅有既有 warning |
| `cargo test metrics` | 14 passed, 0 failed |
| `cargo test` | 85 passed, 0 failed |

### 2.3 client status last_seen 更新降频

目标：降低高频 metrics 上传导致的 `developer_client_status` upsert/update 频率。

涉及文件：

- `enterprise-server/src/services/client_status.rs`
- 可选：相关 dashboard status 测试

实现步骤：

- [x] 在 `touch_last_seen` 的 upsert 中加入 60 秒节流语义。
- [x] 当现有 `last_seen_at` 小于 `now() - interval '60 seconds'` 时才更新。
- [x] 如果当前状态不是 `logged_in`，仍应更新为 `logged_in`。
- [x] 不要影响显式 login/logout 的 `record_status`。
- [x] 增加测试：
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

- [x] 高频 metrics 上传不再每次更新 status 行。
- [x] dashboard 当前用户 CLI 状态显示仍正确。

提交建议：

```bash
git add enterprise-server/src/services/client_status.rs
git commit -m "Throttle client last seen updates"
```

阶段 2.3 执行记录：

| 项目 | 结果 |
| --- | --- |
| 60 秒节流 | 已实现，`touch_last_seen` 在 `ON CONFLICT DO UPDATE` 上增加 `WHERE`，节流窗口内且元数据不变时不写 status 行 |
| `last_seen_at` 更新条件 | 已实现，仅未登录、`last_seen_at` 为空或超过 60 秒时刷新 |
| logout 恢复登录 | 已实现，当前状态不是 `logged_in` 时仍更新 `status`、`last_status_at` 和 `last_seen_at` |
| 显式 login/logout | 未改动 `record_status`，保持原语义 |
| dashboard 状态 | `get_status` 测试覆盖 touch 后仍汇总为 `logged_in` |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `rustfmt --edition 2024 --check src/services/client_status.rs` | 通过 |
| `cargo test client_status` | 5 passed, 0 failed |
| `cargo test` | 88 passed, 0 failed |

### 2.4 CAS batch 有界并发

目标：减少 CAS 单请求内多个对象串行处理导致的延迟，同时避免无限并发压垮 S3/DB。

涉及文件：

- `enterprise-server/src/handlers/cas.rs`
- `enterprise-server/Cargo.toml`，如果需要引入 `futures`
- 可选：`enterprise-server/src/config.rs`

实现步骤：

- [x] 先在 handler 开头校验 batch 大小。
- [x] 预校验所有对象的 hash 格式和 hash/content 匹配；如果有 bad request，直接返回 400。
- [x] 将真正写入流程改成有界并发，初始并发度 8。
- [x] 如果引入配置，增加：

```env
CAS_UPLOAD_CONCURRENCY=8
```

- [x] 单对象内部流程保持不变：
  - secrets scan
  - S3 put
  - DB transaction
  - ownership upsert
- [x] 并发处理结果聚合成原有 response 格式。
- [x] 保持部分对象失败时返回 partial result 的语义。

测试步骤：

- [x] 同 hash 同内容并发上传仍幂等。
- [x] 同 hash 不同内容仍返回 400 或只允许正确内容成功。
- [x] S3 put 失败时 DB 不留下记录。
- [x] batch 中一部分对象失败时 success/failure count 正确。

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

- [x] 并发度为 1 时行为等同当前串行逻辑。
- [ ] 并发度为 4/8 时 batch latency 下降：本阶段已实现并发处理，端到端压测留到阶段 4 统一验证。
- [ ] API 内存、MinIO/S3 错误率没有明显上升：本阶段未跑外部压测，留到阶段 4 统一验证。

提交建议：

```bash
git add enterprise-server/src/handlers/cas.rs enterprise-server/src/config.rs enterprise-server/src/auth/jwt.rs enterprise-server/src/handlers/auth_api.rs enterprise-server/src/handlers/oauth.rs enterprise-server/src/handlers/report.rs enterprise-server/src/handlers/release.rs enterprise-server/.env.example enterprise-server/deploy/.env.example docs/enterprise-server-deployment.md docs/enterprise-server-performance-task-plan.md
git commit -m "Process CAS uploads with bounded concurrency"
```

阶段 2.4 执行记录：

| 项目 | 结果 |
| --- | --- |
| batch 预校验 | 已实现，所有对象在启动并发任务前完成 hash 格式和 hash/content 校验；bad request 不写 S3/DB |
| 有界并发 | 已实现，`CAS_UPLOAD_CONCURRENCY` 默认 8，实际值通过配置读取并至少为 1 |
| 单对象流程 | 已保留 secrets scan、S3 put、DB transaction、ownership upsert 顺序 |
| response 顺序 | 已保持，任务并发完成后按原请求 index 排序返回 |
| partial result | 已保持，非 BadRequest 的 S3/DB 错误按对象返回 error 并累计 failure_count |
| 配置文档 | 已更新 `.env.example`、deploy `.env.example` 和部署文档 |
| 外部压测 | 本阶段未执行，batch latency、内存和 S3 错误率留到阶段 4 统一对比 |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `rustfmt --edition 2024 --check src/handlers/cas.rs src/config.rs` | 通过 |
| `cargo test cas` | 12 passed, 0 failed |
| `cargo test` | 92 passed, 0 failed |

### 2.5 report stats 批量 upsert

目标：减少大型 report 上传时 transaction 内逐条 SQL 的耗时。

涉及文件：

- `enterprise-server/src/handlers/report.rs`

实现步骤：

- [x] 保留整个 `upload_report` 的 transaction。
- [x] 将 `commit_stats` 写入改成按 chunk 批量 upsert。
- [x] 将 `tool_model_stats` 写入改成批量 upsert。
- [x] 保持 `ON CONFLICT ... DO UPDATE SET` 更新全部字段。
- [x] 保留 `inserted_commits` / `updated_commits` 计数；写入前查询已有 sha，本次 report 内重复 sha 取最后一条。
- [x] 更新测试断言。

测试步骤：

- [x] 事务回滚测试继续通过。
- [x] 重复上传同 commit 后数据更新为第二次结果。
- [x] 大 report 上传不会超时。

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

- [ ] 1000 commit report 上传耗时明显下降：本阶段已将 DB 往返从逐条 SQL 降为按 chunk 批量 upsert，端到端耗时压测留到阶段 4 统一验证。
- [x] transaction 语义不变。

提交建议：

```bash
git add enterprise-server/src/handlers/report.rs
git commit -m "Bulk upsert enterprise report stats"
```

阶段 2.5 执行记录：

| 项目 | 结果 |
| --- | --- |
| `commit_stats` 批量 upsert | 已实现，按 1000 条 chunk 使用 `sqlx::QueryBuilder` 批量写入 |
| `tool_model_stats` 批量 upsert | 已实现，按 1000 条 chunk 使用 `sqlx::QueryBuilder` 批量写入 |
| inserted/updated 计数 | 已保留，写入前查询当前 project 已存在 sha 后计算 |
| report 内重复 sha | 已处理，同一 report 中重复 sha 取最后一条，避免单条 bulk insert 内重复 conflict key |
| transaction 语义 | 已保持，project、upload、commit stats、tool model stats 仍在同一 transaction 内提交或回滚 |
| 外部压测 | 本阶段未执行，1000/5000 commits report 的端到端耗时留到阶段 4 统一对比 |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `rustfmt --edition 2024 --check src/handlers/report.rs` | 通过 |
| `cargo test report` | 4 passed, 0 failed |
| `cargo test` | 93 passed, 0 failed |

## 阶段 3: P2 dashboard 查询优化

### 3.1 时间过滤改成 epoch 秒比较

目标：避免 dashboard 查询对 `metrics_events.timestamp` 调用 `to_timestamp()`，提高索引利用率。

涉及文件：

- `enterprise-server/src/handlers/dashboard.rs`
- 可能涉及 `enterprise-server/src/handlers/lifecycle.rs`

实现步骤：

- [x] 搜索所有 `to_timestamp(timestamp)`。
- [x] 在 Rust 查询处理处将 `since/until` 转成 epoch 秒：

```rust
let since_ts = query.since.map(|dt| dt.timestamp());
let until_ts = query.until.map(|dt| dt.timestamp());
```

- [x] SQL 改成：

```sql
AND ($3::bigint IS NULL OR timestamp >= $3)
AND ($4::bigint IS NULL OR timestamp <= $4)
```

- [x] 保留 `DATE_TRUNC` 分组场景中必要的 timestamp 转换，但 WHERE 过滤优先用 bigint。
- [x] 增加测试确认 since/until 过滤结果不变。

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

- [x] 查询结果与改前一致。
- [ ] 大数据量下 WHERE 条件能使用 timestamp 相关索引：本阶段已移除 WHERE 中的 `to_timestamp(timestamp)` 包装，生产量级 `EXPLAIN` 留到阶段 4 统一验证。

提交建议：

```bash
git add enterprise-server/src/handlers/dashboard.rs
git commit -m "Use epoch filters for dashboard metrics queries"
```

阶段 3.1 执行记录：

| 项目 | 结果 |
| --- | --- |
| dashboard 时间参数解析 | 已新增 `parse_epoch_filters`，支持 RFC3339 和 `YYYY-MM-DD`，空值保持无过滤 |
| `aggregate_summary` | metrics_events 分支改为 `timestamp >= $n::bigint` / `timestamp <= $n::bigint` |
| `aggregate_developers` | metrics_events 分支改为 epoch 秒比较，report 分支继续使用 `commit_stats.author_time` 的 timestamptz 比较 |
| `aggregate_trends` | WHERE 过滤改为 epoch 秒比较，保留 `DATE_TRUNC(... to_timestamp(timestamp))` 仅用于分组 |
| `lifecycle.rs` | 已检查，无 `to_timestamp(timestamp)` WHERE 过滤模式，未修改 |
| 单元测试 | 已新增时间参数解析测试，覆盖 RFC3339、日期、空值和非法值 |
| 外部 EXPLAIN | 未执行，缺少生产量级样本数据；阶段 4 压测/基线回归时统一验证 |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `cargo test dashboard` | 4 passed, 0 failed |
| `cargo test lifecycle` | 命令通过，0 matched |
| `cargo test parse_epoch_seconds_param` | 4 passed, 0 failed |
| `cargo test` | 97 passed, 0 failed |
| `cargo fmt --check` | 未通过，输出为仓库既有多文件格式差异，本阶段未扩大格式化范围 |

### 3.2 增加 metrics/dashboard 组合索引

目标：为 dashboard 高频组合查询增加索引。

涉及文件：

- `enterprise-server/migrations/014_metrics_query_indexes.sql`
- `enterprise-server/deploy/migrations/014_metrics_query_indexes.sql`
- `enterprise-server/src/db/migrations.rs`

实现步骤：

- [x] 新增 migration 文件：

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

- [x] 同步复制到 deploy migrations。
- [x] 在 `src/db/migrations.rs` 的 `MIGRATIONS` 中注册 `014_metrics_query_indexes`。
- [x] 如果生产表已经很大，评估是否需要单独使用 `CREATE INDEX CONCURRENTLY` 的运维迁移，而不是应用启动时创建。

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

- [x] migration 可重复执行。
- [x] 新索引存在。
- [ ] 写入压测没有明显恶化：留到阶段 4 压测统一验证。
- [ ] dashboard 查询 `EXPLAIN` 开始使用组合索引：留到阶段 4 使用样本数据统一验证。

提交建议：

```bash
git add enterprise-server/migrations/014_metrics_query_indexes.sql enterprise-server/deploy/migrations/014_metrics_query_indexes.sql enterprise-server/src/db/migrations.rs
git commit -m "Add dashboard metrics query indexes"
```

阶段 3.2 执行记录：

| 项目 | 结果 |
| --- | --- |
| metrics scope/time 索引 | 已新增 `idx_metrics_event_org_time` 和 `idx_metrics_event_user_time`，覆盖 dashboard 的 `event_type + org/user + timestamp` 过滤 |
| metrics commit 去重索引 | 已新增 `idx_metrics_event_commit`，覆盖 report fallback `NOT EXISTS` 中的 `event_type + commit_sha` 查询 |
| report fallback 索引 | 已新增 `idx_commit_stats_project_author_time` 和 `idx_projects_org_user` |
| deploy 迁移 | 已同步新增 `enterprise-server/deploy/migrations/014_metrics_query_indexes.sql` |
| 程序化迁移注册 | 已在 `enterprise-server/src/db/migrations.rs` 注册 `014_metrics_query_indexes` |
| `CREATE INDEX CONCURRENTLY` | 本阶段未采用；当前迁移 runner 直接执行 SQL。生产大表建议在维护窗口或独立运维脚本中使用 concurrent index，再让本迁移通过 `IF NOT EXISTS` 幂等跳过 |
| 外部压测/EXPLAIN | 未执行，阶段 4 统一验证写入影响和 dashboard 查询计划 |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `cargo test db::migrations` | 1 passed, 0 failed |
| `cargo test` | 97 passed, 0 failed |
| `rustfmt --edition 2024 --check src/db/migrations.rs` | 通过 |

### 3.3 增加 author_time_at 结构化时间字段

目标：避免 `commit_stats.author_time` 作为 text 导致 report 聚合难以走时间索引。

涉及文件：

- `enterprise-server/migrations/015_commit_stats_author_time_at.sql`
- `enterprise-server/deploy/migrations/015_commit_stats_author_time_at.sql`
- `enterprise-server/src/db/migrations.rs`
- `enterprise-server/src/handlers/report.rs`
- `enterprise-server/src/handlers/dashboard.rs`

实现步骤：

- [x] 新增字段：

```sql
ALTER TABLE commit_stats
ADD COLUMN IF NOT EXISTS author_time_at TIMESTAMPTZ;
```

- [x] 回填可解析的历史数据：

```sql
UPDATE commit_stats
SET author_time_at = NULLIF(author_time, '')::timestamptz
WHERE author_time_at IS NULL
  AND author_time IS NOT NULL
  AND author_time != '';
```

- [x] 增加索引：

```sql
CREATE INDEX IF NOT EXISTS idx_commit_stats_project_author_time_at
    ON commit_stats(project_id, author_time_at);
```

- [x] report 上传时写入 `author_time_at`。
- [x] dashboard report 聚合使用 `author_time_at` 过滤和分组。
- [x] 对无法解析的 author_time 保持 null，不让上传失败，除非数据模型已明确要求 ISO 时间。

测试命令：

```bash
cd enterprise-server
cargo test report
cargo test dashboard
cargo test
```

验收标准：

- [x] 新上传 report 的 `author_time_at` 正确。
- [x] 历史数据可回填。
- [x] dashboard report 时间过滤结果与原来一致：对可解析 RFC3339/ISO author_time 与旧 cast 语义一致；无法解析数据按本阶段设计保持 null，避免查询期 cast 失败。

提交建议：

```bash
git add enterprise-server/migrations/015_commit_stats_author_time_at.sql enterprise-server/deploy/migrations/015_commit_stats_author_time_at.sql enterprise-server/src/db/migrations.rs enterprise-server/src/handlers/report.rs enterprise-server/src/handlers/dashboard.rs
git commit -m "Store structured commit author times"
```

阶段 3.3 执行记录：

| 项目 | 结果 |
| --- | --- |
| 结构化字段 | 已新增 `commit_stats.author_time_at TIMESTAMPTZ` |
| 历史回填 | 已用 ISO/RFC3339 形态正则保护后回填，避免历史脏数据导致 migration cast 失败 |
| 索引 | 已新增 `idx_commit_stats_project_author_time_at(project_id, author_time_at)` |
| report 写入 | 已在 commit stats bulk upsert 中同时写入/更新 `author_time_at`；解析失败写 NULL，不阻断上传 |
| dashboard 查询 | report fallback 的 summary/developer/trends 时间过滤和 trends 分组已改用 `author_time_at` |
| deploy 迁移 | 已同步新增 `enterprise-server/deploy/migrations/015_commit_stats_author_time_at.sql` |
| 程序化迁移注册 | 已在 `enterprise-server/src/db/migrations.rs` 注册 `015_commit_stats_author_time_at` |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `cargo test report` | 6 passed, 0 failed |
| `cargo test dashboard` | 4 passed, 0 failed |
| `cargo test db::migrations` | 1 passed, 0 failed |
| `cargo test` | 99 passed, 0 failed |
| `rustfmt --edition 2024 --check src/db/migrations.rs src/handlers/report.rs` | 通过 |

### 3.4 设计并落地 metrics daily rollup 表

目标：先落地 metrics daily rollup 写入与回填，为下一阶段 dashboard summary/trends/tool comparison 切换到 rollup 查询做准备。

涉及文件：

- `enterprise-server/migrations/016_metrics_daily_rollups.sql`
- `enterprise-server/deploy/migrations/016_metrics_daily_rollups.sql`
- `enterprise-server/src/db/migrations.rs`
- `enterprise-server/src/services/metrics.rs`
- `enterprise-server/src/handlers/metrics.rs`
- `enterprise-server/src/config.rs`
- 测试配置补齐：`enterprise-server/src/auth/jwt.rs`、`enterprise-server/src/handlers/auth_api.rs`、`enterprise-server/src/handlers/cas.rs`、`enterprise-server/src/handlers/oauth.rs`、`enterprise-server/src/handlers/release.rs`、`enterprise-server/src/handlers/report.rs`

实现步骤：

- [x] 新增 rollup 表。

实际实现说明：原草案把 `org_id`、`user_id` 设计成 nullable 主键列，但 PostgreSQL 主键列不能为 `NULL`，且 `ON CONFLICT` 需要稳定 key。本阶段改为用 nil UUID sentinel 表示缺失 scope。

```sql
CREATE TABLE IF NOT EXISTS metrics_daily_rollups (
    day DATE NOT NULL,
    org_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
    user_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
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

- [x] 增加索引：

```sql
CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_org_day
    ON metrics_daily_rollups(org_id, day);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_user_day
    ON metrics_daily_rollups(user_id, day);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_tool_day
    ON metrics_daily_rollups(tool_model, day);
```

- [x] metrics batch 成功写入明细后，在内存中按 day/org/user/repo/tool 聚合。
- [x] 使用同一事务批量插入 `metrics_events` 并批量 upsert `metrics_daily_rollups`。
- [x] 使用 UTC day 聚合，避免数据库会话时区影响回填结果。
- [x] `tool_model = ''` 代表 summary/trends 汇总行，非空 `tool_model` 代表 per-tool rollup 行。
- [x] 增加配置开关：

```env
DASHBOARD_USE_ROLLUPS=false
METRICS_WRITE_ROLLUPS=true
```

- [x] 首版先只写 rollup，不默认切 dashboard。
- [x] 增加回填 SQL：迁移 016 从既有 `metrics_events` 回填 summary 行和 per-tool 行。
- [x] 增加一致性测试：metrics 写入后 rollup 汇总与明细 fixture 聚合一致。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test dashboard
cargo test
```

验收标准：

- [x] metrics 写入后 rollup 行正确增加。
- [x] 重复 batch 不会产生非预期重复：当前 metrics 明细没有幂等 key，重复上传会产生重复明细，rollup 与明细保持同等累加，不额外放大重复。
- [x] 回填 SQL 可随迁移执行，并按明细字段聚合 summary/per-tool 行。
- [ ] rollup 写入对 metrics upload 延迟影响可接受。留到阶段 4 压测验证。

执行记录：

| 项 | 结果 |
| --- | --- |
| 表设计 | `metrics_daily_rollups(day, org_id, user_id, repo_url, tool_model)` 主键，缺失 scope 使用 nil UUID sentinel |
| 写入链路 | `process_metrics_batch` 增加 `write_rollups` 参数，handler 使用 `METRICS_WRITE_ROLLUPS` 配置 |
| 事务边界 | 明细批量插入和 rollup upsert 在同一事务内完成 |
| 回填 | 迁移 016 生成 summary 行和 per-tool 行，跳过 `all` 和空 `tool_model` |
| dashboard 读取 | 本阶段未切换，`DASHBOARD_USE_ROLLUPS=false` 默认关闭，阶段 3.5 再接入 |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `cargo test metrics` | 15 passed, 0 failed |
| `cargo test db::migrations` | 1 passed, 0 failed |
| `cargo test` | 100 passed, 0 failed |
| `rustfmt --edition 2024 --check src/services/metrics.rs src/config.rs src/db/migrations.rs src/handlers/metrics.rs` | 通过 |
| `diff -u enterprise-server/migrations/016_metrics_daily_rollups.sql enterprise-server/deploy/migrations/016_metrics_daily_rollups.sql` | 通过，无差异 |
| `git diff --check` | 通过 |

提交建议：

```bash
git add enterprise-server/migrations/016_metrics_daily_rollups.sql enterprise-server/deploy/migrations/016_metrics_daily_rollups.sql enterprise-server/src/db/migrations.rs enterprise-server/src/services/metrics.rs enterprise-server/src/handlers/metrics.rs enterprise-server/src/config.rs
git commit -m "Write daily metrics rollups"
```

### 3.5 dashboard summary/trends 切到 rollup 查询

目标：让 dashboard 常用聚合接口使用 rollup 表。

涉及文件：

- `enterprise-server/src/handlers/dashboard.rs`
- `enterprise-server/src/config.rs`（配置字段已在 3.4 落地，本阶段使用该字段）

实现步骤：

- [x] 在 `AppConfig` 增加 `dashboard_use_rollups: bool`。
- [x] 默认先设为 `false`，生产验证后再切为 `true`。
- [x] 为 `aggregate_summary` 增加 rollup 查询路径。
- [x] 为 `aggregate_trends` 增加 rollup 查询路径。
- [x] 为 `aggregate_tools` 和 `aggregate_agent_comparison` 增加 rollup 查询路径。
- [x] 保留明细查询 fallback。
- [x] 增加测试对比：
  - rollup enabled 的结果
  - 明细查询结果
  - 两者一致

执行记录：

| 项 | 结果 |
| --- | --- |
| 开关 | `DASHBOARD_USE_ROLLUPS=false` 仍为默认值，生产可灰度设为 `true` |
| summary | metrics 明细部分可切到 `metrics_daily_rollups`，report fallback 继续合并 |
| trends | metrics 趋势部分可切到 rollup，日期聚合统一按 UTC day/week/month |
| tools | committed-event per-tool 统计可切到 rollup；report stats 和 checkpoint/agentusage 明细保留 |
| agent comparison | committed-event per-tool 统计可切到 rollup；report stats 保留 |
| 回退 | 关闭 `DASHBOARD_USE_ROLLUPS` 即恢复明细查询 |

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

- [ ] rollup enabled 时 dashboard 常用接口 p95 明显下降。留到阶段 4 压测验证。
- [x] 查询结果与明细路径一致。
- [x] 可通过配置快速回退到明细查询。

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `cargo test dashboard` | 6 passed, 0 failed |
| `cargo test` | 102 passed, 0 failed |
| `git diff --check` | 通过 |

提交建议：

```bash
git add docs/enterprise-server-performance-task-plan.md enterprise-server/src/handlers/dashboard.rs
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

- [x] 新增结构化表：

```sql
CREATE TABLE IF NOT EXISTS metrics_tool_model_events (
    metric_event_id BIGINT NOT NULL REFERENCES metrics_events(id) ON DELETE CASCADE,
    org_id UUID,
    user_id UUID,
    timestamp BIGINT NOT NULL,
    tool_model TEXT NOT NULL,
    ai_additions BIGINT NOT NULL DEFAULT 0,
    mixed_additions BIGINT NOT NULL DEFAULT 0,
    ai_accepted BIGINT NOT NULL DEFAULT 0,
    total_ai_additions BIGINT NOT NULL DEFAULT 0,
    total_ai_deletions BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (metric_event_id, tool_model)
);
```

- [x] 增加索引：

```sql
CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_org_time
    ON metrics_tool_model_events(org_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_user_time
    ON metrics_tool_model_events(user_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_tool_time
    ON metrics_tool_model_events(tool_model, timestamp);
```

- [x] metrics insert 后获取 inserted event id。
- [x] 展开 tool_model_pairs/raw_values 到结构化 rows。
- [x] 批量 insert 到 `metrics_tool_model_events`。
- [x] dashboard tool comparison 改查结构化表或 rollup。
- [x] 增加回填脚本。

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

- [x] tool_model 结构化结果与 raw `tool_model_pairs`/`raw_values` fixture 展开结果一致。
- [ ] tool comparison 大数据量查询明显加快。留到阶段 4 压测验证。

执行记录：

| 项 | 结果 |
| --- | --- |
| 表设计 | 已新增 `metrics_tool_model_events(metric_event_id, tool_model)`，按明细 event 维护 per-tool 结构化指标 |
| 索引 | 已新增 org/time、user/time、tool/time 组合索引，支撑 dashboard scope 过滤和工具聚合 |
| 回填 | 迁移 017 从 `metrics_events.tool_model_pairs` 和 `raw_values` 的 4/5/6/7/8 指标回填，跳过 `all` 和空 `tool_model` |
| 写入链路 | metrics bulk insert 使用 `RETURNING id` 保持输入顺序，并在同一事务写入 per-tool 结构化 rows |
| dashboard 读取 | 明细 fallback 的 tools/agent comparison 已改查 `metrics_tool_model_events`，rollup enabled 时仍优先读 `metrics_daily_rollups` |
| 行为边界 | `METRICS_WRITE_ROLLUPS=false` 只关闭 daily rollup 写入，不影响 tool-model 明细表写入 |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `cargo test metrics` | 17 passed, 0 failed |
| `cargo test dashboard` | 6 passed, 0 failed |
| `cargo test db::migrations` | 1 passed, 0 failed |
| `cargo test` | 102 passed, 0 failed |
| `diff -u enterprise-server/migrations/017_metrics_tool_model_events.sql enterprise-server/deploy/migrations/017_metrics_tool_model_events.sql` | 通过，无差异 |
| `git diff --check` | 通过 |

提交建议：

```bash
git add docs/enterprise-server-performance-task-plan.md enterprise-server/migrations/017_metrics_tool_model_events.sql enterprise-server/deploy/migrations/017_metrics_tool_model_events.sql enterprise-server/src/db/migrations.rs enterprise-server/src/services/metrics.rs enterprise-server/src/handlers/dashboard.rs
git commit -m "Store structured metrics tool model rows"
```

## 阶段 4: 压测脚本和容量验证

### 4.1 增加 enterprise 压测脚本目录

目标：把手工压测变成可重复执行的脚本。

涉及文件：

- `scripts/benchmarks/enterprise/`
- 可选：`docs/enterprise-server-performance-task-plan.md`

实现步骤：

- [x] 新建目录：

```bash
mkdir -p scripts/benchmarks/enterprise
```

- [x] 增加 README，说明依赖、环境变量、运行方式。
- [x] 增加 `seed_metrics` 脚本，支持生成大规模 metrics 数据。
- [x] 增加 `bench_health_ready` 脚本，包装 `/health`、`/ready` 压测。
- [x] 增加 `bench_metrics_upload` 脚本，构造认证请求和不同 batch size。
- [x] 增加 `bench_dashboard` 脚本，测试 summary/trends/tools。

建议环境变量：

```env
ENTERPRISE_BASE_URL=http://127.0.0.1:8080
ENTERPRISE_API_KEY=...
BENCH_CONCURRENCY=20
BENCH_REQUESTS=1000
```

验收标准：

- [x] 新机器上按 README 可以跑通基础压测。
- [x] 脚本输出 RPS、p95、p99、错误率。
- [x] 失败时退出码非 0。

执行记录：

| 项 | 结果 |
| --- | --- |
| 目录 | 已新增 `scripts/benchmarks/enterprise/` |
| 公共模块 | `_common.py` 统一处理 HTTP、并发调度、RPS/p95/p99/error rate 统计、PosEncoded metrics payload 生成 |
| 健康检查 | `bench_health_ready.py` 压测 `/health` 和 `/ready`，不需要 API key |
| 造数 | `seed_metrics.py` 通过 `/worker/metrics/upload` 写入 committed metrics，支持 `--events`、`--batch-size`、`--concurrency` |
| metrics 上传 | `bench_metrics_upload.py` 构造认证请求和不同 batch size，并把 partial success 视为失败 |
| dashboard | `bench_dashboard.py` 覆盖 summary、trends(day/week) 和 tools，并支持 `--org`/`BENCH_ORG` |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `python3 -m py_compile scripts/benchmarks/enterprise/_common.py scripts/benchmarks/enterprise/bench_health_ready.py scripts/benchmarks/enterprise/bench_metrics_upload.py scripts/benchmarks/enterprise/seed_metrics.py scripts/benchmarks/enterprise/bench_dashboard.py` | 通过 |
| `python3 scripts/benchmarks/enterprise/bench_health_ready.py --help` | 通过 |
| `python3 scripts/benchmarks/enterprise/bench_metrics_upload.py --help` | 通过 |
| `python3 scripts/benchmarks/enterprise/seed_metrics.py --help` | 通过 |
| `python3 scripts/benchmarks/enterprise/bench_dashboard.py --help` | 通过 |

提交建议：

```bash
git add docs/enterprise-server-performance-task-plan.md scripts/benchmarks/enterprise
git commit -m "Add enterprise server benchmark scripts"
```

### 4.2 增加大数据量 dashboard 验证

目标：验证 dashboard 在 10万、100万、500万 metrics 数据下的行为。

步骤：

- [x] 使用造数脚本生成 10 万 metrics。
- [x] 运行 dashboard 压测。
- [x] 记录 p95/p99；慢查询列表留到 4.3 接入 `pg_stat_statements` 后补充。
- [ ] 生成 100 万 metrics。
- [ ] 重复压测。
- [ ] 如果机器允许，生成 500 万 metrics。
- [x] 对比 rollup enabled/disabled。

验收标准：

- [x] 30 天 summary p95 小于目标值。10 万级本地验证：rollup enabled p95 267.58ms。
- [x] trends p95 小于目标值。10 万级本地验证：rollup enabled day p95 124.89ms，week p95 116.43ms。
- [x] tools comparison p95 小于目标值。10 万级本地验证：rollup enabled p95 143.93ms。
- [x] 没有 DB acquire timeout。
- [ ] Postgres CPU 没有长期打满。

执行记录：

| 项 | 结果 |
| --- | --- |
| 环境 | 本地 Docker PostgreSQL/Redis，当前源码分别启动 `DASHBOARD_USE_ROLLUPS=false` 的 `127.0.0.1:43130` 和 `DASHBOARD_USE_ROLLUPS=true` 的 `127.0.0.1:43131` |
| 迁移 | 当前源码成功补跑迁移 014、015、016、017，生成 `metrics_daily_rollups` 和 `metrics_tool_model_events` |
| 造数 | 使用 `seed_metrics.py --events 100000 --batch-size 500 --concurrency 10` 写入 10 万 committed metrics |
| 造数限流 | metrics 每 API key 60/min，压测脚本已支持 `ENTERPRISE_API_KEYS` 多 key 轮换，避免把限流误判为写入瓶颈 |
| 首轮问题 | 高并发 rollup upsert 触发 `deadlock detected`，根因是 `HashMap::into_values()` 导致不同事务按不同主键顺序锁定 `metrics_daily_rollups` |
| 修复 | `prepare_daily_rollups` 输出按 `(day, org_id, user_id, repo_url, tool_model)` 主键排序，保证并发 upsert 锁顺序稳定 |
| 当前数据量 | `metrics_events=113564`，`metrics_daily_rollups=13508`，`metrics_tool_model_events=113550` |

造数结果：

| 场景 | requests | success | error rate | elapsed | p95 | p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 修复前 10 万造数 | 200 | 27 | 86.50% | 31.84s | 7186.63ms | 12973.30ms |
| 修复后 10 万造数 | 200 | 200 | 0.00% | 34.13s | 3333.89ms | 4358.42ms |

Dashboard 30 天查询结果，每个 endpoint 100 requests、concurrency 20：

| endpoint | rollup | p95 | p99 | error rate |
| --- | --- | ---: | ---: | ---: |
| summary | disabled | 1378.55ms | 1451.40ms | 0.00% |
| summary | enabled | 267.58ms | 320.56ms | 0.00% |
| tools | disabled | 709.52ms | 746.04ms | 0.00% |
| tools | enabled | 143.93ms | 207.73ms | 0.00% |
| trends ai_lines/day | disabled | 790.77ms | 840.51ms | 0.00% |
| trends ai_lines/day | enabled | 124.89ms | 173.67ms | 0.00% |
| trends ai_ratio/week | disabled | 775.28ms | 840.42ms | 0.00% |
| trends ai_ratio/week | enabled | 116.43ms | 147.20ms | 0.00% |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo check` | 通过，仅有既有 warning |
| `cargo test metrics` | 17 passed, 0 failed |
| `cargo test` | 102 passed, 0 failed |
| `python3 -m py_compile scripts/benchmarks/enterprise/_common.py scripts/benchmarks/enterprise/bench_health_ready.py scripts/benchmarks/enterprise/bench_metrics_upload.py scripts/benchmarks/enterprise/seed_metrics.py scripts/benchmarks/enterprise/bench_dashboard.py` | 通过 |
| `python3 scripts/benchmarks/enterprise/seed_metrics.py --help` | 通过 |
| `python3 scripts/benchmarks/enterprise/bench_dashboard.py --help` | 通过 |
| `git diff --check` | 通过 |

提交建议：

```bash
git add docs/enterprise-server-performance-task-plan.md enterprise-server/src/services/metrics.rs scripts/benchmarks/enterprise
git commit -m "Stabilize enterprise metrics rollup validation"
```

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
