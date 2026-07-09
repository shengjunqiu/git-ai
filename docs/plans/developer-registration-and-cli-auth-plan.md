# 开发者注册登录与 CLI 授权任务文档

本文档把“开发者 Web 注册/登录 + CLI 浏览器授权登录”方案拆成可以逐步执行、逐步验收的开发任务。目标是让开发者注册自己的账号，注册时绑定公司组织和部门；`git-ai login` 通过浏览器授权拿到当前开发者本人的 token；metrics/report 上传后能按开发者、组织、部门正确归属。

## 0. 执行规则

按阶段顺序执行。每个任务完成后先跑该任务列出的验收和测试，再进入下一任务。

每个任务默认包含：

- 改动范围：本任务预计触碰的文件或模块。
- 执行步骤：可以照做的实现步骤。
- 验收标准：本任务完成的最低标准。
- 建议测试：完成任务后优先执行的命令或手工验证。

通用要求：

- 新注册用户在公司组织中的角色只能是 `member`。
- 组织和部门必须由管理员预先创建。
- 用户只能加入服务端允许的组织，第一期通过邮箱域名限制。
- CLI 不直接接触密码，只走浏览器 session + authorization code + PKCE。
- 新版 `git-ai login` 不再把设备码 `/verify` 作为主流程。
- 旧 device flow 可以暂时保留兼容，但不得再被新版 CLI 调用。

## 1. 当前问题和目标结果

当前 `git-ai login` 使用设备码流程：

```text
POST /worker/oauth/device/code
GET /verify
POST /verify
POST /worker/oauth/token
```

当前 `/verify` 是临时实现，授权时可能取数据库中第一个用户，导致多个开发者拿到同一个用户 token。第一期要改成：

```text
Web 注册/登录 -> web_session cookie
CLI 打开 /auth/cli/authorize
用户在浏览器确认授权
服务端生成一次性 authorization code
CLI 用 code + PKCE 兑换 access_token / refresh_token
```

完成后的体验：

```bash
git-ai login --server https://git-ai.company.com
```

浏览器流程：

```text
未登录 -> /auth/login 或 /auth/register
已登录 -> /auth/cli/authorize 显示当前账号、组织、部门
点击授权 -> CLI callback 收到 code
CLI 自动兑换并保存 credentials
```

完成后必须满足：

- `git-ai whoami` 显示当前开发者本人邮箱、组织和角色。
- Alice 和 Bob 登录后不会共享 token 或身份。
- 上传 metrics/report 后，Enterprise dashboard 能按开发者、组织、部门聚合。

## 2. 关键文件地图

CLI：

- `src/commands/login.rs`：当前 device flow 登录入口，后续改为浏览器授权。
- `src/auth/client.rs`：OAuth 客户端，新增 authorization code 兑换。
- `src/auth/types.rs`：token/device 响应类型，必要时增加新请求/响应类型。
- `src/commands/whoami.rs`：身份展示验收点。

服务端：

- `enterprise-server/src/routes.rs`：新增 `/auth/*` 路由，扩展 `/worker/oauth/token`。
- `enterprise-server/src/handlers/oauth.rs`：当前 token 签发和 device/refresh/install nonce grant。
- `enterprise-server/src/handlers/login.rs`：当前 dashboard token/API key 登录页，可复用样式或迁移为账号密码登录。
- `enterprise-server/src/handlers/verify.rs`：旧 device flow 页面，第一期保留但不作为主流程。
- `enterprise-server/src/auth/middleware.rs`：当前 Bearer/API key/cookie 认证逻辑，新增 web session 读取。
- `enterprise-server/src/models/auth.rs`：`TokenRequest` 需要增加 authorization code 字段。
- `enterprise-server/src/models/user.rs`：用户、组织、部门、成员模型需要补字段。
- `enterprise-server/src/services/audit.rs`：记录注册、登录、授权、token 兑换等审计事件。
- `enterprise-server/migrations/`：服务端本地迁移。
- `enterprise-server/deploy/migrations/`：部署迁移副本，新增迁移时保持同步。

## 3. 阶段 0：准备和基线

### Task 0.1：确认开发基线

改动范围：

- 不改代码，只确认现状。

执行步骤：

1. 运行 `git status --short`，确认工作区已有改动。
2. 记录当前 `src/commands/login.rs` 仍调用 `start_device_flow()` 和 `poll_for_token()`。
3. 记录当前 `enterprise-server/src/handlers/oauth.rs` 支持的 grant type：device code、refresh token、install nonce。
4. 记录当前 `/login` 是 dashboard token/API key 登录，不是账号密码登录。

验收标准：

- 清楚哪些文件是本功能要改的文件。
- 不覆盖无关本地改动。

建议测试：

```bash
task build
```

### Task 0.2：确定第一期非目标

改动范围：

- 本文档或 PR 描述。

执行步骤：

1. 明确第一期不做 SSO/OIDC。
2. 明确第一期不做管理员审核注册申请。
3. 明确第一期不做多组织切换。
4. 明确第一期不做设备管理和设备撤销。
5. 明确第一期不开放任意邮箱注册进任意组织。

验收标准：

- PR 范围只覆盖 Web 注册登录、组织/部门绑定、CLI 授权码登录、dashboard 归属修正。

### 阶段 0 执行记录（2026-07-06）

状态：已完成。

基线确认：

- `git status --short` 显示已有不相关源码改动：`src/commands/git_handlers.rs`。
- `docs/plans/developer-registration-and-cli-auth-plan.md` 是本任务文档，当前为未跟踪文件。
- `src/commands/login.rs` 当前仍调用 `OAuthClient::start_device_flow()` 和 `OAuthClient::poll_for_token()`。
- `src/auth/client.rs` 当前仍实现 device flow、refresh token 和 install nonce 兑换。
- `enterprise-server/src/handlers/oauth.rs` 当前 `/worker/oauth/token` 支持 3 类 grant：device code、refresh token、install nonce。
- `enterprise-server/src/models/auth.rs` 当前 `TokenRequest` 还没有 `code`、`code_verifier`、`redirect_uri` 字段。
- `enterprise-server/src/handlers/login.rs` 当前 `/login` 是 dashboard 的 API key / Bearer token 登录页，不是账号密码登录页。

第一期非目标确认：

- 不做 SSO/OIDC。
- 不做管理员审核注册申请。
- 不做多组织切换。
- 不做设备管理和设备撤销。
- 不开放任意邮箱注册进任意组织。

验证结果：

- `task build` 通过，执行的是 `cargo build`。
- 构建耗时约 48.63 秒。

## 4. 阶段 1：数据库迁移和模型

### Task 1.1：新增用户账号字段

改动范围：

- `enterprise-server/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/deploy/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/src/models/user.rs`

执行步骤：

1. 给 `users` 增加字段：

```sql
ALTER TABLE users
  ADD COLUMN IF NOT EXISTS password_hash TEXT,
  ADD COLUMN IF NOT EXISTS email_verified_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS default_org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';
```

2. 增加约束或迁移注释，说明 `status` 允许值至少包含 `active`、`disabled`。
3. 更新 `enterprise-server/src/models/user.rs` 的 `User` 结构体，加入新增字段。
4. 检查所有 `SELECT id, email, name, personal_org_id ... FROM users` 查询，必要时保持旧查询不被新字段破坏。

验收标准：

- 迁移可重复执行。
- 旧用户没有 `password_hash` 时不会影响 Bearer/API key/安装 nonce 登录。
- `default_org_id` 可以为空，但新注册用户必须写入公司组织。

建议测试：

```bash
cd enterprise-server
cargo test
```

### Task 1.2：新增邮箱域名到组织的绑定表

改动范围：

- `enterprise-server/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/deploy/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/src/models/user.rs`

执行步骤：

1. 新增 `organization_domains`：

```sql
CREATE TABLE IF NOT EXISTS organization_domains (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  domain TEXT NOT NULL,
  verified BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(org_id, domain)
);

CREATE INDEX IF NOT EXISTS idx_organization_domains_domain
  ON organization_domains(domain);
```

2. 约定 `domain` 全部用小写存储，注册校验时对邮箱域名做 lowercase。
3. 增加模型或查询 DTO，用于返回注册时可加入的组织。

验收标准：

- `linewell.com` 只能匹配 `alice@linewell.com` 这类邮箱。
- 只有 `verified = true` 的域名可用于自助注册。
- 不允许通过请求体绕过域名限制加入其他组织。

### Task 1.3：新增 Web session 表

改动范围：

- `enterprise-server/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/deploy/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/src/models/auth.rs` 或新 session 模型文件

执行步骤：

1. 新增 `web_sessions`：

```sql
CREATE TABLE IF NOT EXISTS web_sessions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  session_token_hash TEXT UNIQUE NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_web_sessions_user_id
  ON web_sessions(user_id);

CREATE INDEX IF NOT EXISTS idx_web_sessions_token_hash
  ON web_sessions(session_token_hash);
```

2. 明确 cookie 只保存随机 session token。
3. 数据库只保存 `jwt::hash_token(session_token)` 或同等级 hash。

验收标准：

- 登录成功后能创建 session。
- 登出能设置 `revoked_at` 或删除 session。
- 过期或 revoked session 不能访问 CLI 授权页。

### Task 1.4：新增 authorization code 表

改动范围：

- `enterprise-server/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/deploy/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/src/models/auth.rs`

执行步骤：

1. 新增 `authorization_codes`：

```sql
CREATE TABLE IF NOT EXISTS authorization_codes (
  code_hash TEXT PRIMARY KEY,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  client_id TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  code_challenge TEXT NOT NULL,
  code_challenge_method TEXT NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_authorization_codes_user_id
  ON authorization_codes(user_id);
```

2. authorization code 明文只返回给 CLI callback，不入库。
3. code 有效期建议 5 分钟。
4. code 只能消费一次。

验收标准：

- 同一个 code 第一次兑换成功，第二次兑换失败。
- 过期 code 兑换失败。
- code 数据库中只保存 hash。

### Task 1.5：补齐历史用户默认组织

改动范围：

- `enterprise-server/migrations/006_developer_registration_cli_auth.sql`
- `enterprise-server/deploy/migrations/006_developer_registration_cli_auth.sql`

执行步骤：

1. 对已有用户补 `default_org_id`：

```sql
UPDATE users u
SET default_org_id = om.org_id
FROM org_members om
WHERE om.user_id = u.id
  AND (u.personal_org_id IS NULL OR om.org_id <> u.personal_org_id)
  AND u.default_org_id IS NULL;
```

2. 如果用户只在个人组织里，保持 `default_org_id` 为空，等待管理员绑定公司组织。
3. 不修改用户现有 role。

验收标准：

- 已有公司组织成员优先拿到公司组织作为默认组织。
- 个人组织不会被误当作 dashboard 聚合范围。

### 阶段 1 执行记录（2026-07-06）

状态：已完成。

已完成改动：

- 新增 `enterprise-server/migrations/006_developer_registration_cli_auth.sql`。
- 新增 `enterprise-server/deploy/migrations/006_developer_registration_cli_auth.sql`，内容与本地迁移保持一致。
- 更新 `enterprise-server/src/db/migrations.rs`，把 `006_developer_registration_cli_auth` 加入服务端自动迁移列表。
- 更新 `enterprise-server/src/models/user.rs`：
  - `User` 增加 `password_hash`、`email_verified_at`、`default_org_id`、`status`。
  - 新增 `OrganizationDomain`、`WebSession`、`AuthorizationCode` 模型。

迁移内容：

- `users` 新增账号字段，并为 `status` 添加 `active` / `disabled` check constraint。
- 新增 `organization_domains`，包含 lowercase domain check、`verified` 标记和查询索引。
- 新增 `web_sessions`，数据库只保存 `session_token_hash`。
- 新增 `authorization_codes`，数据库只保存 `code_hash`，并包含过期和消费状态字段。
- 回填已有用户的 `default_org_id`，优先选择非个人组织 membership。

验证结果：

- `cd enterprise-server && cargo test` 通过。
- 测试结果：16 passed, 0 failed。
- 测试输出仍包含仓库既有 warning；阶段 1 新增模型在后续阶段接入 handler/service 前也会出现在 dead code warning 中。

## 5. 阶段 2：服务端基础服务

### Task 2.1：抽出 token 签发服务

改动范围：

- `enterprise-server/src/handlers/oauth.rs`
- `enterprise-server/src/services/tokens.rs`
- `enterprise-server/src/services/mod.rs`

执行步骤：

1. 新增 `enterprise-server/src/services/tokens.rs`。
2. 把 `generate_token_response(state, user_id)` 从 `handlers/oauth.rs` 移到 service。
3. 保持返回结构不变：

```json
{
  "access_token": "...",
  "token_type": "Bearer",
  "expires_in": 3600,
  "refresh_token": "...",
  "refresh_expires_in": 7776000
}
```

4. 生成 JWT 时优先让 claims 和 middleware 使用 `users.default_org_id` 对应的公司组织。
5. 如果 `default_org_id` 为空，则保持旧行为作为兼容 fallback。

验收标准：

- device code、refresh token、install nonce 仍能签发 token。
- 后续 authorization code grant 可以复用同一函数。
- `git-ai whoami` 所需字段不缺失。

建议测试：

```bash
cd enterprise-server
cargo test
```

### Task 2.2：实现密码 hash 和校验

改动范围：

- `enterprise-server/src/services/auth.rs` 或 `enterprise-server/src/services/passwords.rs`
- `enterprise-server/src/services/mod.rs`
- `enterprise-server/Cargo.toml` 如依赖不够再改

执行步骤：

1. 使用现有 `argon2` 依赖实现 `hash_password(password) -> Result<String, AppError>`。
2. 实现 `verify_password(password, password_hash) -> Result<bool, AppError>`。
3. 密码 hash 使用随机 salt。
4. 不在日志中输出明文密码或 password hash。
5. 增加密码最低要求：至少 8 个字符；复杂度可在后续增强。

验收标准：

- 同一密码多次 hash 结果不同。
- 正确密码校验通过。
- 错误密码校验失败。

建议测试：

```bash
cd enterprise-server
cargo test password
```

### Task 2.3：实现 Web session 服务

改动范围：

- `enterprise-server/src/services/sessions.rs`
- `enterprise-server/src/services/mod.rs`
- `enterprise-server/src/auth/middleware.rs`

执行步骤：

1. 实现 `create_web_session(db, user_id) -> plain_session_token`。
2. 实现 `load_web_session_user(db, plain_session_token) -> user_id`。
3. 实现 `revoke_web_session(db, plain_session_token)`。
4. 每次 session 被使用时可更新 `last_seen_at`。
5. cookie 名固定为 `web_session`。

验收标准：

- cookie 中只有明文随机 token，数据库中只有 hash。
- 过期 session、revoked session、未知 session 都不能通过认证。
- Web session 逻辑不影响现有 `access_token` 和 `api_key` cookie。

### Task 2.4：实现组织和部门注册校验服务

改动范围：

- `enterprise-server/src/services/registration.rs` 或 `enterprise-server/src/handlers/auth_api.rs`

执行步骤：

1. 实现从邮箱提取 lowercase domain。
2. 实现 `list_registerable_organizations(email)`，只返回 domain verified 的组织。
3. 实现 `list_departments_for_org(org_id)`。
4. 实现 `validate_org_domain(email, org_id)`。
5. 实现 `validate_department(org_id, department_id)`。

验收标准：

- `alice@linewell.com` 只看到绑定了 `linewell.com` 且 verified 的组织。
- `department_id` 不属于 `org_id` 时注册失败。
- 请求体不能传 role。

### 阶段 2 执行记录（2026-07-06）

状态：已完成。

已完成改动：

- 新增 `enterprise-server/src/services/tokens.rs`，集中签发 access token / refresh token。
- 更新 `enterprise-server/src/handlers/oauth.rs`，device code、refresh token、install nonce grant 改为复用 token service。
- 新增 `enterprise-server/src/services/passwords.rs`，使用 Argon2 实现密码 hash、校验和最低长度校验。
- 新增 `enterprise-server/src/services/sessions.rs`，实现 Web session token 生成、创建、读取和 revoke。
- 更新 `enterprise-server/src/auth/middleware.rs`，新增只读取 `web_session` cookie 的 helper，不改变现有 Bearer/API key 认证语义。
- 新增 `enterprise-server/src/services/registration.rs`，实现邮箱域名提取、可注册组织查询、部门查询、组织域名校验和部门归属校验。
- 更新 `enterprise-server/src/services/mod.rs`，导出 `tokens`、`passwords`、`sessions`、`registration`。

实现说明：

- token service 保持原有 token response JSON 结构不变。
- token service 查询 `users.default_org_id`，并将该组织 membership 排在 JWT `orgs` 首位；为空时保留旧 membership fallback 顺序。
- Web session cookie 名固定为 `web_session`，数据库只保存 `session_token_hash`。
- `load_web_session_user` 会在有效 session 被读取时更新 `last_seen_at`。
- 注册校验只接受 `organization_domains.verified = true` 的邮箱域名。

验证结果：

- `cd enterprise-server && cargo test` 通过。
- 测试结果：23 passed, 0 failed。
- 新增测试覆盖密码随机 salt、正确/错误密码校验、短密码拒绝、邮箱域名解析、session token 生成。
- 测试输出仍包含仓库既有 warning；阶段 2 新增服务函数在阶段 3/4 接入 handler 前也会出现在 dead code warning 中。

## 6. 阶段 3：Web 注册、登录、登出

### Task 3.1：新增 auth handler 和路由骨架

改动范围：

- `enterprise-server/src/routes.rs`
- `enterprise-server/src/handlers/mod.rs`
- `enterprise-server/src/handlers/auth_pages.rs`
- `enterprise-server/src/handlers/auth_api.rs`
- `enterprise-server/src/handlers/cli_authorize.rs`

执行步骤：

1. 新增 handler 文件并在 `handlers/mod.rs` 导出。
2. 在 `routes.rs` 增加路由：

```text
GET  /auth/register
POST /auth/register
GET  /auth/login
POST /auth/login
POST /auth/logout
GET  /auth/organizations
GET  /auth/organizations/{org_id}/departments
GET  /auth/cli/authorize
POST /auth/cli/authorize
```

3. 先返回 stub 页面或 JSON，确保路由能编译。
4. 保留现有 `/login`、`/logout` dashboard token/API key 登录兼容路径。

验收标准：

- 所有新增路由能通过 router 编译。
- `/login` 旧 dashboard 登录不被破坏。

建议测试：

```bash
cd enterprise-server
cargo test
```

### Task 3.2：实现注册页面和组织/部门查询接口

改动范围：

- `enterprise-server/src/handlers/auth_pages.rs`
- `enterprise-server/src/handlers/auth_api.rs`

执行步骤：

1. `GET /auth/register` 返回注册 HTML。
2. 页面字段包含：姓名、邮箱、密码、确认密码、组织、部门。
3. `GET /auth/organizations?email=alice@linewell.com` 返回可注册组织：

```json
{
  "organizations": [
    {
      "id": "uuid",
      "name": "Linewell",
      "slug": "linewell.com"
    }
  ]
}
```

4. `GET /auth/organizations/{org_id}/departments` 返回部门：

```json
{
  "departments": [
    {
      "id": "uuid",
      "name": "Backend",
      "slug": "backend"
    }
  ]
}
```

5. 前端根据邮箱查询组织，根据组织查询部门。
6. 没有可加入组织时提示联系管理员。

验收标准：

- 未绑定域名的邮箱看不到组织。
- 部门列表只来自所选组织。
- HTML 表单在无 JavaScript 时至少能提交基础字段。

### Task 3.3：实现注册提交

改动范围：

- `enterprise-server/src/handlers/auth_api.rs`
- `enterprise-server/src/services/registration.rs`
- `enterprise-server/src/services/sessions.rs`
- `enterprise-server/src/services/audit.rs`

执行步骤：

1. `POST /auth/register` 接收：

```json
{
  "email": "alice@linewell.com",
  "name": "Alice",
  "password": "secret123",
  "org_id": "uuid",
  "department_id": "uuid"
}
```

2. 校验邮箱格式、密码长度、确认密码、组织域名、部门归属。
3. 检查邮箱未被占用；第一期重复邮箱直接失败。
4. 创建个人组织。
5. 创建 `users`，写入 `password_hash`、`personal_org_id`、`default_org_id = org_id`、`status = active`。
6. 写入个人组织 membership：`role = owner`。
7. 写入公司组织 membership：`role = member`，`department_id = department_id`。
8. 如果以后支持资料更新，`ON CONFLICT` 不得把已有 `admin` 或 `owner` 降为 `member`。
9. 创建 web session 并写入 cookie。
10. 记录审计事件：`user.register`、`org_member.create`。

验收标准：

- 注册成功后浏览器处于登录状态。
- 注册用户公司组织 role 是 `member`。
- 注册用户 `org_members.department_id` 指向所选部门。
- 注册用户 `users.default_org_id` 指向公司组织。
- 非允许域名、跨组织部门、重复邮箱都失败。

建议测试：

```bash
cd enterprise-server
cargo test register
```

### Task 3.4：实现账号密码登录和登出

改动范围：

- `enterprise-server/src/handlers/auth_pages.rs`
- `enterprise-server/src/handlers/auth_api.rs`
- `enterprise-server/src/services/passwords.rs`
- `enterprise-server/src/services/sessions.rs`
- `enterprise-server/src/services/audit.rs`

执行步骤：

1. `GET /auth/login` 返回账号密码登录 HTML。
2. 支持 `return_to` 查询参数，用于登录后回到 CLI 授权页。
3. `POST /auth/login` 接收邮箱和密码。
4. 只允许 `users.status = active` 的用户登录。
5. 密码正确后创建 web session。
6. 设置 cookie：

```text
web_session=<random>; HttpOnly; SameSite=Lax; Path=/
```

7. 生产 HTTPS 环境必须加 `Secure`。
8. `POST /auth/logout` revoke 当前 session 并清除 cookie。
9. 记录审计事件：`user.login`、`user.logout`。

验收标准：

- 正确账号密码能登录。
- 错误密码不能登录。
- disabled 用户不能登录。
- 登出后不能访问 `/auth/cli/authorize`。

### Task 3.5：实现 Web session extractor

改动范围：

- `enterprise-server/src/auth/middleware.rs`
- `enterprise-server/src/services/sessions.rs`

执行步骤：

1. 增加只面向 Web 页面的 session 读取 helper。
2. 不要把 `web_session` 混进 API Bearer/API key 认证语义。
3. CLI 授权页使用该 helper 获取当前浏览器用户。
4. `/me` 可以暂时继续用旧 `access_token` / `api_key` 登录；本任务不强制迁移 dashboard。

验收标准：

- `/auth/cli/authorize` 能识别 web session。
- API worker 路由不会因为 web session cookie 被错误认证。

### 阶段 3 执行记录（2026-07-06）

状态：已完成。

已完成改动：

- 新增 `enterprise-server/src/handlers/auth_pages.rs`：
  - `GET /auth/register` 返回注册页面。
  - `GET /auth/login` 返回账号密码登录页面。
- 新增 `enterprise-server/src/handlers/auth_api.rs`：
  - `GET /auth/organizations?email=...` 返回可注册组织。
  - `GET /auth/organizations/{org_id}/departments` 返回部门列表。
  - `POST /auth/register` 支持 JSON 和 HTML form，完成用户、个人组织、组织成员、web session 创建。
  - `POST /auth/login` 支持 JSON 和 HTML form，校验 active 用户和 Argon2 密码后创建 web session。
  - `POST /auth/logout` revoke 当前 web session 并清除 cookie。
- 新增 `enterprise-server/src/handlers/cli_authorize.rs`：
  - `GET /auth/cli/authorize` 先接入 web session 识别；未登录时跳转 `/auth/login?return_to=...`。
  - `POST /auth/cli/authorize` 暂时返回未实现，完整授权码生成留到阶段 4。
- 更新 `enterprise-server/src/auth/middleware.rs`，新增 `WebSessionUser` extractor。
- 更新 `enterprise-server/src/handlers/mod.rs` 和 `enterprise-server/src/routes.rs`，挂载阶段 3 的 `/auth/*` 路由。

实现说明：

- 保留现有 `/login`、`/logout` dashboard token/API key 登录兼容路径。
- `POST /auth/register` 不接收 role，请求方不能注册成 `admin` 或 `owner`。
- 公司组织 membership 使用 `role = member`，`ON CONFLICT` 只更新 `department_id`，不覆盖已有 role。
- 注册成功记录 `user.register` 和 `org_member.create` 审计事件。
- 登录成功记录 `user.login`，登出记录 `user.logout`。
- `web_session` cookie 使用 `HttpOnly; SameSite=Lax; Path=/`，当 `base_url` 是 HTTPS 时增加 `Secure`。

验证结果：

- `cd enterprise-server && cargo test` 通过。
- 测试结果：23 passed, 0 failed。
- 测试输出仍包含仓库既有 warning。

## 7. 阶段 4：服务端 CLI 授权码流程

### Task 4.1：实现授权页 GET

改动范围：

- `enterprise-server/src/handlers/cli_authorize.rs`
- `enterprise-server/src/auth/middleware.rs`

执行步骤：

1. `GET /auth/cli/authorize` 读取查询参数：

```text
client_id=git-ai-cli
redirect_uri=http://127.0.0.1:<port>/callback
response_type=code
code_challenge=...
code_challenge_method=S256
state=...
```

2. 校验 `client_id = git-ai-cli`。
3. 校验 `response_type = code`。
4. 校验 `code_challenge_method = S256`。
5. 校验 redirect URI 只允许：

```text
http://127.0.0.1:<port>/callback
http://localhost:<port>/callback
```

6. 如果没有有效 `web_session`，重定向：

```text
/auth/login?return_to=<原始 authorize URL>
```

7. 如果已登录，展示当前账号、默认组织、部门和授权按钮。

验收标准：

- 未登录用户会先进入登录页，登录后回到授权页。
- 已登录用户看到自己的邮箱、公司组织、部门。
- 非本地 redirect URI 被拒绝。

### Task 4.2：实现授权确认 POST

改动范围：

- `enterprise-server/src/handlers/cli_authorize.rs`
- `enterprise-server/src/services/audit.rs`

执行步骤：

1. `POST /auth/cli/authorize` 重新校验所有 authorize 参数。
2. 读取当前 web session 用户。
3. 生成高熵 authorization code。
4. 将 code hash、user_id、client_id、redirect_uri、code_challenge、method、expires_at 写入 `authorization_codes`。
5. 重定向到：

```text
http://127.0.0.1:<port>/callback?code=...&state=...
```

6. 用户点击取消时重定向：

```text
http://127.0.0.1:<port>/callback?error=access_denied&state=...
```

7. 记录审计事件：`cli.authorize`。

验收标准：

- 授权成功只生成一次性 code。
- state 原样返回。
- 取消授权时 CLI 能收到 `access_denied`。

### Task 4.3：扩展 token request 模型

改动范围：

- `enterprise-server/src/models/auth.rs`

执行步骤：

1. 给 `TokenRequest` 增加字段：

```rust
#[serde(default)]
pub code: Option<String>,
#[serde(default)]
pub code_verifier: Option<String>,
#[serde(default)]
pub redirect_uri: Option<String>,
```

2. 更新注释，说明 `/worker/oauth/token` 支持 4 种 grant type：

```text
authorization_code
urn:ietf:params:oauth:grant-type:device_code
refresh_token
install_nonce
```

验收标准：

- 旧 grant 请求反序列化不受影响。
- authorization code 请求可以反序列化。

### Task 4.4：实现 authorization_code grant

改动范围：

- `enterprise-server/src/handlers/oauth.rs`
- `enterprise-server/src/services/tokens.rs`
- `enterprise-server/src/services/audit.rs`

执行步骤：

1. 在 `/worker/oauth/token` 中新增分支：

```text
grant_type = authorization_code
```

2. 校验 `client_id = git-ai-cli`。
3. 校验 `code`、`code_verifier`、`redirect_uri` 必填。
4. 对 code 取 hash 查询 `authorization_codes`。
5. 校验未过期、未消费、redirect_uri 完全一致。
6. 校验 PKCE：

```text
BASE64URL(SHA256(code_verifier)) == code_challenge
```

7. 使用事务或条件更新一次性消费 code：

```sql
UPDATE authorization_codes
SET consumed_at = now()
WHERE code_hash = $1
  AND consumed_at IS NULL
  AND expires_at > now();
```

8. 调用 token service 签发 token。
9. 记录审计事件：`token.exchange`。

验收标准：

- 正确 code + verifier 能换 token。
- 错误 verifier 失败。
- 重复兑换同一个 code 失败。
- redirect URI 不一致失败。
- 过期 code 失败。

建议测试：

```bash
cd enterprise-server
cargo test authorization_code
```

### Task 4.5：保留但降级旧 device flow

改动范围：

- `enterprise-server/src/handlers/oauth.rs`
- `enterprise-server/src/handlers/verify.rs`
- 文档

执行步骤：

1. 不删除 `/worker/oauth/device/code`、`/verify`、device code grant。
2. 在代码注释和文档中标记旧 `/verify` 为 deprecated。
3. 确保新版 `git-ai login` 不再调用 device flow。
4. 后续版本再决定删除或隐藏。

验收标准：

- 旧客户端不会立即不可用。
- 新版 CLI 的登录链路没有 `/worker/oauth/device/code` 请求。

### 阶段 4 执行记录（2026-07-06）

状态：已完成。

已完成改动：

- 更新 `enterprise-server/src/handlers/cli_authorize.rs`：
  - `GET /auth/cli/authorize` 校验 `client_id`、`response_type`、PKCE method、state 和本地 redirect URI。
  - 未登录浏览器会跳转 `/auth/login?return_to=...`，登录后回到原授权 URL。
  - 已登录浏览器展示当前邮箱、组织、部门和授权/取消按钮。
  - `POST /auth/cli/authorize` 重新校验 authorize 参数，生成一次性 authorization code。
  - authorization code 只以 hash 写入 `authorization_codes`，明文只通过本地 callback 返回。
  - 用户取消时回调返回 `error=access_denied` 和原始 state。
  - 授权成功记录 `cli.authorize` 审计事件。
- 更新 `enterprise-server/src/models/auth.rs`：
  - `TokenRequest` 增加 `code`、`code_verifier`、`redirect_uri`。
  - 注释更新为 `/worker/oauth/token` 支持 4 种 grant type。
- 更新 `enterprise-server/src/handlers/oauth.rs`：
  - `/worker/oauth/token` 新增 `authorization_code` grant 分支。
  - 校验 code、code verifier、redirect URI、client ID 和 PKCE S256 challenge。
  - 使用条件更新消费 authorization code，避免同一个 code 被重复兑换。
  - 兑换成功后复用现有 token 签发逻辑并记录 `token.exchange` 审计事件。
  - 增加 PKCE challenge 和 base64url no padding 单元测试。
- 更新 `enterprise-server/src/handlers/verify.rs`：
  - 保留旧 `/verify` device flow 页面。
  - 在代码注释中标记旧 device flow 只作为兼容路径保留。

实现说明：

- redirect URI 仅允许 `http://127.0.0.1:<port>/callback` 和 `http://localhost:<port>/callback`，不允许 query、fragment 或非本地 host。
- authorization code 过期时间为 5 分钟，token 交换阶段要求 redirect URI 与授权阶段完全一致。
- 旧 `/worker/oauth/device/code` 和 device code grant 未删除；阶段 4 只新增浏览器 session + authorization code 链路。
- 本阶段没有改动 CLI 调用链路；新版 `git-ai login` 切换到 authorization code flow 留到阶段 5。

验证结果：

- `cd enterprise-server && cargo test` 通过。
- 测试结果：27 passed, 0 failed。
- 测试输出仍包含仓库既有 warning。

## 8. 阶段 5：CLI 登录改造

### Task 5.1：实现 PKCE 和 state 工具

改动范围：

- `src/commands/login.rs` 或新文件 `src/auth/pkce.rs`
- `src/auth/mod.rs`
- `Cargo.toml` 如依赖不够再改

执行步骤：

1. 生成高熵 `state`。
2. 生成高熵 `code_verifier`。
3. 计算 `code_challenge = BASE64URL(SHA256(code_verifier))`。
4. 确认输出不包含 `=` padding。
5. 增加单元测试验证 RFC 风格的 code challenge 计算。

验收标准：

- 每次登录生成不同 state 和 verifier。
- code challenge 与服务端算法一致。

建议测试：

```bash
cargo test pkce
```

### Task 5.2：实现本地 callback listener

改动范围：

- `src/commands/login.rs` 或新文件 `src/auth/cli_callback.rs`

执行步骤：

1. 绑定 `127.0.0.1:0` 获取随机端口。
2. callback 路径固定为 `/callback`。
3. 等待一次浏览器请求。
4. 解析 `code`、`state`、`error`。
5. 返回一个简单 HTML 页面，提示用户可以回到终端。
6. 设置合理超时，例如 5 分钟。
7. 监听结束后释放端口。

验收标准：

- 能收到 `?code=...&state=...`。
- 能收到 `?error=access_denied&state=...`。
- state 不匹配时登录失败。
- 超时后 CLI 明确报错。

### Task 5.3：新增 OAuthClient authorization code 兑换

改动范围：

- `src/auth/client.rs`

执行步骤：

1. 新增方法：

```rust
pub fn exchange_authorization_code(
    &self,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<StoredCredentials, String>
```

2. 请求 `/worker/oauth/token`：

```json
{
  "grant_type": "authorization_code",
  "client_id": "git-ai-cli",
  "code": "...",
  "code_verifier": "...",
  "redirect_uri": "http://127.0.0.1:<port>/callback"
}
```

3. 复用现有 `exchange_token()` 解析 token。
4. 保留 `refresh_access_token()`。
5. 暂时保留 device flow 方法，但新 `handle_login()` 不再调用。

验收标准：

- token response 保存为现有 `StoredCredentials`。
- 服务端错误能显示清晰消息。

建议测试：

```bash
cargo test auth::client
```

### Task 5.4：改造 `git-ai login`

改动范围：

- `src/commands/login.rs`
- `src/commands/git_ai_handlers.rs` 如 help 文案需要更新

执行步骤：

1. 继续支持：

```bash
git-ai login --server https://git-ai.company.com
```

2. 新增可选参数：

```text
--no-browser
```

3. 如果已登录且 refresh token 未过期，保持当前提示行为。
4. 校验 server URL。
5. 启动 callback listener。
6. 生成 state、code_verifier、code_challenge。
7. 拼接授权 URL：

```text
https://git-ai.company.com/auth/cli/authorize?
  client_id=git-ai-cli
  redirect_uri=http://127.0.0.1:<port>/callback
  response_type=code
  code_challenge=...
  code_challenge_method=S256
  state=...
```

8. 默认打开浏览器。
9. 如果 `--no-browser`，只打印授权 URL。
10. 等待 callback。
11. 校验 state。
12. 用 code 兑换 token。
13. 保存 credentials 到现有 `CredentialStore`。
14. 如果传了 `--server`，继续保存 `api_base_url` 到 `~/.git-ai/config.json`。

验收标准：

- 登录过程中不调用 `/worker/oauth/device/code`。
- 浏览器授权后 CLI 自动登录成功。
- `--no-browser` 会打印可复制 URL。
- 授权取消、state 不匹配、超时都有清晰错误。

建议测试：

```bash
cargo test login
task build
```

### Task 5.5：验证 `git-ai whoami`

改动范围：

- `src/commands/whoami.rs`
- `src/auth/identity.rs`
- 服务端 token claims 生成逻辑

执行步骤：

1. 使用新登录流程保存 credentials。
2. 执行：

```bash
git-ai whoami
```

3. 确认输出包含：

```text
API Base URL
Auth state
Email
Name
Personal org ID
Organizations
```

4. 确认 Organizations 中包含公司组织和 `role=member`。
5. 确认 Alice 登录不会显示 Bob 的邮箱。

验收标准：

- `git-ai whoami` 显示当前开发者本人。
- 公司组织不是误取的个人组织。

### 阶段 5 执行记录（2026-07-06）

状态：CLI 代码改造已完成；真实服务端登录后的 `git-ai whoami` 端到端验证留到阶段 7 手工验收。

已完成改动：

- 新增 `src/auth/pkce.rs`：
  - 生成高熵 state。
  - 生成 PKCE `code_verifier`。
  - 计算 `BASE64URL(SHA256(code_verifier))` 形式的 `code_challenge`。
  - 输出不带 `=` padding。
  - 使用 RFC 7636 示例覆盖 challenge 计算。
- 新增 `src/auth/cli_callback.rs`：
  - 绑定 `127.0.0.1:0` 获取随机本地端口。
  - 固定 callback 路径为 `/callback`。
  - 解析 `code`、`state`、`error` 和 `error_description`。
  - 对浏览器返回简单 HTML，提示用户回到终端。
  - 支持超时退出并释放 listener。
- 更新 `src/auth/client.rs`：
  - 新增 `exchange_authorization_code()`。
  - 请求 `/worker/oauth/token` 时使用 `grant_type = authorization_code`。
  - 复用现有 `exchange_token()` 解析 token 并生成 `StoredCredentials`。
  - 保留 `start_device_flow()`、`poll_for_token()`、`refresh_access_token()` 和 `exchange_install_nonce()`。
- 更新 `src/commands/login.rs`：
  - `git-ai login` 改为 browser authorization code + PKCE 主流程。
  - 保留 `--server <url>` 和 `--server=<url>`。
  - 新增 `--no-browser`，只打印可复制授权 URL。
  - 已登录且 refresh token 未过期时继续保持原提示行为。
  - 启动 callback listener 后拼接 `/auth/cli/authorize` URL。
  - 默认尝试打开浏览器，失败时提示手动打开 URL。
  - callback 返回后校验 state，再用 code 兑换 token。
  - token 保存到现有 `CredentialStore`。
  - 如果传入 `--server`，继续写入 `~/.git-ai/config.json` 的 `api_base_url`。
  - 新流程不再调用 `/worker/oauth/device/code`。
- 更新 `src/auth/mod.rs`：
  - 导出 `cli_callback` 和 `pkce` 模块。
- 更新 `src/commands/git_ai_handlers.rs`：
  - 顶层 help 增加 `login --server` 和 `login --no-browser` 文案。

实现说明：

- 本地 callback listener 只监听 `127.0.0.1`，redirect URI 形如 `http://127.0.0.1:<port>/callback`。
- 授权取消会显示 `authorization was cancelled`。
- state 缺失或不匹配会直接失败，不会兑换 token。
- device flow 客户端方法仍在 `OAuthClient` 中保留，供旧兼容路径使用；新版 `handle_login()` 不再调用。
- `whoami` 依赖已保存 credentials 中的 access token claims，阶段 5 未修改 `src/commands/whoami.rs` 和 `src/auth/identity.rs`。

验证结果：

- `cargo test pkce` 通过。
- `cargo test login` 通过。
- `cargo test callback` 通过；该测试需要本地 loopback listener，沙箱内会因 `127.0.0.1:0` 绑定权限失败，提升权限后通过。
- `cargo test auth::client` 通过。
- `cargo test auth` 通过；同样需要提升权限覆盖 callback listener 测试。
- `task build` 通过。

## 9. 阶段 6：Dashboard 和数据归属修正

### Task 6.1：让认证身份优先使用 default_org_id

改动范围：

- `enterprise-server/src/auth/middleware.rs`
- `enterprise-server/src/services/tokens.rs`

执行步骤：

1. 当前 middleware 中存在 `SELECT org_id FROM org_members WHERE user_id = $1 LIMIT 1` 的逻辑。
2. 改为优先读取 `users.default_org_id`。
3. 再读取该组织对应的 `org_members.role` 和组织 slug。
4. 如果 `default_org_id` 为空，才 fallback 到旧 `LIMIT 1`。

验收标准：

- 普通成员的数据范围是公司组织。
- 个人组织不会抢占 dashboard 默认范围。
- 旧用户仍能通过 fallback 使用 dashboard。

### Task 6.2：确认部门聚合使用 org_members.department_id

改动范围：

- `enterprise-server/src/handlers/dashboard.rs`
- `enterprise-server/src/handlers/report.rs`
- `enterprise-server/src/handlers/metrics.rs`

执行步骤：

1. 检查开发者、部门、组织聚合查询。
2. 确保部门归属来自 `org_members.department_id`。
3. 确保查询同时约束用户所属组织，避免跨组织混算。
4. 对普通 member 应限制为本人数据。
5. 对 admin/owner 可看同组织成员数据。

验收标准：

- Alice 数据归属 Linewell / Backend。
- Bob 数据不会归到 Alice。
- member 只能看自己数据。
- admin/owner 可以看同组织成员数据。

### Task 6.3：验证 metrics/report 上传链路

改动范围：

- `src/commands/flush_metrics_db.rs`
- `enterprise-server/src/handlers/metrics.rs`
- `enterprise-server/src/handlers/report.rs`
- `enterprise-server/src/handlers/dashboard.rs`

执行步骤：

1. 用 Alice 新流程登录 CLI。
2. 产生一次可上传的 metrics 或 report。
3. 执行现有上传命令，例如 `git-ai flush-metrics-db`。
4. 登录 dashboard 查看开发者列表、组织汇总、部门汇总。
5. 用 Bob 重复一次，确认两人数据分离。

验收标准：

- dashboard 开发者列表出现 Alice。
- Alice 的数据归属 Linewell / Backend。
- Bob 的数据独立显示，不会归到 Alice。

### 阶段 6 执行记录（2026-07-06）

状态：代码修正已完成；Alice/Bob 的真实浏览器登录、上传和 dashboard 手工核验留到阶段 7 端到端验收。

已完成改动：

- 新增 `enterprise-server/src/services/org_scope.rs`：
  - 集中封装用户当前组织范围查询。
  - 优先使用 `users.default_org_id` 对应的 `org_members`。
  - 如果 `default_org_id` 为空，优先选择非个人组织 membership，再按组织创建时间兜底。
  - 返回 `org_id`、`org_slug` 和用户在该组织内的 role。
- 更新 `enterprise-server/src/auth/middleware.rs`：
  - Bearer token 路径不再用 `SELECT org_id FROM org_members ... LIMIT 1`。
  - access token cookie 路径不再用旧 `LIMIT 1`。
  - API key 未绑定 `org_id` 时也使用默认组织范围。
  - API key 绑定 `org_id` 时读取该组织内的 role 和 slug。
- 更新 `enterprise-server/src/services/tokens.rs`：
  - access token claims 中的 `orgs` 排序优先 `default_org_id`。
  - 旧用户没有 `default_org_id` 时优先非个人组织，避免个人组织抢占第一个 org。
- 更新 `enterprise-server/src/services/metrics.rs`：
  - metrics 上传写入 `metrics_events.org_id` 时改用默认组织范围。
- 更新 `enterprise-server/src/handlers/dashboard.rs`：
  - 部门聚合和 team comparison 的 `org_members` join 增加 `om.org_id = d.org_id`。
  - metrics join 增加 `m.org_id = om.org_id`，避免同一用户跨组织数据混入部门统计。
  - 继续沿用 `build_data_filters()`：普通 member 限制本人，admin/owner 限制当前组织。
- 更新 `enterprise-server/src/handlers/report.rs`：
  - report upload 的 project upsert 改为按 `(remote_url_hash, org_id, user_id)` 冲突更新。
  - 不再让同 repo hash 的上传覆盖已有项目的 `org_id` / `user_id`。
- 新增 `enterprise-server/migrations/007_project_scope_by_user_org.sql` 并接入 migration runner：
  - 移除旧的全局 `projects.remote_url_hash` 唯一约束。
  - 新增 `(remote_url_hash, org_id, user_id)` 唯一约束。

验证结果：

- `cd enterprise-server && cargo test` 通过。
- 测试结果：27 passed, 0 failed。
- 测试输出仍包含仓库既有 warning。

## 10. 阶段 7：测试矩阵

### Task 7.1：服务端单元和 handler 测试

改动范围：

- `enterprise-server/src/**`

执行步骤：

1. 覆盖密码 hash/verify。
2. 覆盖 Web session 创建、读取、过期、撤销。
3. 覆盖邮箱域名到组织查询。
4. 覆盖部门归属校验。
5. 覆盖 authorization code 一次性消费。
6. 覆盖 PKCE 成功和失败。

验收标准：

- 关键安全逻辑均有单元测试或 handler 测试。

建议测试：

```bash
cd enterprise-server
cargo test
```

### Task 7.2：CLI 单元测试

改动范围：

- `src/commands/login.rs`
- `src/auth/client.rs`
- 新增 PKCE/callback 模块测试

执行步骤：

1. 测试 `--server` 和 `--server=...` 解析仍可用。
2. 测试 `--no-browser` 解析。
3. 测试 PKCE 计算。
4. 测试 state 不匹配失败。
5. 测试 OAuthClient authorization code 请求体。

验收标准：

- 新登录流程核心分支有测试。
- device flow 方法保留但不被 login handler 调用。

建议测试：

```bash
cargo test auth
cargo test login
task build
```

### Task 7.3：端到端手工验收

改动范围：

- 全链路。

执行步骤：

1. 启动数据库和 enterprise server。
2. 初始化组织、部门、域名：

```text
Organization: Linewell
Slug: linewell.com
Departments: Backend, Frontend, QA
organization_domains: linewell.com -> Linewell, verified = true
```

3. 打开 `/auth/register` 注册 `alice@linewell.com`，选择 Backend。
4. 执行：

```bash
git-ai login --server http://localhost:<port>
git-ai whoami
```

5. 用 Alice 上传一次 metrics/report。
6. 注册并登录 Bob，重复上传。
7. 用 admin 登录 dashboard 查看组织、部门、开发者聚合。

验收标准：

- Alice 可以注册到 Linewell / Backend。
- 非允许域名不能注册到 Linewell。
- `git-ai login` 打开浏览器并自动完成 token 保存。
- `git-ai whoami` 显示 Alice。
- Alice 和 Bob 的 dashboard 数据分开。

### 阶段 7 执行记录（2026-07-06）

状态：服务端和 CLI 的无数据库单元/handler 覆盖已补齐；真实 Postgres、浏览器、Alice/Bob 上传和 dashboard 聚合验收留到手工 E2E。

已完成改动：

- 更新 `enterprise-server/src/services/registration.rs`：
  - `email_domain()` 增加空 local part 和多 `@` 拒绝。
  - 增加邮箱域名 lowercase、非法邮箱、首尾空白裁剪测试。
- 更新 `enterprise-server/src/handlers/auth_api.rs`：
  - 将 session cookie 拼装拆成可单测 helper。
  - 增加注册 JSON/form 解析、登录 form 解析、必填字段拒绝、`safe_return_to()` 本地路径限制、cookie `Secure`/`HttpOnly`/`SameSite` 和 cookie 提取测试。
- 更新 `enterprise-server/src/models/auth.rs`：
  - 增加 `authorization_code` token request 反序列化测试。
  - 增加旧 device grant 字段保持 optional 的兼容性测试。
- 更新 `enterprise-server/src/handlers/oauth.rs`：
  - 增加 PKCE 错误 verifier 不匹配测试。
- 更新 `src/commands/login.rs`：
  - 增加 `--help`、未知参数拒绝、OAuth 错误描述保留测试。
  - 既有测试继续覆盖 `--server`、`--server=...`、`--no-browser`、state mismatch 和授权 URL 参数。

验证结果：

- `cd enterprise-server && cargo test` 通过：39 passed, 0 failed。
- `cargo test login` 通过：10 passed, 0 failed。
- `cargo test pkce` 通过：4 passed, 0 failed。
- `cargo test auth::client` 通过：16 passed, 0 failed。
- `cargo test callback` 首次在沙箱内因本地 listener 绑定 `127.0.0.1` 被拒绝；放开 loopback 后通过：4 passed, 0 failed。
- `cargo test auth` 放开 loopback 后通过，包括 lib、integration 和 notes sync 匹配用例。
- `task build` 通过。

未自动化项：

- Web session 创建/读取/过期/撤销、authorization code 一次性消费、部门归属校验依赖真实 Postgres 数据和 migration 后 schema，当前阶段记录为数据库集成/手工 E2E 验收项。
- Alice/Bob 注册、CLI 浏览器授权、`git-ai whoami`、metrics/report 上传和 dashboard 聚合分离需要启动数据库、enterprise server 和浏览器后手工执行。

## 11. 阶段 8：文档和运维

### Task 8.1：更新开发者文档

改动范围：

- `docs/guides/developer-end-to-end-workflow.md`
- `docs/guides/developer-install-guide.md`
- `docs/guides/local-run-guide.md`

执行步骤：

1. 把开发者登录说明改为浏览器授权流程。
2. 删除或弱化设备码 `/verify` 主流程说明。
3. 增加 `--no-browser` 场景说明。
4. 增加 `git-ai whoami` 验证身份的步骤。

验收标准：

- 新开发者能按文档完成注册、登录、身份验证。

### Task 8.2：更新部署和管理员文档

改动范围：

- `docs/guides/server-deployment.md`
- `docs/enterprise/enterprise-server-deployment.md`
- `docs/architecture/system-roles-and-usage.md`

执行步骤：

1. 说明管理员需要先创建组织和部门。
2. 说明如何配置 `organization_domains`。
3. 说明只有 `verified = true` 的域名允许自助注册。
4. 说明新注册用户默认 role 是 `member`。
5. 说明旧 `/verify` device flow 已 deprecated。

验收标准：

- 管理员能按文档配置组织域名和部门。

### Task 8.3：补充发布说明和兼容策略

改动范围：

- PR 描述或 release notes。

执行步骤：

1. 写明新版 `git-ai login` 改为 browser authorization code + PKCE。
2. 写明旧 device flow 暂时保留。
3. 写明新增数据库迁移。
4. 写明 dashboard 组织归属改为优先使用 `default_org_id`。
5. 写明生产环境 cookie 需要 HTTPS `Secure`。

验收标准：

- 运维能评估迁移风险。
- 旧客户端兼容边界清楚。

## 12. 最终验收清单

注册：

- [ ] `alice@linewell.com` 可以注册到 Linewell。
- [ ] 注册时可以选择 Backend 部门。
- [ ] 注册后 `users.email = alice@linewell.com`。
- [ ] 注册后 `users.password_hash` 不为空且不是明文密码。
- [ ] 注册后 `users.default_org_id` 指向 Linewell。
- [ ] 注册后公司组织 `org_members.role = member`。
- [ ] 注册后公司组织 `org_members.department_id` 指向 Backend。
- [ ] 注册后个人组织 membership 是 `owner`。

非法注册：

- [ ] 非允许域名邮箱不能加入 Linewell。
- [ ] 未 verified 的域名不能用于注册。
- [ ] 用户不能提交不属于该组织的 `department_id`。
- [ ] 用户不能注册成 `admin` 或 `owner`。
- [ ] 重复邮箱不能重复创建用户。

Web 登录：

- [ ] 正确账号密码能登录。
- [ ] 错误密码不能登录。
- [ ] disabled 用户不能登录。
- [ ] 登录成功写入 `web_session` HttpOnly cookie。
- [ ] 登出后 session 失效。

CLI 登录：

- [ ] `git-ai login --server ...` 会打开浏览器。
- [ ] `git-ai login --server ... --no-browser` 只打印授权 URL。
- [ ] 浏览器未登录时先登录。
- [ ] 浏览器已登录时显示授权页。
- [ ] 授权页显示当前用户、组织、部门。
- [ ] 点击授权后 CLI 自动登录成功。
- [ ] 授权取消时 CLI 显示清晰错误。
- [ ] state 不匹配时 CLI 拒绝登录。
- [ ] authorization code 只能兑换一次。
- [ ] 新版 `git-ai login` 不再调用 `/worker/oauth/device/code`。

身份：

- [ ] `git-ai whoami` 显示 `alice@linewell.com`。
- [ ] `git-ai whoami` 显示 Linewell 和 `role=member`。
- [ ] Alice 登录不会拿到 Bob 或管理员身份。

Dashboard：

- [ ] Alice 提交代码并上传 metrics/report 后，dashboard 开发者列表出现 Alice。
- [ ] Alice 的数据归属 Linewell / Backend。
- [ ] Bob 的数据不会归到 Alice。
- [ ] 普通 member 只能看自己的数据。
- [ ] admin/owner 可以看同组织成员数据。

兼容：

- [ ] 旧 device flow 接口暂时仍存在。
- [ ] `/verify` 不再作为文档推荐路径。
- [ ] refresh token grant 仍可用。
- [ ] install nonce grant 仍可用。

## 13. 建议提交顺序

1. `Add developer auth database migrations`
2. `Add web session and password services`
3. `Add registration and login pages`
4. `Add CLI authorization code grant`
5. `Switch git-ai login to browser auth`
6. `Use default organization for dashboard scope`
7. `Document developer registration auth flow`

每个提交尽量对应一个阶段或少量相邻任务，避免把数据库、服务端、CLI、dashboard 全部混在一个不可回滚提交里。
