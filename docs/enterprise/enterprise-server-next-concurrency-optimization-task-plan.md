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

- `docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md`
- `scripts/benchmarks/enterprise/`

### 0.1 确认工作区和服务状态

步骤：

- [x] 查看工作区状态：

```bash
git status --short
```

- [x] 启动依赖和 API：

```bash
cd enterprise-server
docker compose up -d postgres redis minio
docker compose up -d --build api
```

- [x] 确认服务状态：

```bash
docker compose ps
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/ready
```

- [x] 确认 API 关键环境变量：

```bash
docker compose exec -T api printenv METRICS_WRITE_ROLLUPS
docker compose exec -T api printenv DASHBOARD_USE_ROLLUPS
docker compose exec -T api printenv AUTH_PASSWORD_CONCURRENCY
docker compose exec -T api printenv RATE_LIMIT_AUTH_MAX_REQUESTS
docker compose exec -T api printenv RATE_LIMIT_OAUTH_MAX_REQUESTS
```

验收标准：

- [x] API、Postgres、Redis、MinIO 正常。
- [x] `/health` 和 `/ready` 正常。
- [x] 运行环境变量与本轮要测试的配置一致。

### 0.2 准备本地 benchmark API key

目标：避免单 API key 限流污染 metrics/CAS/report 上传压测。

步骤：

- [x] 优先使用管理页面或 admin API 创建 5-10 个测试 API key。
- [x] 如果只在本地 Docker 测试库执行，可用临时 SQL 创建本地 key；不要把明文 key 写入仓库。
- [x] 导出：

```bash
export ENTERPRISE_BASE_URL=http://127.0.0.1:8080
export ENTERPRISE_API_KEYS=key-1,key-2,key-3
```

验收标准：

- [x] `bench_metrics_upload.py --requests 1 --batch-size 1` 成功。
- [x] `bench_cas_upload.py --requests 1 --objects-per-request 1` 成功。
- [x] `bench_report_upload.py --requests 1 --commit-count 1` 成功。

### 0.3 复跑基线压测

步骤：

- [x] metrics 上传：

```bash
python3 scripts/benchmarks/enterprise/bench_metrics_upload.py \
  --requests 500 \
  --batch-size 100 \
  --concurrency 50
```

- [x] CAS 上传：

```bash
python3 scripts/benchmarks/enterprise/bench_cas_upload.py \
  --requests 200 \
  --objects-per-request 10 \
  --content-bytes 2048 \
  --concurrency 40
```

- [x] report 大报告：

```bash
python3 scripts/benchmarks/enterprise/bench_report_upload.py \
  --requests 20 \
  --commit-count 1000 \
  --concurrency 10 \
  --timeout 120
```

- [x] 登录：

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

- [x] 每组 p50/p95/p99。
- [x] 401/409/429/500。
- [x] 压测前后 `/health`、`/ready`。
- [x] `docker stats --no-stream`。
- [x] Postgres 连接状态。

验收标准：

- [x] 能复现 metrics upload p95 明显高于 CAS/report。
- [x] 无大面积 429 或 500。
- [x] 结果写入本文档的阶段 0 执行记录。

提交建议：

```bash
git add docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Record next enterprise concurrency baseline"
```

### 阶段 0 执行记录

执行日期：2026-07-09。

环境确认：

| 项 | 结果 |
| --- | --- |
| `git status --short` | 开始执行时工作区干净 |
| `docker compose up -d postgres redis minio` | Postgres、Redis、MinIO 已启动 |
| `docker compose up -d --build api` | 成功，release build 用时约 6m41s；存在既有 warning，无构建失败 |
| `docker compose ps` | API healthy，Postgres healthy，Redis healthy，MinIO running |
| `pg_isready` | `/var/run/postgresql:5432 - accepting connections` |
| `redis-cli ping` | `PONG` |
| `/health` | `2.771ms` |
| `/ready` | `3.288ms` |

运行配置：

| 配置 | 值 |
| --- | --- |
| `METRICS_WRITE_ROLLUPS` | `true` |
| `DASHBOARD_USE_ROLLUPS` | `true` |
| `AUTH_PASSWORD_CONCURRENCY` | `12` |
| `RATE_LIMIT_AUTH_MAX_REQUESTS` | `300` |
| `RATE_LIMIT_OAUTH_MAX_REQUESTS` | `600` |

备注：标准 `docker compose up -d --build api` 后 `AUTH_PASSWORD_CONCURRENCY` 为默认 `8`。为复现上一轮登录基线，本阶段已用 `AUTH_PASSWORD_CONCURRENCY=12 RATE_LIMIT_AUTH_MAX_REQUESTS=300 RATE_LIMIT_OAUTH_MAX_REQUESTS=600 docker compose up -d --force-recreate --no-deps api` 重新创建 API 容器。

本地 benchmark API key：

| 项 | 结果 |
| --- | --- |
| benchmark key 数量 | 10 |
| 创建方式 | 本地 Docker 测试库临时 SQL，使用服务端同样的 SHA256 hash 规则；明文 key 未写入仓库 |
| metrics 烟测 | 1/1 成功，p95 `58.88ms` |
| CAS 烟测 | 1/1 成功，p95 `45.81ms` |
| report 烟测 | 1/1 成功，p95 `39.80ms` |

压测前数据量：

| 表 | 行数 |
| --- | ---: |
| `metrics_events` | 163591 |
| `metrics_daily_rollups` | 13912 |
| `metrics_tool_model_events` | 163559 |
| `cas_objects` | 2059 |
| `report_uploads` | 122 |
| `commit_stats` | 30004 |
| `users` | 304 |

压测前资源快照：

| 容器 | CPU | 内存 |
| --- | ---: | ---: |
| API | 0.20% | 7.055MiB |
| Postgres | 0.00% | 86.2MiB |
| Redis | 0.72% | 18.51MiB |
| MinIO | 0.04% | 150.4MiB |

基线压测结果：

| 链路 | 参数 | 成功/错误 | RPS | p50 | p95 | p99 | 结论 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| metrics upload | 500 requests / 50 concurrency / batch=100 / 50000 events | 500/0 | 22.45 req/s | 1740.77ms | 5002.62ms | 7226.40ms | 仍是最慢写入链路，约 2245 events/s |
| CAS upload | 200 requests / 40 concurrency / 10 objects/request / 2000 objects | 200/0 | 57.17 req/s | 645.07ms | 946.09ms | 998.38ms | 明显快于 metrics，但仍可继续做批量 DB 写入 |
| report upload large | 20 requests / 10 concurrency / 1000 commits/report / 20000 commits | 20/0 | 20.42 req/s | 403.39ms | 532.73ms | 555.09ms | 当前不是瓶颈 |
| auth login | 1000 requests / 100 concurrency / 100 users / 200 IP pool / `AUTH_PASSWORD_CONCURRENCY=12` | 1000/0 | 93.73 req/s | 1048.27ms | 1117.96ms | 1149.90ms | 全部 200，无 401/409/429/500 |

压测后状态：

| 项 | 值 |
| --- | ---: |
| `/health` | 5.843ms |
| `/ready` | 6.261ms |
| Postgres 连接 | active=2, idle=20 |
| `metrics_events` | 213595 |
| `metrics_daily_rollups` | 14313 |
| `metrics_tool_model_events` | 213561 |
| `cas_objects` | 4061 |
| `report_uploads` | 143 |
| `commit_stats` | 50005 |
| `users` | 304 |

压测后资源快照：

| 容器 | CPU | 内存 |
| --- | ---: | ---: |
| API | 14.21% | 1.111GiB |
| Postgres | 8.73% | 335.9MiB |
| Redis | 9.87% | 18.91MiB |
| MinIO | 0.30% | 334.7MiB |

阶段 0 结论：

- 四条核心链路均无 500，登录无 401/409/429。
- metrics upload p95 `5002.62ms`，仍明显高于 CAS `946.09ms` 和 report `532.73ms`，阶段 1 应优先拆解 metrics 写入耗时。
- 登录在 `AUTH_PASSWORD_CONCURRENCY=12` 下表现好于上一轮记录，100 并发 p95 `1117.96ms`；阶段 4 仍需要补 Argon2 队列观测，避免生产盲调。
- health/ready 在压测后仍为 6ms 内，说明轻量接口未被本轮压测明显拖慢。

## 阶段 1: 拆解 metrics 写入耗时

目标：把 metrics upload 的总耗时拆成 decode、raw events insert、tool-model rows insert、daily rollup upsert 四段，先定位主要耗时来源。

涉及文件：

- `enterprise-server/src/services/metrics.rs`
- 可选：`enterprise-server/src/config.rs`
- 可选：`enterprise-server/.env.example`
- `docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md`

### 1.1 增加 metrics 写入阶段耗时日志

实现步骤：

- [x] 在 `insert_metrics_chunk` 内使用 `std::time::Instant` 记录：
  - `insert_metrics_events_chunk`
  - `insert_metrics_tool_model_events_chunk`
  - `upsert_metrics_daily_rollups`
  - transaction commit
- [x] 日志建议带上：
  - `rows.len()`
  - tool-model row 数量
  - daily rollup row 数量
  - `write_rollups`
  - 每段耗时 ms
- [x] 日志级别先用 `tracing::info!` 或受配置控制的 `tracing::debug!`。
- [x] 不改变响应格式和写入语义。

测试命令：

```bash
cd enterprise-server
cargo test metrics
cargo test
```

验收标准：

- [x] metrics 测试全部通过。
- [x] 压测时日志能看出每个 chunk 的阶段耗时。
- [x] 没有引入额外 DB 查询。

### 1.2 对比 rollup 开关

步骤：

- [x] 使用 `METRICS_WRITE_ROLLUPS=true` 跑 metrics 上传基线。
- [x] 重建 API，使用 `METRICS_WRITE_ROLLUPS=false` 跑同样参数。
- [x] 两次都记录 p95/p99 和阶段耗时日志。

命令示例：

```bash
cd enterprise-server
METRICS_WRITE_ROLLUPS=false \
DASHBOARD_USE_ROLLUPS=true \
AUTH_PASSWORD_CONCURRENCY=12 \
docker compose up -d --force-recreate --no-deps api
```

验收标准：

- [x] 明确 daily rollup upsert 对 p95 的影响比例。
- [x] 如果关闭 rollup 后 p95 明显下降，进入阶段 2。
- [x] 如果关闭 rollup 后仍慢，优先进入阶段 3。

提交建议：

```bash
git add enterprise-server/src/services/metrics.rs docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Instrument enterprise metrics write phases"
```

### 阶段 1 执行记录

执行日期：2026-07-09。

代码改动：

- 在 `process_metrics_batch` 中增加 decode/prepare、storage 总耗时、成功/失败 chunk 数的 `tracing::debug!` 日志。
- 在 `insert_metrics_chunk` 中增加 transaction begin、raw events insert、tool-model insert、daily rollup upsert、commit、total 的分段耗时日志。
- `insert_metrics_tool_model_events_chunk` 返回实际 tool-model row 数。
- `upsert_metrics_daily_rollups` 返回实际 daily rollup row 数。
- `METRICS_WRITE_ROLLUPS=false` 时 `daily_rollup_rows=0`、`daily_rollup_upsert_ms=0.0`，避免把分支判断时间误记为 rollup 写入时间。

启用日志方式：

```bash
RUST_LOG="git_ai_enterprise_server=info,git_ai_enterprise_server::services::metrics=debug"
```

验证命令：

| 命令 | 结果 |
| --- | --- |
| `rustfmt --edition 2024 --check src/services/metrics.rs` | 通过 |
| `cargo test metrics` | 通过，17 passed |
| `cargo test` | 通过，113 passed |
| `cargo build --bin git-ai-enterprise-server` | 通过；仅存在既有 warning |

压测环境：

| 项 | 值 |
| --- | --- |
| API | 本地 debug 二进制，`127.0.0.1:43140` |
| 数据库 | 本地 Docker Postgres，`127.0.0.1:5433` |
| 参数 | 500 requests / 50 concurrency / batch=100 / 50000 events |
| 日志 | `/tmp/git-ai-metrics-rollups-true.log`、`/tmp/git-ai-metrics-rollups-false.log` |

请求级压测结果：

| 配置 | 成功/错误 | RPS | p50 | p95 | p99 | max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `METRICS_WRITE_ROLLUPS=true` | 500/0 | 27.50 req/s | 1440.09ms | 3589.87ms | 5074.11ms | 8429.10ms |
| `METRICS_WRITE_ROLLUPS=false` | 500/0 | 57.68 req/s | 689.42ms | 1543.15ms | 1616.33ms | 1718.95ms |

chunk 写入分段耗时：

| 配置 | 阶段 | avg | p50 | p95 | p99 | max |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| rollups=true | transaction begin | 309.74ms | 278.30ms | 750.86ms | 884.82ms | 1010.57ms |
| rollups=true | raw events insert | 38.87ms | 31.07ms | 82.47ms | 163.42ms | 221.28ms |
| rollups=true | tool-model insert | 12.55ms | 10.35ms | 23.92ms | 41.09ms | 134.11ms |
| rollups=true | daily rollup upsert | 611.18ms | 249.16ms | 2225.92ms | 3659.93ms | 7478.40ms |
| rollups=true | commit | 5.22ms | 3.90ms | 10.82ms | 24.28ms | 51.63ms |
| rollups=true | total | 977.55ms | 728.50ms | 2679.96ms | 4626.58ms | 7655.78ms |
| rollups=false | transaction begin | 118.52ms | 79.38ms | 399.70ms | 459.88ms | 498.84ms |
| rollups=false | raw events insert | 121.18ms | 104.84ms | 264.10ms | 385.29ms | 539.71ms |
| rollups=false | tool-model insert | 37.01ms | 26.04ms | 89.92ms | 211.60ms | 254.46ms |
| rollups=false | daily rollup upsert | 0.00ms | 0.00ms | 0.00ms | 0.00ms | 0.00ms |
| rollups=false | commit | 32.30ms | 17.74ms | 98.54ms | 245.33ms | 348.43ms |
| rollups=false | total | 309.01ms | 238.59ms | 655.87ms | 874.14ms | 920.57ms |

阶段 1 结论：

- 同步 `metrics_daily_rollups` upsert 是当前 metrics upload 的主要尾延迟来源：开启 rollup 时 daily rollup upsert p95 `2225.92ms`，chunk total p95 `2679.96ms`。
- 关闭同步 rollup 后，请求 p95 从 `3589.87ms` 降至 `1543.15ms`，chunk total p95 从 `2679.96ms` 降至 `655.87ms`。
- raw events 和 tool-model 明细写入不是当前最大瓶颈，应优先进入阶段 2，把 rollup 写入从请求同步路径移出。

## 阶段 2: metrics rollup 异步化

目标：把 daily rollup 从请求同步路径移出，降低 metrics upload 尾延迟，同时保持 dashboard rollup 查询最终一致。

推荐方案：dirty scope 追加队列 + 后台重建。

设计思路：

- metrics upload 请求路径继续同步写：
  - `metrics_events`
  - `metrics_tool_model_events`
- 请求路径不再同步 upsert `metrics_daily_rollups`。
- 请求路径只把受影响的 `(day, org_id, user_id)` 追加写入 dirty queue。
- 后台 worker 批量读取 dirty queue，在事务内按 day/org/user 去重后从明细表重新聚合并覆盖对应 rollup。
- 该方案是幂等重建，不会因为 worker 重试导致 rollup 重复累加。
- 不使用 `(day, org_id, user_id)` 唯一键做请求路径 upsert；本地压测证明该方案会在 worker 持锁重建时让 upload 请求等待唯一键冲突，导致连接池超时。

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
- `docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md`

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

- [x] 如果 `METRICS_ROLLUP_WRITE_MODE` 未设置，继续使用现有 `METRICS_WRITE_ROLLUPS` 的语义。
- [x] 初始默认仍为 `sync`，先不改变生产行为。
- [x] `.env.example` 和 deploy `.env.example` 写清楚推荐灰度方式。

### 2.2 新增 dirty scope 表

迁移建议：

```sql
CREATE TABLE IF NOT EXISTS metrics_rollup_dirty_scopes (
    id BIGSERIAL PRIMARY KEY,
    day DATE NOT NULL,
    org_id UUID NOT NULL,
    user_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_metrics_rollup_dirty_claim
    ON metrics_rollup_dirty_scopes (claimed_at NULLS FIRST, id);

CREATE INDEX IF NOT EXISTS idx_metrics_rollup_dirty_scope
    ON metrics_rollup_dirty_scopes (day, org_id, user_id);
```

实现步骤：

- [x] 新增本地和 deploy 迁移。
- [x] 注册迁移。
- [x] 增加迁移测试，确认表和索引存在。

测试命令：

```bash
cd enterprise-server
cargo test db::migrations
cargo run -- --migrate
```

### 2.3 请求路径标记 dirty scope

实现步骤：

- [x] 在 `insert_metrics_chunk` 中根据配置分支：
  - `sync`: 继续调用 `upsert_metrics_daily_rollups`
  - `dirty_async`: 调用 `mark_metrics_rollup_dirty_scopes`
  - `off`: 不写 rollup，不标记 dirty
- [x] `mark_metrics_rollup_dirty_scopes` 从本 chunk 的 rows 提取唯一 `(day, org_id, user_id)`。
- [x] 使用 bulk insert 追加 dirty queue：

```sql
INSERT INTO metrics_rollup_dirty_scopes (day, org_id, user_id)
VALUES ...
```

验收标准：

- [x] `sync` 模式行为完全保持。
- [x] `dirty_async` 模式下 metrics upload 响应成功，dirty 表出现记录。
- [x] `off` 模式下不写 rollup、不写 dirty。

### 2.4 实现 rollup rebuild worker

实现步骤：

- [x] 增加服务函数：

```rust
pub async fn process_metrics_rollup_dirty_scopes(pool: &PgPool, batch_size: i64) -> Result<u64, AppError>
```

- [x] 使用事务领取 dirty scope：

```sql
SELECT id, day, org_id, user_id
FROM metrics_rollup_dirty_scopes
WHERE claimed_at IS NULL OR claimed_at < now() - interval '5 minutes'
ORDER BY id
LIMIT $1
FOR UPDATE SKIP LOCKED;
```

- [x] 对每个 scope，在同一事务内：
  - 删除 `metrics_daily_rollups` 中对应 day/org/user 的旧 rollup。
  - 从 `metrics_events` 和 `metrics_tool_model_events` 聚合生成新 rollup。
  - 插入新的 `metrics_daily_rollups`。
  - 删除已领取的 dirty queue row。
- [x] 在 `main.rs` 中按配置启动后台 task。
- [x] worker 失败只记录 warn，不影响 API 主进程。

验收标准：

- [x] worker 可重复执行，结果不重复累加。
- [x] worker 中途失败后 dirty scope 不丢失，可重试。
- [x] dashboard rollup 数据与 sync 模式结果一致。

### 2.5 压测和灰度

压测步骤：

- [x] `sync` 模式跑 metrics upload。
- [x] `dirty_async` 模式跑相同参数。
- [x] 压测后等待 worker 追平 dirty scope。
- [x] 跑 dashboard benchmark，确认数据可见。

验收标准：

- [x] metrics upload p95 明显低于 `sync` 模式。
- [x] `metrics_rollup_dirty_scopes` 可在压测后归零或保持低水位。
- [x] dashboard rollup 查询结果与 sync 模式误差为 0。
- [x] health/ready 不受 worker 明显影响。

### 阶段 2 执行记录

代码改动：

- 新增 `METRICS_ROLLUP_WRITE_MODE=sync|dirty_async|off`，保留 `METRICS_WRITE_ROLLUPS` 作为未设置新变量时的兼容语义。
- 新增 `METRICS_ROLLUP_WORKER_ENABLED`、`METRICS_ROLLUP_WORKER_INTERVAL_SECONDS`、`METRICS_ROLLUP_WORKER_BATCH_SIZE`。
- 新增 `metrics_rollup_dirty_scopes` 追加队列表，字段为 `id/day/org_id/user_id/created_at/claimed_at`，并增加领取索引和 scope 查询索引。
- `dirty_async` 模式下请求事务只写明细表和追加 dirty queue，不再同步 upsert `metrics_daily_rollups`。
- 后台 worker 按 `FOR UPDATE SKIP LOCKED` 领取 dirty queue，按 scope 去重后重建 daily rollup，并删除已领取 queue rows。

验证命令：

```bash
cd enterprise-server
cargo test

ENTERPRISE_API_KEYS=... \
python3 scripts/benchmarks/enterprise/bench_metrics_upload.py \
  --base-url http://127.0.0.1:43140 \
  --requests 500 \
  --batch-size 100 \
  --concurrency 50 \
  --start-seed 202607116000

ENTERPRISE_API_KEYS=... \
python3 scripts/benchmarks/enterprise/bench_dashboard.py \
  --base-url http://127.0.0.1:43140 \
  --requests 300 \
  --concurrency 20 \
  --days 30
```

测试结果：

- `cargo test`：117 passed，0 failed。
- metrics upload `dirty_async`：500 requests，500 success，0 errors，RPS `54.57`，avg `883.82ms`，p50 `800.83ms`，p95 `1728.25ms`，p99 `1818.47ms`。
- 同步 rollup 基线：500 requests，500 success，0 errors，RPS `27.52`，avg `1748.90ms`，p95 `3589.87ms`，p99 `5074.11ms`。
- upload p95 从 `3589.87ms` 降至 `1728.25ms`，约降低 `51.9%`；RPS 从 `27.52` 提升至 `54.57`，约提升 `98.3%`。
- 请求内分段耗时：dirty queue 标记 p95 `17.55ms`，chunk total p95 `711.65ms`；同步 rollup 的 daily rollup upsert p95 为 `2225.92ms`。
- 压测后 `metrics_rollup_dirty_scopes` 短暂剩余 `194` 行，worker 随后自然追平到 `0`。
- 压测后 health 正常，响应约 `5ms`；服务日志没有连接池超时或 worker 失败。

设计调整记录：

- 初版唯一键 dirty scope 在 50 并发 upload 下出现连接池超时：500 requests 中 474 success、26 errors，p95 `5567ms`。
- 将唯一键 upsert 改为 `ON CONFLICT DO NOTHING` 仍会被 worker 的 row lock 和唯一键冲突拖慢：500 requests 中 469 success、31 errors，p95 `8075ms`。
- 最终采用追加队列，避免请求路径等待 worker 正在处理的 scope 行锁。

残留问题：

- dashboard benchmark 全部成功但仍慢：summary p95 `10657.70ms`，trends p95 约 `4.3s`，tools p95 `372.26ms`。
- 该慢点发生在 dirty queue 已归零、worker 无 backlog 的情况下，属于 dashboard 聚合查询路径问题，不是阶段 2 的 rollup 异步化请求路径问题。
- 后续如果继续优化并发体感，优先级应转向 dashboard summary/trends 查询计划和索引，而不是继续压缩 metrics upload 的 rollup 写入。

提交建议：

```bash
git add enterprise-server/migrations/019_metrics_rollup_dirty_scopes.sql enterprise-server/deploy/migrations/019_metrics_rollup_dirty_scopes.sql enterprise-server/src/db/migrations.rs enterprise-server/src/config.rs enterprise-server/src/main.rs enterprise-server/src/services/metrics.rs enterprise-server/.env.example enterprise-server/deploy/.env.example docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Add async metrics rollup rebuild mode"
```

## 阶段 3: 优化 metrics tool-model 明细写入

目标：如果阶段 1 显示 `metrics_tool_model_events` 写入仍是主要耗时，则继续减少 tool-model 明细写入放大。

阶段 2 之后的判断：`tool_model_insert_ms` p95 已降到约 `101.85ms`，不是当前最大瓶颈；本阶段只做参数上限保护和不退化验证，不做激进索引调整。

涉及文件：

- `enterprise-server/src/services/metrics.rs`
- 可选：`enterprise-server/migrations/020_metrics_tool_model_write_indexes.sql`
- `scripts/benchmarks/enterprise/bench_metrics_upload.py`
- `docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md`

### 3.1 记录 tool-model 行数放大

实现步骤：

- [x] 在阶段 1 的阶段耗时日志中增加 `tool_model_rows`。
- [x] benchmark 输出或日志记录：
  - events 数。
  - tool-model rows 数。
  - tool-model rows/event。

验收标准：

- [x] 能判断真实客户端 payload 下 row multiplier。

### 3.2 优化 bulk insert chunk

步骤：

- [x] 检查 `insert_metrics_tool_model_events_chunk` 的 SQL 参数数量，确保不会接近 Postgres 参数上限。
- [x] 如果单 chunk 参数过多，单独为 tool-model rows 分 chunk。
- [x] 保持 event chunk size 为 `500`，仅对放大后的 tool-model rows 按参数上限分块。
- [ ] 如果后续写入仍是瓶颈，再对比 event chunk size：
  - 100 events
  - 250 events
  - 500 events
- [ ] 选择 p95 最稳定的 event chunk。

验收标准：

- [x] metrics upload 不出现 DB 参数上限错误。
- [x] p95/p99 不比当前 500-event chunk 更差。

### 3.3 评估明细表索引写入成本

步骤：

- [ ] 通过 `pg_stat_statements` 和 `EXPLAIN (ANALYZE, BUFFERS)` 确认写入期间最重索引。
- [ ] 检查 `metrics_tool_model_events` 是否存在过多非必要索引。
- [ ] 对 dashboard 必需索引和写入成本做取舍。

验收标准：

- [ ] 不删除 dashboard 必需索引。
- [ ] 如果删除或调整索引，dashboard benchmark 必须无退化。

### 阶段 3 执行记录

代码改动：

- 保持 metrics event chunk size 为 `500`，该路径每 event 约 `25` 个 bind，单条 INSERT 约 `12,500` 个 bind，低于 Postgres `65,535` 参数上限。
- 新增 tool-model 明细行独立 chunk：每行 `10` 个 bind，单 chunk `5,000` 行，最大约 `50,000` 个 bind。
- 将 `insert_metrics_tool_model_events_chunk` 拆成准备 rows 和实际 rows chunk INSERT 两层。
- 新增测试 `process_metrics_batch_chunks_large_tool_model_rows`，用 `5,001` 条 tool-model 明细验证会跨 chunk 写入且总数正确。

验证命令：

```bash
cd enterprise-server
cargo test metrics
cargo build --bin git-ai-enterprise-server

ENTERPRISE_API_KEYS=... \
python3 scripts/benchmarks/enterprise/bench_metrics_upload.py \
  --base-url http://127.0.0.1:43140 \
  --requests 500 \
  --batch-size 100 \
  --concurrency 50 \
  --start-seed 202607117000
```

测试结果：

- `cargo test metrics`：22 passed，0 failed。
- `cargo build --bin git-ai-enterprise-server`：通过。
- metrics upload：500 requests，500 success，0 errors，RPS `70.05`，avg `685.04ms`，p50 `671.44ms`，p95 `935.67ms`，p99 `1021.72ms`。
- 阶段 2 `dirty_async` 对照：p95 `1728.25ms`，p99 `1818.47ms`。
- 阶段 3 分段耗时：tool-model insert p95 `59.80ms`，dirty queue 标记 p95 `14.29ms`，chunk total p95 `396.17ms`。
- 压测后 `metrics_rollup_dirty_scopes` 从短暂 `52` 行追平到 `0`；服务日志没有连接池超时或 worker 失败。

阶段结论：

- tool-model 明细写入已具备参数上限保护，真实 payload 出现较高 row multiplier 时不会依赖单条超大 INSERT。
- 目前 upload 写入瓶颈已经不在 tool-model 明细表；继续优化并发体感时，应优先处理 dashboard summary/trends 的慢查询。
- 阶段 3.3 索引成本评估暂不执行：当前没有证据显示 tool-model 索引写入是主瓶颈，贸然删索引会增加 dashboard 查询风险。

提交建议：

```bash
git add enterprise-server/src/services/metrics.rs docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Tune metrics tool model event writes"
```

## 阶段 4: 登录链路观测和保护

目标：登录当前可以支撑 100 并发，但需要观测 Argon2 队列等待，避免生产盲调 `AUTH_PASSWORD_CONCURRENCY`。

涉及文件：

- `enterprise-server/src/services/passwords.rs`
- `enterprise-server/src/handlers/auth_api.rs`
- `enterprise-server/src/config.rs`
- `scripts/benchmarks/enterprise/bench_auth_login.py`
- `docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md`

### 4.1 增加密码计算阶段耗时

实现步骤：

- [x] 在 `hash_password_blocking` 和 `verify_password_blocking` 内记录：
  - semaphore acquire 等待耗时。
  - `spawn_blocking` 内 Argon2 执行耗时。
  - 总耗时。
- [x] 日志字段包括 operation=`hash|verify` 和 configured concurrency。
- [x] 不记录密码、hash、邮箱等敏感数据。

验收标准：

- [x] 登录压测时能看到队列等待是否超过 Argon2 执行时间。
- [x] 不泄露认证敏感信息。

### 4.2 增加账号维度保护

目标：避免单个账号被高并发重复登录拖慢整体 Argon2 队列。

实现步骤：

- [x] 评估是否使用 Redis 增加 email hash 维度的短窗口限流。
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

- [x] 在 deploy README 或部署文档中记录：
  - 默认 `AUTH_PASSWORD_CONCURRENCY=8`。
  - 4-8 核实例可灰度 `12`。
  - 不建议直接 `16+`，除非有压测证明 p95/p99 不恶化。
- [x] 增加压测模板命令。

验收标准：

- [x] 文档说明清楚如何灰度和回滚。

### 阶段 4 执行记录

代码改动：

- `hash_password_blocking` 和 `verify_password_blocking` 统一通过 `run_password_operation_blocking` 记录阶段耗时。
- 新增日志字段：`operation`、`configured_concurrency`、`acquire_wait_ms`、`argon_ms`、`total_ms`、`result`。
- 登录和注册 handler 传入 `state.config.auth_password_concurrency`，确保日志记录的是配置并发度。
- 日志不包含密码、password hash、邮箱、用户 id 或账号是否存在。
- `enterprise-server/deploy/README.md` 增加 `AUTH_PASSWORD_CONCURRENCY` 灰度建议、回滚方式和登录压测模板。

验证命令：

```bash
cd enterprise-server
cargo test password
cargo test auth_api
cargo build --bin git-ai-enterprise-server

python3 scripts/benchmarks/enterprise/bench_auth_login.py \
  --base-url http://127.0.0.1:43141 \
  --mode login \
  --login-user-count 100 \
  --login-email-domain linewell.com \
  --login-email-prefix bench-login-pool \
  --login-run-id 20260709-001 \
  --login-password correct-horse-battery \
  --requests 100 \
  --concurrency 20 \
  --client-ip-mode pool \
  --client-ip-pool-size 100
```

测试结果：

- `cargo test password`：6 passed，0 failed。
- `cargo test auth_api`：16 passed，0 failed。
- `cargo build --bin git-ai-enterprise-server`：通过。
- 登录压测：100 requests，100 success，0 errors，RPS `8.13`，p50 `2127.01ms`，p95 `2887.00ms`，p99 `3096.93ms`，HTTP 200=100，401/409/429/500=0。
- password timing 日志：100 条 `operation="verify"` 样本；`acquire_wait_ms` p95 `1897.79ms`，`argon_ms` p95 `1046.62ms`，`total_ms` p95 `2850.54ms`。

阶段结论：

- 新增观测已经能判断 Argon2 semaphore 队列等待是否超过实际密码计算时间。
- 本轮 `AUTH_PASSWORD_CONCURRENCY=8`、20 并发登录下，排队等待 p95 已高于 Argon2 执行 p95，说明登录尾延迟主要来自密码计算队列。
- 暂不直接实现账号维度限流：现有 auth tier 已按 client/IP 做全局保护；账号维度保护需要先明确“限制所有登录尝试”还是“只限制失败尝试”。如果在查询/验证前限制所有登录尝试，能保护 Argon2 队列但可能误伤同账号多设备；如果只限制失败登录，则无法阻止失败请求进入 Argon2 verify。后续应单独设计带配置开关的 email-hash tier。

提交建议：

```bash
git add enterprise-server/src/services/passwords.rs enterprise-server/src/handlers/auth_api.rs enterprise-server/src/config.rs docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md
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
- `docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md`

### 5.1 保留对象存储并发，合并 DB 写入

实现步骤：

- [x] 将每对象处理拆成两段：
  - 并发执行 S3/MinIO put。
  - 收集成功对象后，按 batch 批量写 DB。
- [x] 新增 `insert_cas_db_rows_chunk`：
  - bulk insert `cas_objects`
  - bulk insert `cas_ownership`
  - 单 transaction 覆盖整个成功对象集合或分 chunk
- [x] 保持 partial failure 语义：
  - S3 失败的对象返回 error。
  - S3 成功但 DB batch 失败的对象返回 error。
- [x] DB 写入失败时记录足够日志，方便定位。

验收标准：

- [x] CAS hash mismatch 仍在写 S3 前失败。
- [x] 同 hash 同内容并发上传仍幂等。
- [x] S3 失败不留下 DB ready 记录。
- [x] CAS 2000 objects 压测 p95 低于当前 `1472.66ms` 或 DB transaction 数明显下降。

阶段 5 执行记录：

- `process_cas_uploads` 保留 bounded concurrency 的对象存储上传，上传成功后统一进入 `insert_cas_db_rows`。
- `insert_cas_db_rows` 在单个 Postgres transaction 内按 chunk bulk insert `cas_objects` 和 `cas_ownership`，将每请求 DB transaction 数从“成功对象数”降到 1 个。
- 新增 DB batch 失败回归测试：对象已写入 CAS store 但 DB FK/transaction 失败时，该对象返回 `error`，且不留下 `cas_objects` / `cas_ownership` 记录。
- 验证：`cargo test cas`、`cargo test` 通过。

测试命令：

```bash
cd enterprise-server
cargo test cas
cargo test
```

提交建议：

```bash
git add enterprise-server/src/handlers/cas.rs docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md
git commit -m "Batch database writes for CAS uploads"
```

## 阶段 6: 部署容量和横向扩容验证

目标：确认单实例优化后，服务能通过多实例扩容继续提升并发，而不是被连接池、Redis、MinIO 或负载均衡配置卡住。

涉及文件：

- `enterprise-server/deploy/README.md`
- `enterprise-server/deploy/docker-compose.yml`
- `docs/enterprise/enterprise-server-deployment.md`
- `docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md`

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
git add enterprise-server/deploy/README.md docs/enterprise/enterprise-server-deployment.md docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md
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
git add docs/enterprise/enterprise-server-next-concurrency-optimization-task-plan.md
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
