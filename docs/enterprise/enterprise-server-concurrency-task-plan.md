# Enterprise Server 并发优化任务清单

本文档把 [Enterprise Server 并发优化计划](./enterprise-server-concurrency-optimization.md) 拆成可以逐步执行、逐步验收的工程任务。

建议执行方式：

1. 每次只处理一个任务块。
2. 每个任务块单独提交。
3. P0 完成后再处理 P1，P1 完成后再处理 P2。
4. 每个任务块都要包含代码、测试和必要的迁移。

## 阶段 0: 准备工作

### 0.1 确认本地服务端测试环境

目标：确保可以在本地运行 enterprise-server 测试和数据库相关测试。

步骤：

- [ ] 确认 Docker 可用。
- [ ] 启动 enterprise-server 依赖：

```bash
cd enterprise-server
docker compose up -d postgres redis minio
```

- [ ] 确认 PostgreSQL 就绪：

```bash
docker compose exec postgres pg_isready -U gitai -d gitai_enterprise
```

- [ ] 跑服务端测试：

```bash
cd enterprise-server
cargo test
```

验收标准：

- [ ] `cargo test` 通过。
- [ ] 本地能连接 `DATABASE_URL` 指向的 Postgres。

## 阶段 1: P0 migration 并发启动保护

### 1.1 给 migration runner 加 PostgreSQL advisory lock

目标：多实例同时启动时，只有一个实例执行 migration，其他实例等待后跳过已执行 migration。

涉及文件：

- `enterprise-server/src/db/migrations.rs`

实现步骤：

- [ ] 在 `run_migrations(pool: &PgPool)` 中通过 `pool.acquire().await?` 获取单个数据库连接。
- [ ] 在同一个连接上执行：

```sql
SELECT pg_advisory_lock(hashtext('git-ai-enterprise-server:migrations'));
```

- [ ] 后续创建 `_migrations`、检查 migration、执行 migration、插入 `_migrations` 都使用同一个连接。
- [ ] 在函数结束前执行：

```sql
SELECT pg_advisory_unlock(hashtext('git-ai-enterprise-server:migrations'));
```

- [ ] 确保出错时连接关闭会释放 lock。可以不写复杂的 drop guard，但要避免 lock 后继续使用 pool 随机连接执行 migration。

建议实现细节：

- 如果 sqlx 类型签名处理较繁琐，可以先引入内部函数：

```rust
async fn run_migrations_locked(conn: &mut sqlx::PgConnection) -> anyhow::Result<()>
```

测试步骤：

- [ ] 新增测试：两个 async task 同时调用 `run_migrations(&pool)`。
- [ ] 断言两个 task 都成功。
- [ ] 断言 `_migrations` 中每个 migration name 只有一条。

验收命令：

```bash
cd enterprise-server
cargo test db::migrations
cargo test
```

验收标准：

- [ ] 并发调用 migration 不报错。
- [ ] `_migrations` 不重复。
- [ ] 两个服务实例同时启动不会因为 migration 冲突退出。

提交建议：

```bash
git add enterprise-server/src/db/migrations.rs
git commit -m "Serialize enterprise migrations"
```

## 阶段 2: P0 OAuth 一次性凭证原子消费

### 2.1 原子消费 refresh token

目标：同一个 refresh token 并发兑换时，只有一个请求成功。

涉及文件：

- `enterprise-server/src/handlers/oauth.rs`

实现步骤：

- [ ] 修改 `handle_refresh_token_grant`。
- [ ] 删除先 `SELECT` 再 `UPDATE` 的逻辑。
- [ ] 改成单条 SQL：

```sql
UPDATE refresh_tokens
SET revoked_at = now()
WHERE token_hash = $1
  AND revoked_at IS NULL
  AND expires_at > now()
RETURNING user_id;
```

- [ ] 如果没有返回行，返回 `invalid_grant("Invalid or revoked refresh token")` 或等价错误。
- [ ] 成功返回 `user_id` 后再调用 `token_response`。

测试步骤：

- [ ] 新增 refresh token 并发兑换测试。
- [ ] 准备一个用户和 refresh token。
- [ ] 同时发起两个 `refresh_token` grant。
- [ ] 断言只有一个成功。
- [ ] 断言只有一个新 refresh token 被签发。

验收标准：

- [ ] 单请求 refresh token 兑换仍可用。
- [ ] 并发双请求只成功一个。
- [ ] 已撤销或过期 token 返回 `invalid_grant`。

### 2.2 原子消费 install nonce

目标：同一个 install nonce 并发兑换时，只有一个请求成功。

涉及文件：

- `enterprise-server/src/handlers/oauth.rs`

实现步骤：

- [ ] 修改 `handle_install_nonce_grant`。
- [ ] 删除先 `SELECT used` 再 `UPDATE used = true` 的逻辑。
- [ ] 改成单条 SQL：

```sql
UPDATE install_nonces
SET used = true,
    used_at = now()
WHERE nonce = $1
  AND used = false
RETURNING user_id;
```

- [ ] 如果没有返回行，返回 `invalid_grant("Invalid install nonce")` 或 `invalid_grant("Install nonce already used")`。

测试步骤：

- [ ] 新增 install nonce 并发兑换测试。
- [ ] 同时发起两个 `install_nonce` grant。
- [ ] 断言只有一个成功。

验收标准：

- [ ] install nonce 只能兑换一次。
- [ ] 并发请求不会签发两份 token。

### 2.3 原子消费 device code

目标：同一个 device code 被授权后，并发 token polling 时只有一个请求成功。

涉及文件：

- `enterprise-server/src/handlers/oauth.rs`
- 可选：`enterprise-server/migrations/*.sql`
- 可选：`enterprise-server/deploy/migrations/*.sql`

实现选项 A：继续用删除即消费。

```sql
DELETE FROM oauth_devices
WHERE device_code = $1
  AND expires_at > now()
  AND user_id IS NOT NULL
RETURNING user_id;
```

实现选项 B：保留审计记录，新增 `consumed_at` 字段。

迁移：

```sql
ALTER TABLE oauth_devices
ADD COLUMN IF NOT EXISTS consumed_at TIMESTAMPTZ;
```

消费：

```sql
UPDATE oauth_devices
SET consumed_at = now()
WHERE device_code = $1
  AND expires_at > now()
  AND user_id IS NOT NULL
  AND consumed_at IS NULL
RETURNING user_id;
```

建议优先选项 A，改动更小。

测试步骤：

- [ ] 新增 device code 并发兑换测试。
- [ ] 准备一个已授权 device code。
- [ ] 同时发起两个 device code grant。
- [ ] 断言只有一个成功。

验收命令：

```bash
cd enterprise-server
cargo test oauth
cargo test
```

验收标准：

- [ ] refresh token、install nonce、device code 都只能被消费一次。
- [ ] authorization code 原有测试继续通过。

提交建议：

```bash
git add enterprise-server/src/handlers/oauth.rs
git commit -m "Make OAuth grants atomic"
```

## 阶段 3: P1 report 上传事务和幂等

### 3.1 将 report 上传放入事务

目标：report 上传要么完整成功，要么完整失败，不留下半成品。

涉及文件：

- `enterprise-server/src/handlers/report.rs`

实现步骤：

- [ ] 在 `upload_report` 开始写 DB 前创建 transaction：

```rust
let mut tx = state.db.begin().await.map_err(AppError::Database)?;
```

- [ ] `projects` upsert 使用 `&mut *tx`。
- [ ] `report_uploads` insert 使用 `&mut *tx`。
- [ ] `commit_stats` 循环 insert 使用 `&mut *tx`。
- [ ] `tool_model_stats` 循环 upsert 使用 `&mut *tx`。
- [ ] 全部成功后 `tx.commit().await`。
- [ ] 不要吞掉 `tool_model_stats` 错误；去掉 `.ok()`，失败应回滚整个 report。

测试步骤：

- [ ] 新增测试模拟 `tool_model_stats` 写入失败。
- [ ] 断言 `report_uploads` 和 `commit_stats` 没有残留。

验收标准：

- [ ] 任一步失败都会回滚。
- [ ] 成功上传行为不变。

### 3.2 明确 commit_stats 冲突策略

目标：同一项目同一 commit 多次上传时行为稳定。

策略选项：

1. 保留“首次上传为准”：继续 `DO NOTHING`，但文档和返回值明确 duplicate。
2. 最新上传覆盖：改成 `DO UPDATE SET ...` 更新全部统计字段。

建议：公司场景优先选择“最新上传覆盖”，因为算法修复或客户端升级后，重新上传应能修正历史统计。

实现步骤：

- [ ] 将 `ON CONFLICT (project_id, sha) DO NOTHING` 改为 `DO UPDATE SET`。
- [ ] 更新全部统计字段：author、author_time、subject、has_authorship_note、diff 统计、AI/Human/Mixed/Unknown、time_waiting_for_ai。
- [ ] 返回值中区分 `inserted_commits` 和 `updated_commits`。

测试步骤：

- [ ] 上传同一 commit 两次，第二次使用不同 stats。
- [ ] 断言数据库最终是第二次 stats。

### 3.3 修正 tool_model_stats 更新字段

目标：`tool_model_stats` 冲突更新时不能只更新部分字段。

实现步骤：

- [ ] 在 `ON CONFLICT (project_id, tool_model) DO UPDATE SET` 中更新全部字段：
  - `ai_additions`
  - `mixed_additions`
  - `ai_accepted`
  - `total_ai_additions`
  - `total_ai_deletions`
  - `time_waiting_for_ai`

测试步骤：

- [ ] 上传同一 tool_model 两次，第二次每个字段不同。
- [ ] 断言全部字段更新。

验收命令：

```bash
cd enterprise-server
cargo test report
cargo test
```

提交建议：

```bash
git add enterprise-server/src/handlers/report.rs
git commit -m "Make report uploads transactional"
```

## 阶段 4: P1 CAS 一致性

### 4.1 服务端校验 content hash

目标：服务端不信任客户端传入 hash，避免同 hash 不同内容覆盖。

涉及文件：

- `enterprise-server/src/handlers/cas.rs`
- 可能涉及 `enterprise-server/src/pos_encoded.rs`

实现步骤：

- [ ] 序列化 `object.content` 得到 canonical 或当前稳定 JSON 字节。
- [ ] 服务端计算 SHA256。
- [ ] 与 `object.hash` 比较。
- [ ] 不一致返回 400。

注意：

- 如果客户端 hash 的计算方式不是普通 JSON 字符串 SHA256，先确认客户端 CAS hash 算法，再保持一致。
- 不要因为 serde_json 字段顺序不同导致误判。必要时使用 canonical JSON。

测试步骤：

- [ ] hash 与 content 匹配时上传成功。
- [ ] hash 与 content 不匹配时返回 400。

### 4.2 避免 DB ready 记录早于对象存储成功

目标：S3/MinIO 失败时，不留下可读但无对象的数据。

方案 A：先写 S3，再写 DB。

步骤：

- [ ] 在 `process_cas_object` 中先执行 `state.cas_store.put(...)`。
- [ ] S3 成功后写 `cas_objects` 和 `cas_ownership`。
- [ ] 读路径 DB 找不到时是否允许 S3 fallback，需要保持当前行为或明确收紧。

方案 B：增加对象状态。

迁移：

```sql
ALTER TABLE cas_objects
ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'ready';
```

步骤：

- [ ] insert `pending`。
- [ ] S3 put 成功。
- [ ] update `ready`。
- [ ] 读路径只返回 `status = 'ready'` 的对象。

建议先做方案 A，改动小；如果后续要后台修复/重试，再做方案 B。

测试步骤：

- [ ] mock 或注入失败的 object store。
- [ ] S3 put 失败时，DB 不应留下可读记录。
- [ ] 并发上传同 hash 同内容成功。
- [ ] 并发上传同 hash 不同内容被拒绝。

验收命令：

```bash
cd enterprise-server
cargo test cas
cargo test
```

提交建议：

```bash
git add enterprise-server/src/handlers/cas.rs
git commit -m "Harden CAS upload consistency"
```

## 阶段 5: P1 Redis rate limit

### 5.1 将 rate limiter 改为 Redis 共享计数

目标：多实例部署时 rate limit 仍然全局生效。

涉及文件：

- `enterprise-server/src/services/rate_limit.rs`
- `enterprise-server/src/main.rs`
- `enterprise-server/src/routes.rs`

实现步骤：

- [ ] 修改 `RateLimiter`，让它持有 `redis::Client` 或由 middleware 从 `AppState.redis` 获取连接。
- [ ] 用 Redis key 记录计数：

```text
git-ai:rate-limit:{tier}:{client-key}:{window-start}
```

- [ ] 使用 Lua 脚本或 Redis pipeline 实现原子 `INCR + EXPIRE`。
- [ ] 保留内存 fallback 时，必须打 warn 日志。
- [ ] 明确生产配置：Redis 不可用时是 fail open、fail closed 还是 fallback。

推荐 Lua 逻辑：

```lua
local current = redis.call('INCR', KEYS[1])
if current == 1 then
  redis.call('EXPIRE', KEYS[1], ARGV[1])
end
return current
```

测试步骤：

- [ ] 单实例请求超过阈值会被限制。
- [ ] 两个 `RateLimiter` 实例共享同一个 Redis 时，计数共享。
- [ ] Redis 不可用时行为符合配置。

验收命令：

```bash
cd enterprise-server
cargo test rate_limit
cargo test
```

提交建议：

```bash
git add enterprise-server/src/services/rate_limit.rs enterprise-server/src/main.rs enterprise-server/src/routes.rs
git commit -m "Use Redis for enterprise rate limits"
```

## 阶段 6: P2 client status 多设备

### 6.1 修改状态表唯一维度

目标：同一个开发者多台设备状态不互相覆盖。

涉及文件：

- `enterprise-server/migrations/*.sql`
- `enterprise-server/deploy/migrations/*.sql`
- `enterprise-server/src/services/client_status.rs`
- `enterprise-server/src/handlers/client_status.rs`
- `enterprise-server/src/handlers/dashboard.rs`

迁移建议：

```sql
ALTER TABLE developer_client_status
ADD COLUMN IF NOT EXISTS device_key TEXT;

UPDATE developer_client_status
SET device_key = COALESCE(distinct_id, hostname, 'unknown')
WHERE device_key IS NULL;

ALTER TABLE developer_client_status
ALTER COLUMN device_key SET NOT NULL;

ALTER TABLE developer_client_status
DROP CONSTRAINT IF EXISTS developer_client_status_pkey;

ALTER TABLE developer_client_status
ADD PRIMARY KEY (user_id, device_key);
```

实现步骤：

- [ ] 从 `distinct_id`、`hostname` 计算 `device_key`。
- [ ] 将 upsert 从 `ON CONFLICT (user_id)` 改成 `ON CONFLICT (user_id, device_key)`。
- [ ] dashboard 聚合时按 user 聚合多设备状态。

测试步骤：

- [ ] 同一用户两个 distinct_id 登录后有两行状态。
- [ ] 一台设备登出不影响另一台设备。
- [ ] dashboard 聚合状态符合预期。

提交建议：

```bash
git add enterprise-server/migrations enterprise-server/deploy/migrations enterprise-server/src/services/client_status.rs enterprise-server/src/handlers/client_status.rs enterprise-server/src/handlers/dashboard.rs
git commit -m "Track client status per device"
```

## 阶段 7: P2 注册冲突错误映射

### 7.1 并发注册同邮箱返回稳定 409

目标：并发注册同一邮箱时，一个成功，一个稳定返回 conflict，而不是裸数据库错误。

涉及文件：

- `enterprise-server/src/handlers/auth_api.rs`

实现步骤：

- [ ] 保留当前 `email_exists` 前置检查。
- [ ] 捕获 insert 用户时的唯一约束错误。
- [ ] 将用户邮箱唯一冲突映射为 `AppError::Conflict("Email already exists")`。

测试步骤：

- [ ] 两个请求并发注册同一邮箱。
- [ ] 断言只有一个成功。
- [ ] 断言另一个返回 409。

提交建议：

```bash
git add enterprise-server/src/handlers/auth_api.rs
git commit -m "Normalize duplicate registration errors"
```

## 阶段 8: P2 release asset 发布一致性

### 8.1 改为不可变 release asset key

目标：避免覆盖同一个 S3 key 时，下载方看到短暂不一致。

涉及文件：

- `enterprise-server/src/handlers/release.rs`
- `enterprise-server/src/services/cas.rs`
- `enterprise-server/migrations/*.sql`
- `enterprise-server/deploy/migrations/*.sql`

设计：

- S3 key 使用：

```text
releases/{version}/{filename}
```

- `release_channels` 负责将 `channel` 指向 `version`。
- `release_assets` 记录 version、filename、sha256、size、storage_path。

实现步骤：

- [ ] 新增或调整 `release_assets` 的 version 字段。
- [ ] 上传 asset 时写入 version 目录。
- [ ] 只有当某个 version 的所有资产上传完成并校验通过后，才更新 channel。
- [ ] 删除接口区分“下架 DB 记录”和“物理删除 S3 对象”。

测试步骤：

- [ ] 上传 v1 asset，channel 指向 v1。
- [ ] 上传 v2 asset 但不切 channel，下载仍返回 v1。
- [ ] 切 channel 后下载返回 v2。
- [ ] 并发下载和上传不会返回半成品。

提交建议：

```bash
git add enterprise-server/src/handlers/release.rs enterprise-server/src/services/cas.rs enterprise-server/migrations enterprise-server/deploy/migrations
git commit -m "Version enterprise release assets"
```

## 总体验收

完成所有阶段后执行：

```bash
cargo test
cd enterprise-server
cargo test
```

如果本地启动依赖：

```bash
cd enterprise-server
docker compose up -d postgres redis minio
cargo run -- --migrate
cargo run
```

手动验收清单：

- [ ] 两个 enterprise-server 实例同时启动，migration 不冲突。
- [ ] 同一 refresh token 并发兑换只有一个成功。
- [ ] 同一 install nonce 并发兑换只有一个成功。
- [ ] 同一 device code 并发兑换只有一个成功。
- [ ] report 上传失败不会留下部分数据。
- [ ] report 重复上传行为符合选定幂等策略。
- [ ] CAS 上传失败不会留下可读但无对象的记录。
- [ ] 多实例 rate limit 共享计数。
- [ ] 同一用户多设备状态独立。
- [ ] 并发注册同邮箱返回稳定结果。
- [ ] release asset 发布不会让下载方看到半成品。

