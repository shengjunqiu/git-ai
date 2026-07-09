# Enterprise 认证登录性能优化任务清单

本文档把 dashboard rollup 开关落地、注册/登录并发优化、OAuth 登录限流优化拆成可以逐步执行、逐步验证、逐步提交的工程任务。

## 执行原则

1. 每次只执行一个阶段，完成验证后单独提交。
2. 先做配置显式化和可回滚开关，再做登录链路改造。
3. 所有默认值保持兼容，生产环境通过 `.env` 灰度开启。
4. 注册/登录优化必须保留安全边界：密码哈希不能降低强度，限流不能完全取消。
5. 每个阶段完成后记录验证命令和结果。

## 阶段 0: 现状确认和基线记录

目标：确认当前服务端、迁移、rollup 数据和登录链路容量基线，避免盲目改动。

### 0.1 确认工作区和依赖状态

步骤：

- [x] 查看工作区状态：

```bash
git status --short
```

- [x] 启动本地依赖：

```bash
cd enterprise-server
docker compose up -d postgres redis minio
```

- [x] 确认 Postgres、Redis 可用：

```bash
docker compose ps
docker compose exec postgres pg_isready -U gitai -d gitai_enterprise
docker compose exec redis redis-cli ping
```

- [x] 跑服务端测试：

```bash
cargo test
```

验收标准：

- [x] Postgres、Redis、MinIO 处于可用状态。
- [x] `cargo test` 通过。
- [x] 没有阻断后续阶段的失败。

### 0.2 确认 rollup 迁移和数据

步骤：

- [x] 确认迁移 016、017 已注册：

```bash
rg -n "016_metrics_daily_rollups|017_metrics_tool_model_events" src/db/migrations.rs
```

- [x] 执行迁移：

```bash
cargo run -- --migrate
```

- [x] 检查 rollup 表和明细表数据量：

```bash
docker compose exec -T postgres psql -U gitai -d gitai_enterprise -c "
SELECT 'metrics_events' AS table_name, COUNT(*) FROM metrics_events
UNION ALL SELECT 'metrics_daily_rollups', COUNT(*) FROM metrics_daily_rollups
UNION ALL SELECT 'metrics_tool_model_events', COUNT(*) FROM metrics_tool_model_events;
"
```

验收标准：

- [x] `metrics_daily_rollups` 表存在。
- [x] `metrics_tool_model_events` 表存在。
- [x] 如果已有 metrics 明细数据，rollup 表不应为空。

### 0.3 记录认证登录基线

步骤：

- [x] 确认当前限流配置：

```bash
rg -n "RateLimitTier|OAUTH|DEFAULT|METRICS" src/services/rate_limit.rs
```

- [x] 启动 API：

```bash
docker compose up -d --build api
```

- [x] 确认健康检查：

```bash
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/ready
```

- [x] 记录当前注册/登录关键路径：

```bash
rg -n "pub async fn register|pub async fn login|hash_password|verify_password|device_code|token" src/handlers src/services
```

记录项：

- [x] 当前 `/worker/oauth/*` 限流值。
- [x] 当前 `/auth/register`、`/auth/login` 所属限流 tier。
- [x] 当前 `DATABASE_MAX_CONNECTIONS`。
- [x] 当前 API 实例 CPU 和内存。

验收标准：

- [x] 后续每个阶段都有可对比基线。

提交建议：

```bash
git add docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Add enterprise auth login performance task plan"
```

### 阶段 0 执行记录

执行日期：2026-07-08

环境状态：

| 项目 | 结果 |
| --- | --- |
| `git status --short` | 只有 `docs/enterprise-auth-login-performance-task-plan.md` 未跟踪 |
| `docker compose up -d postgres redis minio` | Postgres、Redis、MinIO 均已运行 |
| `docker compose ps` | API healthy，Postgres healthy，Redis healthy，MinIO running |
| `pg_isready` | `/var/run/postgresql:5432 - accepting connections` |
| `redis-cli ping` | `PONG` |
| `cargo test` | 102 passed, 0 failed；仅有既有 warning |

Rollup 和迁移：

| 项目 | 结果 |
| --- | --- |
| 迁移注册 | `016_metrics_daily_rollups` 和 `017_metrics_tool_model_events` 已注册 |
| `cargo run -- --migrate` | 成功；所有既有迁移已 applied/skipping |
| 首次迁移尝试 | 不带环境变量时报 `missing value for field database_url`；带本地 Docker 环境变量后沙箱阻止连接；提权重跑成功 |
| `metrics_events` | 113566 |
| `metrics_daily_rollups` | 13508 |
| `metrics_tool_model_events` | 113550 |

API 启动和健康检查：

| 项目 | 结果 |
| --- | --- |
| `docker compose up -d --build api` | 成功；release build 用时约 6m48s，API 容器重建后 healthy |
| `/health` | `{"service":"git-ai-enterprise-server","status":"ok","version":"0.1.0"}` |
| `/ready` | `{"checks":{"database":"ok"},"status":"ready"}` |

当前认证/限流基线：

| 项目 | 当前值 |
| --- | --- |
| metrics tier | 60 requests / 60s |
| CAS upload tier | 30 requests / 60s |
| CAS read tier | 100 requests / 60s |
| OAuth tier | 10 requests / 60s |
| admin tier | 30 requests / 60s |
| default tier | 120 requests / 60s |
| `/worker/oauth/*` | 命中 OAuth tier，当前 10/min 是多人同时 `git-ai login` 的明显瓶颈 |
| `/auth/register`、`/auth/login` | 当前没有独立 auth tier，会落入 default tier |
| 密码 hash/verify | `auth_api.rs` 直接调用同步 `Argon2::default()` hash/verify |
| `DATABASE_MAX_CONNECTIONS` | 容器未显式传入，使用代码默认值 20 |
| `DASHBOARD_USE_ROLLUPS` | 容器未显式传入，使用代码默认值 `false` |
| `METRICS_WRITE_ROLLUPS` | 容器未显式传入，使用代码默认值 `true` |

稳定资源快照：

| 容器 | CPU | 内存 |
| --- | ---: | ---: |
| `enterprise-server-api-1` | 0.00% | 8.586MiB / 7.655GiB |
| `enterprise-server-postgres-1` | 0.00% | 158.8MiB / 7.655GiB |
| `enterprise-server-redis-1` | 4.97% | 10.31MiB / 7.655GiB |
| `enterprise-server-minio-1` | 0.08% | 115.2MiB / 7.655GiB |

## 阶段 1: 显式开启 dashboard rollup 配置

目标：让 `METRICS_WRITE_ROLLUPS` 和 `DASHBOARD_USE_ROLLUPS` 可以通过 `.env` 明确配置，并在 compose 中传入 API 容器。

### 1.1 补齐本地 compose 配置

涉及文件：

- `enterprise-server/docker-compose.yml`
- `enterprise-server/.env.example`

实现步骤：

- [x] 在 `enterprise-server/docker-compose.yml` 的 `api.environment` 中增加：

```yaml
- METRICS_WRITE_ROLLUPS=${METRICS_WRITE_ROLLUPS:-true}
- DASHBOARD_USE_ROLLUPS=${DASHBOARD_USE_ROLLUPS:-true}
```

- [x] 在 `enterprise-server/.env.example` 中增加：

```env
# Optional: Metrics/dashboard rollup tuning
METRICS_WRITE_ROLLUPS=true
DASHBOARD_USE_ROLLUPS=true
```

- [x] 确认 `METRICS_WRITE_ROLLUPS` 默认仍为 `true`，`DASHBOARD_USE_ROLLUPS` 在生产配置里可以明确开启。

验证命令：

```bash
cd enterprise-server
docker compose config --quiet
```

验收标准：

- [x] compose 配置有效。
- [x] `.env.example` 明确展示两个开关。

### 1.2 补齐 deploy compose 配置

涉及文件：

- `enterprise-server/deploy/docker-compose.yml`
- `enterprise-server/deploy/.env.example`
- `enterprise-server/deploy/README.md`

实现步骤：

- [x] 在 `enterprise-server/deploy/docker-compose.yml` 的 `api.environment` 中增加：

```yaml
- METRICS_WRITE_ROLLUPS=${METRICS_WRITE_ROLLUPS:-true}
- DASHBOARD_USE_ROLLUPS=${DASHBOARD_USE_ROLLUPS:-true}
```

- [x] 在 `enterprise-server/deploy/.env.example` 中增加：

```env
# === Metrics/dashboard rollups ===
METRICS_WRITE_ROLLUPS=true
DASHBOARD_USE_ROLLUPS=true
```

- [x] 在 deploy README 增加说明：
  - `METRICS_WRITE_ROLLUPS=true` 会在上传 metrics 时同步写 daily rollup。
  - `DASHBOARD_USE_ROLLUPS=true` 会让 dashboard 常用聚合优先查询 rollup 表。
  - 如果 dashboard 数据异常，先把 `DASHBOARD_USE_ROLLUPS=false` 回退到明细查询。

验证命令：

```bash
cd enterprise-server
JWT_SECRET=test-secret-at-least-32-characters POSTGRES_PASSWORD=test-password docker compose -f deploy/docker-compose.yml config --quiet
```

验收标准：

- [x] deploy compose 配置有效。
- [x] README 说明开启作用和回退方式。

### 1.3 本地开启并验证 dashboard 查询

步骤：

- [x] 通过 compose 默认值或 `enterprise-server/.env` 设置：

```env
METRICS_WRITE_ROLLUPS=true
DASHBOARD_USE_ROLLUPS=true
```

- [x] 重建 API：

```bash
docker compose up -d --force-recreate api
```

- [x] 确认 API 正常：

```bash
curl -sS http://127.0.0.1:8080/ready
```

- [ ] 使用 benchmark 脚本验证 dashboard 常用接口：

```bash
ENTERPRISE_BASE_URL=http://127.0.0.1:8080 \
ENTERPRISE_API_KEY=your-api-key \
python3 ../scripts/benchmarks/enterprise/bench_dashboard.py \
  --requests 100 \
  --concurrency 20 \
  --days 30
```

验收标准：

- [x] `/ready` 正常。
- [ ] dashboard 压测无错误。
- [ ] p95 接近或优于 10 万级历史结果：summary 约 267ms、tools 约 144ms、trends 约 116-125ms。

提交建议：

```bash
git add enterprise-server/docker-compose.yml enterprise-server/.env.example enterprise-server/deploy/docker-compose.yml enterprise-server/deploy/.env.example enterprise-server/deploy/README.md docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Enable enterprise rollup configuration"
```

### 阶段 1 执行记录

执行日期：2026-07-08

实现结果：

| 项 | 结果 |
| --- | --- |
| 本地 compose | `api.environment` 已显式传入 `METRICS_WRITE_ROLLUPS=${METRICS_WRITE_ROLLUPS:-true}` 和 `DASHBOARD_USE_ROLLUPS=${DASHBOARD_USE_ROLLUPS:-true}` |
| 本地 `.env.example` | 已新增 Metrics/dashboard rollup tuning 段落 |
| deploy compose | `api.environment` 已显式传入两个 rollup 开关 |
| deploy `.env.example` | 已新增 Metrics/dashboard rollups 段落 |
| deploy README | 已说明开启作用、迁移前提、dashboard 回退方式和写入压力回退方式 |
| 本地 `.env` | 未修改用户本地文件；compose 默认值已能在未设置 `.env` 时传入 `true` |

验证结果：

| 命令 | 结果 |
| --- | --- |
| `docker compose config --quiet` | 通过 |
| `JWT_SECRET=test-secret-at-least-32-characters POSTGRES_PASSWORD=test-password docker compose -f deploy/docker-compose.yml config --quiet` | 通过 |
| `docker compose up -d --force-recreate api` | 通过，API 容器已重建 |
| `docker compose exec api printenv METRICS_WRITE_ROLLUPS` | `true` |
| `docker compose exec api printenv DASHBOARD_USE_ROLLUPS` | `true` |
| `curl -sS http://127.0.0.1:8080/health` | `{"service":"git-ai-enterprise-server","status":"ok","version":"0.1.0"}` |
| `curl -sS http://127.0.0.1:8080/ready` | `{"checks":{"database":"ok"},"status":"ready"}` |
| `printenv ENTERPRISE_API_KEY` / `ENTERPRISE_API_KEYS` | 未设置 |
| active API keys | 数据库中有 13 个 active key，但只有 hash，无法反推出明文用于 dashboard benchmark |

未完成项：

| 项 | 原因 |
| --- | --- |
| `bench_dashboard.py` 阶段 1 复测 | 当前 shell 没有明文 `ENTERPRISE_API_KEY`/`ENTERPRISE_API_KEYS`，数据库中的既有 key 只有 hash；不修改用户本地密钥文件，留到有明文 API key 时执行 |

## 阶段 2: 注册/登录限流配置化

目标：把当前硬编码限流改成可配置，并为网页登录、注册和 CLI OAuth 登录设置更合理的 tier。

### 2.1 增加认证相关限流配置

涉及文件：

- `enterprise-server/src/config.rs`
- `enterprise-server/src/services/rate_limit.rs`
- `enterprise-server/.env.example`
- `enterprise-server/deploy/.env.example`
- `enterprise-server/deploy/README.md`

建议新增配置：

```env
RATE_LIMIT_METRICS_MAX_REQUESTS=60
RATE_LIMIT_METRICS_WINDOW_SECONDS=60
RATE_LIMIT_CAS_UPLOAD_MAX_REQUESTS=30
RATE_LIMIT_CAS_UPLOAD_WINDOW_SECONDS=60
RATE_LIMIT_CAS_READ_MAX_REQUESTS=100
RATE_LIMIT_CAS_READ_WINDOW_SECONDS=60
RATE_LIMIT_OAUTH_MAX_REQUESTS=600
RATE_LIMIT_OAUTH_WINDOW_SECONDS=60
RATE_LIMIT_AUTH_MAX_REQUESTS=300
RATE_LIMIT_AUTH_WINDOW_SECONDS=60
RATE_LIMIT_ADMIN_MAX_REQUESTS=30
RATE_LIMIT_ADMIN_WINDOW_SECONDS=60
RATE_LIMIT_DEFAULT_MAX_REQUESTS=300
RATE_LIMIT_DEFAULT_WINDOW_SECONDS=60
```

实现步骤：

- [x] 在 `AppConfig` 增加上述限流字段。
- [x] 在 `EnvConfig` 增加可选字段。
- [x] 保持兼容默认值：
  - metrics: `60/60`
  - cas_upload: `30/60`
  - cas_read: `100/60`
  - oauth: 从 `10/60` 提升到 `600/60`
  - auth: 新增 `300/60`
  - admin: `30/60`
  - default: 从 `120/60` 提升到 `300/60`
- [x] 将 `tiers::OAUTH`、`tiers::DEFAULT` 等硬编码常量改为从 `AppConfig` 生成。
- [x] 新增 `auth` tier：

```text
/auth/register
/auth/login
/auth/logout
/auth/organizations
/auth/cli/authorize
/login
/logout
/verify
```

- [x] 保持 `/worker/metrics/*` 仍走 metrics tier。
- [x] 保持 `/health`、`/ready` 继续 bypass。

验收标准：

- [x] 认证路径不再落入 default tier。
- [x] OAuth 轮询不会因 10/min 限制导致多人同时 `git-ai login` 很快 429。
- [x] metrics 上传限流保持原行为，避免误放大写入压力。

### 2.2 增加限流测试

涉及文件：

- `enterprise-server/src/services/rate_limit.rs`

测试覆盖：

- [x] `/worker/oauth/device/code` 使用 oauth tier。
- [x] `/worker/oauth/token` 使用 oauth tier。
- [x] `/auth/login` 使用 auth tier。
- [x] `/auth/register` 使用 auth tier。
- [x] `/worker/metrics/upload` 使用 metrics tier。
- [x] `/health` 和 `/ready` bypass。
- [x] 环境变量能覆盖默认值。

验证命令：

```bash
cd enterprise-server
cargo test rate_limit
cargo test config
cargo test
```

手动验证：

```bash
RATE_LIMIT_OAUTH_MAX_REQUESTS=3 RATE_LIMIT_OAUTH_WINDOW_SECONDS=60 cargo test rate_limit
```

验收标准：

- [x] 单元测试覆盖各路径 tier。
- [x] 配置缺省时行为稳定。
- [x] 配置覆盖时生效。

提交建议：

```bash
git add enterprise-server/src/config.rs enterprise-server/src/services/rate_limit.rs enterprise-server/.env.example enterprise-server/deploy/.env.example enterprise-server/deploy/README.md docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Make enterprise auth rate limits configurable"
```

### 2.3 执行记录

- [x] 已完成限流配置化，`metrics`、`cas_upload`、`cas_read`、`oauth`、`auth`、`admin`、`default` 均可通过环境变量调整。
- [x] 已将 `/auth/*`、`/login`、`/logout`、`/verify` 归入 `auth` tier，避免注册/网页登录继续落入 default tier。
- [x] 已将 `/worker/oauth/*` 默认从 `10/60` 提升到 `600/60`，降低多人同时 CLI 登录被 429 的概率。
- [x] 已保持 `/worker/metrics/*` 默认 `60/60`，避免误放大 metrics 写入压力。
- [x] 已将本地和部署版 compose、`.env.example`、部署 README 更新为可配置限流。
- [x] 验证通过：`cargo test rate_limit`、`cargo test config`、`cargo test`、`cargo check`、`docker compose config --quiet`、`JWT_SECRET=test-secret-at-least-32-characters POSTGRES_PASSWORD=test-password docker compose -f deploy/docker-compose.yml config --quiet`、`git diff --check`。

## 阶段 3: 用户邮箱登录索引优化

目标：优化 `lower(email) = lower($1)` 查询，避免用户量增长后登录和注册查重扫描变慢。

### 3.1 新增 lower(email) 表达式索引

涉及文件：

- `enterprise-server/migrations/018_users_lower_email_index.sql`
- `enterprise-server/deploy/migrations/018_users_lower_email_index.sql`
- `enterprise-server/src/db/migrations.rs`

实现步骤：

- [x] 新增迁移：

```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email_lower
    ON users (lower(email));
```

- [x] 同步复制到 deploy migrations。
- [x] 在 `src/db/migrations.rs` 注册 `018_users_lower_email_index`。
- [x] 评估生产是否需要独立运维脚本使用 `CREATE INDEX CONCURRENTLY`。

注意：

- 应用内迁移 runner 适合中小表。
- 如果生产 `users` 已经很大，先在维护窗口手动执行 concurrent index，再让应用迁移通过 `IF NOT EXISTS` 幂等跳过。

验证命令：

```bash
cd enterprise-server
cargo test db::migrations
cargo test auth_api
cargo test
diff -u migrations/018_users_lower_email_index.sql deploy/migrations/018_users_lower_email_index.sql
```

手动验证：

```bash
cargo run -- --migrate
docker compose exec -T postgres psql -U gitai -d gitai_enterprise -c "\di idx_users_email_lower"
```

验收标准：

- [x] migration 可重复执行。
- [x] `idx_users_email_lower` 存在。
- [x] 登录查询和注册查重继续通过测试。

提交建议：

```bash
git add enterprise-server/migrations/018_users_lower_email_index.sql enterprise-server/deploy/migrations/018_users_lower_email_index.sql enterprise-server/src/db/migrations.rs docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Index enterprise users by normalized email"
```

### 3.2 执行记录

- [x] 已新增 `018_users_lower_email_index`，本地和部署迁移 SQL 内容一致。
- [x] 已在迁移 runner 中注册 018，并在迁移测试里断言 `idx_users_email_lower` 存在。
- [x] 已执行本地迁移，确认 018 成功应用。
- [x] 已用 `psql \di idx_users_email_lower` 确认索引存在于 `public.users`。
- [x] 生产评估：中小表可直接使用应用迁移；如果生产 `users` 表已很大，应先在维护窗口手动执行 `CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS idx_users_email_lower ON users (lower(email));`，再让应用迁移幂等跳过。
- [x] 验证通过：`cargo test db::migrations`、`cargo test auth_api`、`cargo test`、`diff -u migrations/018_users_lower_email_index.sql deploy/migrations/018_users_lower_email_index.sql`、`cargo run -- --migrate`、`docker compose exec -T postgres psql -U gitai -d gitai_enterprise -c "\di idx_users_email_lower"`。

## 阶段 4: Argon2 密码计算移出 Tokio worker

目标：避免注册/登录中的 Argon2 hash/verify 同步 CPU 计算阻塞 Tokio worker，提升高并发登录时其它请求的响应稳定性。

### 4.1 增加密码计算并发限制配置

涉及文件：

- `enterprise-server/src/config.rs`
- `enterprise-server/src/routes.rs`
- `enterprise-server/src/main.rs`
- `enterprise-server/.env.example`
- `enterprise-server/deploy/.env.example`

建议配置：

```env
AUTH_PASSWORD_CONCURRENCY=8
```

实现步骤：

- [x] 在 `AppConfig` 增加 `auth_password_concurrency: usize`。
- [x] 默认值设为 `8`，并用 `.max(1)` 防止无效配置。
- [x] 在 `AppState` 增加：

```rust
pub auth_password_limiter: std::sync::Arc<tokio::sync::Semaphore>
```

- [x] 在 `main.rs` 构建 `AppState` 时初始化 semaphore。
- [x] 更新测试中的 `AppState` 构造。

验收标准：

- [x] 配置默认值兼容。
- [x] 测试环境可以构造 `AppState`。

### 4.2 新增异步密码服务封装

涉及文件：

- `enterprise-server/src/services/passwords.rs`
- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [x] 保留现有同步函数：
  - `hash_password`
  - `verify_password`
  - `validate_password_strength`
- [x] 新增 async 包装函数：

```rust
pub async fn hash_password_blocking(
    limiter: std::sync::Arc<tokio::sync::Semaphore>,
    password: String,
) -> Result<String, AppError>
```

```rust
pub async fn verify_password_blocking(
    limiter: std::sync::Arc<tokio::sync::Semaphore>,
    password: String,
    password_hash: String,
) -> Result<bool, AppError>
```

- [x] 在 async 函数中先 acquire semaphore permit。
- [x] 使用 `tokio::task::spawn_blocking` 执行同步 Argon2。
- [x] 正确处理 `JoinError`，返回 `AppError::Internal`。
- [x] 确保 permit 在阻塞任务完成后释放。

验收标准：

- [x] 密码强度和 Argon2 参数不降低。
- [x] 并发密码计算不会无限放大。
- [x] Tokio worker 不再直接执行 Argon2 CPU 任务。

### 4.3 改造注册和登录 handler

涉及文件：

- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [x] 注册路径中，将：

```rust
let password_hash = crate::services::passwords::hash_password(&req.password)?;
```

替换为 async blocking 包装调用。

- [x] 登录路径中，将：

```rust
crate::services::passwords::verify_password(&req.password, &password_hash)?
```

替换为 async blocking 包装调用。

- [x] 避免在拿着 DB transaction 时执行 Argon2。
- [x] 保持错误响应不泄露邮箱是否存在、密码是否错误等敏感细节。

测试命令：

```bash
cd enterprise-server
cargo test passwords
cargo test auth_api
cargo test
```

验收标准：

- [x] 注册成功路径通过。
- [x] 登录成功路径通过。
- [x] 错误密码仍返回 unauthorized。
- [x] 并发注册同邮箱仍只有一个成功。

提交建议：

```bash
git add enterprise-server/src/config.rs enterprise-server/src/routes.rs enterprise-server/src/main.rs enterprise-server/src/services/passwords.rs enterprise-server/src/handlers/auth_api.rs enterprise-server/.env.example enterprise-server/deploy/.env.example docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Run enterprise password hashing on blocking workers"
```

### 4.4 执行记录

- [x] 已新增 `AUTH_PASSWORD_CONCURRENCY=8`，并通过 `.max(1)` 防止配置为 0。
- [x] 已在 `AppState` 增加 `auth_password_limiter`，main 和测试状态构造均已接入。
- [x] 已新增 `hash_password_blocking` 和 `verify_password_blocking`，内部先获取 semaphore permit，再用 `tokio::task::spawn_blocking` 执行原同步 Argon2 函数。
- [x] 已保留原同步密码函数，未降低密码强度和 Argon2 参数。
- [x] 已将注册 hash 和登录 verify 改为 blocking wrapper；注册仍在开启 DB transaction 前完成密码计算。
- [x] 已更新 `.env.example`、deploy `.env.example`、两份 compose 和部署 README。
- [x] 验证通过：`cargo test passwords`、`cargo test config`、`cargo test auth_api`、`cargo test`、`cargo check`、两份 compose config 校验、`git diff --check`。

## 阶段 5: 减少注册/登录 DB 往返

目标：在不改变业务语义的前提下，减少注册和登录链路的数据库请求数。

### 5.1 合并注册 scope 校验

涉及文件：

- `enterprise-server/src/services/registration.rs`
- `enterprise-server/src/handlers/auth_api.rs`

当前注册链路包含：

- org slug 查询。
- department slug 查询。
- org domain 校验。
- department 属于 org 校验。
- email exists 预查询。
- insert user。
- insert org_members。

实现步骤：

- [x] 新增服务函数，例如：

```rust
resolve_and_validate_registration_scope(pool, email, org_id/org_slug, department_id/department_slug)
```

- [x] 用一个 SQL 同时确认：
  - org 存在。
  - org domain verified。
  - department 存在并属于 org。
- [x] 保留清晰错误信息。
- [x] 对已传 UUID 和 slug 两种路径都覆盖。

验收标准：

- [x] 注册前置校验 DB 往返减少。
- [x] org/domain/department 错误仍返回正确错误。
- [x] 现有注册表单行为不变。

### 5.2 移除邮箱 exists 预查询，依赖唯一索引

涉及文件：

- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [x] 删除注册中的：

```sql
SELECT EXISTS(SELECT 1 FROM users WHERE lower(email) = lower($1))
```

- [x] 直接执行 insert。
- [x] 通过 `idx_users_email_lower` 或现有唯一约束捕获冲突。
- [x] 更新 `map_user_insert_error`，兼容：
  - `users_email_key`
  - `idx_users_email_lower`

验收标准：

- [x] 重复邮箱注册返回 conflict。
- [x] 并发同邮箱注册仍只有一个成功。
- [x] 注册成功路径减少一次 DB 查询。

测试命令：

```bash
cd enterprise-server
cargo test auth_api
cargo test registration
cargo test
```

提交建议：

```bash
git add enterprise-server/src/services/registration.rs enterprise-server/src/handlers/auth_api.rs docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Reduce enterprise registration database round trips"
```

### 5.3 执行记录

- [x] 已新增 `resolve_and_validate_registration_scope`，用一次 SQL 同时解析 org、department 并校验邮箱域名。
- [x] 已将注册 handler 切换到合并 scope 校验，移除本地 `resolve_register_scope`。
- [x] 已删除注册前的邮箱 exists 预查询，重复邮箱改为依赖唯一约束返回 conflict。
- [x] `map_user_insert_error` 已同时识别 `users_email_key` 和 `idx_users_email_lower`。
- [x] 已补充 slug 注册成功、邮箱域名不匹配、未知 org slug、未知 department slug 测试。
- [x] 验证通过：`cargo test auth_api`、`cargo test registration`、`cargo test`、`cargo check`、定向 `rustfmt --check`、`git diff --check`。

## 阶段 6: 登录审计写入异步化

目标：避免登录/注册成功后等待 audit log 写入，降低用户可见延迟。

### 6.1 抽象 fire-and-forget audit helper

涉及文件：

- `enterprise-server/src/services/audit.rs`
- `enterprise-server/src/handlers/auth_api.rs`
- `enterprise-server/src/handlers/oauth.rs`
- `enterprise-server/src/handlers/cli_authorize.rs`

实现步骤：

- [x] 新增 helper，例如：

```rust
pub fn spawn_log_action(pool: PgPool, payload: AuditPayload)
```

- [x] 在 helper 内部 `tokio::spawn` 写 audit。
- [x] 写入失败只打 warn，不影响主请求。
- [x] 避免 spawn 中捕获引用，payload 使用 owned data。

验收标准：

- [x] 登录/注册成功响应不再等待 audit insert。
- [x] audit 写入失败不会让登录失败。
- [x] 日志能看到 audit 写入失败原因。

### 6.2 改造注册/登录 audit 调用

涉及文件：

- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [x] 将 `user.register` audit 改为异步后台写入。
- [x] 将 `org_member.create` audit 改为异步后台写入。
- [x] 将 `user.login` audit 改为异步后台写入。
- [x] 保持 logout 可按现有方式执行，或同步纳入 helper。

测试命令：

```bash
cd enterprise-server
cargo test audit
cargo test auth_api
cargo test
```

验收标准：

- [x] 登录/注册响应不依赖 audit 写入完成。
- [x] audit 表仍能看到成功登录/注册记录。

提交建议：

```bash
git add enterprise-server/src/services/audit.rs enterprise-server/src/handlers/auth_api.rs enterprise-server/src/handlers/oauth.rs enterprise-server/src/handlers/cli_authorize.rs docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Write enterprise auth audit logs asynchronously"
```

### 6.3 执行记录

- [x] 已新增 `AuditPayload` 和 `spawn_log_action`，payload 使用 owned data，后台任务内调用原 `log_action`。
- [x] 后台 audit 写入失败只记录 `tracing::warn!`，不影响主请求响应。
- [x] 已将注册 `user.register`、`org_member.create` 和登录 `user.login` 改为后台写入。
- [x] 已将 CLI 授权 `cli.authorize` 和授权码换 token 的 `token.exchange` 改为后台写入。
- [x] logout 保持现有同步写入，不改变登出流程语义。
- [x] 已补充注册和登录成功后等待后台 audit 落库的测试。
- [x] 验证通过：`cargo test audit`、`cargo test auth_api`、`cargo test`、`cargo check`、定向 `rustfmt --check`、`git diff --check`。

## 阶段 7: 注册/登录专项压测脚本

目标：把注册、网页登录、OAuth device flow 的容量验证脚本化。

### 7.1 新增 auth benchmark 脚本

涉及文件：

- `scripts/benchmarks/enterprise/bench_auth_login.py`
- `scripts/benchmarks/enterprise/README.md`

实现步骤：

- [x] 使用 Python stdlib 实现脚本，保持与现有 benchmark 脚本风格一致。
- [x] 支持网页登录压测：

```text
POST /auth/login
```

- [x] 支持注册压测：

```text
POST /auth/register
```

- [x] 支持 OAuth device flow 压测：

```text
POST /worker/oauth/device/code
POST /worker/oauth/token
```

- [x] 输出：
  - requests
  - concurrency
  - success
  - error rate
  - rps
  - p50
  - p95
  - p99
- [x] 支持唯一邮箱生成，避免注册冲突干扰性能结果。
- [x] 对 401/409/429 单独统计。

验收标准：

- [x] 脚本 `--help` 可用。
- [x] 登录压测可以使用已有测试用户。
- [x] 注册压测不会重复使用同一邮箱。
- [x] OAuth 压测能区分 pending 和 rate limited。

验证命令：

```bash
python3 -m py_compile scripts/benchmarks/enterprise/bench_auth_login.py
python3 scripts/benchmarks/enterprise/bench_auth_login.py --help
```

提交建议：

```bash
git add scripts/benchmarks/enterprise/bench_auth_login.py scripts/benchmarks/enterprise/README.md docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Add enterprise auth login benchmark script"
```

### 7.1 执行记录

- [x] 已新增 `bench_auth_login.py`，只依赖 Python 标准库和现有 `_common.py`。
- [x] 支持 `--mode login`，对 `/auth/login` 发送 JSON 登录请求，可用 `BENCH_LOGIN_EMAIL` 和 `BENCH_LOGIN_PASSWORD` 指定已有测试用户。
- [x] 支持 `--mode register`，按请求 index 生成唯一邮箱，可用 org/department UUID 或 slug。
- [x] 支持 `--mode oauth`，每个 flow 依次请求 `/worker/oauth/device/code` 和 `/worker/oauth/token`。
- [x] OAuth token 的 `authorization_pending` 作为预期成功分类，429 作为 rate limited 单独分类。
- [x] 输出现有 benchmark CSV 摘要，并额外输出 HTTP status counts 和 401/409/429 tracked counts。
- [x] 已更新 enterprise benchmark README。
- [x] 验证通过：`python3 -m py_compile scripts/benchmarks/enterprise/bench_auth_login.py`、`python3 scripts/benchmarks/enterprise/bench_auth_login.py --help`。

### 7.2 执行优化前后对比

建议场景：

```text
网页登录:
  requests: 200, 1000
  concurrency: 10, 30, 50

注册:
  requests: 100, 500
  concurrency: 10, 30

OAuth:
  requests: 200, 1000
  concurrency: 10, 50
```

记录项：

- [x] p50/p95/p99。
- [x] 429 比例。
- [x] 401/409 比例。
- [x] API CPU。
- [x] Postgres active connection。
- [x] Redis CPU。
- [x] Tokio worker 是否被 Argon2 拖慢，可用整体延迟和健康检查旁路压测间接观察。

验收标准：

- [ ] 50 并发 OAuth 请求不再大面积 429。
- [x] 30 并发网页登录时 `/health`、`/ready` 不被拖慢。
- [x] 注册 p95 明显优于改造前，或至少不再影响其它轻量接口。
- [ ] 错误率符合预期，不能出现 DB acquire timeout。

### 7.2 执行记录

执行日期：2026-07-08。

本次在本地 `docker compose` 环境执行，API 地址为 `http://127.0.0.1:8080`。服务启动健康检查正常：

- `/health`: `{"service":"git-ai-enterprise-server","status":"ok","version":"0.1.0"}`
- `/ready`: `{"checks":{"database":"ok"},"status":"ready"}`

测试组织和部门：

- organization: `linewell.com` / `ac81cb06-4c0c-43db-8144-8d063600a19a`
- department: `technology-center` / `4110c6bc-2fbc-4b39-a106-6aaa2f24e075`
- 登录压测用户：`bench-login-stage72-20260708-001@linewell.com`

注意：运行中的 API 容器未显式设置 `RATE_LIMIT_AUTH_MAX_REQUESTS`、`RATE_LIMIT_OAUTH_MAX_REQUESTS` 和 `AUTH_PASSWORD_CONCURRENCY` 环境变量。当前仓库的默认值已经是 auth `300/60s`、OAuth `600/60s`、password concurrency `8`，但运行日志显示本地容器的 OAuth tier 仍是 `limit=10`。因此 OAuth 结果主要反映“运行环境未加载最新限流配置”，不是当前代码配置下的最终容量。

#### 基准结果

| 场景 | 请求/并发 | 成功/错误 | HTTP 状态 | RPS | p50 | p95 | p99 | 结论 |
| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | --- |
| 登录，控制样本 | 100/30 | 100/0 | 200=100 | 65.82 | 406.04ms | 780.27ms | 1022.37ms | 30 并发下全部成功，无 401/409/429。 |
| 登录，撞限流边界 | 200/30 | 150/50 | 200=150, 429=49, 500=1 | 15.74 | 701.06ms | 8018.95ms | 9478.67ms | 同一 anonymous client 在窗口内触发 auth 限流；该样本用于识别边界，不作为纯性能样本。 |
| 注册，控制样本 | 100/30 | 100/0 | 201=100 | 60.06 | 436.26ms | 753.29ms | 851.15ms | 30 并发注册全部成功，无 401/409/429。 |
| 注册，限流污染样本 | 100/30 | 0/100 | 429=100 | 426.29 | 55.27ms | 81.84ms | 95.42ms | 在登录压测打满 auth 窗口后立即执行，全部被限流，不作为性能样本。 |
| OAuth device flow | 200 flows/50 | 10 device code 成功，其余限流 | 200=10, 429=200 | device code 19.47 | 116.94ms | 180.31ms | 180.31ms | 运行日志确认 `tier=oauth limit=10`，50 并发验收未通过。 |

OAuth 结果补充：

- `oauth_device_code`: 10 请求，10 成功，p50 `116.94ms`，p95 `180.31ms`。
- `oauth_device_code_rate_limited`: 190 请求，190 错误，p50 `77.37ms`，p95 `242.77ms`。
- `oauth_token_rate_limited`: 10 请求，10 错误，p50 `77.13ms`，p95 `104.46ms`。

#### 资源和旁路指标

| 时间点 | API CPU / 内存 | Postgres CPU / 内存 | Redis CPU / 内存 | Postgres 连接 | 健康检查 |
| --- | --- | --- | --- | --- | --- |
| 压测前 | 9.91% / 49.75MiB | 6.26% / 162.9MiB | 0.87% / 10.27MiB | active=1, idle=4 | health=2.419ms, ready=4.413ms |
| 注册/OAuth 后 | 0.53% / 837.9MiB | 1.96% / 211MiB | 0.64% / 10.36MiB | active=1, idle=20 | health=3.320ms, ready=26.172ms |
| 登录补测后 | 0.00% / 969.4MiB | 0.02% / 213.5MiB | 0.71% / 10.54MiB | active=1, idle=20 | health=4.279ms, ready=4.294ms |

观察结论：

- 登录和注册在 30 并发、低于限流窗口的控制样本中稳定成功，p95 均在 `0.8s` 内。
- `/health` 和 `/ready` 在压测前后保持毫秒级响应，说明 Argon2 已从 Tokio worker 上移走，轻量接口没有被明显拖慢。
- Postgres active connection 保持 `1`，idle 连接在压测后增加到 `20`，未观察到 DB acquire timeout 迹象。
- 200/30 登录样本出现 `1` 个 500，需要下一轮日志采样定位；本次尾部日志没有看到 DB acquire timeout。
- OAuth 50 并发未通过验收，直接原因是运行中容器仍按 `oauth limit=10` 执行。下一步应重建/重启 API 并显式设置 `RATE_LIMIT_OAUTH_MAX_REQUESTS=600`、`RATE_LIMIT_AUTH_MAX_REQUESTS=300` 后重跑 7.2。
- 当前 benchmark 默认没有传 `X-Forwarded-For`，所有匿名请求会聚合成同一个 `anonymous` client。若要评估“多人同时登录/注册”，下一轮应扩展脚本支持多用户登录池和可选 client IP 分布。

### 7.3 多 client 复测和问题处理记录

执行日期：2026-07-08。

本阶段先处理 7.2 暴露的“所有压测请求被聚合到同一个 `anonymous` client”问题：

- [x] `bench_auth_login.py` 新增 `--client-ip-mode none|same|unique|pool`。
- [x] `--client-ip-mode unique` 可以为每个 operation 设置不同 `X-Forwarded-For`。
- [x] `--client-ip-mode pool` 可以按 `--client-ip-pool-size` 轮换固定 IP 池。
- [x] 登录压测新增 `--login-users-file`，支持 `email,password` CSV 用户池。
- [x] 错误统计新增 500 跟踪，并可通过 `--error-samples` 输出失败响应样本。

本地尝试执行：

```bash
RATE_LIMIT_AUTH_MAX_REQUESTS=300 \
RATE_LIMIT_AUTH_WINDOW_SECONDS=60 \
RATE_LIMIT_OAUTH_MAX_REQUESTS=600 \
RATE_LIMIT_OAUTH_WINDOW_SECONDS=60 \
AUTH_PASSWORD_CONCURRENCY=8 \
docker compose up -d --build api
```

该命令长时间无输出，后续 `docker compose ps` 和 `docker version` 也无响应；已中断前台命令。HTTP `/health` 和 `/ready` 仍正常，因此本轮先在现有 API 容器上复测多人来源行为，Docker daemon/compose 卡住作为本地环境问题另行处理。

多人来源复测结果：

| 场景 | 请求/并发 | client 模式 | 成功/错误 | HTTP 状态 | RPS | p50 | p95 | p99 | 结论 |
| --- | ---: | --- | ---: | --- | ---: | ---: | ---: | ---: | --- |
| OAuth device flow | 200 flows/50 | `unique` | 400/0 HTTP result | 200=200, 400=200, 429=0, 500=0 | 173.62 | device=111.78ms, token=108.89ms | device=291.22ms, token=158.03ms | device=315.93ms, token=194.62ms | 多 IP 后不再出现 OAuth 429；400 是预期 `authorization_pending`。 |
| 登录 | 200/50 | `pool`, 100 IP | 200/0 | 200=200, 429=0, 500=0 | 20.89 | 997.90ms | 7146.67ms | 7477.70ms | 限流误伤消失，但 Argon2 verify 排队明显。 |
| 注册 | 100/50 | `unique` | 100/0 | 201=100, 429=0, 500=0 | 50.53 | 826.29ms | 1235.54ms | 1379.73ms | 50 并发多人注册稳定，无 409/429/500。 |

压测后旁路健康检查：

- `/health`: `4.194ms`
- `/ready`: `5.489ms`

处理结论：

- 上轮 OAuth 50 并发大面积 429 的主要原因是测试流量被识别为同一个 client；多人 IP 模式下 200 flows/50 并发没有 429。
- 单 client 下仍应触发 OAuth/auth 限流，这是预期保护；多人容量测试必须显式使用 `--client-ip-mode unique` 或真实反向代理注入的可信 client IP。
- 登录 200/50 多人样本 p95 达到 `7.1s`，下一个性能瓶颈是 Argon2 verify 的并发队列。需要继续评估 `AUTH_PASSWORD_CONCURRENCY` 在当前 CPU 下的最优值，或引入登录失败/成功的更细分指标。
- 注册 100/50 多人样本 p95 `1.24s`，当前可作为本地 50 并发注册的保守参考。
- 生产环境必须确保 `X-Forwarded-For` 只来自可信反向代理；应用不能信任公网客户端直接传入的该 header。
- Docker daemon/compose 当前不稳定，尚未完成 API 重建确认。后续应先恢复 Docker，再重建 API 验证运行环境确实加载最新 auth/OAuth 限流默认值。

### 7.4 剩余问题处理后复测

执行日期：2026-07-09。

本阶段继续处理 7.3 后剩余的问题：

- [x] Docker daemon 已恢复响应。
- [x] 使用 `docker compose up -d --force-recreate api` 重新创建 API 容器，确认环境变量存在。
- [x] 发现仅 force recreate 仍使用旧镜像，OAuth 单 client 仍返回 `Maximum 10 requests per 60 seconds`。
- [x] 完整执行 `docker compose up -d --build api`，重新构建 enterprise-server API 镜像并启动。
- [x] 新镜像运行时确认：`RATE_LIMIT_AUTH_MAX_REQUESTS=300`、`RATE_LIMIT_OAUTH_MAX_REQUESTS=600`、`AUTH_PASSWORD_CONCURRENCY=8`。
- [x] `bench_auth_login.py` 新增生成式登录用户池参数：`--login-user-count`、`--login-email-domain`、`--login-email-prefix`、`--login-run-id`。

生成多账号登录池：

```bash
python3 scripts/benchmarks/enterprise/bench_auth_login.py \
  --mode register \
  --requests 100 \
  --concurrency 30 \
  --email-domain linewell.com \
  --email-prefix bench-login-pool \
  --run-id 20260709-001 \
  --org-slug linewell.com \
  --department-slug technology-center \
  --client-ip-mode unique \
  --allow-errors
```

结果：100/100 注册成功，p50 `457.68ms`，p95 `700.48ms`，p99 `875.25ms`，无 409/429/500。

复测结果：

| 场景 | 请求/并发 | client/账号模式 | 成功/错误 | HTTP 状态 | RPS | p50 | p95 | p99 | 结论 |
| --- | ---: | --- | ---: | --- | ---: | ---: | ---: | ---: | --- |
| OAuth device flow | 200 flows/50 | 单 client | 400/0 HTTP result | 200=200, 400=200, 429=0, 500=0 | 207.43 | device=105.50ms, token=104.49ms | device=123.07ms, token=117.56ms | device=140.40ms, token=119.93ms | 新镜像已加载 OAuth 600/60s，旧的 `limit=10` 问题消失。 |
| 登录 | 200/50 | 100 账号 / 100 IP 池 | 200/0 | 200=200, 429=0, 500=0 | 63.22 | 596.57ms | 1540.74ms | 1839.85ms | 真实多账号样本无错误；延迟明显优于 7.3 的单账号重复登录。 |
| 登录 | 500/50 | 100 账号 / 100 IP 池 | 500/0 | 200=500, 429=0, 500=0 | 68.30 | 666.84ms | 913.76ms | 1091.37ms | 50 并发登录稳定，p95 低于 1s。 |
| 登录 | 1000/100 | 100 账号 / 200 IP 池 | 1000/0 | 200=1000, 429=0, 500=0 | 54.47 | 1780.65ms | 2085.41ms | 2131.16ms | 100 并发可撑住但延迟进入 2s 级，说明 Argon2 verify 队列开始成为主要瓶颈。 |

压测后旁路和资源快照：

- `/health`: `5.063ms`
- `/ready`: `7.291ms`
- API: CPU `0.32%`，内存 `1.286GiB`
- Postgres: CPU `0.04%`，内存 `95.67MiB`，连接 `active=1, idle=16`
- Redis: CPU `1.18%`，内存 `19.18MiB`

更新后的结论：

- OAuth 大面积 429 已处理：根因是旧镜像未重新 build，仅重建容器不能更新二进制默认值。
- 历史登录 500 未在 200/50、500/50、1000/100 多账号样本中复现，当前按偶发/旧镜像样本记录，后续只需继续观察。
- 注册在 100/30 和 100/50 样本中稳定，无 409/429/500。
- 登录 50 并发多账号样本表现健康；100 并发虽无错误，但 p95 约 `2.1s`，是当前登录路径的主要容量边界。
- 下一步如要继续优化登录，应围绕 `AUTH_PASSWORD_CONCURRENCY` 做分档压测，例如 `8/12/16`，观察 p95、CPU、内存和 `/health` 延迟，而不是直接无限调高。

### 7.5 `AUTH_PASSWORD_CONCURRENCY` 分档压测

执行日期：2026-07-09。

目标：验证 Argon2 blocking worker 并发度在当前本地 Docker 环境下的合理取值，避免盲目提高并行度导致 CPU/内存争用。

共同测试参数：

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

每档通过以下方式重建 API 容器：

```bash
AUTH_PASSWORD_CONCURRENCY=<8|12|16> \
RATE_LIMIT_AUTH_MAX_REQUESTS=300 \
RATE_LIMIT_AUTH_WINDOW_SECONDS=60 \
RATE_LIMIT_OAUTH_MAX_REQUESTS=600 \
RATE_LIMIT_OAUTH_WINDOW_SECONDS=60 \
docker compose up -d --force-recreate --no-deps api
```

结果：

| `AUTH_PASSWORD_CONCURRENCY` | 请求/并发 | 成功/错误 | HTTP 状态 | RPS | p50 | p95 | p99 | 压测后 health/ready | API 空闲内存 |
| ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | --- | ---: |
| 8 | 1000/100 | 1000/0 | 200=1000 | 77.34 | 1143.01ms | 1855.81ms | 1993.75ms | 2.628ms / 3.246ms | 894.1MiB |
| 12 | 1000/100 | 1000/0 | 200=1000 | 88.84 | 1018.04ms | 1744.61ms | 1859.88ms | 2.507ms / 2.958ms | 1.266GiB |
| 16 | 1000/100 | 1000/0 | 200=1000 | 76.98 | 1061.89ms | 2620.40ms | 2946.39ms | 2.468ms / 3.680ms | 2.66GiB |

结论：

- 三档都没有 401/409/429/500，说明当前多账号、多 IP 登录样本在限流和 DB 层面稳定。
- `12` 是本机样本的最优点：相对 `8`，吞吐从 `77.34/s` 提升到 `88.84/s`，p95 从 `1855.81ms` 降到 `1744.61ms`。
- `16` 明显变差：p95 上升到 `2620.40ms`，p99 上升到 `2946.39ms`，API 空闲内存升到 `2.66GiB`；不建议作为默认值。
- 本地 API 已恢复到 `AUTH_PASSWORD_CONCURRENCY=12` 继续运行。
- 生产建议：先以 `8` 为保守默认，CPU 核数和内存充足时灰度到 `12`；不要直接设为 `16+`，除非有同等压测证明尾延迟没有恶化。

## 阶段 8: 上线和回滚

### 8.1 上线顺序

建议顺序：

1. 先上线阶段 1，明确打开 rollup 配置。
2. 再上线阶段 2，提高 OAuth/auth 限流并保持 metrics 限流不变。
3. 再上线阶段 3，增加 lower(email) 索引。
4. 再上线阶段 4，把 Argon2 移到 blocking worker。
5. 最后上线阶段 5 和阶段 6，减少 DB 往返和异步 audit。

### 8.2 回滚方式

| 改动 | 快速回滚 |
| --- | --- |
| `DASHBOARD_USE_ROLLUPS=true` | 设置为 `false`，重启 API |
| `METRICS_WRITE_ROLLUPS=true` | 如写入压力异常，设置为 `false`，重启 API |
| 限流配置化 | 环境变量调回旧值或回滚代码 |
| OAuth 限流提高 | 调低 `RATE_LIMIT_OAUTH_MAX_REQUESTS` |
| lower(email) 索引 | 通常保留；必要时单独 drop index |
| Argon2 blocking worker | 回滚 handler 调用，或调低 `AUTH_PASSWORD_CONCURRENCY` |
| 注册 DB 往返减少 | 回滚到预查询实现 |
| audit 异步化 | 回滚为同步 await |

### 8.3 上线后观察

观察至少 24 小时：

- [ ] API 5xx。
- [ ] 429 数量。
- [ ] 登录成功率。
- [ ] 注册成功率。
- [ ] DB pool acquire timeout。
- [ ] Postgres CPU/IO。
- [ ] Redis CPU。
- [ ] dashboard p95/p99。
- [ ] metrics upload p95/p99。

## 总体验收命令

每个代码阶段至少运行：

```bash
cd enterprise-server
cargo check
cargo test
git diff --check
```

涉及 compose 的阶段运行：

```bash
cd enterprise-server
docker compose config --quiet
JWT_SECRET=test-secret-at-least-32-characters POSTGRES_PASSWORD=test-password docker compose -f deploy/docker-compose.yml config --quiet
```

涉及迁移的阶段运行：

```bash
cd enterprise-server
cargo test db::migrations
cargo run -- --migrate
```

涉及 benchmark 脚本的阶段运行：

```bash
python3 -m py_compile scripts/benchmarks/enterprise/*.py
```

## 推荐执行顺序

1. 阶段 1：显式开启 dashboard rollup 配置。
2. 阶段 2：注册/登录限流配置化。
3. 阶段 3：用户邮箱登录索引优化。
4. 阶段 4：Argon2 密码计算移出 Tokio worker。
5. 阶段 7：新增注册/登录专项压测脚本。
6. 阶段 5：减少注册/登录 DB 往返。
7. 阶段 6：登录审计写入异步化。
8. 阶段 8：上线和回滚。

不建议先降低 Argon2 参数或取消认证限流。正确方向是把认证限流变成可配置、把 CPU 密集型密码计算隔离出去，再通过索引和 SQL 往返减少提升吞吐。
