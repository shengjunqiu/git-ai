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
RATE_LIMIT_AUTH_MAX_REQUESTS=300
RATE_LIMIT_AUTH_WINDOW_SECONDS=60
RATE_LIMIT_OAUTH_MAX_REQUESTS=600
RATE_LIMIT_OAUTH_WINDOW_SECONDS=60
RATE_LIMIT_DEFAULT_MAX_REQUESTS=300
RATE_LIMIT_DEFAULT_WINDOW_SECONDS=60
RATE_LIMIT_METRICS_MAX_REQUESTS=60
RATE_LIMIT_METRICS_WINDOW_SECONDS=60
```

实现步骤：

- [ ] 在 `AppConfig` 增加上述限流字段。
- [ ] 在 `EnvConfig` 增加可选字段。
- [ ] 保持兼容默认值：
  - metrics: `60/60`
  - oauth: 从 `10/60` 提升到 `600/60`
  - auth: 新增 `300/60`
  - default: 从 `120/60` 提升到 `300/60`
- [ ] 将 `tiers::OAUTH`、`tiers::DEFAULT` 等硬编码常量改为从 `AppConfig` 生成。
- [ ] 新增 `auth` tier：

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

- [ ] 保持 `/worker/metrics/*` 仍走 metrics tier。
- [ ] 保持 `/health`、`/ready` 继续 bypass。

验收标准：

- [ ] 认证路径不再落入 default tier。
- [ ] OAuth 轮询不会因 10/min 限制导致多人同时 `git-ai login` 很快 429。
- [ ] metrics 上传限流保持原行为，避免误放大写入压力。

### 2.2 增加限流测试

涉及文件：

- `enterprise-server/src/services/rate_limit.rs`

测试覆盖：

- [ ] `/worker/oauth/device/code` 使用 oauth tier。
- [ ] `/worker/oauth/token` 使用 oauth tier。
- [ ] `/auth/login` 使用 auth tier。
- [ ] `/auth/register` 使用 auth tier。
- [ ] `/worker/metrics/upload` 使用 metrics tier。
- [ ] `/health` 和 `/ready` bypass。
- [ ] 环境变量能覆盖默认值。

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

- [ ] 单元测试覆盖各路径 tier。
- [ ] 配置缺省时行为稳定。
- [ ] 配置覆盖时生效。

提交建议：

```bash
git add enterprise-server/src/config.rs enterprise-server/src/services/rate_limit.rs enterprise-server/.env.example enterprise-server/deploy/.env.example enterprise-server/deploy/README.md docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Make enterprise auth rate limits configurable"
```

## 阶段 3: 用户邮箱登录索引优化

目标：优化 `lower(email) = lower($1)` 查询，避免用户量增长后登录和注册查重扫描变慢。

### 3.1 新增 lower(email) 表达式索引

涉及文件：

- `enterprise-server/migrations/018_users_lower_email_index.sql`
- `enterprise-server/deploy/migrations/018_users_lower_email_index.sql`
- `enterprise-server/src/db/migrations.rs`

实现步骤：

- [ ] 新增迁移：

```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email_lower
    ON users (lower(email));
```

- [ ] 同步复制到 deploy migrations。
- [ ] 在 `src/db/migrations.rs` 注册 `018_users_lower_email_index`。
- [ ] 评估生产是否需要独立运维脚本使用 `CREATE INDEX CONCURRENTLY`。

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

- [ ] migration 可重复执行。
- [ ] `idx_users_email_lower` 存在。
- [ ] 登录查询和注册查重继续通过测试。

提交建议：

```bash
git add enterprise-server/migrations/018_users_lower_email_index.sql enterprise-server/deploy/migrations/018_users_lower_email_index.sql enterprise-server/src/db/migrations.rs docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Index enterprise users by normalized email"
```

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

- [ ] 在 `AppConfig` 增加 `auth_password_concurrency: usize`。
- [ ] 默认值设为 `8`，并用 `.max(1)` 防止无效配置。
- [ ] 在 `AppState` 增加：

```rust
pub auth_password_limiter: std::sync::Arc<tokio::sync::Semaphore>
```

- [ ] 在 `main.rs` 构建 `AppState` 时初始化 semaphore。
- [ ] 更新测试中的 `AppState` 构造。

验收标准：

- [ ] 配置默认值兼容。
- [ ] 测试环境可以构造 `AppState`。

### 4.2 新增异步密码服务封装

涉及文件：

- `enterprise-server/src/services/passwords.rs`
- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [ ] 保留现有同步函数：
  - `hash_password`
  - `verify_password`
  - `validate_password_strength`
- [ ] 新增 async 包装函数：

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

- [ ] 在 async 函数中先 acquire semaphore permit。
- [ ] 使用 `tokio::task::spawn_blocking` 执行同步 Argon2。
- [ ] 正确处理 `JoinError`，返回 `AppError::Internal`。
- [ ] 确保 permit 在阻塞任务完成后释放。

验收标准：

- [ ] 密码强度和 Argon2 参数不降低。
- [ ] 并发密码计算不会无限放大。
- [ ] Tokio worker 不再直接执行 Argon2 CPU 任务。

### 4.3 改造注册和登录 handler

涉及文件：

- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [ ] 注册路径中，将：

```rust
let password_hash = crate::services::passwords::hash_password(&req.password)?;
```

替换为 async blocking 包装调用。

- [ ] 登录路径中，将：

```rust
crate::services::passwords::verify_password(&req.password, &password_hash)?
```

替换为 async blocking 包装调用。

- [ ] 避免在拿着 DB transaction 时执行 Argon2。
- [ ] 保持错误响应不泄露邮箱是否存在、密码是否错误等敏感细节。

测试命令：

```bash
cd enterprise-server
cargo test passwords
cargo test auth_api
cargo test
```

验收标准：

- [ ] 注册成功路径通过。
- [ ] 登录成功路径通过。
- [ ] 错误密码仍返回 unauthorized。
- [ ] 并发注册同邮箱仍只有一个成功。

提交建议：

```bash
git add enterprise-server/src/config.rs enterprise-server/src/routes.rs enterprise-server/src/main.rs enterprise-server/src/services/passwords.rs enterprise-server/src/handlers/auth_api.rs enterprise-server/.env.example enterprise-server/deploy/.env.example docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Run enterprise password hashing on blocking workers"
```

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

- [ ] 新增服务函数，例如：

```rust
resolve_and_validate_registration_scope(pool, email, org_id/org_slug, department_id/department_slug)
```

- [ ] 用一个 SQL 同时确认：
  - org 存在。
  - org domain verified。
  - department 存在并属于 org。
- [ ] 保留清晰错误信息。
- [ ] 对已传 UUID 和 slug 两种路径都覆盖。

验收标准：

- [ ] 注册前置校验 DB 往返减少。
- [ ] org/domain/department 错误仍返回正确错误。
- [ ] 现有注册表单行为不变。

### 5.2 移除邮箱 exists 预查询，依赖唯一索引

涉及文件：

- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [ ] 删除注册中的：

```sql
SELECT EXISTS(SELECT 1 FROM users WHERE lower(email) = lower($1))
```

- [ ] 直接执行 insert。
- [ ] 通过 `idx_users_email_lower` 或现有唯一约束捕获冲突。
- [ ] 更新 `map_user_insert_error`，兼容：
  - `users_email_key`
  - `idx_users_email_lower`

验收标准：

- [ ] 重复邮箱注册返回 conflict。
- [ ] 并发同邮箱注册仍只有一个成功。
- [ ] 注册成功路径减少一次 DB 查询。

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

## 阶段 6: 登录审计写入异步化

目标：避免登录/注册成功后等待 audit log 写入，降低用户可见延迟。

### 6.1 抽象 fire-and-forget audit helper

涉及文件：

- `enterprise-server/src/services/audit.rs`
- `enterprise-server/src/handlers/auth_api.rs`
- `enterprise-server/src/handlers/oauth.rs`
- `enterprise-server/src/handlers/cli_authorize.rs`

实现步骤：

- [ ] 新增 helper，例如：

```rust
pub fn spawn_log_action(pool: PgPool, payload: AuditPayload)
```

- [ ] 在 helper 内部 `tokio::spawn` 写 audit。
- [ ] 写入失败只打 warn，不影响主请求。
- [ ] 避免 spawn 中捕获引用，payload 使用 owned data。

验收标准：

- [ ] 登录/注册成功响应不再等待 audit insert。
- [ ] audit 写入失败不会让登录失败。
- [ ] 日志能看到 audit 写入失败原因。

### 6.2 改造注册/登录 audit 调用

涉及文件：

- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [ ] 将 `user.register` audit 改为异步后台写入。
- [ ] 将 `org_member.create` audit 改为异步后台写入。
- [ ] 将 `user.login` audit 改为异步后台写入。
- [ ] 保持 logout 可按现有方式执行，或同步纳入 helper。

测试命令：

```bash
cd enterprise-server
cargo test audit
cargo test auth_api
cargo test
```

验收标准：

- [ ] 登录/注册响应不依赖 audit 写入完成。
- [ ] audit 表仍能看到成功登录/注册记录。

提交建议：

```bash
git add enterprise-server/src/services/audit.rs enterprise-server/src/handlers/auth_api.rs docs/enterprise-auth-login-performance-task-plan.md
git commit -m "Write enterprise auth audit logs asynchronously"
```

## 阶段 7: 注册/登录专项压测脚本

目标：把注册、网页登录、OAuth device flow 的容量验证脚本化。

### 7.1 新增 auth benchmark 脚本

涉及文件：

- `scripts/benchmarks/enterprise/bench_auth_login.py`
- `scripts/benchmarks/enterprise/README.md`

实现步骤：

- [ ] 使用 Python stdlib 实现脚本，保持与现有 benchmark 脚本风格一致。
- [ ] 支持网页登录压测：

```text
POST /auth/login
```

- [ ] 支持注册压测：

```text
POST /auth/register
```

- [ ] 支持 OAuth device flow 压测：

```text
POST /worker/oauth/device/code
POST /worker/oauth/token
```

- [ ] 输出：
  - requests
  - concurrency
  - success
  - error rate
  - rps
  - p50
  - p95
  - p99
- [ ] 支持唯一邮箱生成，避免注册冲突干扰性能结果。
- [ ] 对 401/409/429 单独统计。

验收标准：

- [ ] 脚本 `--help` 可用。
- [ ] 登录压测可以使用已有测试用户。
- [ ] 注册压测不会重复使用同一邮箱。
- [ ] OAuth 压测能区分 pending 和 rate limited。

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

- [ ] p50/p95/p99。
- [ ] 429 比例。
- [ ] 401/409 比例。
- [ ] API CPU。
- [ ] Postgres active connection。
- [ ] Redis CPU。
- [ ] Tokio worker 是否被 Argon2 拖慢，可用整体延迟和健康检查旁路压测间接观察。

验收标准：

- [ ] 50 并发 OAuth 请求不再大面积 429。
- [ ] 30 并发网页登录时 `/health`、`/ready` 不被拖慢。
- [ ] 注册 p95 明显优于改造前，或至少不再影响其它轻量接口。
- [ ] 错误率符合预期，不能出现 DB acquire timeout。

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
