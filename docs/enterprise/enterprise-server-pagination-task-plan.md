# Enterprise Server 数据分页改造任务清单

本文档把 Enterprise Server 中适合分页的数据读取接口拆成可以逐步执行、逐步验证、逐步提交的工程任务。

目标不是给所有接口都机械增加 `page` 参数，而是优先处理会随使用时间和客户规模持续增长的数据面：日志、PR 列表、用户/API key 管理列表、dashboard 明细聚合表。

## 执行原则

1. 每次只执行一个阶段，完成测试后单独提交。
2. 先做通用分页协议和日志类接口，再做管理后台列表，最后做 dashboard 聚合类接口。
3. 所有改造默认保持向后兼容：未传分页参数时继续返回第一页，默认 `limit` 不超过 100。
4. 对日志、PR、按时间排序的列表优先使用 cursor/keyset pagination，避免大 offset 扫描。
5. 对 dashboard 聚合接口，禁止只在 Rust 里 `fetch_all` 后切片；必须把分页下推到 SQL、rollup、物化视图或等价的数据库层。
6. 每个阶段都要记录测试命令、响应样例和性能对比。

## 接口优先级

### P0: 必须优先分页

| 接口 | 当前问题 | 推荐分页方式 |
| --- | --- | --- |
| `GET /api/v1/audit-log` | 只有 `limit`，没有 `cursor`，不能稳定翻页 | cursor，排序键 `(created_at, id)` |
| `GET /api/admin/cas-access-log` | 只有 `limit`，没有 `cursor`，CAS 访问日志会持续增长 | cursor，排序键 `(created_at, id)` |
| `GET /api/v1/aggregate/pull-requests` | `ORDER BY merged_at DESC` 后 `fetch_all`，PR 数据会持续增长 | cursor，排序键 `(merged_at, id)` |
| `GET /api/admin/users/list` | 一次性返回所有用户，并聚合每个用户的 API keys | cursor，排序键 `(created_at, id)` |
| `GET /api/v1/aggregate/projects` | 从两套来源全量拉取后在 Rust 中 merge/sort | 数据库层分页或项目 rollup |
| `GET /api/v1/aggregate/developers` | 大 CTE + JSON 聚合后 `fetch_all` | 数据库层分页或开发者 rollup |

### P1: 中优先级分页

| 接口 | 当前问题 | 推荐分页方式 |
| --- | --- | --- |
| `GET /api/admin/api-keys` | 全量返回所有未撤销 API keys | cursor，排序键 `(created_at, id)` |
| `GET /api/admin/users/{id}/api-keys` | 单用户 API keys 全量返回 | cursor，排序键 `(created_at, id)` |
| `GET /api/admin/organizations/list` | 组织全量返回，`include_personal=true` 时可能较大 | keyset，排序键 `(name, id)` |
| `GET /api/admin/departments` | 部门全量返回，未传 `org_id` 时跨组织 | keyset，排序键 `(org_name, name, id)` |
| `GET /api/v1/aggregate/organizations` | dashboard 组织表全量返回 | keyset 或 `limit` + `cursor` |
| `GET /api/v1/aggregate/departments` | dashboard 部门表全量返回 | keyset 或 `limit` + `cursor` |
| `GET /api/v1/aggregate/tools` | 工具/模型聚合后全量返回 | `limit` + 排名 cursor，或 top N |
| `GET /api/v1/agent-readiness` | readiness 历史/工具模型全量返回 | cursor 或 top N |

### P2: 不建议优先分页

| 接口 | 原因 | 建议 |
| --- | --- | --- |
| `GET /api/v1/aggregate/summary` | 返回标量摘要，不是列表 | 保持不分页 |
| `GET /api/v1/aggregate/trends` | 时间序列更适合限制时间范围 | 限制 `since/until` 和最大 bucket 数 |
| `GET /api/v1/ai-code-persistence` | 主要返回最新 snapshot 和趋势 | 限制时间范围和 bucket 数 |
| `GET /api/v1/ai-code-lifecycle` | 单 commit 生命周期 | 对 CI/alert 子列表可加最大数量，不做通用分页 |
| `GET /api/admin/feature-flags` | 配置量级数据 | 保持不分页 |
| `GET /api/admin/releases/assets` | 通常数据量较小 | 暂不分页，后续按版本保留策略处理 |
| 上传类接口 | 请求体批量写入，不是响应列表 | 做 batch size limit，不做分页 |

## 统一分页协议

### 请求参数

所有新增分页接口统一支持：

```text
limit=50
cursor=<opaque-cursor>
```

约束：

- `limit` 默认值：`100`。
- `limit` 最小值：`1`。
- `limit` 最大值：`1000`，对 dashboard 聚合接口建议最大 `200`。
- `cursor` 是服务端生成的不透明字符串，客户端不能解析其内部结构。
- 需要兼容历史请求：未传 `limit` 和 `cursor` 时返回第一页。

### 响应结构

推荐响应结构：

```json
{
  "items": [],
  "pagination": {
    "limit": 100,
    "has_more": false,
    "next_cursor": null
  }
}
```

对已有响应字段保持兼容。例如 `audit-log` 当前返回 `entries`，可以先返回：

```json
{
  "entries": [],
  "count": 0,
  "pagination": {
    "limit": 100,
    "has_more": false,
    "next_cursor": null
  }
}
```

### Cursor 内容

cursor 可以使用 base64url 编码的 JSON，内部包含排序键和版本：

```json
{
  "v": 1,
  "created_at": "2026-07-09T10:00:00Z",
  "id": 123
}
```

实现要求：

- cursor 解码失败时返回 `400 Bad Request`。
- cursor 字段必须与接口排序字段匹配。
- cursor 不应暴露敏感数据。
- 获取数据时查询 `limit + 1` 条，用多出来的一条判断 `has_more`。

### SQL 模式

时间倒序列表使用 keyset 查询：

```sql
WHERE
  ($cursor_created_at IS NULL OR (created_at, id) < ($cursor_created_at, $cursor_id))
ORDER BY created_at DESC, id DESC
LIMIT $limit_plus_one
```

如果排序字段可能为空，先明确空值位置。例如 PR 使用：

```sql
ORDER BY merged_at DESC NULLS LAST, id DESC
```

并在 cursor 条件中匹配同样的空值规则。更简单的第一版可以只分页 `merged_at IS NOT NULL` 的主列表，未合并 PR 另设过滤或排在最后。

## 阶段 0: 基线和接口确认

目标：在改代码前记录当前行为、数据量和性能基线。

### 0.1 确认工作区状态

步骤：

- [x] 查看当前工作区改动：

```bash
git status --short
```

- [x] 确认本次只改分页相关文件和测试。
- [x] 如果已有其他未提交改动，记录它们，不要顺手重构。

验收标准：

- [x] 工作区状态已记录。
- [x] 本次改造范围明确。

### 0.2 确认服务端测试可运行

步骤：

- [x] 进入服务端目录：

```bash
cd enterprise-server
```

- [x] 启动本地依赖：

```bash
docker compose up -d postgres redis minio
```

- [x] 确认依赖健康：

```bash
docker compose ps
docker compose exec postgres pg_isready -U gitai -d gitai_enterprise
docker compose exec redis redis-cli ping
```

- [x] 运行测试：

```bash
cargo test
```

验收标准：

- [x] Postgres、Redis、MinIO 可用。
- [x] `cargo test` 通过，或失败原因已记录且与分页无关。

### 0.3 记录核心表数据量

步骤：

- [x] 查询日志、PR、用户和 dashboard 明细表数据量：

```bash
docker compose exec -T postgres psql -U gitai -d gitai_enterprise -c "
SELECT 'audit_log' AS table_name, COUNT(*) FROM audit_log
UNION ALL SELECT 'cas_access_log', COUNT(*) FROM cas_access_log
UNION ALL SELECT 'pull_requests', COUNT(*) FROM pull_requests
UNION ALL SELECT 'users', COUNT(*) FROM users
UNION ALL SELECT 'api_keys', COUNT(*) FROM api_keys
UNION ALL SELECT 'organizations', COUNT(*) FROM organizations
UNION ALL SELECT 'departments', COUNT(*) FROM departments
UNION ALL SELECT 'metrics_events', COUNT(*) FROM metrics_events
UNION ALL SELECT 'projects', COUNT(*) FROM projects
UNION ALL SELECT 'commit_stats', COUNT(*) FROM commit_stats
UNION ALL SELECT 'tool_model_stats', COUNT(*) FROM tool_model_stats;
"
```

- [x] 记录结果到 PR 描述或执行记录。

验收标准：

- [x] 已知道哪些表在本地或测试环境中数据量最大。
- [x] 后续性能对比有基准。

### 0.4 记录当前接口响应

步骤：

- [x] 启动 API：

```bash
docker compose up -d --build api
```

- [x] 准备一个 admin token 或测试环境认证方式。
- [x] 记录以下接口当前响应字段：

```bash
curl -sS "http://127.0.0.1:8080/api/v1/audit-log?limit=3" -H "Authorization: Bearer $TOKEN"
curl -sS "http://127.0.0.1:8080/api/admin/cas-access-log?limit=3" -H "Authorization: Bearer $TOKEN"
curl -sS "http://127.0.0.1:8080/api/v1/aggregate/pull-requests" -H "Authorization: Bearer $TOKEN"
curl -sS "http://127.0.0.1:8080/api/admin/users/list" -H "Authorization: Bearer $TOKEN"
```

验收标准：

- [x] 已确认每个接口当前顶层字段名。
- [x] 后续改造不会破坏既有字段。

### 阶段 0 执行记录

执行日期：2026-07-09

工作区状态：

| 文件 | 状态 | 说明 |
| --- | --- | --- |
| `enterprise-server/src/handlers/dashboard.rs` | modified | 阶段 0 开始前已存在的未提交改动，本次未修改 |
| `docs/enterprise/enterprise-server-pagination-task-plan.md` | untracked | 本次新增的分页任务文档 |

环境和测试：

| 项目 | 结果 |
| --- | --- |
| `docker compose up -d postgres redis minio` | Postgres、Redis、MinIO 已运行；API 容器也已处于 running |
| `docker compose ps` | API healthy，Postgres healthy，Redis healthy，MinIO running |
| `pg_isready` | `/var/run/postgresql:5432 - accepting connections` |
| `redis-cli ping` | `PONG` |
| `cargo test` | 120 passed, 0 failed；存在既有 warning |
| `docker compose up -d --build api` | 镜像构建成功；Compose 返回一次容器名冲突错误，但随后 API 容器已重建并 healthy |
| `/health` | `{"service":"git-ai-enterprise-server","status":"ok","version":"0.1.0"}` |
| `/ready` | `{"checks":{"database":"ok"},"status":"ready"}` |

核心表数据量：

| 表 | 行数 |
| --- | ---: |
| `audit_log` | 10104 |
| `cas_access_log` | 0 |
| `pull_requests` | 0 |
| `users` | 305 |
| `api_keys` | 23 |
| `organizations` | 3 |
| `departments` | 3 |
| `metrics_events` | 558688 |
| `projects` | 102 |
| `commit_stats` | 50005 |
| `tool_model_stats` | 304 |

认证方式：

| 项目 | 结果 |
| --- | --- |
| admin 用户 | 使用本地库中的 `admin@linewell.com` owner 用户 |
| token | 使用运行中 API 容器的 `JWT_SECRET` 生成 1 小时有效的临时 Bearer JWT |
| 数据库写入 | 未创建用户、未创建 API key、未写入认证数据 |

接口响应形状：

| 接口 | 状态 | 顶层字段 | 数量字段 | 首条记录字段 |
| --- | --- | --- | --- | --- |
| `GET /api/v1/audit-log?limit=3` | 200 | `count`, `entries` | `count=3`, `entries_len=3` | `action`, `created_at`, `details`, `id`, `ip_address`, `org_id`, `resource_id`, `resource_type`, `user_agent`, `user_id` |
| `GET /api/admin/cas-access-log?limit=3` | 200 | `count`, `entries` | `count=0`, `entries_len=0` | 无数据 |
| `GET /api/admin/users/list` | 200 | `users` | `users_len=305` | `api_keys`, `created_at`, `email`, `id`, `name`, `personal_org_id` |
| `GET /api/v1/aggregate/pull-requests` | 500 | `error` | 无 | 无 |

已发现问题：

| 问题 | 影响 | 证据 |
| --- | --- | --- |
| `GET /api/v1/aggregate/pull-requests` 当前返回 500 | 阶段 4 改造前需要先修正 SQL 类型转换，否则无法做成功响应基线 | API 日志显示 Postgres `42883`: `operator does not exist: timestamp with time zone >= text` |

修复记录：

| 日期 | 修复 | 验证 |
| --- | --- | --- |
| 2026-07-09 | 将 `aggregate_pull_requests` 查询中的 `merged_at >= $3` 和 `merged_at <= $4` 改为显式 `timestamptz` cast | 临时本地服务 `GET /api/v1/aggregate/pull-requests` 返回 200，顶层字段为 `pull_requests`, `summary` |

阶段 0 结论：

- 本地服务端测试和基础依赖可用，可以进入阶段 1。
- 数据量最大的是 `metrics_events`、`commit_stats`、`audit_log`，符合分页优先级判断。
- P0 中的 `pull-requests` 接口存在既有 500；已在 2026-07-09 修复 SQL cast 并验证返回 200。

## 阶段 1: 增加通用分页基础设施

目标：提供所有接口共用的 `limit` 校验、cursor 编解码、分页响应构造，避免每个 handler 自己实现一套。

### 1.1 新增分页模块

涉及文件：

- `enterprise-server/src/handlers/mod.rs` 或 `enterprise-server/src/pagination.rs`
- `enterprise-server/src/main.rs` 或对应模块声明文件

实现步骤：

- [x] 新增 `pagination` 模块。
- [x] 定义分页查询结构：

```rust
#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}
```

- [x] 增加 `clamp_limit(input, default, max) -> i64`。
- [x] 增加 cursor encode/decode helper。
- [x] 为日志类 cursor 定义结构：

```rust
pub struct TimeIdCursor {
    pub timestamp: DateTime<Utc>,
    pub id: i64,
}
```

- [x] 为 UUID id 列表定义结构：

```rust
pub struct TimeUuidCursor {
    pub timestamp: DateTime<Utc>,
    pub id: Uuid,
}
```

- [x] 增加 `has_more` 和 `next_cursor` 构造 helper。

验收标准：

- [x] helper 不依赖具体业务 handler。
- [x] cursor 对客户端保持 opaque。
- [x] 错误统一走现有 `AppError::BadRequest`。

### 1.2 增加单元测试

涉及文件：

- `enterprise-server/src/pagination.rs`

测试步骤：

- [x] 测试 `limit=None` 返回默认值。
- [x] 测试 `limit=0` 被提升到最小值。
- [x] 测试 `limit` 超过最大值时被 clamp。
- [x] 测试 cursor encode 后可以 decode。
- [x] 测试非法 cursor 返回错误。
- [x] 测试 cursor 版本不支持时返回错误。

测试命令：

```bash
cd enterprise-server
cargo test pagination
```

验收标准：

- [x] `cargo test pagination` 通过。
- [x] 不影响其他 handler 编译。

提交建议：

```bash
git add enterprise-server/src
git commit -m "Add pagination helpers"
```

### 阶段 1 执行记录

执行日期：2026-07-09

实现内容：

| 文件 | 内容 |
| --- | --- |
| `enterprise-server/src/pagination.rs` | 新增分页 query、limit clamp、cursor encode/decode、`TimeIdCursor`、`TimeUuidCursor`、`pagination_meta`、`truncate_to_limit` |
| `enterprise-server/src/main.rs` | 挂载 `pagination` 模块 |
| `enterprise-server/src/handlers/lifecycle.rs` | 修复 `aggregate_pull_requests` 的 `since/until` SQL cast |

验证结果：

| 命令或检查 | 结果 |
| --- | --- |
| `cargo fmt` | 执行过；产生全仓格式化噪音，已回退非任务范围改动 |
| `cargo test pagination` | 9 passed, 0 failed |
| `cargo test lifecycle` | 0 matched tests, 编译通过 |
| `cargo test` | 129 passed, 0 failed |
| 临时服务 `GET /api/v1/aggregate/pull-requests` | 200；`pull_requests_len=0`，`summary` 字段存在 |

备注：

- 阶段 1 的 helper 暂未接入业务 handler，已在模块内允许阶段性 `dead_code`，避免在阶段 2 前产生新增未使用 warning。
- 当前测试输出仍有既有 warning，未在本阶段处理。

## 阶段 2: 改造日志类接口

目标：先处理增长最快、风险最低的日志类接口。

### 2.1 改造 audit log

涉及文件：

- `enterprise-server/src/handlers/admin.rs`
- 可能新增 migration 文件

实现步骤：

- [x] 扩展 `AuditLogQuery`，增加 `cursor: Option<String>`。
- [x] 继续保留 `user_id`、`org_id`、`action`、`limit` 参数。
- [x] 解码 cursor 为 `(created_at, id)`。
- [x] 查询改为 `ORDER BY created_at DESC, id DESC`。
- [x] 增加 cursor 条件：

```sql
AND (
  $4::timestamptz IS NULL
  OR (created_at, id) < ($4::timestamptz, $5::bigint)
)
```

- [x] 查询 `limit + 1` 条。
- [x] 如果结果超过 `limit`，截断最后一条并生成 `next_cursor`。
- [x] 响应中保留 `entries` 和 `count`，新增 `pagination`。

建议索引：

```sql
CREATE INDEX IF NOT EXISTS idx_audit_log_created_id_desc
ON audit_log (created_at DESC, id DESC);
```

如果常用过滤较多，可补充：

```sql
CREATE INDEX IF NOT EXISTS idx_audit_log_org_created_id_desc
ON audit_log (org_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_audit_log_user_created_id_desc
ON audit_log (user_id, created_at DESC, id DESC);
```

验收标准：

- [x] `GET /api/v1/audit-log?limit=10` 返回最多 10 条。
- [x] 第一页有更多数据时返回 `pagination.next_cursor`。
- [x] 使用 `cursor` 请求第二页时不重复第一页最后一条。
- [x] `user_id/org_id/action` 过滤和 cursor 同时生效。
- [x] 非法 cursor 返回 400。

### 2.2 改造 CAS access log

涉及文件：

- `enterprise-server/src/handlers/admin.rs`
- 可能新增 migration 文件

实现步骤：

- [x] 扩展 `CasAccessLogQuery`，增加 `cursor: Option<String>`。
- [x] 继续保留 `cas_hash`、`user_id`、`org_id`、`limit` 参数。
- [x] 查询改为 `ORDER BY created_at DESC, id DESC`。
- [x] 使用 `(created_at, id)` cursor 条件。
- [x] 查询 `limit + 1` 条。
- [x] 响应中保留 `entries` 和 `count`，新增 `pagination`。

建议索引：

```sql
CREATE INDEX IF NOT EXISTS idx_cas_access_log_created_id_desc
ON cas_access_log (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_cas_access_log_hash_created_id_desc
ON cas_access_log (cas_hash, created_at DESC, id DESC);
```

验收标准：

- [x] `GET /api/admin/cas-access-log?limit=10` 返回最多 10 条。
- [x] `cursor` 翻页无重复、无明显漏项。
- [x] `cas_hash/user_id/org_id` 过滤和 cursor 同时生效。

### 2.3 日志类接口测试

测试步骤：

- [x] 增加或更新 handler/integration 测试，插入至少 3 条 audit log。
- [x] 用 `limit=2` 断言第一页 2 条、`has_more=true`。
- [x] 用 `next_cursor` 断言第二页返回剩余数据。
- [x] 插入相同 `created_at` 的多条记录，确认 `id` 作为 tie breaker 生效。
- [x] 对 CAS access log 做同样测试。

测试命令：

```bash
cd enterprise-server
cargo test audit_log
cargo test cas_access_log
cargo test
```

提交建议：

```bash
git add enterprise-server/src enterprise-server/migrations
git commit -m "Add cursor pagination for audit logs"
```

### 阶段 2 执行记录

执行日期：2026-07-09

实现内容：

| 文件 | 内容 |
| --- | --- |
| `enterprise-server/src/handlers/admin.rs` | `audit_log` 和 `cas_access_log` 增加 cursor 参数、稳定排序、`limit + 1` 查询、`pagination` 响应元数据和数据库级 handler 测试 |
| `enterprise-server/migrations/020_log_pagination_indexes.sql` | 新增日志分页复合索引，覆盖无过滤、user、org、action/cas_hash 过滤场景 |
| `enterprise-server/src/db/migrations.rs` | 注册 `020_log_pagination_indexes` |

验证结果：

| 命令或检查 | 结果 |
| --- | --- |
| `cd enterprise-server && cargo test decode_log_cursor` | 3 passed, 0 failed |
| `cd enterprise-server && cargo test audit_log_cursor_paginates_without_repeating_tie_breaker_ids` | 1 passed, 0 failed |
| `cd enterprise-server && cargo test cas_access_log_cursor_paginates_with_hash_filter` | 1 passed, 0 failed |
| `cd enterprise-server && cargo test pagination` | 9 passed, 0 failed |
| `cd enterprise-server && cargo test db::migrations` | 1 passed, 0 failed |
| `cd enterprise-server && cargo test` | 134 passed, 0 failed |
| 本地 Postgres `PREPARE/EXECUTE` 同形 SQL | `audit_log` 和 `cas_access_log` 查询均可执行 |
| `cargo run -- --migrate` | 当前开发库已应用 `020_log_pagination_indexes` |

备注：

- 响应兼容旧字段：`entries` 和 `count` 保留，仅新增 `pagination`。
- 当前测试输出仍有既有 warning，未在本阶段处理。

## 阶段 3: 改造管理后台列表

目标：处理 admin 页面中可能随企业规模增长的列表接口。

### 3.1 改造 users list

涉及文件：

- `enterprise-server/src/handlers/admin.rs`

实现步骤：

- [x] 为 `list_users` 增加 query extractor。
- [x] 支持 `limit` 和 `cursor`。
- [x] 查询排序改为 `ORDER BY u.created_at DESC, u.id DESC`。
- [x] cursor 使用 `(u.created_at, u.id)`。
- [x] 查询 `limit + 1` 个用户。
- [x] 保留当前 `users` 字段，新增 `pagination`。
- [x] 评估是否继续在列表中返回完整 `api_keys`：
  - 第一版可保持兼容。
  - 后续优化建议列表只返回 `api_key_count`，详情页调用 `/api/admin/users/{id}/api-keys`。

验收标准：

- [x] 旧请求 `/api/admin/users/list` 仍返回 `users`。
- [x] 新请求 `/api/admin/users/list?limit=20` 返回最多 20 个用户。
- [x] cursor 翻页不会重复用户。
- [x] 用户 API keys 聚合不破坏分页条数。

### 3.2 改造 API keys 列表

涉及文件：

- `enterprise-server/src/handlers/admin.rs`

实现步骤：

- [x] 为 `list_api_keys` 增加 `limit` 和 `cursor`。
- [x] 排序改为 `ORDER BY created_at DESC, id DESC`。
- [x] cursor 使用 `(created_at, id)`。
- [x] 响应保留当前 keys 字段名，新增 `pagination`。
- [x] 为 `list_user_api_keys` 做同样改造，额外保留 `user_id = $1` 过滤。

建议索引：

```sql
CREATE INDEX IF NOT EXISTS idx_api_keys_active_created_id_desc
ON api_keys (created_at DESC, id DESC)
WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_api_keys_user_active_created_id_desc
ON api_keys (user_id, created_at DESC, id DESC)
WHERE revoked_at IS NULL;
```

验收标准：

- [x] 全局 API keys 列表可翻页。
- [x] 单用户 API keys 列表可翻页。
- [x] revoked keys 不出现在分页结果中。

### 3.3 改造 organizations 和 departments

涉及文件：

- `enterprise-server/src/handlers/admin.rs`

实现步骤：

- [x] `list_organizations` 增加 `limit` 和 `cursor`。
- [x] 排序保持 `ORDER BY o.name ASC, o.id ASC`。
- [x] cursor 使用 `(name, id)`。
- [x] `include_personal` 过滤必须在 cursor 条件前后保持一致。
- [x] `list_departments` 增加 `limit` 和 `cursor`。
- [x] 排序保持 `ORDER BY o.name ASC, d.name ASC, d.id ASC`。
- [x] cursor 使用 `(org_name, department_name, department_id)`。

验收标准：

- [x] 组织列表名称排序稳定。
- [x] 部门列表跨组织排序稳定。
- [x] `org_id` 过滤和 cursor 同时生效。
- [x] `include_personal=true` 和默认行为都可分页。

### 3.4 管理后台列表测试

测试步骤：

- [x] 为 users 创建至少 5 条数据，用 `limit=2` 测三页。
- [x] 为 API keys 创建 active/revoked 混合数据，确认只分页 active。
- [x] 为 organizations 创建同名或相近名称数据，确认 id tie breaker 生效。
- [x] 为 departments 创建跨组织数据，确认排序和过滤稳定。

测试命令：

```bash
cd enterprise-server
cargo test admin
cargo test
```

提交建议：

```bash
git add enterprise-server/src enterprise-server/migrations
git commit -m "Paginate admin list APIs"
```

### 阶段 3 执行记录

执行日期：2026-07-09

实现内容：

| 文件 | 内容 |
| --- | --- |
| `enterprise-server/src/handlers/admin.rs` | `users`、全局 `api_keys`、单用户 `api_keys`、`organizations`、`departments` 列表增加 cursor 分页和 `pagination` 响应元数据 |
| `enterprise-server/migrations/021_admin_list_pagination_indexes.sql` | 新增 users、api_keys、organizations、departments、org_members 相关复合索引 |
| `enterprise-server/src/db/migrations.rs` | 注册 `021_admin_list_pagination_indexes` |

验证结果：

| 命令或检查 | 结果 |
| --- | --- |
| `cd enterprise-server && cargo test admin` | 10 passed, 0 failed |
| `cd enterprise-server && cargo test pagination` | 14 passed, 0 failed |
| `cd enterprise-server && cargo test db::migrations` | 1 passed, 0 failed |
| `cd enterprise-server && cargo test` | 137 passed, 0 failed |
| `cargo run -- --migrate` | 当前开发库已应用 `021_admin_list_pagination_indexes` |

备注：

- users 列表当前继续返回完整 `api_keys` 聚合，以保持兼容；后续可单独优化为 `api_key_count` + 详情接口。
- 当前测试输出仍有既有 warning，未在本阶段处理。

## 阶段 4: 改造 PR 聚合接口

目标：让 PR 明细列表可分页，同时保留 summary 的全量统计语义。

### 4.1 改造 pull request list

涉及文件：

- `enterprise-server/src/handlers/lifecycle.rs`

实现步骤：

- [x] 扩展 `PrAggregateQuery`，增加 `limit` 和 `cursor`。
- [x] 明确排序：`ORDER BY merged_at DESC NULLS LAST, id DESC`。
- [x] cursor 使用 `(merged_at, id)`。
- [x] 查询 PR 列表时使用 `limit + 1`。
- [x] 响应保留 `pull_requests`，新增 `pagination`。

验收标准：

- [x] `repo/org/since/until` 过滤和 cursor 同时生效。
- [x] `limit=20` 返回最多 20 条 PR。
- [x] 第二页不重复第一页数据。

### 4.2 拆分 summary 查询

当前 `summary.total_prs`、`avg_pct_ai` 和 size distribution 是基于已经返回的 `prs` 计算的。分页后如果继续这样做，summary 会变成“当前页 summary”，语义会改变。

实现步骤：

- [x] 新增独立 SQL 查询，按同样过滤条件计算全量 summary。
- [x] summary 查询不受 `limit/cursor` 影响。
- [x] 响应中保留原 `summary` 字段。
- [ ] 可选新增 `page_summary`，如果前端需要当前页统计。

验收标准：

- [x] 第一页和第二页返回的 `summary.total_prs` 一致。
- [x] `summary.total_prs` 等于过滤条件下的全量 PR 数。
- [x] `pull_requests.len()` 只代表当前页条数。

### 4.3 PR 索引和测试

建议索引：

```sql
CREATE INDEX IF NOT EXISTS idx_pull_requests_merged_id_desc
ON pull_requests (merged_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_pull_requests_org_merged_id_desc
ON pull_requests (org_id, merged_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_pull_requests_repo_merged_id_desc
ON pull_requests (repo_url, merged_at DESC, id DESC);
```

测试步骤：

- [x] 插入至少 5 条 PR。
- [x] 用 `limit=2` 翻三页。
- [x] 测试 `repo` 过滤。
- [x] 测试 `org` 过滤。
- [x] 测试 `since/until` 过滤。
- [x] 确认 summary 不随 page 改变。

测试命令：

```bash
cd enterprise-server
cargo test pull_requests
cargo test lifecycle
cargo test
```

提交建议：

```bash
git add enterprise-server/src enterprise-server/migrations
git commit -m "Paginate pull request aggregation"
```

### 阶段 4 执行记录

执行日期：2026-07-09

实现内容：

| 文件 | 内容 |
| --- | --- |
| `enterprise-server/src/handlers/lifecycle.rs` | `aggregate_pull_requests` 增加 cursor 分页、稳定排序、`pagination` 响应元数据，并把 summary 拆成独立全量聚合 SQL |
| `enterprise-server/migrations/022_pull_request_pagination_indexes.sql` | 新增 PR 分页复合索引，覆盖无过滤、org 过滤和 repo 过滤场景 |
| `enterprise-server/src/db/migrations.rs` | 注册 `022_pull_request_pagination_indexes` |

验证结果：

| 命令或检查 | 结果 |
| --- | --- |
| `cd enterprise-server && cargo test pull_requests` | 2 passed, 0 failed |
| `cd enterprise-server && cargo test lifecycle` | 2 passed, 0 failed |
| `cd enterprise-server && cargo test db::migrations` | 1 passed, 0 failed |
| `cd enterprise-server && cargo test` | 139 passed, 0 failed |

备注：

- `pull_requests` 旧字段保留；新增 `pagination`。
- `summary` 继续表示过滤条件下的全量统计，不随当前页变化。
- 当前测试输出仍有既有 warning，未在本阶段处理。

## 阶段 5: 改造 dashboard 聚合明细

目标：处理最重的 dashboard 表格接口。此阶段要谨慎，不要用内存切片伪分页。

### 5.1 改造 developers aggregation

涉及文件：

- `enterprise-server/src/handlers/dashboard.rs`

实现步骤：

- [x] 为 `aggregate_developers` 增加 `limit` 和 `cursor`。
- [x] 保持当前排序语义：`ai_added_lines DESC, total_commits DESC, name ASC`。
- [x] cursor 包含：

```json
{
  "ai_added_lines": 1000,
  "total_commits": 50,
  "name": "Alice",
  "user_id": "..."
}
```

- [x] 将排序和 cursor 条件放到 SQL 最终查询中。
- [x] 查询 `limit + 1` 条。
- [x] 确保 `git_identities` JSON 聚合只对当前页或候选页执行；如果 SQL 结构复杂，先用 CTE 得到分页 user ids，再 join identities。

推荐 SQL 结构：

1. `developer_stats` 聚合出所有候选开发者统计。
2. `ranked_developers` 应用排序和 cursor 条件。
3. `paged_developers` `LIMIT limit + 1`。
4. 最后只对 `paged_developers` join users、departments、git identities。

验收标准：

- [x] 排序和改造前一致。
- [x] 第一页和第二页没有重复开发者。
- [x] `since/until/org` 过滤仍然生效。
- [ ] 大数据集下 `EXPLAIN ANALYZE` 显示返回行数和后续 join 行数明显降低。

### 5.2 改造 projects aggregation

涉及文件：

- `enterprise-server/src/handlers/dashboard.rs`

当前风险：

- `aggregate_projects` 分别从 `metrics_events` 和 `projects + commit_stats` 全量查询。
- Rust 中用 `HashMap` merge，再 sort。
- 如果只在 `result` 上切片，只能减少响应大小，不能减少数据库和服务端内存成本。

推荐方案 A：建立项目 rollup 查询或视图。

实现步骤：

- [x] 用 SQL `UNION ALL` 统一 metrics source 和 report source。
- [x] 在 SQL 中按项目 key 聚合。
- [x] 在 SQL 中完成排序和分页。
- [x] Rust handler 只负责序列化当前页。

推荐方案 B：新增项目 rollup 表。

适用场景：

- `metrics_events` 和 `commit_stats` 数据量已经很大。
- dashboard projects 页面是高频访问。
- 需要跨 org、时间范围、项目名排序稳定。

实现步骤：

- [ ] 新增 rollup 表，例如 `project_aggregate_rollups`。
- [ ] 写入 metrics/report 时同步维护 rollup。
- [ ] dashboard 查询直接读 rollup。
- [ ] 用 `(project_name, project_key)` 做 keyset pagination。

第一版建议：

- [x] 先做方案 A，避免引入新的写入路径复杂度。
- [ ] 如果压测仍然慢，再进入 rollup 表方案。

验收标准：

- [x] 不再出现两个大 `fetch_all` 后 Rust merge 全量数据的路径。
- [x] 分页发生在 SQL 层。
- [x] 项目名称排序稳定。
- [ ] 同一个项目来自 metrics/report 两套来源时仍正确合并。

### 5.3 改造 organizations、departments、tools

涉及文件：

- `enterprise-server/src/handlers/dashboard.rs`

实现步骤：

- [x] `aggregate_organizations` 支持 `limit/cursor`，排序 `(organization_name, org_slug)`。
- [x] `aggregate_departments` 支持 `limit/cursor`，排序 `(organization_name, department_name, dept_slug)`。
- [x] `aggregate_tools` 支持 `limit/cursor` 或 `limit/top_n`。
- [x] 对 tools 保持当前排序 `ai_additions DESC`，cursor 使用 `(ai_additions, tool_model)`。
- [x] 如果 tools 仍需从三套来源 merge，先在 SQL/rollup 层合并；不能只切最终 `Vec`。

验收标准：

- [x] 每个接口旧字段名保持不变。
- [x] 新增 `pagination`。
- [x] 排序稳定。
- [x] 过滤条件和 cursor 同时生效。

### 5.4 dashboard 聚合性能验证

步骤：

- [ ] 准备包含大量 `metrics_events` 的测试库。
- [ ] 记录改造前 `aggregate_developers`、`aggregate_projects` 响应时间。
- [ ] 记录改造后第一页响应时间。
- [ ] 记录第二页响应时间。
- [ ] 对关键 SQL 运行 `EXPLAIN ANALYZE`。

建议命令：

```bash
docker compose exec -T postgres psql -U gitai -d gitai_enterprise -c "
EXPLAIN ANALYZE
SELECT ...
"
```

验收标准：

- [ ] 首屏 dashboard 表格响应时间下降。
- [ ] API 内存峰值下降。
- [ ] JSON 响应体大小下降。
- [ ] SQL 没有新增明显全表排序瓶颈；如果有，补充索引或 rollup。

提交建议：

```bash
git add enterprise-server/src enterprise-server/migrations
git commit -m "Paginate dashboard aggregate APIs"
```

### 阶段 5 执行记录

执行日期：2026-07-09

实现内容：

| 文件 | 内容 |
| --- | --- |
| `enterprise-server/src/handlers/dashboard.rs` | dashboard 聚合接口增加 `limit/cursor`、cursor 编解码、数据库层排序分页和 handler 级分页测试 |
| `enterprise-server/migrations/023_department_rollup_indexes.sql` | 保留既有 department rollup 查询索引 |
| `enterprise-server/migrations/024_dashboard_aggregate_pagination_indexes.sql` | 新增 dashboard 聚合分页相关复合索引 |
| `enterprise-server/src/db/migrations.rs` | 注册 `023_department_rollup_indexes` 和 `024_dashboard_aggregate_pagination_indexes` |

验证结果：

| 命令或检查 | 结果 |
| --- | --- |
| `cd enterprise-server && cargo test dashboard` | 9 passed, 0 failed |
| `cd enterprise-server && cargo test db::migrations` | 1 passed, 0 failed |
| `cd enterprise-server && cargo test` | 141 passed, 0 failed |
| `cargo run -- --migrate` | 当前开发库已应用 `024_dashboard_aggregate_pagination_indexes`；`023_department_rollup_indexes` 已处于 applied 状态 |

备注：

- `organizations`、`departments`、`projects`、`developers`、`tools` 旧列表字段保留，仅新增 `pagination`。
- `projects` 已改为 SQL CTE 聚合和分页，不再在 Rust 中对两个全量 `fetch_all` 结果做 `HashMap` merge 后切片。
- `tools` 已改为 SQL 层合并 report、metrics、checkpoint 来源后分页。
- 本阶段尚未做大数据集 `EXPLAIN ANALYZE` 和前后性能对比，阶段 5.4 保持未完成。

## 阶段 6: 时间序列和生命周期接口保护

目标：对不适合分页的接口增加范围限制，避免大时间窗口返回过多数据。

### 6.1 限制 trends bucket 数

涉及文件：

- `enterprise-server/src/handlers/dashboard.rs`

实现步骤：

- [ ] 为 `aggregate_trends` 增加最大 bucket 规则。
- [ ] day 粒度最大 366 个 bucket。
- [ ] week 粒度最大 260 个 bucket。
- [ ] month 粒度最大 120 个 bucket。
- [ ] 超出范围时返回 400，提示缩小 `since/until` 或降低粒度。

验收标准：

- [ ] 无 `since` 的请求仍有默认范围，或明确被拒绝。
- [ ] 超大时间范围不会返回无限长数组。

### 6.2 限制 persistence trend

涉及文件：

- `enterprise-server/src/handlers/lifecycle.rs`

实现步骤：

- [ ] 为 `get_ai_code_persistence` 增加 `until` 或默认最近一年。
- [ ] trend 查询增加时间范围。
- [ ] 对无时间范围请求使用安全默认值。

验收标准：

- [ ] 默认响应不会随多年 snapshot 无限增长。
- [ ] 客户端可以通过明确时间范围获取历史数据。

### 6.3 限制 single commit lifecycle 子列表

涉及文件：

- `enterprise-server/src/handlers/lifecycle.rs`

实现步骤：

- [ ] 对单 commit 的 CI events 和 alert events 增加合理上限。
- [ ] 如果超过上限，在响应中返回 `truncated: true`。
- [ ] 或新增独立事件列表接口后做分页。

验收标准：

- [ ] 单 commit 异常多事件时不会返回超大响应。
- [ ] 响应能提示客户端数据被截断。

提交建议：

```bash
git add enterprise-server/src
git commit -m "Bound time series dashboard responses"
```

## 阶段 7: API 文档和前端适配

目标：让分页协议对调用方清晰，并确保 UI 能正确翻页。

### 7.1 更新服务端 API 文档

涉及文件：

- `docs/ai-usage-reporting-tool/server_api.md`
- 其他引用 Enterprise API 的文档

实现步骤：

- [ ] 为每个已分页接口补充 `limit` 和 `cursor` 参数。
- [ ] 补充响应中的 `pagination` 字段。
- [ ] 给出第一页和下一页请求示例。
- [ ] 说明 cursor 是 opaque，不保证永久有效。

验收标准：

- [ ] 文档中的请求和实际接口一致。
- [ ] 老调用方可以忽略 `pagination` 字段。

### 7.2 前端适配

步骤：

- [ ] 找到管理后台用户/API keys/组织/部门页面。
- [ ] 找到 dashboard developers/projects/tools 表格页面。
- [ ] 把一次性全量加载改成第一页加载。
- [ ] 增加下一页、上一页或无限滚动交互。
- [ ] 切换过滤条件时清空 cursor 并重新请求第一页。
- [ ] loading 状态不阻塞整个页面，只阻塞当前表格。
- [ ] 空状态和错误状态保持原体验。

验收标准：

- [ ] 首屏不再依赖全量数据。
- [ ] 翻页不会重复或跳过数据。
- [ ] 改变过滤条件后不会沿用旧 cursor。

## 阶段 8: 回归、压测和发布

目标：确认分页改造没有破坏兼容性，并量化收益。

### 8.1 全量回归测试

命令：

```bash
cd enterprise-server
cargo test
```

如根仓库有相关测试，也运行：

```bash
task test
```

验收标准：

- [ ] 服务端测试通过。
- [ ] 根仓库相关测试通过，或失败原因已记录且与分页无关。

### 8.2 API 兼容性检查

步骤：

- [ ] 不传分页参数调用每个已改造接口。
- [ ] 确认旧顶层字段仍存在。
- [ ] 确认新增 `pagination` 不影响旧客户端。
- [ ] 用 `limit=1` 调用每个接口。
- [ ] 用非法 cursor 调用每个接口。

验收标准：

- [ ] 不传参数时接口可用。
- [ ] `limit=1` 可用。
- [ ] 非法 cursor 返回 400，而不是 500。

### 8.3 性能对比

记录项：

- [ ] 改造前后 P0 接口 p50/p95/p99。
- [ ] 改造前后响应体大小。
- [ ] 改造前后 API 内存峰值。
- [ ] 改造前后 Postgres CPU 和查询耗时。
- [ ] dashboard 聚合接口第一页和第二页耗时。

建议压测命令：

```bash
ab -n 200 -c 20 "http://127.0.0.1:8080/api/v1/audit-log?limit=100"
ab -n 200 -c 20 "http://127.0.0.1:8080/api/v1/aggregate/pull-requests?limit=100"
```

如果接口需要认证，使用支持 header 的压测工具，例如 `wrk`：

```bash
wrk -t4 -c20 -d30s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:8080/api/v1/audit-log?limit=100"
```

验收标准：

- [ ] 日志类接口响应时间随表数据量增长更平稳。
- [ ] dashboard 表格接口首屏响应明显下降。
- [ ] 没有新增 500 错误。

### 8.4 发布和回滚方案

发布步骤：

- [ ] 先发布后端，保持旧字段兼容。
- [ ] 再发布前端分页交互。
- [ ] 观察慢查询、错误率、响应体大小。
- [ ] 如果 dashboard 聚合分页引发问题，优先回滚 dashboard 阶段，不影响日志和 admin 列表分页。

回滚标准：

- [ ] 新增 500 错误率明显上升。
- [ ] 慢查询数量明显上升。
- [ ] 前端核心页面无法加载。

## 建议提交顺序

1. `Add pagination helpers`
2. `Add cursor pagination for audit logs`
3. `Paginate admin list APIs`
4. `Paginate pull request aggregation`
5. `Paginate dashboard aggregate APIs`
6. `Bound time series dashboard responses`
7. `Document paginated enterprise APIs`

## 最终完成定义

分页改造完成时应满足：

- [ ] P0 接口全部支持 `limit/cursor`。
- [ ] P1 接口至少完成 admin 列表分页。
- [ ] 所有已改造接口都返回 `pagination`。
- [ ] 所有 cursor 都是稳定排序，不因同 timestamp 或同 name 数据重复/跳页。
- [ ] 非法 cursor 返回 400。
- [ ] dashboard 聚合分页发生在 SQL/rollup 层，不是 Rust 内存切片。
- [ ] API 文档已更新。
- [ ] 测试和性能对比结果已记录。
