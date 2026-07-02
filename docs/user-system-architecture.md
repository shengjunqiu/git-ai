# git-ai 用户体系架构分析

> 生成日期：2026-06-04

## 一、核心实体关系

### 1.1 ER 关系图

```
┌─────────────┐       1:N       ┌──────────────┐       1:N       ┌──────────────┐
│   User      │ ◄────────────── │  OrgMember   │ ──────────────► │ Organization │
│─────────────│                 │──────────────│                 │──────────────│
│ id (UUID)   │                 │ user_id (PK) │                 │ id (UUID)    │
│ email       │                 │ org_id (PK)  │                 │ name         │
│ name        │                 │ department_id│──┐              │ slug         │
│personal_org │──┐              │ role         │  │              └──────────────┘
└─────────────┘  │              └──────────────┘  │                     ▲
                 │                                │                     │
                 │              ┌──────────────┐  │                     │
                 │              │ Department   │──┘              1:N    │
                 │              │──────────────│───────────────────────┘
                 │              │ id (UUID)    │
                 │              │ org_id (FK)  │
                 │              │ name         │
                 │              │ slug         │
                 │              └──────────────┘
                 │
                 │  1:N         ┌──────────────┐
                 └─────────────►│   ApiKey      │
                                │──────────────│
                                │ id (UUID)    │
                                │ user_id (FK) │
                                │ org_id (FK)  │──► Organization (可选)
                                │ key_prefix   │
                                │ key_hash     │
                                │ name         │
                                │ scopes       │
                                │ expires_at   │
                                │ revoked_at   │
                                └──────────────┘
```

### 1.2 三大核心关系

| 关系 | 类型 | 说明 |
|------|------|------|
| User ↔ Organization | **N:N**（通过 `org_members`） | 一个用户可属于多个组织，一个组织可有多个用户 |
| Organization ↔ Department | **1:N** | 一个组织包含多个部门 |
| User → ApiKey | **1:N** | 一个用户可拥有多个 API Key |

---

## 二、用户 (User)

### 2.1 用户模型

```sql
users (
  id UUID PK,              -- 全局唯一标识
  email TEXT UNIQUE,        -- 登录标识，唯一
  name TEXT,                -- 显示名称
  personal_org_id UUID,     -- 个人组织 ID（每个用户自动创建一个）
  created_at, updated_at
)
```

### 2.2 关键设计：个人组织 (Personal Org)

每个用户注册时自动创建一个 **个人组织**，`personal_org_id` 指向该组织。这是一个重要的设计决策：

- 用户的个人项目数据归属到个人组织
- 即使用户不属于任何"真实组织"，数据也有归属
- 个人组织与正式组织在数据模型上完全一致，只是用途不同

### 2.3 用户角色体系

角色定义在 `org_members.role`，有三种：

| 角色 | 权限范围 |
|------|---------|
| `owner` | 组织所有者，拥有完全管理权限 |
| `admin` | 管理员，可管理用户/API Key/部门等 |
| `member` | 普通成员，仅可使用分配的资源 |

**管理员判定逻辑**（`AuthIdentity::is_admin()`）：
```rust
// Bearer Token 认证时：检查 org_members 中的角色
role == "owner" || role == "admin"
// API Key 认证时：检查 scopes 中是否包含 "admin"
scopes.contains("admin")
```

---

## 三、API 密钥 (API Key)

### 3.1 API Key 模型

```sql
api_keys (
  id UUID PK,
  user_id UUID FK → users,          -- 所属用户（必须）
  org_id UUID FK → organizations,    -- 关联组织（可选）
  key_prefix TEXT,                    -- 前8字符，如 "gai_1a2b"
  key_hash TEXT UNIQUE,               -- SHA256(完整key)，不存明文
  name TEXT,                          -- 描述，如 "CI/CD Pipeline"
  scopes TEXT[],                      -- 权限范围
  expires_at,                         -- 过期时间
  last_used_at,                       -- 最后使用时间
  revoked_at                          -- 撤销时间
)
```

### 3.2 API Key 格式

```
gai_ + 66位十六进制字符（33字节随机数的hex编码）
示例：gai_1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b
```

- `key_prefix`：前 8 字符（如 `gai_1a2b`），用于识别显示
- `key_hash`：完整 key 的 SHA256 哈希，用于验证（不存储明文）

### 3.3 API Key 的三级归属

```
API Key ──► User（必须关联，标识"谁创建的"）
         ──► Organization（可选关联，标识"用于哪个组织的数据"）
```

**关键区别**：
- `user_id` 是**必填的**：API Key 必须属于某个用户
- `org_id` 是**可选的**：API Key 可以关联到某个组织，也可以不关联

当 API Key 关联了 `org_id` 时：
- 中间件提取身份时，会使用该 `org_id` 作为数据隔离的依据
- 适用于 CI/CD 场景，确保流水线操作归属到正确的组织

### 3.4 API Key 的 Scopes（权限范围）

默认 scopes：
```rust
["metrics:write", "cas:write", "cas:read", "reports:write"]
```

特殊 scope：
- `"admin"` — 赋予管理员权限，可访问 admin API

### 3.5 API Key vs JWT Token 的认证差异

| 维度 | JWT Bearer Token | API Key |
|------|-----------------|---------|
| 认证方式 | `Authorization: Bearer <token>` | `X-API-Key: gai_xxx` 或 Cookie |
| 来源 | OAuth 登录流程签发 | 管理员通过 Admin API 创建 |
| 用户身份 | 直接关联 User + 多个 Org | 关联 User + 可选 Org |
| Scopes | 固定全权限 (metrics/cas/reports) | 可自定义 |
| 过期 | Access Token 1小时 + Refresh Token 90天 | 可配置 expires_at 或永不过期 |
| 适用场景 | 交互式用户（CLI/浏览器） | CI/CD、自动化脚本、非交互式 |
| 角色来源 | JWT claims 中的 `orgs[].role` | `org_members` 表查询，或回退 `"api_key"` |

---

## 四、组织 (Organization)

### 4.1 组织模型

```sql
organizations (
  id UUID PK,
  name TEXT,          -- 组织名称
  slug TEXT UNIQUE,   -- URL 友好标识
  created_at
)
```

### 4.2 组织的两重身份

| 类型 | 创建时机 | 用途 |
|------|---------|------|
| **个人组织** | 用户注册时自动创建 | 归属用户的个人项目数据 |
| **正式组织** | 管理员手动创建 | 企业团队、部门数据隔离 |

### 4.3 数据隔离

组织是**数据隔离的核心边界**，用户是**数据隔离的细粒度边界**。数据隔离规则如下：

| 用户角色 | 组织级可见范围 | 用户级可见范围 | 说明 |
|---------|-------------|-------------|------|
| 管理员 (owner/admin) | 本组织所有数据 | 所有用户的数据 | 管理员可查看组织内全部数据 |
| 普通成员 (member) | 本组织数据 | 仅自己的数据 | 同组织成员间数据相互隔离 |
| API Key (admin scope) | 关联组织所有数据 | 所有用户的数据 | 等同管理员权限 |
| API Key (默认 scope) | 关联组织数据 | 仅关联用户的数据 | 等同普通成员权限 |

```rust
// build_data_filters() 的核心逻辑
// 管理员: (None, Some(org_id)) — 可看组织内所有用户的数据
// 普通成员: (Some(user_id), Some(org_id)) — 只能看自己的数据
pub fn build_data_filters(auth: &AuthIdentity) -> (Option<Uuid>, Option<Uuid>) {
    if auth.is_admin() {
        (None, auth.org_id)          // 管理员：按组织过滤，不过滤用户
    } else {
        (Some(auth.user_id), auth.org_id)  // 普通成员：同时按组织和用户过滤
    }
}
```

**SQL 过滤模式**：
```sql
-- 管理员查询（只按组织过滤）
WHERE ($1::uuid IS NULL OR user_id = $1)     -- $1=NULL, 不过滤用户
  AND ($2::uuid IS NULL OR org_id = $2)      -- $2=org_id, 过滤组织

-- 普通成员查询（按组织+用户双重过滤）
WHERE ($1::uuid IS NULL OR user_id = $1)     -- $1=user_id, 过滤用户
  AND ($2::uuid IS NULL OR org_id = $2)      -- $2=org_id, 过滤组织
```

**数据隔离覆盖的端点**：

| 端点 | 过滤字段 | 说明 |
|------|---------|------|
| `GET /api/v1/aggregate/summary` | user_id + org_id | 提交统计 |
| `GET /api/v1/aggregate/organizations` | user_id + org_id | 组织列表 |
| `GET /api/v1/aggregate/departments` | user_id + org_id | 部门列表 |
| `GET /api/v1/aggregate/projects` | user_id + org_id | 项目列表 |
| `GET /api/v1/aggregate/developers` | user_id + org_id | 开发者列表 |
| `GET /api/v1/aggregate/tools` | user_id + org_id | 工具统计 |
| `GET /api/v1/aggregate/trends` | user_id + org_id | 趋势数据 |
| `GET /api/v1/aggregate/agent-comparison` | user_id + org_id | Agent 对比 |
| `GET /api/v1/aggregate/team-comparison` | user_id + org_id | 团队对比 |
| `GET /api/v1/aggregate/pull-requests` | org_id | PR 聚合 |
| `GET /api/v1/ai-code-persistence` | user_id + org_id | AI 代码持久化 |
| `GET /api/v1/agent-readiness` | user_id + org_id | Agent 准备度 |
| `GET /api/v1/ai-code-lifecycle` | user_id + org_id | AI 代码生命周期 |
| `GET /worker/cas/` | user_id + org_id | CAS 对象读取 |
| `POST /api/bundles` | user_id (创建时) | Bundle 创建 |

---

## 五、服务端 ↔ 客户端用户体系交互

### 5.1 完整认证流程

```
┌────────────────────────────────────────────────────────────────────┐
│                     git-ai 客户端认证流程                           │
├────────────────────────────────────────────────────────────────────┤
│                                                                    │
│  场景A: 交互式登录 (git-ai login)                                   │
│  ════════════════════════════════════                                 │
│  1. CLI → POST /worker/oauth/device/code                           │
│     ← { device_code, user_code, verification_uri, expires_in }    │
│                                                                    │
│  2. 用户在浏览器打开 verification_uri，输入 user_code 授权           │
│     (服务端 verify 页面 → 更新 oauth_devices.user_id)              │
│                                                                    │
│  3. CLI 轮询 POST /worker/oauth/token                              │
│     grant_type=urn:ietf:params:oauth:grant-type:device_code       │
│     ← { access_token(1h), refresh_token(90d) }                    │
│                                                                    │
│  4. 客户端存储凭证到 ~/.git-ai/internal/credentials                │
│     (或系统 Keyring，取决于 feature flag)                          │
│                                                                    │
│  场景B: 安装脚本自动登录 (install.ps1 / install.sh)                 │
│  ═══════════════════════════════════════════════                   │
│  1. Web管理后台生成 install_nonce，关联到 user_id                   │
│     传入环境变量 INSTALL_NONCE + API_BASE                          │
│                                                                    │
│  2. 安装脚本调用 git-ai exchange-nonce                              │
│     CLI → POST /worker/oauth/token                                 │
│     grant_type=install_nonce                                       │
│     ← { access_token, refresh_token }  (nonce 一次性消费)          │
│                                                                    │
│  3. 客户端自动存储凭证                                              │
│                                                                    │
│  场景C: API Key 认证 (CI/CD)                                       │
│  ══════════════════════════════════                                   │
│  1. 管理员在 Admin 面板创建 API Key                                 │
│     关联 user_id + 可选 org_id                                     │
│                                                                    │
│  2. 客户端通过环境变量 GIT_AI_API_KEY 或 config.json 配置           │
│     请求时使用 X-API-Key: gai_xxx 头                               │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

### 5.2 客户端凭证存储

客户端存储的 `StoredCredentials` 结构：

```rust
pub struct StoredCredentials {
    pub access_token: String,            // JWT，1小时有效
    pub refresh_token: String,           // 90天有效
    pub access_token_expires_at: i64,    // Unix 时间戳
    pub refresh_token_expires_at: i64,   // Unix 时间戳
}
```

存储位置优先级：
1. **系统 Keyring**（macOS Keychain / Windows Credential Manager / Linux Secret Service）— 需 `auth_keyring` feature flag
2. **文件存储**（fallback）：`~/.git-ai/internal/credentials`

### 5.3 客户端 Token 自动刷新

```
客户端每次发请求时:
┌─ 加载凭证
├─ refresh_token 过期? → 无法认证，需要重新登录
├─ access_token 有效(>5min)? → 直接使用
└─ access_token 即将过期? → 自动刷新
   ├─ POST /worker/oauth/token (grant_type=refresh_token)
   ├─ 服务端: 撤销旧 refresh_token, 签发新 access_token + refresh_token
   └─ 客户端: 更新存储的凭证
```

### 5.4 客户端身份提取

客户端**不验证** JWT 签名，而是直接解码 JWT payload 提取身份信息：

```rust
// identity.rs - 客户端侧（无需服务端 secret）
pub fn extract_identity_from_access_token(access_token: &str) -> TokenIdentity {
    // Base64 解码 JWT payload（不验证签名）
    // 提取: user_id, email, name, personal_org_id, orgs[]
}
```

JWT Claims 结构（服务端签发，客户端消费）：

```rust
pub struct JwtClaims {
    sub: String,                    // user UUID
    email: String,
    name: String,
    personal_org_id: Option<String>,
    orgs: Vec<JwtOrg>,              // [{org_id, org_name, org_slug, role}]
    iat: i64,
    exp: i64,                       // 1小时后过期
}
```

### 5.5 客户端请求认证方式

客户端发请求时的认证优先级：

```
1. Bearer Token (OAuth credentials)
   Authorization: Bearer <access_token>

2. API Key (环境变量或配置文件)
   X-API-Key: gai_xxx

3. 无认证 (公开端点)
```

服务端中间件验证优先级：
```
1. Authorization: Bearer → 验证 JWT 签名
2. Cookie: access_token → 验证 JWT 签名（浏览器 Dashboard）
3. X-API-Key / Cookie: api_key → 查询 api_keys 表
```

---

## 六、三种登录场景的端到端流程

### 6.1 交互式登录（Developer 日常使用）

```
Developer          git-ai CLI           Enterprise Server        Browser
   │                   │                      │                     │
   │  git-ai login     │                      │                     │
   │──────────────────►│                      │                     │
   │                   │  POST /device/code   │                     │
   │                   │─────────────────────►│                     │
   │                   │  device_code+user_code│                     │
   │                   │◄─────────────────────│                     │
   │                   │                      │                     │
   │  "Open URL &      │                      │                     │
   │   enter code"     │                      │                     │
   │◄──────────────────│                      │                     │
   │                   │                      │                     │
   │  打开浏览器 ───────────────────────────────────────────────────►│
   │                   │                      │    验证 user_code   │
   │                   │                      │◄────────────────────│
   │                   │                      │  更新 authorized    │
   │                   │                      │                     │
   │                   │  POST /token (轮询)  │                     │
   │                   │─────────────────────►│                     │
   │                   │  access+refresh token│                     │
   │                   │◄─────────────────────│                     │
   │                   │                      │                     │
   │  "Logged in!"     │                      │                     │
   │◄──────────────────│                      │                     │
```

### 6.2 安装脚本自动登录（企业部署）

```
Admin Dashboard     Install Script       git-ai CLI          Enterprise Server
      │                  │                  │                       │
  生成 nonce          │                  │                       │
  关联 user_id        │                  │                       │
      │                  │                  │                       │
      │  INSTALL_NONCE   │                  │                       │
      │  API_BASE        │                  │                       │
      │─────────────────►│                  │                       │
      │                  │ git-ai           │                       │
      │                  │ exchange-nonce   │                       │
      │                  │─────────────────►│                       │
      │                  │                  │  POST /token          │
      │                  │                  │  grant_type=          │
      │                  │                  │  install_nonce        │
      │                  │                  │──────────────────────►│
      │                  │                  │                       │
      │                  │                  │    验证 nonce          │
      │                  │                  │    标记 used=true     │
      │                  │                  │    签发 JWT           │
      │                  │                  │◄──────────────────────│
      │                  │  ✓ Logged in     │                       │
      │                  │◄─────────────────│                       │
```

### 6.3 API Key 认证（CI/CD 流水线）

```
Admin Dashboard     CI/CD Pipeline       git-ai CLI          Enterprise Server
      │                  │                  │                       │
  创建 API Key         │                  │                       │
  user_id + org_id     │                  │                       │
  scopes=[...]         │                  │                       │
      │                  │                  │                       │
      │  GIT_AI_API_KEY  │                  │                       │
      │─────────────────►│                  │                       │
      │                  │ git commands     │                       │
      │                  │─────────────────►│                       │
      │                  │                  │  X-API-Key: gai_xxx   │
      │                  │                  │──────────────────────►│
      │                  │                  │                       │
      │                  │                  │    查 api_keys 表     │
      │                  │                  │    验证 hash+过期+撤销 │
      │                  │                  │◄──────────────────────│
```

---

## 七、核心源码索引

| 模块 | 文件路径 | 职责 |
|------|---------|------|
| 服务端 JWT | `enterprise-server/src/auth/jwt.rs` | JWT 签发与验证 |
| 服务端中间件 | `enterprise-server/src/auth/middleware.rs` | 请求认证提取 (Bearer / API Key / Cookie) |
| 服务端 OAuth | `enterprise-server/src/handlers/oauth.rs` | Device Flow / Refresh Token / Install Nonce |
| 服务端 Admin | `enterprise-server/src/handlers/admin.rs` | 用户/组织/API Key 管理 |
| 服务端模型 | `enterprise-server/src/models/user.rs` | User / Organization / ApiKey / OrgMember 数据模型 |
| 客户端凭证 | `src/auth/credentials.rs` | 凭证存储与加载 |
| 客户端身份 | `src/auth/identity.rs` | JWT payload 解码提取身份 |
| 客户端认证 | `src/auth/client.rs` | 认证 HTTP 客户端 (自动附加 Token/API Key) |
| 客户端状态 | `src/auth/state.rs` | 认证状态机 (凭证加载/刷新/过期) |
| 客户端登录 | `src/commands/login.rs` | `git-ai login` 命令 |
| 客户端 Nonce | `src/commands/exchange_nonce.rs` | `git-ai exchange-nonce` 命令 |
| 客户端 Whoami | `src/commands/whoami.rs` | `git-ai whoami` 命令 |

---

## 八、总结：核心设计要点

1. **用户与组织是 N:N 关系**，通过 `org_members` 关联，每个用户注册时自动创建个人组织
2. **API Key 必须关联 User**，可选关联 Organization；这是 CI/CD 非交互式认证的唯一方式
3. **JWT 承载完整的用户+组织身份**（sub, email, orgs[]），客户端无需回查服务端即可知道用户身份
4. **数据隔离按组织+用户双重边界**：管理员可看组织内所有数据，普通成员只能看自己的数据；同一组织内成员间数据相互隔离
5. **三种登录方式**覆盖不同场景：Device Flow（交互式）、Install Nonce（自动部署）、API Key（CI/CD）
6. **客户端凭证自包含**：JWT payload 中嵌入身份信息，客户端直接解码获取（不验证签名，签名验证在服务端中间件完成）
7. **默认组织为 linewell.com**（slug: `linewell.com`），管理员邮箱 `admin@linewell.com`
