# Enterprise Server 并发优化计划

本文档记录 `enterprise-server/` 在公司多人使用、服务多实例部署、客户端重复请求或并发请求时需要优先处理的并发与一致性问题。

结论：当前服务端大部分共享状态在 PostgreSQL 中，单实例 Rust 进程本身没有明显线程安全风险。真正需要优化的是数据库原子性、幂等语义、多实例启动和对象存储/数据库的一致性。

## 范围

本文关注企业服务端：

- `enterprise-server/src/main.rs`
- `enterprise-server/src/db/migrations.rs`
- `enterprise-server/src/handlers/oauth.rs`
- `enterprise-server/src/handlers/report.rs`
- `enterprise-server/src/handlers/cas.rs`
- `enterprise-server/src/services/rate_limit.rs`
- `enterprise-server/src/services/client_status.rs`

不包含客户端 Git Notes 同步问题。客户端 `refs/notes/ai` 并发推送已经在 CLI 侧处理，不属于 enterprise-server 直接写入路径。

## 优先级概览

| 优先级 | 问题 | 影响 | 建议动作 |
| --- | --- | --- | --- |
| P0 | 多实例同时跑 migration | 启动失败、重复执行迁移、部分 schema 状态 | 加 PostgreSQL advisory lock，或部署时只允许运行一次 migration job |
| P0 | OAuth 一次性凭证先读后写 | refresh token、install nonce、device code 可能被并发兑换多次 | 改为单条 `UPDATE/DELETE ... RETURNING` 原子消费 |
| P1 | report 上传非事务 | 中途失败留下半个 report，并发上传时数据语义不清 | 整个 upload 使用 transaction，增加幂等键或“新报告覆盖旧报告”策略 |
| P1 | CAS 写 DB 和写对象存储不原子 | DB 有记录但 S3/MinIO 无对象，或 hash 与内容不匹配 | 服务端计算 hash，写入状态改为 `pending/ready`，或 S3 成功后提交 DB |
| P1 | rate limit 是单实例内存态 | 多实例部署时限流被实例数放大 | 用 Redis 做共享计数器 |
| P2 | client status 按用户覆盖 | 同一开发者多设备登录/登出互相覆盖 | 改成按 `(user_id, distinct_id)` 或 `(user_id, hostname)` 记录 |
| P2 | 注册邮箱先查后插 | 并发注册同邮箱时可能返回数据库错误而不是稳定 409 | 依赖唯一约束并映射冲突错误 |
| P2 | release asset 写 S3 与 DB 非事务 | 删除/覆盖与下载并发时可能短暂不一致 | 明确发布流程，使用版本化 key 或发布后原子切 channel |

## P0-1: migration 并发启动

### 现状

`enterprise-server/src/main.rs` 启动时会调用 `db::run_migrations(&db_pool)`。`enterprise-server/src/db/migrations.rs` 当前逻辑是：

1. 创建 `_migrations` 表。
2. 对每个 migration 执行 `SELECT EXISTS(...)`。
3. 如果不存在，执行 migration SQL。
4. 插入 `_migrations` 记录。

关键位置：

- `enterprise-server/src/db/migrations.rs`
- `run_migrations`
- `_migrations`

### 风险

如果公司部署多实例，两个实例同时启动，可能都判断某个 migration 未执行，然后同时执行同一段 SQL。即使大部分 SQL 使用 `IF NOT EXISTS` 或 `ON CONFLICT`，也不能保证所有迁移都安全。风险包括：

- 一个实例执行到一半，另一个实例开始执行同一迁移。
- `_migrations` 唯一约束冲突导致启动失败。
- 非幂等数据迁移重复执行。

### 建议实现

优先方案：在 `run_migrations` 开始时获取 PostgreSQL advisory lock。

示例语义：

```sql
SELECT pg_advisory_lock(hashtext('git-ai-enterprise-server:migrations'));
-- run migrations
SELECT pg_advisory_unlock(hashtext('git-ai-enterprise-server:migrations'));
```

实现注意点：

- lock 必须绑定在同一个数据库连接上，不能通过普通 pool query 随机换连接。
- 用 `pool.acquire().await?` 拿一个连接，在该连接上执行 lock、migration、unlock。
- unlock 应尽量放在作用域结束前执行；如果进程崩溃，Postgres 会随连接关闭释放 advisory lock。

部署侧也可以补一道保险：生产环境只在 release job 中执行 `enterprise-server --migrate`，应用实例启动时不自动迁移。但代码层 advisory lock 仍建议保留。

### 验证

- 新增集成测试：两个 async task 同时调用 `run_migrations`，最终 `_migrations` 中每个 name 只有一条，两个调用都成功。
- 手动验证：同时启动两个 enterprise-server 容器，观察只有一个实例执行 migration，另一个等待后跳过。

## P0-2: OAuth 一次性凭证原子消费

### 现状

`enterprise-server/src/handlers/oauth.rs` 中以下 grant 存在先读后写：

- `refresh_token`
- `install_nonce`
- `device_code`

当前模式大致是：

1. `SELECT` 凭证状态。
2. 在应用层判断是否过期/是否已使用。
3. `UPDATE` 或 `DELETE` 标记已使用。
4. 签发新 token。

### 风险

两个请求同时带着同一个凭证进来时，都可能在第一步读到“可用”，然后都成功签发 token。

这对公司使用的影响是：

- refresh token 可以被并发重放。
- install nonce 可能被重复兑换。
- device code 可能被重复兑换。

authorization code 路径已经使用 `UPDATE ... WHERE consumed_at IS NULL ... RETURNING`，相对更安全。

### 建议实现

#### Refresh Token

把读取和撤销合并成单条 SQL：

```sql
UPDATE refresh_tokens
SET revoked_at = now()
WHERE token_hash = $1
  AND revoked_at IS NULL
  AND expires_at > now()
RETURNING user_id;
```

如果没有返回行，返回 `invalid_grant`。

#### Install Nonce

```sql
UPDATE install_nonces
SET used = true,
    used_at = now()
WHERE nonce = $1
  AND used = false
RETURNING user_id;
```

如果没有返回行，返回 `invalid_grant`。

#### Device Code

如果继续使用删除即消费：

```sql
DELETE FROM oauth_devices
WHERE device_code = $1
  AND expires_at > now()
  AND user_id IS NOT NULL
RETURNING user_id;
```

如果需要保留审计记录，建议新增 `consumed_at` 字段，改成：

```sql
UPDATE oauth_devices
SET consumed_at = now()
WHERE device_code = $1
  AND expires_at > now()
  AND user_id IS NOT NULL
  AND consumed_at IS NULL
RETURNING user_id;
```

### 验证

每种 grant 各加一个并发测试：

1. 准备一个可用凭证。
2. 同时发起两个 token exchange。
3. 断言只有一个成功，另一个返回 `invalid_grant`。
4. 断言数据库状态只消费一次。

## P1-1: report 上传事务和幂等

### 现状

`enterprise-server/src/handlers/report.rs` 的 `upload_report` 分多步写入：

1. upsert `projects`。
2. insert `report_uploads`。
3. 循环 insert `commit_stats`。
4. 循环 upsert `tool_model_stats`。

这些 SQL 当前不在同一个事务中。

### 风险

- 中途失败会留下 `report_uploads` 但缺少部分 `commit_stats`。
- 同项目并发上传时，`commit_stats` 使用 `ON CONFLICT (project_id, sha) DO NOTHING`，先到的数据会赢，后到报告不会更新已有 commit。
- `tool_model_stats` 有 `DO UPDATE`，但只更新部分字段，可能留下旧的 `ai_accepted`、`total_ai_additions` 等字段。

### 建议实现

第一步：把整个上传放入 transaction。

```text
BEGIN
  upsert project
  insert report_upload
  insert/update commit_stats
  insert/update tool_model_stats
COMMIT
```

第二步：明确幂等语义，二选一：

1. **同一报告幂等**：客户端上传 `report_id` 或服务端计算 payload hash，`report_uploads` 对 `(project_id, report_hash)` 加唯一约束。重复上传直接返回已有 upload。
2. **最新报告覆盖**：按 `generated_at` 或 `head_commit` 判断新旧，更新 commit/tool stats；旧报告不覆盖新报告。

第三步：统一 conflict 策略：

- `commit_stats` 不建议长期 `DO NOTHING`，除非明确“首次上传为准”。
- `tool_model_stats` 的 `DO UPDATE` 应更新全部统计字段。

### 验证

- 模拟中途失败，事务回滚后不应留下半个 report。
- 同一 report 并发上传两次，最终只有一份有效数据。
- 较新的 report 与较旧的 report 并发上传时，最终结果符合选定策略。

## P1-2: CAS 数据库和对象存储一致性

### 现状

`enterprise-server/src/handlers/cas.rs` 中 CAS 上传路径：

1. 校验 hash 是十六进制。
2. `INSERT INTO cas_objects ... ON CONFLICT (hash) DO NOTHING`。
3. 写 S3/MinIO。
4. 写 `cas_ownership`。

### 风险

- DB insert 成功但 S3 put 失败，后续查询 DB 可能认为对象存在，但对象内容不可用。
- 服务端只校验 hash 是 hex，没有校验 hash 是否等于 content 的实际哈希。
- 两个用户并发上传同 hash 但不同内容时，DB 的 `ON CONFLICT DO NOTHING` 会保留先到元数据，而 S3 key 可能被后到内容覆盖。

### 建议实现

1. 服务端重新计算 content hash，不信任客户端传入 hash。
2. 如果客户端传入 hash 与服务端计算值不一致，返回 400。
3. `cas_objects` 增加状态字段：

```sql
ALTER TABLE cas_objects
ADD COLUMN status TEXT NOT NULL DEFAULT 'ready';
```

推荐状态流：

```text
pending -> ready
```

上传流程：

1. 插入或锁定 `cas_objects(hash)` 为 `pending`。
2. 写 S3/MinIO。
3. 在 DB 中标记 `ready`，写 ownership。

如果不想引入状态字段，至少调整为：

1. 先写 S3。
2. 再写 DB。
3. 读路径只把 DB 作为授权/索引，S3 不存在时返回明确错误并记录修复任务。

### 验证

- 模拟 S3 put 失败，DB 不应留下 `ready` 对象。
- 并发上传同 hash 同内容，最终只有一个对象，多条 ownership 正确。
- 并发上传同 hash 不同内容，应拒绝 hash 不匹配或内容冲突。

## P1-3: rate limit 多实例共享

### 现状

`enterprise-server/src/services/rate_limit.rs` 当前使用内存 `RwLock<HashMap<...>>` 保存计数。`enterprise-server/src/main.rs` 初始化了 Redis client，但 rate limiter 没有使用 Redis。

### 风险

多实例部署时，每个实例独立计数。假设每实例限制 60 次/分钟，4 个实例后同一用户实际可打到约 240 次/分钟。

### 建议实现

改成 Redis 原子计数。

推荐 key：

```text
git-ai:rate-limit:{tier}:{client-key}:{window-start}
```

推荐命令：

```text
INCR key
EXPIRE key window_seconds
```

注意：

- 第一次 `INCR` 后设置 TTL。
- 可以用 Lua 脚本保证 `INCR + EXPIRE` 原子。
- Redis 不可用时是否 fallback 到内存，需要明确：生产建议 fail closed 或降级但打告警。

### 验证

- 两个 server 实例共享同一个 Redis，连续请求跨实例仍被全局限流。
- Redis 不可用时行为符合配置。

## P2-1: client status 多设备覆盖

### 现状

`enterprise-server/src/services/client_status.rs` 使用 `ON CONFLICT (user_id)` 写 `developer_client_status`。

### 风险

同一个开发者在多台机器上使用时，后到请求会覆盖前一个设备的状态。例如一台机器登出后，可能把另一台仍在线的机器显示为 logged out。

### 建议实现

把唯一维度从 `user_id` 改为设备维度：

- 优先 `(user_id, distinct_id)`
- 如果 distinct_id 缺失，退化到 `(user_id, hostname)`
- 再不行使用 `(user_id, 'unknown')`

展示层再聚合：

- 只要任意设备最近 N 分钟 `logged_in`，用户整体可显示为 online。
- 设备列表展示每台机器的 last_seen、cli_version、os、arch、hostname。

### 验证

- 同一用户两台设备同时登录，状态表有两行。
- 一台设备登出，另一台仍保持在线。

## P2-2: 注册邮箱并发冲突

### 现状

`enterprise-server/src/handlers/auth_api.rs` 注册路径先查邮箱是否存在，再 insert 用户。数据库有唯一约束兜底，但并发注册同邮箱时，第二个请求可能收到原始数据库错误，而不是稳定的 409。

### 建议实现

- 保留前置检查用于更好的用户体验。
- insert 失败时识别唯一约束错误，将其映射为 `AppError::Conflict("Email already exists")`。

### 验证

- 两个并发注册同一邮箱，只有一个成功，另一个稳定返回 409。

## P2-3: release asset 发布一致性

### 现状

`enterprise-server/src/handlers/release.rs` 上传 release asset 时先写 S3/MinIO，再 upsert DB 元数据。删除时只删 DB，不删对象存储。

### 风险

- 上传和下载并发时，DB 和 S3 状态短暂不一致。
- 覆盖同一 `(channel, filename)` 时，下载方可能看到旧/新内容混杂，取决于对象存储读写可见性。
- 删除只删 DB，S3 对象会残留。

### 建议实现

发布系统使用不可变对象 key：

```text
releases/{version}/{filename}
```

channel 只保存指向某个 version 的元数据。发布流程：

1. 上传 version 下所有 asset。
2. 校验 sha256。
3. 原子更新 `release_channels` 指向新 version。

删除时区分：

- 下架：只删除 DB 中 channel/asset 记录。
- 物理清理：后台 job 删除不再引用的 S3 对象。

## 实施顺序

建议按以下顺序推进：

1. P0: migration advisory lock。
2. P0: OAuth 一次性凭证原子消费。
3. P1: report upload transaction。
4. P1: CAS hash 校验和 ready 状态。
5. P1: Redis rate limit。
6. P2: client status 多设备。
7. P2: 注册冲突错误映射。
8. P2: release asset 不可变发布流程。

## 发布前检查清单

- 多实例同时启动不会重复执行 migration。
- 同一个 refresh token 并发兑换时只有一个请求成功。
- 同一个 install nonce 并发兑换时只有一个请求成功。
- 同一个 device code 并发兑换时只有一个请求成功。
- report 上传失败不会留下部分数据。
- CAS 上传失败不会留下 `ready` 但无法读取的对象。
- 多实例部署时 rate limit 结果与单实例一致。
- 同一用户多设备状态不会互相覆盖。

