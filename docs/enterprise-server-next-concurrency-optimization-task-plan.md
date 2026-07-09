# Enterprise Server 下一轮并发优化任务清单

本文档把当前压测发现的并发瓶颈拆成可以逐步执行、逐步验证、逐步提交的工程任务。它接续以下已完成文档：

- [Enterprise Server 性能优化任务清单](./enterprise-server-performance-task-plan.md)
- [Enterprise 认证登录性能优化任务清单](./enterprise-auth-login-performance-task-plan.md)

## 当前基线

基线日期：2026-07-09。

本地 Docker 环境已经完成一轮上传和登录压测，关键结果如下：

| 链路 | 样本 | 结果 | 当前判断 |
| --- | --- | --- | --- |
| metrics upload | 500 请求 / 50 并发 / 50000 events / batch=100 | 全成功，p95 `7825.79ms`，p99 `12267.32ms` | 当前最主要写入瓶颈 |
| CAS upload | 200 请求 / 40 并发 / 2000 objects / 约 2KB object | 全成功，p95 `1472.66ms` | 可接受，但仍有批量 DB 写入优化空间 |
| report upload | 20 请求 / 10 并发 / 1000 commits per report | 全成功，p95 `603.45ms` | 当前不是瓶颈 |
| auth login | 1000 请求 / 100 并发 / 100 用户 / 200 IP pool / `AUTH_PASSWORD_CONCURRENCY=12` | 全成功，p95 `1744.61ms` | 可支撑 100 并发登录，但需要观测 Argon2 队列 |
| auth login | 同上，`AUTH_PASSWORD_CONCURRENCY=16` | 全成功，p95 `2620.40ms`，API 空闲内存 `2.66GiB` | 并发度过高会恶化尾延迟 |
| health/ready | 上传和登录压测前后 | 保持 2-4ms | 轻量接口目前没有被压测拖慢 |

总体优先级：

1. P0：metrics 写入链路拆解和异步 rollup。
2. P1：登录链路观测和安全限流细化。
3. P1：CAS 批量 DB 写入。
4. P2：部署容量、连接池、横向扩容验证。

## 执行原则

1. 每次只处理一个阶段。
2. 每个阶段单独提交，便于回滚和压测对比。
3. 先做观测和可回滚开关，再改请求路径行为。
4. 任何改变写入语义的阶段，都必须保留默认兼容配置。
5. 每个阶段完成后，必须记录：
   - 代码改动。
   - 测试命令和结果。
   - 压测命令和核心 p95/p99。
   - 是否建议进入下一阶段。

## 阶段 0: 复现基线和准备观测

目标：在本机或测试环境复现当前基线，确保后续优化有可比较的数字。

涉及文件：

- `docs/enterprise-server-next-concurrency-optimization-task-plan.md`
- `scripts/benchmarks/enterprise/`

### 0.1 确认工作区和服务状态

步骤：

- [ ] 查看工作区状态：

```bash
git status --short
```

- [ ] 启动依赖和 API：

```bash
cd enterprise-server
docker compose up -d postgres redis minio
docker compose up -d --build api
```

- [ ] 确认服务状态：

```bash
docker compose ps
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/ready
```

- [ ] 确认 API 关键环境变量：

```bash
docker compose exec -T api printenv METRICS_WRITE_ROLLUPS
docker compose exec -T api printenv DASHBOARD_USE_ROLLUPS
docker compose exec -T api printenv AUTH_PASSWORD_CONCURRENCY
docker compose exec -T api printenv RATE_LIMIT_AUTH_MAX_REQUESTS
docker compose exec -T api printenv RATE_LIMIT_OAUTH_MAX_REQUESTS
```

验收标准：

- [ ] API、Postgres、Redis、MinIO 正常。
- [ ] `/health` 和 `/ready` 正常。
- [ ] 运行环境变量与本轮要测试的配置一致。

### 0.2 准备本地 benchmark API key

目标：避免单 API key 限流污染 metrics/CAS/report 上传压测。

步骤：

- [ ] 优先使用管理页面或 admin API 创建 5-10 个测试 API key。
- [ ] 如果只在本地 Docker 测试库执行，可用临时 SQL 创建本地 key；不要把明文 key 写入仓库。
- [ ] 导出：

```bash
export ENTERPRISE_BASE_URL=http://127.0.0.1:8080
export ENTERPRISE_API_KEYS=key-1,key-2,key-3
```

验收标准：

- [ ] `bench_metrics_upload.py --requests 1 --batch-size 1` 成功。
- [ ] `bench_cas_upload.py --requests 1 --objects-per-request 1` 成功。
- [ ] `bench_report_upload.py --requests 1 --commit-count 1` 成功。

### 0.3 复跑基线压测

步骤：

- [ ] metrics 上传：

```bash
python3 scripts/benchmarks/enterprise/bench_metrics_upload.py \
  --requests 500 \
  --batch-size 100 \
  --concurrency 50
```

- [ ] CAS 上传：

```bash
python3 scripts/benchmarks/enterprise/bench_cas_upload.py \
  --requests 200 \
  --objects-per-request 10 \
  --content-bytes 2048 \
  --concurrency 40
```

- [ ] report 大报告：

```bash
python3 scripts/benchmarks/enterprise/bench_report_upload.py \
  --requests 20 \
  --commit-count 1000 \
  --concurrency 10 \
  --timeout 120
```

- [ ] 登录：

```bash
python3 scripts/benchmarks/enterprise/bench_auth_login.py \
  --mode login \
  --login-user-count 100 \
  --login-email-domain linewell.com \
  --login-email-prefix bench-login-pool \
  --login-run-id 20260709-001 \
  --login-password correct-horse-battery \
  --requests 1000 \
  --concurrency 100 \
  --client-ip-mode pool \
  --client-ip-pool-size 200 \
  --allow-errors
```

记录项：

- [ ] 每组 p50/p95/p99。
- [ ] 401/409/429/500。
- [ ] 压测前后 `/health`、`/ready`。
- [ ] `docker stats --no-stream`。
- [ ] Postgres 连接状态。

验收标准：

- [ ] 能复现 metrics upload p95 明显高于 CAS/report。
- [ ] 无大面积 429 或 500。
- [ ] 结果写入本文档的阶段 0 执行记录。

提交建议：

```bash
git add docs/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Record next enterprise concurrency baseline"
```

## 阶段 1: 拆解 metrics 写入耗时

目标：把 metrics upload 的总耗时拆成 decode、raw events insert、tool-model rows insert、daily rollup upsert 四段，先定位主要耗时来源。

涉及文件：

- `enterprise-server/src/services/metrics.rs`
- 可选：`enterprise-server/src/config.rs`
- 可选：`enterprise-server/.env.example`
- `docs/enterprise-server-next-concurrency-optimization-task-plan.md`

### 1.1 增加 metrics 写入阶段耗时日志

实现步骤：

- [ ] 在 `insert_metrics_chunk` 内使用 `std::time::Instant` 记录：
  - `insert_metrics_events_chunk`
  - `insert_metrics_tool_model_events_chunk`
  - `upsert_metrics_daily_rollups`
  - transaction commit
- [ ] 日志建议带上：
  - `rows.len()`
  - tool-model row 数量
  - daily rollup row 数量
  - `write_rollups`
  - 每段耗时 ms
- [ ] 日志级别先用 `tracing::info!` 或受配置控制的 `tracing::debug!`。
- [ ] 不改变响应格式和写入语义。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test
```

验收标准：

- [ ] metrics 测试全部通过。
- [ ] 压测时日志能看出每个 chunk 的阶段耗时。
- [ ] 没有引入额外 DB 查询。

### 1.2 对比 rollup 开关

步骤：

- [ ] 使用 `METRICS_WRITE_ROLLUPS=true` 跑 metrics 上传基线。
- [ ] 重建 API，使用 `METRICS_WRITE_ROLLUPS=false` 跑同样参数。
- [ ] 两次都记录 p95/p99 和阶段耗时日志。

命令示例：

```bash
cd enterprise-server
METRICS_WRITE_ROLLUPS=false \
DASHBOARD_USE_ROLLUPS=true \
AUTH_PASSWORD_CONCURRENCY=12 \
docker compose up -d --force-recreate --no-deps api
```

验收标准：

- [ ] 明确 daily rollup upsert 对 p95 的影响比例。
- [ ] 如果关闭 rollup 后 p95 明显下降，进入阶段 2。
- [ ] 如果关闭 rollup 后仍慢，优先进入阶段 3。

提交建议：

```bash
git add enterprise-server/src/services/metrics.rs docs/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Instrument enterprise metrics write phases"
```

## 阶段 2: metrics rollup 异步化

目标：把 daily rollup 从请求同步路径移出，降低 metrics upload 尾延迟，同时保持 dashboard rollup 查询最终一致。

推荐方案：dirty scope + 后台重建。

设计思路：

- metrics upload 请求路径继续同步写：
  - `metrics_events`
  - `metrics_tool_model_events`
- 请求路径不再同步 upsert `metrics_daily_rollups`。
- 请求路径只把受影响的 `(day, org_id, user_id)` 标记为 dirty。
- 后台 worker 批量读取 dirty scope，在事务内按 day/org/user 从明细表重新聚合并覆盖对应 rollup。
- 该方案是幂等重建，不会因为 worker 重试导致 rollup 重复累加。

涉及文件：

- `enterprise-server/migrations/019_metrics_rollup_dirty_scopes.sql`
- `enterprise-server/deploy/migrations/019_metrics_rollup_dirty_scopes.sql`
- `enterprise-server/src/db/migrations.rs`
- `enterprise-server/src/config.rs`
- `enterprise-server/src/main.rs`
- `enterprise-server/src/services/metrics.rs`
- 可选：`enterprise-server/src/services/metrics_rollups.rs`
- `enterprise-server/.env.example`
- `enterprise-server/deploy/.env.example`
- `docs/enterprise-server-next-concurrency-optimization-task-plan.md`

### 2.1 增加 rollup 模式配置

建议新增：

```env
METRICS_ROLLUP_WRITE_MODE=sync
METRICS_ROLLUP_WORKER_ENABLED=false
METRICS_ROLLUP_WORKER_INTERVAL_SECONDS=5
METRICS_ROLLUP_WORKER_BATCH_SIZE=100
```

模式语义：

| 模式 | 语义 |
| --- | --- |
| `sync` | 当前兼容行为：请求内同步写 daily rollup |
| `dirty_async` | 请求内只标记 dirty scope，后台 worker 重建 rollup |
| `off` | 不写 rollup，也不标记 dirty |

兼容要求：

- [ ] 如果 `METRICS_ROLLUP_WRITE_MODE` 未设置，继续使用现有 `METRICS_WRITE_ROLLUPS` 的语义。
- [ ] 初始默认仍为 `sync`，先不改变生产行为。
- [ ] `.env.example` 和 deploy `.env.example` 写清楚推荐灰度方式。

### 2.2 新增 dirty scope 表

迁移建议：

```sql
CREATE TABLE IF NOT EXISTS metrics_rollup_dirty_scopes (
    day DATE NOT NULL,
    org_id UUID NOT NULL,
    user_id UUID NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ,
    PRIMARY KEY (day, org_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_metrics_rollup_dirty_claim
    ON metrics_rollup_dirty_scopes (claimed_at NULLS FIRST, updated_at);
```

实现步骤：

- [ ] 新增本地和 deploy 迁移。
- [ ] 注册迁移。
- [ ] 增加迁移测试，确认表和索引存在。

测试命令：

```bash
cd enterprise-server
cargo test db::migrations
cargo run -- --migrate
```

### 2.3 请求路径标记 dirty scope

实现步骤：

- [ ] 在 `insert_metrics_chunk` 中根据配置分支：
  - `sync`: 继续调用 `upsert_metrics_daily_rollups`
  - `dirty_async`: 调用 `mark_metrics_rollup_dirty_scopes`
  - `off`: 不写 rollup，不标记 dirty
- [ ] `mark_metrics_rollup_dirty_scopes` 从本 chunk 的 rows 提取唯一 `(day, org_id, user_id)`。
- [ ] 使用 bulk insert：

```sql
INSERT INTO metrics_rollup_dirty_scopes (day, org_id, user_id)
VALUES ...
ON CONFLICT (day, org_id, user_id)
DO UPDATE SET updated_at = now(), claimed_at = NULL;
```

验收标准：

- [ ] `sync` 模式行为完全保持。
- [ ] `dirty_async` 模式下 metrics upload 响应成功，dirty 表出现记录。
- [ ] `off` 模式下不写 rollup、不写 dirty。

### 2.4 实现 rollup rebuild worker

实现步骤：

- [ ] 增加服务函数：

```rust
pub async fn process_metrics_rollup_dirty_scopes(pool: &PgPool, batch_size: i64) -> Result<u64, AppError>
```

- [ ] 使用事务领取 dirty scope：

```sql
SELECT day, org_id, user_id
FROM metrics_rollup_dirty_scopes
WHERE claimed_at IS NULL OR claimed_at < now() - interval '5 minutes'
ORDER BY updated_at
LIMIT $1
FOR UPDATE SKIP LOCKED;
```

- [ ] 对每个 scope，在同一事务内：
  - 删除 `metrics_daily_rollups` 中对应 day/org/user 的旧 rollup。
  - 从 `metrics_events` 和 `metrics_tool_model_events` 聚合生成新 rollup。
  - 插入新的 `metrics_daily_rollups`。
  - 删除对应 dirty scope。
- [ ] 在 `main.rs` 中按配置启动后台 task。
- [ ] worker 失败只记录 warn，不影响 API 主进程。

验收标准：

- [ ] worker 可重复执行，结果不重复累加。
- [ ] worker 中途失败后 dirty scope 不丢失，可重试。
- [ ] dashboard rollup 数据与 sync 模式结果一致。

### 2.5 压测和灰度

压测步骤：

- [ ] `sync` 模式跑 metrics upload。
- [ ] `dirty_async` 模式跑相同参数。
- [ ] 压测后等待 worker 追平 dirty scope。
- [ ] 跑 dashboard benchmark，确认数据可见。

验收标准：

- [ ] metrics upload p95 明显低于 `sync` 模式。
- [ ] `metrics_rollup_dirty_scopes` 可在压测后归零或保持低水位。
- [ ] dashboard rollup 查询结果与 sync 模式误差为 0。
- [ ] health/ready 不受 worker 明显影响。

提交建议：

```bash
git add enterprise-server/migrations/019_metrics_rollup_dirty_scopes.sql enterprise-server/deploy/migrations/019_metrics_rollup_dirty_scopes.sql enterprise-server/src/db/migrations.rs enterprise-server/src/config.rs enterprise-server/src/main.rs enterprise-server/src/services/metrics.rs enterprise-server/.env.example enterprise-server/deploy/.env.example docs/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Add async metrics rollup rebuild mode"
```

## 阶段 3: 优化 metrics tool-model 明细写入

目标：如果阶段 1 显示 `metrics_tool_model_events` 写入仍是主要耗时，则继续减少 tool-model 明细写入放大。

涉及文件：

- `enterprise-server/src/services/metrics.rs`
- 可选：`enterprise-server/migrations/020_metrics_tool_model_write_indexes.sql`
- `scripts/benchmarks/enterprise/bench_metrics_upload.py`
- `docs/enterprise-server-next-concurrency-optimization-task-plan.md`

### 3.1 记录 tool-model 行数放大

实现步骤：

- [ ] 在阶段 1 的阶段耗时日志中增加 `tool_model_rows`。
- [ ] benchmark 输出或日志记录：
  - events 数。
  - tool-model rows 数。
  - tool-model rows/event。

验收标准：

- [ ] 能判断真实客户端 payload 下 row multiplier。

### 3.2 优化 bulk insert chunk

步骤：

- [ ] 检查 `insert_metrics_tool_model_events_chunk` 的 SQL 参数数量，确保不会接近 Postgres 参数上限。
- [ ] 如果单 chunk 参数过多，单独为 tool-model rows 分 chunk。
- [ ] 对比 chunk size：
  - 100 events
  - 250 events
  - 500 events
- [ ] 选择 p95 最稳定的 chunk。

验收标准：

- [ ] metrics upload 不出现 DB 参数上限错误。
- [ ] p95/p99 不比当前 500-event chunk 更差。

### 3.3 评估明细表索引写入成本

步骤：

- [ ] 通过 `pg_stat_statements` 和 `EXPLAIN (ANALYZE, BUFFERS)` 确认写入期间最重索引。
- [ ] 检查 `metrics_tool_model_events` 是否存在过多非必要索引。
- [ ] 对 dashboard 必需索引和写入成本做取舍。

验收标准：

- [ ] 不删除 dashboard 必需索引。
- [ ] 如果删除或调整索引，dashboard benchmark 必须无退化。

提交建议：

```bash
git add enterprise-server/src/services/metrics.rs docs/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Tune metrics tool model event writes"
```

## 阶段 4: 登录链路观测和保护

目标：登录当前可以支撑 100 并发，但需要观测 Argon2 队列等待，避免生产盲调 `AUTH_PASSWORD_CONCURRENCY`。

涉及文件：

- `enterprise-server/src/services/passwords.rs`
- `enterprise-server/src/handlers/auth_api.rs`
- `enterprise-server/src/config.rs`
- `scripts/benchmarks/enterprise/bench_auth_login.py`
- `docs/enterprise-server-next-concurrency-optimization-task-plan.md`

### 4.1 增加密码计算阶段耗时

实现步骤：

- [ ] 在 `hash_password_blocking` 和 `verify_password_blocking` 内记录：
  - semaphore acquire 等待耗时。
  - `spawn_blocking` 内 Argon2 执行耗时。
  - 总耗时。
- [ ] 日志字段包括 operation=`hash|verify` 和 configured concurrency。
- [ ] 不记录密码、hash、邮箱等敏感数据。

验收标准：

- [ ] 登录压测时能看到队列等待是否超过 Argon2 执行时间。
- [ ] 不泄露认证敏感信息。

### 4.2 增加账号维度保护

目标：避免单个账号被高并发重复登录拖慢整体 Argon2 队列。

实现步骤：

- [ ] 评估是否使用 Redis 增加 email hash 维度的短窗口限流。
- [ ] 只对失败登录或所有登录加小窗口限制，需要先明确产品策略。
- [ ] 错误响应保持不泄露账号是否存在。
- [ ] 增加测试：
  - 同账号高频请求触发限制。
  - 不同账号并发不互相影响。

验收标准：

- [ ] 撞库式单账号高频请求被限制。
- [ ] 正常 100 用户登录池压测不受影响。

### 4.3 生产参数建议

步骤：

- [ ] 在 deploy README 或部署文档中记录：
  - 默认 `AUTH_PASSWORD_CONCURRENCY=8`。
  - 4-8 核实例可灰度 `12`。
  - 不建议直接 `16+`，除非有压测证明 p95/p99 不恶化。
- [ ] 增加压测模板命令。

验收标准：

- [ ] 文档说明清楚如何灰度和回滚。

提交建议：

```bash
git add enterprise-server/src/services/passwords.rs enterprise-server/src/handlers/auth_api.rs enterprise-server/src/config.rs docs/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Instrument enterprise password verification latency"
```

## 阶段 5: CAS 上传批量 DB 写入

目标：降低 CAS upload 中每对象一个 DB transaction 的成本。

当前问题：

- CAS 每个对象先写 MinIO/S3，再开启 DB transaction 写 `cas_objects` 和 `cas_ownership`。
- 2000 objects、40 并发 p95 `1472.66ms`，可接受但仍有明显批量化空间。

涉及文件：

- `enterprise-server/src/handlers/cas.rs`
- `enterprise-server/src/services/cas.rs`
- `docs/enterprise-server-next-concurrency-optimization-task-plan.md`

### 5.1 保留对象存储并发，合并 DB 写入

实现步骤：

- [ ] 将每对象处理拆成两段：
  - 并发执行 S3/MinIO put。
  - 收集成功对象后，按 batch 批量写 DB。
- [ ] 新增 `insert_cas_db_rows_chunk`：
  - bulk insert `cas_objects`
  - bulk insert `cas_ownership`
  - 单 transaction 覆盖整个成功对象集合或分 chunk
- [ ] 保持 partial failure 语义：
  - S3 失败的对象返回 error。
  - S3 成功但 DB batch 失败的对象返回 error。
- [ ] DB 写入失败时记录足够日志，方便定位。

验收标准：

- [ ] CAS hash mismatch 仍在写 S3 前失败。
- [ ] 同 hash 同内容并发上传仍幂等。
- [ ] S3 失败不留下 DB ready 记录。
- [ ] CAS 2000 objects 压测 p95 低于当前 `1472.66ms` 或 DB transaction 数明显下降。

测试命令：

```bash
cd enterprise-server
cargo test cas
cargo test
```

提交建议：

```bash
git add enterprise-server/src/handlers/cas.rs docs/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Batch database writes for CAS uploads"
```

## 阶段 6: 部署容量和横向扩容验证

目标：确认单实例优化后，服务能通过多实例扩容继续提升并发，而不是被连接池、Redis、MinIO 或负载均衡配置卡住。

涉及文件：

- `enterprise-server/deploy/README.md`
- `enterprise-server/deploy/docker-compose.yml`
- `docs/enterprise-server-deployment.md`
- `docs/enterprise-server-next-concurrency-optimization-task-plan.md`

### 6.1 连接池容量计算

步骤：

- [ ] 明确生产 Postgres `max_connections`。
- [ ] 计算：

```text
api_instances * DATABASE_MAX_CONNECTIONS + migration/admin/psql_reserved < postgres_max_connections
```

- [ ] 如果多实例后连接数紧张，评估 PgBouncer。
- [ ] 文档中给出推荐值：
  - 小环境：2 实例 * 20 connections。
  - 中等环境：4 实例 * 15 connections + PgBouncer。
  - 大环境：应用池更小，依赖 PgBouncer 和 DB 规格扩容。

验收标准：

- [ ] 部署文档明确每实例连接池如何设置。

### 6.2 多实例限流验证

步骤：

- [ ] 启动 2 个 API 实例。
- [ ] 确认都使用 Redis rate limiter，不回退内存。
- [ ] 对同 API key 执行超限请求，确认总限流不会被实例数放大。
- [ ] 对 `/health`、`/ready` 确认仍 bypass。

验收标准：

- [ ] Redis 正常时，多实例共享限流。
- [ ] Redis 不可用时有明确 warn，并知道限流会降级为单实例内存计数。

### 6.3 多实例压测

步骤：

- [ ] 2 实例下跑：
  - metrics upload
  - CAS upload
  - auth login
  - dashboard
- [ ] 与单实例结果对比。

验收标准：

- [ ] 登录吞吐接近线性提升，或明确瓶颈在 CPU/DB。
- [ ] metrics upload 如果仍不提升，说明瓶颈在 Postgres 写入，需要继续优化阶段 2/3。
- [ ] CAS 如果不提升，检查 MinIO/S3 资源。

提交建议：

```bash
git add enterprise-server/deploy/README.md docs/enterprise-server-deployment.md docs/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Document enterprise concurrency capacity planning"
```

## 阶段 7: 最终回归和容量结论

目标：完成本轮优化后，给出可以对外使用的容量判断和生产配置建议。

### 7.1 全链路回归

必须执行：

```bash
cd enterprise-server
cargo check
cargo test
docker compose config --quiet
JWT_SECRET=test-secret-at-least-32-characters POSTGRES_PASSWORD=test-password docker compose -f deploy/docker-compose.yml config --quiet
```

benchmark：

```bash
python3 -m py_compile scripts/benchmarks/enterprise/*.py
python3 scripts/benchmarks/enterprise/bench_health_ready.py --requests 1000 --concurrency 50
python3 scripts/benchmarks/enterprise/bench_metrics_upload.py --requests 500 --batch-size 100 --concurrency 50
python3 scripts/benchmarks/enterprise/bench_cas_upload.py --requests 200 --objects-per-request 10 --concurrency 40
python3 scripts/benchmarks/enterprise/bench_report_upload.py --requests 20 --commit-count 1000 --concurrency 10 --timeout 120
python3 scripts/benchmarks/enterprise/bench_auth_login.py --mode login --requests 1000 --concurrency 100 --client-ip-mode pool --client-ip-pool-size 200 --allow-errors
```

### 7.2 容量结论模板

完成后在本文档补充：

| 链路 | 优化前 p95 | 优化后 p95 | 错误率 | 建议轻松并发 |
| --- | ---: | ---: | ---: | ---: |
| metrics upload | 7825.79ms | 待填 | 待填 | 待填 |
| CAS upload | 1472.66ms | 待填 | 待填 | 待填 |
| report upload large | 603.45ms | 待填 | 待填 | 待填 |
| auth login | 1744.61ms | 待填 | 待填 | 待填 |
| health/ready | 2-4ms | 待填 | 待填 | 待填 |

验收标准：

- [ ] 所有核心链路无 500。
- [ ] 429 只在预期限流场景出现。
- [ ] metrics upload p95 明显低于当前 `7.83s`。
- [ ] 登录 p95 不高于当前 `1.74s`。
- [ ] dashboard rollup 数据正确。
- [ ] 文档中有明确推荐配置和回滚方式。

提交建议：

```bash
git add docs/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Record enterprise concurrency optimization results"
```

## 回滚策略

| 改动 | 快速回滚 |
| --- | --- |
| metrics rollup `dirty_async` | 设置 `METRICS_ROLLUP_WRITE_MODE=sync`，重启 API |
| rollup worker | 设置 `METRICS_ROLLUP_WORKER_ENABLED=false` |
| metrics 阶段耗时日志 | 降低日志级别或关闭对应配置 |
| `AUTH_PASSWORD_CONCURRENCY=12` | 调回 `8`，重启 API |
| 登录账号维度限流 | 关闭新增配置或调高阈值 |
| CAS 批量 DB 写入 | 回滚到每对象事务实现 |
| 多实例扩容 | 降回单实例，确认 Redis/DB 连接恢复 |

## 推荐执行顺序

1. 阶段 0：复现基线和准备观测。
2. 阶段 1：拆解 metrics 写入耗时。
3. 阶段 2：metrics rollup 异步化。
4. 阶段 3：优化 metrics tool-model 明细写入。
5. 阶段 4：登录链路观测和保护。
6. 阶段 5：CAS 上传批量 DB 写入。
7. 阶段 6：部署容量和横向扩容验证。
8. 阶段 7：最终回归和容量结论。

当前最建议先做阶段 1 和阶段 2，因为 metrics upload 是唯一 p95 已经进入 7 秒以上的核心链路，优化收益最大。
