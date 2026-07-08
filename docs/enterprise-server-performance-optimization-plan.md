# Enterprise Server 性能优化执行计划

本文档记录 `enterprise-server/` 当前服务端并发性能现状、主要瓶颈、优化优先级、具体实现建议和验收方式。它关注吞吐、延迟和大数据量查询性能；并发一致性问题另见 `docs/enterprise-server-concurrency-optimization.md`。

## 结论摘要

当前服务端的基础并发模型是可用的：

- HTTP 框架使用 Axum + Tokio 多线程 runtime。
- 主要共享状态放在 PostgreSQL、Redis 和 S3/MinIO 中。
- migration、OAuth 一次性凭证、CAS、report 上传等关键一致性问题已经有测试覆盖。
- 本机 Docker 小数据量轻量采样中，`/health` 和 `/ready` 能处理约 800-1100 req/s，p99 在 20-30ms 级别。

但系统还没有完成高吞吐优化。真实上量后的主要瓶颈会集中在：

1. 所有请求都经过 Redis 限流，且当前限流路径每次请求都会获取 Redis 连接。
2. 数据库连接池硬编码 20 个连接，缺少按部署环境调节能力。
3. metrics 批量上传逐事件串行处理，每条事件会产生多次 DB 往返。
4. CAS 批量上传逐对象串行处理，每个对象执行 S3 写入和 DB transaction。
5. dashboard 聚合直接扫原始表，大数据量下会被 `metrics_events`、`commit_stats`、JSONB 展开和 `NOT EXISTS` 子查询拖慢。

建议分三批推进：

| 批次 | 目标 | 典型收益 |
| --- | --- | --- |
| P0 | 修正明显低风险瓶颈 | 避免健康检查被限流、减少 Redis 连接开销、让 DB pool 可配置 |
| P1 | 提升写入吞吐 | metrics/CAS/report 上传在批量和并发下更稳 |
| P2 | 支撑 dashboard 大数据量查询 | 从扫明细表变成查 rollup/索引友好的数据 |

## 当前基线

### 代码现状

关键入口：

- `enterprise-server/src/main.rs`
- `enterprise-server/src/routes.rs`
- `enterprise-server/src/services/rate_limit.rs`
- `enterprise-server/src/services/metrics.rs`
- `enterprise-server/src/handlers/cas.rs`
- `enterprise-server/src/handlers/report.rs`
- `enterprise-server/src/handlers/dashboard.rs`

已确认的良好基础：

- DB pool 使用 SQLx `PgPool`，服务可并发获取连接。
- Redis rate limit 已支持多实例共享计数。
- migration 使用 PostgreSQL advisory lock 串行化。
- OAuth grant 使用 `UPDATE/DELETE ... RETURNING` 做原子消费。
- report 上传已包在 transaction 中。
- CAS 上传已做服务端 hash 校验，并先写对象存储再写 DB。
- client status 已按设备维度记录。

### 本地轻量采样

采样环境：

- 本机 Docker compose
- API/Postgres/Redis/MinIO 均在本地容器中
- 数据量很小：`metrics_events` 约几十行
- 使用 ApacheBench

结果：

```text
GET /health
  concurrency: 20
  requests: 100
  success: 100
  throughput: ~1108 req/s
  p99: ~23ms

GET /ready
  concurrency: 20
  requests: 100
  success: 100
  throughput: ~839 req/s
  p99: ~26ms
```

注意：

- 这只能说明基础 HTTP/DB 链路健康。
- 不能代表 metrics 上传、CAS 上传、dashboard 聚合在生产数据量下的容量。
- 1000 请求采样会触发默认限流，说明 `/health`、`/ready` 当前也会消耗默认限流额度。

## 性能目标

### 短期目标

适合小团队到中等团队规模：

- 健康检查和 readiness 不被业务限流影响。
- metrics 单批 100-500 条事件能稳定在 200ms-1000ms 内完成，具体取决于 DB 和网络。
- CAS 单批 10-50 个对象不会因串行处理导致明显排队。
- dashboard 常用聚合在 30 天数据范围内 p95 小于 500ms。

### 中期目标

适合多团队、多实例部署：

- API 多实例横向扩展时，rate limit 全局一致。
- 写入接口在 DB pool 内稳定排队，不能无限制创建 DB/S3 压力。
- dashboard 查询避免直接扫大明细表。
- 所有关键接口都有压测脚本和基线记录。

### 长期目标

适合企业持续使用：

- metrics 明细表按时间分区或归档。
- dashboard 默认查 rollup 表。
- 长耗时查询有超时、慢查询日志和可观测性指标。
- 容量规划基于实际 QPS、事件量、对象大小和保留期。

## P0: 低风险立即优化

### P0-1: `/health` 和 `/ready` 跳过业务限流

#### 问题

当前 `rate_limit_middleware` 挂在全局 router layer 上，所有路由都会经过限流：

- `/health`
- `/ready`
- dashboard 页面
- admin API
- worker upload API

健康检查被限流后，负载均衡器、容器编排系统或监控系统可能误判服务不可用。

#### 建议实现

在 `enterprise-server/src/services/rate_limit.rs` 的 `rate_limit_middleware` 开头加 bypass：

```rust
let path = request.uri().path().to_string();

if path == "/health" || path == "/ready" {
    return Ok(next.run(request).await);
}
```

如果后续还有静态资源、登录页等非业务写入路径，也可以按需加入 bypass 列表，但不要绕过真正需要保护的上传接口。

#### 验收

```bash
cd enterprise-server
cargo test rate_limit
ab -n 1000 -c 50 http://127.0.0.1:8080/health
ab -n 1000 -c 50 http://127.0.0.1:8080/ready
```

验收标准：

- `/health` 和 `/ready` 1000 请求不出现 429。
- 业务接口仍然按 tier 限流。

### P0-2: Redis rate limiter 复用连接管理器

#### 问题

当前 `RateLimiter::check_redis` 每次请求都会调用：

```rust
redis.get_multiplexed_async_connection().await?
```

这会在高 QPS 下增加连接获取成本。虽然 multiplexed connection 本身可复用，但在每次 check 中获取连接仍然不是最优路径。

#### 建议实现

把 `RateLimiter` 从持有 `redis::Client` 改成持有 `redis::aio::ConnectionManager`。

建议结构：

```rust
#[derive(Debug, Clone)]
pub struct RateLimiter {
    redis: Option<redis::aio::ConnectionManager>,
    counters: Arc<RwLock<HashMap<String, HashMap<String, (u32, Instant)>>>>,
}
```

初始化：

```rust
let redis_client = redis::Client::open(config.redis_url.clone())?;
let rate_limiter = services::rate_limit::RateLimiter::with_redis(redis_client).await?;
```

执行脚本时：

```rust
let mut connection = manager.clone();
redis::Script::new(REDIS_RATE_LIMIT_SCRIPT)
    .key(redis_key)
    .arg(limit.window_seconds.max(1))
    .invoke_async(&mut connection)
    .await
```

实现注意点：

- `ConnectionManager` 是 async 初始化，`with_redis` 需要改成 async。
- 需要更新 `main.rs` 调用点。
- 测试里构造 `RateLimiter::with_redis` 的地方也要 `.await`。
- Redis 不可用时当前策略是 fallback 到内存并打 warn；生产是否允许 fail open 需要单独配置化。

#### 验收

```bash
cd enterprise-server
cargo test rate_limit
cargo test
```

压测对比：

- 改前记录 `/health`、一个被限流保护的轻量端点的 p95/p99。
- 改后在相同并发下确认 Redis CPU 和 API latency 不变差。

### P0-3: 数据库连接池配置化

#### 问题

当前 DB pool 上限硬编码为 20：

```rust
PgPoolOptions::new()
    .max_connections(20)
```

不同部署环境的 CPU、Postgres 规格和实例数量不同，固定 20 不利于容量调整。

#### 建议实现

在 `AppConfig` 中增加：

```rust
pub database_max_connections: u32,
pub database_min_connections: u32,
pub database_acquire_timeout_seconds: u64,
```

环境变量默认值：

```env
DATABASE_MAX_CONNECTIONS=20
DATABASE_MIN_CONNECTIONS=1
DATABASE_ACQUIRE_TIMEOUT_SECONDS=5
```

初始化：

```rust
let db_pool = sqlx::postgres::PgPoolOptions::new()
    .max_connections(config.database_max_connections)
    .min_connections(config.database_min_connections)
    .acquire_timeout(Duration::from_secs(config.database_acquire_timeout_seconds))
    .connect(&config.database_url)
    .await?;
```

#### 容量建议

单个 Postgres 实例不要盲目把连接数调很大。建议从下面的公式开始：

```text
每个 API 实例 max_connections =
  floor((postgres_max_connections - 预留连接数) / API 实例数)
```

示例：

```text
Postgres max_connections = 200
预留连接数 = 40
API 实例数 = 4
每实例 max_connections = 40
```

#### 验收

```bash
DATABASE_MAX_CONNECTIONS=30 cargo run
```

确认启动日志和数据库连接数符合预期。

### P0-4: 加请求体大小和 batch 大小保护

#### 问题

`tower-http` 已启用 `limit` feature，但当前 router 没有看到明确的全局 body limit。上传接口如果没有 batch 大小上限，单请求可能占用过多内存、DB 连接和 S3 带宽。

#### 建议实现

为这些接口加明确限制：

| 接口 | 建议限制 |
| --- | --- |
| `/worker/metrics/upload` | batch events 最多 500 或 1000 |
| `/worker/cas/upload` | objects 最多 50 或 100 |
| `/api/v1/reports` | body 大小按实际报告控制，先从 10-25MB 开始 |
| release asset 上传 | 按二进制包大小单独设置 |

metrics 示例：

```rust
if batch.events.len() > 500 {
    return Err(AppError::BadRequest("Maximum 500 events per batch".into()));
}
```

CAS 示例：

```rust
if req.objects.len() > 100 {
    return Err(AppError::BadRequest("Maximum 100 CAS objects per batch".into()));
}
```

#### 验收

- 正常 batch 继续成功。
- 超大 batch 返回 400。
- 压测时单请求不会造成内存明显尖峰。

## P1: 写入吞吐优化

### P1-1: metrics 上传避免每条事件查询 org scope

#### 问题

当前 metrics batch 处理逻辑里，每条事件都会在 `store_event` 中执行：

```rust
crate::services::org_scope::preferred_org_scope(pool, uid).await?
```

但 handler 已经通过 `AuthExtractor` 得到了 `auth.0.org_id`。同一个请求内用户和组织 scope 是固定的，没有必要每条事件重复查。

#### 建议实现

修改函数签名：

```rust
pub async fn process_metrics_batch(
    pool: &PgPool,
    events: Vec<MetricEvent>,
    user_id: Option<Uuid>,
    org_id: Option<Uuid>,
    distinct_id: Option<String>,
) -> MetricsUploadResponse
```

`upload_metrics` 调用：

```rust
process_metrics_batch(
    &state.db,
    batch.events,
    Some(auth.0.user_id),
    auth.0.org_id,
    distinct_id.clone(),
).await
```

`store_event` 直接使用传入的 `org_id`，删除每条事件里的 `preferred_org_scope` 查询。

#### 预期收益

如果一个 batch 有 100 条事件，原来至少有 100 次额外 org scope 查询。改完后这部分 DB 往返变成 0。

#### 验收

新增或调整测试：

- 带 `auth.0.org_id` 上传 metrics，落库 org_id 正确。
- 没有 org_id 时仍可上传，org_id 为 null。

压测：

- 构造 100、500 条 event 的 batch。
- 对比接口耗时和 Postgres query count。

### P1-2: metrics 批量 insert

#### 问题

当前 `process_metrics_batch` 是：

1. 遍历事件。
2. decode。
3. 每条事件调用一次 `INSERT INTO metrics_events`。

这会导致 batch 中 N 条事件产生 N 次 DB 往返。

#### 建议实现

分两阶段：

1. 在内存中 decode 全部事件，收集成功行和错误。
2. 对成功行按 chunk 使用 `sqlx::QueryBuilder` 做多行 insert。

伪代码：

```rust
let mut decoded_rows = Vec::new();
let mut errors = Vec::new();

for (idx, event) in events.iter().enumerate() {
    match decode_event(event) {
        Ok(decoded) => decoded_rows.push((idx, decoded)),
        Err(error) => errors.push(...),
    }
}

for chunk in decoded_rows.chunks(500) {
    insert_metrics_chunk(pool, chunk, user_id, org_id, &distinct_id).await?;
}
```

`QueryBuilder` 形式：

```rust
let mut builder = sqlx::QueryBuilder::new(
    "INSERT INTO metrics_events (...) "
);

builder.push_values(rows, |mut b, row| {
    b.push_bind(row.event_type)
     .push_bind(row.timestamp)
     .push_bind(row.user_id);
});

builder.build().execute(pool).await?;
```

#### 错误语义

当前接口支持 partial success。批量 insert 后需要明确：

- decode 错误仍然按事件返回 partial error。
- DB insert chunk 失败时，该 chunk 内成功/失败无法逐条确认。

建议策略：

1. 首版实现：DB chunk 失败时，将该 chunk 所有事件标记为 storage error。
2. 如果需要更细粒度，再 fallback 到逐条 insert 找出坏行。

#### 验收

测试：

- 全部成功时，落库行数等于 event 数。
- 部分 decode 失败时，成功事件落库，失败事件出现在 errors。
- DB chunk 失败时，返回对应 storage errors。

压测：

```bash
# 建议写专用脚本，而不是手工 curl
batch sizes: 1, 10, 100, 500, 1000
concurrency: 1, 5, 20
```

记录：

- p50/p95/p99
- rows/s
- DB CPU
- pool acquire timeout 数量

### P1-3: client status 更新降频或合并

#### 问题

`upload_metrics` 在成功写入后会调用 `touch_last_seen`。如果客户端高频上传 metrics，这会带来额外 update/upsert。

#### 建议实现

给 `touch_last_seen` 加节流语义：

- 如果同一设备 `last_seen_at` 距离现在小于 60 秒，则不更新。
- 或使用 Redis 短 TTL key 做请求级节流。

SQL 方案：

```sql
ON CONFLICT (user_id, device_key) DO UPDATE SET
    last_seen_at = CASE
        WHEN developer_client_status.last_seen_at IS NULL
          OR developer_client_status.last_seen_at < now() - interval '60 seconds'
        THEN now()
        ELSE developer_client_status.last_seen_at
    END,
    updated_at = CASE
        WHEN developer_client_status.last_seen_at IS NULL
          OR developer_client_status.last_seen_at < now() - interval '60 seconds'
        THEN now()
        ELSE developer_client_status.updated_at
    END
```

#### 验收

- 高频 metrics 上传不会每次改 `developer_client_status.updated_at`。
- dashboard 仍然能正确显示最近在线状态。

### P1-4: CAS 批量上传改成有界并发

#### 问题

当前 CAS upload 对 `req.objects` 逐个串行调用 `process_cas_object`。单对象内部还包含：

- canonical JSON 序列化
- SHA256
- secrets scan
- S3 put
- DB transaction

如果 batch 中有多个对象，单请求延迟会线性增加。

#### 建议实现

使用有界并发，建议初始并发度为 4 或 8：

```rust
use futures::{stream, StreamExt};

let results = stream::iter(req.objects.into_iter())
    .map(|object| {
        let state = state.clone();
        let identity = auth.0.clone();
        let headers = headers.0.clone();
        async move {
            let hash = object.hash.clone();
            let result = process_cas_object(&state, &object, &identity, &headers).await;
            (hash, result)
        }
    })
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
```

实现注意点：

- 不要无限并发。
- 单对象内部顺序不变：hash 校验 -> secrets scan -> S3 put -> DB transaction。
- `BadRequest` 的处理语义要保留。当前遇到 hash mismatch 会直接返回 400；并发后建议预校验所有对象 hash 和 batch 大小，确认无 400 后再并发处理。

#### 验收

测试：

- 同 hash 同内容并发上传仍幂等。
- 同 hash 不同内容仍拒绝。
- S3 失败时 DB 不留下记录。
- batch 中部分对象存储失败时返回 partial result。

压测：

- 10、50、100 个 CAS object batch。
- 对比串行和有界并发的 p95。
- 监控 MinIO/S3 错误率和 API 内存。

### P1-5: report 上传批量化

#### 问题

report 上传已经事务化，但 commit stats 和 tool model stats 仍然在 transaction 内逐条执行 SQL。大型 report 会长时间持有 transaction 和 DB 连接。

#### 建议实现

使用 `QueryBuilder` 批量 upsert `commit_stats` 和 `tool_model_stats`。

关键点：

- 保持整个 report 的 transaction。
- 对 commits 分 chunk，例如每 500 条一批。
- `ON CONFLICT ... DO UPDATE SET` 语义保持不变。
- inserted/updated 计数如果很难通过批量 SQL 精确返回，可以先返回 `processed_commits`，或者使用 CTE 返回计数。

#### 验收

- 已有 report transaction 测试继续通过。
- 重复上传同 commit 仍会更新。
- 大 report 上传耗时明显下降。

## P2: dashboard 查询优化

### P2-1: 时间过滤改成索引友好

#### 问题

dashboard 查询中经常出现：

```sql
to_timestamp(timestamp) >= $3::timestamptz
```

`metrics_events.timestamp` 是 bigint epoch 秒。对列调用函数会降低普通 btree 索引利用率。

#### 建议实现

在 Rust 层把 `since/until` 转成 epoch 秒：

```rust
let since_ts = query.since.map(|dt| dt.timestamp());
let until_ts = query.until.map(|dt| dt.timestamp());
```

SQL 改成：

```sql
AND ($3::bigint IS NULL OR timestamp >= $3)
AND ($4::bigint IS NULL OR timestamp <= $4)
```

涉及接口：

- `aggregate_summary`
- `aggregate_trends`
- `aggregate_developers`
- `aggregate_projects`
- 其他使用 `to_timestamp(timestamp)` 的聚合接口

#### 验收

使用 `EXPLAIN (ANALYZE, BUFFERS)` 对比改前改后：

```sql
EXPLAIN (ANALYZE, BUFFERS)
SELECT ...
FROM metrics_events
WHERE event_type = 1
  AND org_id = '<org-id>'
  AND timestamp >= 1780000000;
```

验收标准：

- 大数据量下走合适索引。
- 查询结果与改前一致。

### P2-2: 增加组合索引

#### 问题

当前 migrations 中有不少单列索引：

- `idx_metrics_org_id`
- `idx_metrics_user_id`
- `idx_metrics_timestamp`
- `idx_metrics_event_type`
- `idx_metrics_commit_sha`

dashboard 查询常见条件是组合条件：

```sql
event_type = 1
org_id = ?
timestamp between ? and ?
```

单列索引不一定足够。

#### 建议新增迁移

新增 `014_metrics_query_indexes.sql`：

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

注意：

- 如果 `commit_stats.author_time` 长期是 text，按时间查询会很难优化。中期建议新增 `author_time_at TIMESTAMPTZ`。
- 对大表建索引在生产可能需要 `CONCURRENTLY`，但当前程序式 migration 在 transaction/单连接场景下使用 `CREATE INDEX CONCURRENTLY` 需要额外处理。生产大表建议单独 migration job。

#### 验收

- 迁移可重复执行。
- `EXPLAIN` 使用组合索引。
- 写入延迟没有明显恶化。

### P2-3: dashboard rollup 表

#### 问题

当前 dashboard 聚合直接读明细：

- `aggregate_summary` 汇总 `metrics_events` 和 `commit_stats`
- `aggregate_trends` 按时间 `DATE_TRUNC`
- `aggregate_agent_comparison` 展开 JSONB 数组
- 多个查询用 `NOT EXISTS` 避免 metrics 与 report commit 重复

当 `metrics_events` 到百万级、千万级后，dashboard 会变成主要压力源。

#### 建议设计

先做日粒度 rollup：

```sql
CREATE TABLE metrics_daily_rollups (
    day DATE NOT NULL,
    org_id UUID,
    user_id UUID,
    repo_url TEXT,
    tool_model TEXT,
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

索引：

```sql
CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_org_day
    ON metrics_daily_rollups(org_id, day);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_user_day
    ON metrics_daily_rollups(user_id, day);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_tool_day
    ON metrics_daily_rollups(tool_model, day);
```

写入策略：

1. metrics ingest 成功插入明细后，同步 upsert rollup。
2. 每个 event 计算 day、org_id、user_id、repo_url、tool_model。
3. 对同一 batch 内相同维度先在内存聚合，再一次性 upsert。

upsert 示例：

```sql
INSERT INTO metrics_daily_rollups (
    day, org_id, user_id, repo_url, tool_model,
    commits, total_lines, ai_lines, human_lines, mixed_lines, ai_accepted
) VALUES (...)
ON CONFLICT (day, org_id, user_id, repo_url, tool_model)
DO UPDATE SET
    commits = metrics_daily_rollups.commits + EXCLUDED.commits,
    total_lines = metrics_daily_rollups.total_lines + EXCLUDED.total_lines,
    ai_lines = metrics_daily_rollups.ai_lines + EXCLUDED.ai_lines,
    human_lines = metrics_daily_rollups.human_lines + EXCLUDED.human_lines,
    mixed_lines = metrics_daily_rollups.mixed_lines + EXCLUDED.mixed_lines,
    ai_accepted = metrics_daily_rollups.ai_accepted + EXCLUDED.ai_accepted,
    updated_at = now();
```

查询策略：

- 最近 N 天 summary、trends、tool comparison 优先查 rollup。
- 明细表只用于 drilldown、导出和回溯。
- report 数据可以单独做 `report_daily_rollups`，或在统一 rollup 中增加 `source` 字段。

#### 回填策略

新增脚本或 admin job：

```sql
INSERT INTO metrics_daily_rollups (...)
SELECT
    to_timestamp(timestamp)::date AS day,
    org_id,
    user_id,
    repo_url,
    COALESCE(tool || '::' || model, 'unknown') AS tool_model,
    COUNT(*) AS commits,
    SUM(git_diff_added_lines) AS total_lines,
    SUM(ai_additions) AS ai_lines,
    SUM(GREATEST(COALESCE(git_diff_added_lines, 0) - COALESCE(ai_additions, 0), 0)) AS human_lines,
    SUM(mixed_additions) AS mixed_lines,
    SUM(ai_accepted) AS ai_accepted
FROM metrics_events
WHERE event_type = 1
GROUP BY 1, 2, 3, 4, 5;
```

如果需要按 `tool_model_pairs` 做精确多工具拆分，回填逻辑要复用当前 `aggregate_agent_comparison` 的 JSONB 展开语义。

#### 验收

- rollup 总数与明细聚合结果一致。
- dashboard 查询 p95 明显下降。
- metrics ingest 写入延迟增加在可接受范围内。

### P2-4: JSONB 展开结果预计算

#### 问题

`aggregate_agent_comparison` 里会对 `metrics_events.raw_values` 和 `tool_model_pairs` 做 lateral JSONB 展开。这类查询灵活但不适合高频 dashboard。

#### 建议实现

新增明细表：

```sql
CREATE TABLE metrics_tool_model_events (
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

ingest 时把 JSONB 数组展开成结构化行。dashboard tool comparison 直接 group by `tool_model`。

这是 P2 后半段优化，优先级低于 daily rollup。

## P3: 数据生命周期和存储扩展

### P3-1: metrics_events 分区

当 `metrics_events` 到千万级后，建议按月分区：

```sql
CREATE TABLE metrics_events_2026_07 PARTITION OF metrics_events
FOR VALUES FROM (1782864000) TO (1785542400);
```

实现前需要先评估：

- 当前 migrations 是否适合改造分区表。
- 历史数据迁移窗口。
- 索引策略。
- 删除过期数据是否可以按 partition drop。

### P3-2: 数据保留策略

建议：

| 数据 | 默认保留期 | 说明 |
| --- | --- | --- |
| metrics 明细 | 180-365 天 | 支持回溯和导出 |
| daily rollup | 长期保留 | dashboard 默认数据源 |
| CAS prompts | 按合规要求 | 可能涉及敏感内容和审计 |
| audit log | 365 天或更长 | 企业审计 |
| release assets | 保留最近 N 个版本 | 旧版本按策略清理 |

### P3-3: 后台任务

可增加后台 worker：

- rollup 回填。
- 过期数据清理。
- CAS 孤儿对象扫描。
- release asset 孤儿对象清理。
- 慢查询统计。

## 压测方案

### 工具

轻量：

```bash
ab -n 1000 -c 50 http://127.0.0.1:8080/health
ab -n 1000 -c 50 http://127.0.0.1:8080/ready
```

更推荐：

- `wrk`
- `hey`
- `k6`
- 自定义 Rust/Node/Python 脚本，用于构造真实 auth、metrics、CAS payload

### 测试矩阵

| 场景 | 请求数 | 并发 | 数据规模 |
| --- | --- | --- | --- |
| health | 1000/10000 | 50/100 | 无 |
| ready | 1000/10000 | 50/100 | DB 查询 |
| metrics upload | 100/1000 | 5/20/50 | batch 1/100/500 |
| CAS upload | 100/500 | 5/20 | object 1/10/50 |
| dashboard summary | 100/1000 | 10/50 | 1万/100万 metrics |
| dashboard trends | 100/1000 | 10/50 | 30/90/365 天 |

### 指标

必须记录：

- RPS
- p50/p95/p99 latency
- error rate
- HTTP status 分布
- API CPU/memory
- Postgres CPU/memory
- Postgres active/idle connections
- Redis CPU/memory
- MinIO/S3 latency
- DB slow query

Postgres 连接观察：

```sql
SELECT state, COUNT(*)
FROM pg_stat_activity
WHERE datname = 'gitai_enterprise'
GROUP BY state;
```

慢查询候选：

```sql
SELECT query, calls, mean_exec_time, p95_exec_time
FROM pg_stat_statements
ORDER BY mean_exec_time DESC
LIMIT 20;
```

如果没有启用 `pg_stat_statements`，生产建议启用。

### 造数建议

本地小数据量无法验证 dashboard 大表性能。建议提供造数脚本：

```text
scripts/benchmarks/enterprise/seed_metrics.rs 或 .py
```

参数：

```text
--orgs 5
--users-per-org 100
--repos-per-user 10
--events 1000000
--days 90
```

造数字段要覆盖：

- event_type = 1
- timestamp 分布
- org_id/user_id
- repo_url
- tool/model
- tool_model_pairs
- raw_values
- commit_sha

## 上线策略

### 改动顺序

建议按以下顺序提交：

1. 健康检查跳过限流。
2. Redis rate limiter 复用连接管理器。
3. DB pool 配置化。
4. metrics org scope 查询去重。
5. metrics bulk insert。
6. CAS 有界并发。
7. dashboard 时间过滤改 epoch。
8. 组合索引迁移。
9. daily rollup 表和 dashboard 查询改造。

### 回滚策略

| 改动 | 回滚方式 |
| --- | --- |
| health/ready bypass | 回滚代码即可 |
| Redis manager | 回滚到 redis client 获取连接 |
| DB pool 配置 | 环境变量调回默认值 |
| metrics bulk insert | 保留逐条 insert fallback |
| CAS 有界并发 | 并发度配置为 1 |
| 索引迁移 | 一般不回滚，除非写入性能明显变差 |
| rollup 表 | dashboard 切回明细查询，保留 rollup 表 |

### 配置开关

建议新增：

```env
DATABASE_MAX_CONNECTIONS=20
DATABASE_MIN_CONNECTIONS=1
DATABASE_ACQUIRE_TIMEOUT_SECONDS=5
RATE_LIMIT_BYPASS_HEALTH=true
METRICS_BULK_INSERT=true
METRICS_BULK_INSERT_CHUNK_SIZE=500
CAS_UPLOAD_CONCURRENCY=8
DASHBOARD_USE_ROLLUPS=false
```

首版可以只实现必要开关，不要把所有优化都做成配置项。

## 验收清单

P0 完成后：

- [ ] `/health` 和 `/ready` 不会触发 429。
- [ ] Redis rate limit 测试通过。
- [ ] DB pool 可通过环境变量调整。
- [ ] `cargo test` 通过。

P1 完成后：

- [ ] metrics batch 上传不再每事件查 org scope。
- [ ] metrics bulk insert 测试覆盖 partial success。
- [ ] CAS batch 有界并发，不破坏一致性测试。
- [ ] report 大批量上传耗时下降。
- [ ] 写入压测记录 p95/p99 和 rows/s。

P2 完成后：

- [ ] dashboard 时间过滤使用 epoch 秒。
- [ ] 大表查询 `EXPLAIN` 使用组合索引。
- [ ] rollup 表回填结果与明细聚合一致。
- [ ] dashboard summary/trends/tool comparison p95 达标。

## 风险清单

| 风险 | 说明 | 缓解 |
| --- | --- | --- |
| DB pool 调太大 | 可能拖垮 Postgres | 按实例数和 Postgres max_connections 计算 |
| bulk insert 错误粒度变粗 | chunk 失败时难定位单条坏数据 | 失败时 fallback 到逐条 insert |
| CAS 并发过高 | S3/MinIO 或 DB transaction 压力上升 | 有界并发并配置化 |
| 新索引影响写入 | 每次 insert 维护更多索引 | 压测写入延迟，必要时减少索引 |
| rollup 与明细不一致 | 写入或回填 bug 导致 dashboard 数据偏差 | 增加一致性校验 job |
| 健康检查绕过限流被滥用 | `/health` 被高频访问 | 只返回固定小 JSON，不访问重资源 |

## 推荐下一步

优先执行 P0 和 P1-1：

1. `/health`、`/ready` 跳过限流。
2. Redis rate limiter 改成连接管理器。
3. DB pool 配置化。
4. metrics 上传复用 `auth.0.org_id`，去掉每条事件 org scope 查询。

这四项改动小、收益明确、回归风险低。完成后再做 metrics bulk insert 和 dashboard rollup。
