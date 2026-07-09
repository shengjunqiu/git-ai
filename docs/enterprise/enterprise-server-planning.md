# git-ai 企业服务端功能规划文档

> 基于 git-ai v1.3.2 客户端代码逆向分析，规划与客户端完全兼容的企业服务端

## 一、项目概述

### 1.1 目标

构建一个与 git-ai 客户端完全兼容的企业服务端，替代默认的 `https://usegitai.com`，实现：

- 完整的用户认证体系（OAuth Device Authorization Flow）
- Metrics 使用统计数据接收与存储（支持部分成功响应）
- CAS（Content Addressable Storage）Prompt 数据接收、存储与读取（支持 IDE 插件获取对话记录）
- Report 报告数据接收与查询
- Summary 摘要数据接收
- Bundle/Share 分享功能
- 版本更新分发（含二进制文件托管）
- Sentry 遥测接收（双 DSN 支持）
- PostHog 分析集成
- Git Authorship Notes 同步支持
- Feature Flags 远程管理
- JetBrains 插件下载代理
- 数据统计看板（Web Dashboard）
- 企业配置管理
- PR 级别聚合、代码持久性追踪、Agent 就绪度评估
- CI/CD 集成与 AI 代码全生命周期追踪
- 客户端归属追踪数据管道完整兼容（Checkpoint → Working Log → Authorship Note）
- Daemon 异步架构通信协议兼容
- 14 种 AI Agent 生态兼容
- 离线缓冲与恢复机制支持
- 跨平台部署支持（Windows + Unix）

### 1.2 核心兼容性要求

客户端通过修改 `api_base_url`（配置文件或环境变量 `GIT_AI_API_BASE_URL`）指向企业服务端。服务端必须：

- 实现客户端调用的所有 `/worker/*` 和 `/api/*` 端点
- 遵循客户端期望的请求/响应格式（严格匹配字段名和数据结构）
- 支持 `Authorization: Bearer` 和 `X-API-Key` 两种认证方式
- 支持 `X-Distinct-ID`、`X-Author-Identity` 等自定义请求头
- CAS 读取端点返回完整 Prompt 内容（不仅是存在性检查）
- Metrics 上传支持部分成功响应（200 + errors 数组）
- Dashboard URL 路径为 `/me`（不是 `/dashboard`）
- 支持 Git `refs/notes/ai` 的 push/fetch（Authorship Notes 同步）
- 支持 Daemon 模式下的 3 秒 flush 循环（Metrics/CAS 批量上传）
- 支持离线缓冲恢复（幂等处理、延迟到达数据的正确处理）
- 支持 Windows Named Pipe 和 Unix Socket 两种 Daemon 通信方式
- 支持 14 种 AI Agent 的不同 PromptRecord 格式

### 1.3 客户端认证门控逻辑

```rust
should_upload = !using_default_api || client.is_logged_in() || client.has_api_key()
```

即：当使用非默认 API 地址时，始终上传数据；使用默认地址时，需登录或有 API Key。

> **注意**：`ApiContext::new()` 构造时自动尝试加载存储的凭据（`try_load_auth_token()`），如果 access_token 即将过期（5 分钟缓冲），会自动使用 refresh_token 刷新（通过全局 `REFRESH_LOCK: Mutex` 防止并发刷新）。因此即使客户端未显式登录，只要之前登录过且 refresh_token 未过期，仍会携带有效 token。

---

## 二、API 端点完整清单

### 2.1 OAuth 认证端点

#### POST `/worker/oauth/device/code` — 启动设备授权

| 项目 | 说明 |
|------|------|
| 认证 | 无 |
| 请求头 | `Content-Type: application/json`, `User-Agent: git-ai/{version}`, `X-Distinct-ID: {id}` |
| 请求体 | `{}` (空 JSON) |
| 成功响应 (200) | 见下方 |
| 错误响应 | 纯文本或 `{"error":"...","error_description":"..."}` |

```json
{
  "device_code": "string (必需)",
  "user_code": "string (必需)",
  "verification_uri": "string (必需)",
  "verification_uri_complete": "string (可选)",
  "expires_in": 900,
  "interval": 5
}
```

**服务端实现要求**：
- 生成唯一的 `device_code` 和 `user_code`
- 提供 Web 验证页面（`verification_uri`）
- 设备码有效期通常 900 秒
- 轮询间隔建议 5 秒

---

#### POST `/worker/oauth/token` — 令牌交换（三种用途）

**用途 1: 设备码轮询**

```json
{
  "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
  "device_code": "string",
  "client_id": "git-ai-cli"
}
```

**用途 2: 刷新令牌**

```json
{
  "grant_type": "refresh_token",
  "refresh_token": "string",
  "client_id": "git-ai-cli"
}
```

**用途 3: 安装 Nonce 交换**

```json
{
  "grant_type": "install_nonce",
  "install_nonce": "string",
  "client_id": "git-ai-cli"
}
```

**成功响应 (200)**：

```json
{
  "access_token": "string (JWT)",
  "token_type": "Bearer",
  "expires_in": 3600,
  "refresh_token": "string",
  "refresh_expires_in": 7776000
}
```

**错误响应（RFC 8628）**：

```json
{
  "error": "authorization_pending | slow_down | access_denied | expired_token | invalid_grant",
  "error_description": "string (可选)"
}
```

**服务端实现要求**：
- `access_token` 为 JWT，Payload 包含：`sub`(用户ID)、`email`、`name`、`orgs`(数组)、`personal_org_id`
- `client_id` 固定为 `"git-ai-cli"`
- 访问令牌有效期 3600 秒（1小时）
- 刷新令牌有效期 7776000 秒（约90天）
- 设备码轮询需返回 `authorization_pending` 直到用户授权
- 收到 `slow_down` 时客户端自动增加 5 秒间隔
- 安装 Nonce 用于安装脚本自动登录，需一次性验证

---

### 2.2 Metrics 上传端点

#### POST `/worker/metrics/upload` — 批量上传 Metrics 事件

| 项目 | 说明 |
|------|------|
| 认证 | `Authorization: Bearer {token}` 或 `X-API-Key: {key}` |
| 请求头 | `Content-Type: application/json`, `User-Agent: git-ai/{version}`, `X-Distinct-ID: {id}` |
| 请求体 | `MetricsBatch` JSON（见下方数据结构） |
| 成功响应 (200) | `MetricsUploadResponse` JSON（见下方，支持部分成功） |
| 失败响应 | 400/401/500 等非 200 状态码 |

**MetricsBatch 结构**：

```json
{
  "api_version": "metrics/1.0.0",
  "events": [
    {
      "t": 1700000000,
      "e": 1,
      "v": { "0": "value1", "1": "value2" },
      "a": { "0": "attr1", "1": "attr2" }
    }
  ]
}
```

> **注意**：客户端使用 PosEncoded 编码，字段名是数字索引字符串而非语义名称。服务端需要参照 `src/metrics/pos_encoded.rs` 和 `src/metrics/attrs.rs` 中的编码映射来还原字段含义。

**Metrics 事件类型（e 字段）**：

| event_id | 类型 | 说明 |
|----------|------|------|
| 0 | InstallHooks | 安装 git hooks |
| 1 | Committed | 提交事件 |
| 2 | AgentUsage | AI 代理使用 |
| 3 | Checkpoint | Checkpoint 事件 |

**Committed 事件关键字段**（v 字段编码后）：

| 原始字段名 | 类型 | 说明 |
|-----------|------|------|
| commit_sha | String | 提交 SHA |
| human_additions | u32 | 人工新增行数 |
| ai_additions | Vec\<u32\> | 各工具/模型 AI 新增行数 |
| git_diff_added_lines | u32 | Git diff 总新增行 |
| git_diff_deleted_lines | u32 | Git diff 总删除行 |
| tool_model_pairs | Vec\<String\> | 工具/模型对列表（如 "claude-code::claude-3.5"） |

**EventAttributes 关键字段**（a 字段编码后）：

| 原始字段名 | 类型 | 说明 |
|-----------|------|------|
| version | String | git-ai 版本 |
| repo_url | Option\<String\> | 仓库 URL |
| author | Option\<String\> | 提交者邮箱 |
| tool | Option\<String\> | AI 工具名 |
| distinct_id | Option\<String\> | 客户端唯一标识 |
| custom_attributes | HashMap\<String, String\> | 自定义属性 |

**MetricsUploadResponse 成功响应 (200)**：

```json
{
  "errors": []
}
```

> 当 `errors` 为空数组时表示全部事件上传成功。当部分事件验证失败时，HTTP 状态码仍为 200，但 `errors` 数组包含失败事件的索引和错误信息：

```json
{
  "errors": [
    { "index": 2, "error": "Invalid event data" },
    { "index": 5, "error": "Missing required field" }
  ]
}
```

客户端使用 `successful_indices(batch_size)` 方法从 `errors` 数组中提取成功的事件索引，仅将验证失败的事件日志记录到 Sentry，不做重试（验证错误重试不会成功）。

**服务端实现要求**：
- 接收批量事件，支持幂等（客户端不做重试逻辑，但会重传）
- 事件为 PosEncoded 编码，需解码后存储
- 按 `distinct_id` 或 `author` 关联用户
- 支持按组织/仓库/工具/模型聚合查询
- 200 响应即使包含 errors 也视为成功；400/401/500 才触发客户端重试

**客户端重试策略**：
- **API 层（telemetry_worker 常规上传）**：仅单次重试（首次 + 60 秒后 1 次重试 = 共 2 次尝试）。源码 `src/api/metrics.rs` 中 `RETRY_DELAYS_SECS: [u64; 1] = [60]`
- **手动刷新（flush-metrics-db 命令）**：使用 `upload_metrics_with_retry()` 同一函数，同样为 1 次重试
- 部分成功（200 + errors）不会触发重试，仅记录到 Sentry

---

### 2.3 CAS 上传端点

#### POST `/worker/cas/upload` — 批量上传 CAS 对象

| 项目 | 说明 |
|------|------|
| 认证 | `Authorization: Bearer {token}` 或 `X-API-Key: {key}` + `X-Author-Identity: {git_committer}` |
| 请求体 | `CasUploadRequest` JSON |

```json
{
  "objects": [
    {
      "content": { /* AI Prompt 记录 JSON (PromptRecord) */ },
      "hash": "sha256-hash-string",
      "metadata": {
        "key1": "value1",
        "key2": "value2"
      }
    }
  ]
}
```

> **注意**：`metadata` 类型为 `HashMap<String, String>`（键值对），而非 JSON 字符串。空 metadata 时不序列化（`skip_serializing_if`）。

**成功响应**：

```json
{
  "results": [
    {
      "hash": "sha256-hash-string",
      "status": "ok | error",
      "error": "optional error message (仅 status=error 时)"
    }
  ],
  "success_count": 1,
  "failure_count": 0
}
```

> `error` 字段仅在 `status` 为 `"error"` 时存在，`status` 为 `"ok"` 时不序列化。

**服务端实现要求**：
- 按 hash 去重存储（Content Addressable）
- `content` 为 PromptRecord 的 JSON 表示，包含 AI 对话记录
- `metadata` 为可选的键值对元数据
- 需做密钥脱敏（客户端在上传前已做 `secrets.rs` 中的脱敏处理）
- hash 值需验证为十六进制字符（防注入）

---

#### GET `/worker/cas/?hashes={comma-separated-hashes}` — 批量读取 CAS 对象

| 项目 | 说明 |
|------|------|
| 认证 | `Authorization: Bearer {token}` 或 `X-API-Key: {key}` |
| 参数 | `hashes`：逗号分隔的 SHA256 哈希列表（最多 100 个/次） |
| 成功响应 (200) | `CAPromptStoreReadResponse` JSON（见下方） |
| 全部不存在 (404) | 返回空结果（非错误） |

**CAPromptStoreReadResponse 成功响应 (200)**：

```json
{
  "results": [
    {
      "hash": "sha256-hash-string",
      "status": "ok",
      "content": { /* Prompt 记录完整 JSON */ }
    },
    {
      "hash": "nonexistent-hash",
      "status": "error",
      "error": "Not found"
    }
  ],
  "success_count": 1,
  "failure_count": 1
}
```

> **重要**：此端点返回的是 Prompt 的**完整内容**（`content` 字段），而不仅仅是存在性检查。VS Code 扩展的 `blame-service.ts` 使用 `fetchPromptFromCAS` 函数调用此端点获取 AI 对话记录并在 IDE 中展示。

**服务端实现要求**：
- 同时用于**去重检查**（客户端上传前检查是否已存在）和**内容读取**（IDE 插件获取 Prompt 内容）
- 客户端在调用前会验证所有 hash 值仅包含十六进制字符（防注入），服务端也应做相同校验
- 每次最多 100 个 hash
- 404 响应（远程无 notes ref）时客户端优雅处理为空结果
- `content` 字段包含完整的 `PromptRecord` JSON

---

### 2.4 Bundle/Share 端点

#### POST `/api/bundles` — 创建分享 Bundle

| 项目 | 说明 |
|------|------|
| 认证 | `Authorization: Bearer {token}`（必须登录，未认证返回 401） |
| 请求体 | `CreateBundleRequest` JSON |

**客户端发送的 Bundle 数据结构**：

```json
{
  "title": "string (至少1字符)",
  "data": {
    "prompts": {
      "prompt_hash_1": { /* PromptRecord JSON */ },
      "prompt_hash_2": { /* PromptRecord JSON */ }
    },
    "files": {
      "path/to/file.ts": {
        "annotations": {
          "prompt_hash_1": [1, [5, 10], 20],
          "prompt_hash_2": [[3, 8]]
        },
        "diff": "git diff output string",
        "base_content": "original file content before changes"
      }
    }
  }
}
```

**字段说明**：

| 字段 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `title` | String | 是 | Bundle 标题 |
| `data` | BundleData | 是 | Bundle 数据 |
| `data.prompts` | HashMap\<String, PromptRecord\> | 是 | 至少包含一个 Prompt 记录 |
| `data.files` | HashMap\<String, ApiFileRecord\> | 否 | 文件 diff 和归属注释 |
| `files.*.annotations` | HashMap\<String, Vec\<Value\>\> | — | Prompt hash → 行号/行范围映射。单行：`5`，范围：`[1, 5]` |
| `files.*.diff` | String | — | Git diff 输出 |
| `files.*.base_content` | String | — | 变更前的原始文件内容 |

**CreateBundleResponse 成功响应**：

```json
{
  "success": true,
  "id": "bundle-uuid",
  "url": "https://example.com/bundle/bundle-uuid"
}
```

**服务端实现要求**：
- 必须验证用户登录状态（401 未认证返回）
- 生成唯一 Bundle ID 和分享 URL
- 存储关联的 Prompt 记录和文件 diff/注释数据
- 提供公开访问的分享页面

---

### 2.5 版本更新端点

#### GET `/worker/releases` — 获取发布版本信息

| 项目 | 说明 |
|------|------|
| 认证 | 可能携带 `Authorization: Bearer` 或 `X-API-Key`（通过 `ApiContext::new()` 构造，自动附加凭据） |
| 请求头 | `User-Agent: git-ai/{version}`, `X-Distinct-ID: {id}` |
| 响应 | `ReleasesResponse` JSON |

```json
{
  "channels": {
    "latest": {
      "version": "1.3.2",
      "checksum": "sha256-hash-of-sha256sums-file"
    },
    "next": {
      "version": "1.4.0-beta.1",
      "checksum": "sha256-hash"
    },
    "enterprise-latest": {
      "version": "1.3.2",
      "checksum": "sha256-hash"
    },
    "enterprise-next": {
      "version": "1.4.0-beta.1",
      "checksum": "sha256-hash"
    }
  }
}
```

#### GET `/worker/releases/{channel}/download/SHA256SUMS` — 下载校验和文件

| 项目 | 说明 |
|------|------|
| 认证 | 无（直接 HTTP GET） |
| 响应 | 纯文本，格式：`<sha256-hash>  <filename>` |

#### GET `/worker/releases/{channel}/download/install.sh` — 下载 Linux/macOS 安装脚本

| 项目 | 说明 |
|------|------|
| 认证 | 无 |
| 响应 | Shell 脚本文件 |

#### GET `/worker/releases/{channel}/download/install.ps1` — 下载 Windows 安装脚本

| 项目 | 说明 |
|------|------|
| 认证 | 无 |
| 响应 | PowerShell 脚本文件 |

#### GET `/worker/releases/{channel}/download/{filename}` — 下载二进制文件

| 项目 | 说明 |
|------|------|
| 认证 | 无 |
| 响应 | 二进制文件（git-ai 可执行文件） |

**支持的文件名示例**：
- `git-ai-aarch64-apple-darwin` — macOS ARM64
- `git-ai-x86_64-apple-darwin` — macOS x86_64
- `git-ai-x86_64-unknown-linux-gnu` — Linux x86_64
- `git-ai-x86_64-pc-windows-msvc.exe` — Windows x86_64

> **注意**：安装脚本 (`install.sh` / `install.ps1`) 会从同一 releases 端点下载对应平台的二进制文件，服务端需要托管所有平台架构的二进制文件。

**服务端实现要求**：
- 支持 4 个频道：`latest`、`next`、`enterprise-latest`、`enterprise-next`
- `checksum` 字段为 SHA256SUMS 文件的 SHA256 哈希
- 客户端会验证 SHA256SUMS 文件的完整性（先下载 SHA256SUMS，验证其 hash 与 `checksum` 匹配，再验证安装脚本的 hash 与 SHA256SUMS 中的条目匹配）
- 安装脚本和二进制文件的下载 URL 必须在同一 base URL 下

---

### 2.6 Report 报告端点

#### POST `/api/v1/reports` — 上传报告数据

| 项目 | 说明 |
|------|------|
| 认证 | `Authorization: Bearer {token}` 或 `X-API-Key: {key}` |
| 请求体 | `ReportDocument` JSON |

**ReportDocument 结构**：

```json
{
  "schema_version": "git-ai-report/1.0.0",
  "generated_at": "ISO8601 datetime",
  "tool_version": "1.3.2",
  "repo": {
    "workdir": "optional path",
    "remote_url_hash": "optional hash",
    "branch": "optional branch",
    "head_commit": "optional sha"
  },
  "range": {
    "mode": "head | range | branch | date",
    "from": "optional",
    "to": "optional",
    "since": "optional",
    "until": "optional",
    "commit_count": 10,
    "commits_with_authorship": 8,
    "commits_without_authorship": 2
  },
  "summary": {
    "git_diff_added_lines": 150,
    "git_diff_deleted_lines": 30,
    "ai_additions": 100,
    "human_additions": 50,
    "mixed_additions": 10,
    "unknown_additions": 5,
    "ai_accepted": 80,
    "total_ai_additions": 120,
    "total_ai_deletions": 10,
    "time_waiting_for_ai": 3600
  },
  "ratios": {
    "ai": 0.625,
    "human": 0.3125,
    "mixed": 0.0625,
    "unknown": 0.03125
  },
  "tool_model_breakdown": {
    "claude-code::claude-3.5": { ... },
    "cursor::gpt-4": { ... }
  },
  "commits": [ ... ]
}
```

**成功响应**：

```json
{
  "project_id": 1,
  "upload_id": 2,
  "inserted_commits": 8,
  "duplicate_commits": 2
}
```

> `project_id` 和 `upload_id` 由服务端分配，客户端当前未使用但会解析响应为 JSON。服务端应返回包含这些字段的完整 `IngestResponse`。

#### POST `/api/v1/summaries` — 上传摘要数据

| 项目 | 说明 |
|------|------|
| 认证 | 无（直接 HTTP POST，不携带 Bearer/API-Key） |
| 请求头 | `User-Agent: git-ai/{version}`, `Content-Type: application/json` |
| 请求体 | `ProjectSummaryReport` JSON（详见 2.12 节完整结构） |

---

### 2.7 Sentry 遥测端点

#### POST `https://{sentry_host}/api/{project_id}/store/` — Sentry 事件上报

| 项目 | 说明 |
|------|------|
| 协议 | Sentry Envelope 协议 |
| 认证 | 无（DSN 内嵌认证信息） |
| 数据 | 错误事件、性能事件 |

**服务端实现要求**：
- 客户端支持双 Sentry DSN 配置：
  - `SENTRY_OSS`：OSS 遥测 DSN（编译时嵌入）
  - `SENTRY_ENTERPRISE`：企业遥测 DSN（配置文件 `telemetry_enterprise_dsn` 或环境变量）
- 企业版会同时发送到 OSS DSN 和企业 DSN
- 完整 Sentry 协议实现，或直接使用开源 Sentry（如 getsentry/self-hosted）

---

### 2.8 PostHog 分析端点

#### POST `https://{posthog_host}/capture/` — PostHog 事件捕获

| 项目 | 说明 |
|------|------|
| 认证 | PostHog API Key |
| 数据 | 用户行为分析事件 |

**服务端实现要求**：
- 客户端编译时嵌入 `POSTHOG_API_KEY` 和 `POSTHOG_HOST`
- 默认 PostHog Host: `https://us.i.posthog.com`
- 可自部署 PostHog（[posthog/posthog](https://github.com/PostHog/posthog)）

---

### 2.9 Git Authorship Notes 同步

> **注意**：这不是 HTTP API 端点，而是通过 Git 协议实现的 Authorship Notes 自动同步机制，企业部署时需特别注意。

客户端在 Git 操作的 post-hooks 中自动同步 `refs/notes/ai` 引用：

#### Fetch（拉取 Notes）

在 `git fetch`、`git pull`、`git clone` 的 post-hook 中自动执行：

```bash
git fetch {remote} +refs/notes/ai:refs/notes/ai-remotes/{remote}
# 然后将远程 notes 合并到本地 refs/notes/ai（使用 -s ours 策略）
```

**安全机制**：
- 使用 tracking ref（`refs/notes/ai-remotes/{remote}`）隔离不同远程的 notes
- 合并策略为 `ours`，本地 notes 永不被远程覆盖
- 如果 `git notes merge` 崩溃（混合 fanout 问题），有 fallback 手动合并逻辑
- 如果远程无 `refs/notes/ai` 引用，优雅跳过

#### Push（推送 Notes）

在 `git push` 的 post-hook 中自动执行：

```bash
git push {remote} refs/notes/ai:refs/notes/ai
# 不使用 --force，要求快进合并
```

**重试机制**：
- 非快进错误时最多重试 3 次（`PUSH_NOTES_MAX_ATTEMPTS = 3`）
- 每次重试先 fetch + merge 远程 notes，再 push
- 适用于繁忙 monorepo 中并发推送场景

#### 企业部署建议

1. **Git 服务器端**：确保 Git 服务器（如 GitLab、Gitea、Gogs）允许 `refs/notes/*` 命名空间的 push
2. **权限配置**：部分 Git 服务器默认限制 notes push，需显式允许
3. **代理/网关**：如果企业使用 Git SSH/HTTPS 代理，确保 `refs/notes/ai` 的 fetch/push 不被过滤
4. **CI/CD 集成**：CI runner 需配置 `core.hooksPath` 或安装 git-ai 以触发 notes 同步

---

### 2.10 Feature Flags 远程管理

客户端支持以下 Feature Flags（源码 `src/feature_flags.rs`）：

| Flag | 配置名 | Debug 默认 | Release 默认 | 说明 |
|------|--------|-----------|-------------|------|
| `rewrite_stash` | `rewrite_stash` | true | true | Stash 操作时重写 authorship notes |
| `inter_commit_move` | `checkpoint_inter_commit_move` | false | false | 跨提交归属移动 |
| `auth_keyring` | `auth_keyring` | false | false | 使用系统密钥链存储凭据 |
| `async_mode` | `async_mode` | false | true | 异步守护进程模式 |
| `git_hooks_enabled` | `git_hooks_enabled` | false | false | Git hooks 模式（已废弃，自动迁移到 async_mode） |
| `git_hooks_externally_managed` | `git_hooks_externally_managed` | false | false | Git hooks 由外部管理 |

**优先级**：环境变量 (`GIT_AI_*` 前缀) > 配置文件 > 编译时默认值

**企业服务端建议实现**：

```
GET /worker/config/feature-flags
```

```json
{
  "rewrite_stash": true,
  "checkpoint_inter_commit_move": false,
  "auth_keyring": true,
  "async_mode": true,
  "git_hooks_enabled": false,
  "git_hooks_externally_managed": false
}
```

> 当前客户端不主动拉取远程 feature flags，但可通过配置管理 API + 安装脚本注入实现。未来版本客户端可能支持远程 flags 拉取。

---

### 2.11 JetBrains 插件下载代理

客户端内置 JetBrains 插件安装功能（`src/mdm/jetbrains/download.rs`），默认直连 JetBrains Marketplace：

```
https://plugins.jetbrains.com/pluginManager?action=download&id={plugin_id}&build={product_code}-{build_number}
```

**企业部署建议**：提供代理端点，使内网用户无需直连外网：

```
GET /worker/plugins/jetbrains/download?id={plugin_id}&build={product_code}-{build_number}
```

**实现方式**：
- 服务端作为反向代理，从 JetBrains Marketplace 拉取插件 ZIP 后转发
- 可预缓存常用 IDE 版本的插件
- 客户端当前硬编码了 JetBrains Marketplace URL，需要修改客户端代码或通过 HTTP 代理实现

---

### 2.12 Summary（摘要）上传详细结构

#### POST `/api/v1/summaries` — 上传项目摘要数据

| 项目 | 说明 |
|------|------|
| 认证 | 无（直接 HTTP POST，不携带 Bearer/API-Key） |
| 请求头 | `User-Agent: git-ai/{version}`, `Content-Type: application/json` |
| 请求体 | `ProjectSummaryReport` JSON |

**ProjectSummaryReport 结构**：

```json
{
  "project_name": "my-project",
  "git_url": "https://github.com/org/repo (可选)",
  "branch": "main (可选)",
  "total_commits": 50,
  "developers": [
    {
      "name": "Developer Name",
      "email": "dev@example.com",
      "commits": 25,
      "added_lines": 1000,
      "ai_additions": 600,
      "human_additions": 400,
      "ai_ratio": 0.6,
      "human_ratio": 0.4
    }
  ],
  "project_ratios": {
    "ai": 0.6,
    "human": 0.4
  },
  "organization": "ACME Corp (可选)",
  "department": "Engineering (可选)",
  "reporter_name": "Manager Name (可选)",
  "reporter_email": "mgr@example.com (可选)",
  "report_period": "2026-Q2 (可选)"
}
```

> **重要**：Summary 上传**不携带认证信息**（通过 `crate::http::build_agent` 直接构建 HTTP 请求），与 Report 上传的行为一致。服务端如果需要认证，需要在 URL 路径中嵌入 token 或接受无认证上传。

---

## 三、用户体系设计

### 3.1 用户模型

```sql
CREATE TABLE users (
    id TEXT PRIMARY KEY,              -- UUID，对应 JWT sub
    email TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    personal_org_id TEXT,             -- 个人组织 ID
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE organizations (
    id TEXT PRIMARY KEY,              -- UUID
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,        -- URL 友好标识
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE departments (
    id TEXT PRIMARY KEY,              -- UUID
    org_id TEXT NOT NULL REFERENCES organizations(id),
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,        -- URL 友好标识
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE org_members (
    user_id TEXT NOT NULL REFERENCES users(id),
    org_id TEXT NOT NULL REFERENCES organizations(id),
    department_id TEXT REFERENCES departments(id),
    role TEXT NOT NULL DEFAULT 'member',  -- owner, admin, member
    PRIMARY KEY (user_id, org_id)
);

CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    key_hash TEXT UNIQUE NOT NULL,     -- API Key 的哈希值
    name TEXT,                         -- 密钥名称/描述
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP,
    last_used_at TIMESTAMP
);

CREATE TABLE oauth_devices (
    device_code TEXT PRIMARY KEY,
    user_code TEXT UNIQUE NOT NULL,
    verification_uri TEXT NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    interval_seconds INTEGER DEFAULT 5,
    user_id TEXT,                      -- 授权后填入
    authorized_at TIMESTAMP,
    client_id TEXT DEFAULT 'git-ai-cli'
);

CREATE TABLE refresh_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    token_hash TEXT UNIQUE NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    revoked_at TIMESTAMP
);

CREATE TABLE install_nonces (
    nonce TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    used BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    used_at TIMESTAMP
);
```

### 3.2 JWT Access Token 规范

```json
{
  "sub": "user-uuid",
  "email": "user@example.com",
  "name": "User Name",
  "personal_org_id": "org-uuid",
  "orgs": [
    {
      "org_id": "org-uuid",
      "org_name": "Organization Name",
      "org_slug": "org-slug",
      "role": "owner"
    }
  ],
  "iat": 1700000000,
  "exp": 1700003600
}
```

- 算法：RS256 或 HS256
- 有效期：3600 秒（1小时）
- 客户端通过 Base64 解码 JWT Payload 提取用户身份（不做签名验证）

### 3.3 OAuth Device Authorization Flow 实现

```
1. 客户端 → POST /worker/oauth/device/code → 服务端
   服务端生成 device_code + user_code，返回验证 URL

2. 客户端打开浏览器 → 用户在验证页面输入 user_code → 授权

3. 客户端轮询 → POST /worker/oauth/token (grant_type=device_code)
   - 未授权 → 返回 {"error": "authorization_pending"}
   - 已授权 → 返回 access_token + refresh_token

4. Token 过期时：
   - access_token 过期 → POST /worker/oauth/token (grant_type=refresh_token)
   - refresh_token 过期 → 需重新登录
```

### 3.4 API Key 认证

- 请求头：`X-API-Key: {api_key}`
- 附加请求头：`X-Author-Identity: {git_committer_identity}`（通过 `git var GIT_COMMITTER_IDENT` 获取）
- 适用于 CI/CD 等非交互场景
- 与 OAuth 认证互为替代

---

## 四、数据存储设计

### 4.1 Metrics 数据库

```sql
CREATE TABLE metrics_events (
    id BIGSERIAL PRIMARY KEY,
    event_type SMALLINT NOT NULL,         -- 0=InstallHooks, 1=Committed, 2=AgentUsage, 3=Checkpoint
    timestamp BIGINT NOT NULL,            -- Unix 时间戳（秒）
    user_id TEXT,                          -- 关联用户（从 token 或 API key 解析）
    distinct_id TEXT,                      -- 客户端唯一标识（X-Distinct-ID）
    org_id TEXT,                           -- 关联组织
    repo_url TEXT,
    author_email TEXT,
    tool TEXT,
    model TEXT,
    commit_sha TEXT,
    human_additions INTEGER DEFAULT 0,
    ai_additions INTEGER DEFAULT 0,
    mixed_additions INTEGER DEFAULT 0,
    unknown_additions INTEGER DEFAULT 0,
    ai_accepted INTEGER DEFAULT 0,
    git_diff_added_lines INTEGER DEFAULT 0,
    git_diff_deleted_lines INTEGER DEFAULT 0,
    tool_model_pairs JSONB,               -- 工具/模型对数组
    ai_additions_by_tool JSONB,           -- 各工具 AI 行数
    prompt_id TEXT,                        -- AgentUsage/Checkpoint 事件关联的 Prompt ID
    session_id TEXT,                       -- AI 会话 ID
    file_path TEXT,                        -- Checkpoint 事件关联的文件路径
    custom_attributes JSONB,              -- 自定义属性
    raw_values JSONB,                      -- 原始编码值（PosEncoded）
    raw_attrs JSONB,                       -- 原始属性值
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_metrics_user_id ON metrics_events(user_id);
CREATE INDEX idx_metrics_org_id ON metrics_events(org_id);
CREATE INDEX idx_metrics_repo_url ON metrics_events(repo_url);
CREATE INDEX idx_metrics_author ON metrics_events(author_email);
CREATE INDEX idx_metrics_tool ON metrics_events(tool);
CREATE INDEX idx_metrics_timestamp ON metrics_events(timestamp);
CREATE INDEX idx_metrics_commit_sha ON metrics_events(commit_sha);
CREATE INDEX idx_metrics_event_type ON metrics_events(event_type);
```

### 4.2 CAS 数据库

```sql
CREATE TABLE cas_objects (
    hash TEXT PRIMARY KEY,                -- SHA256 哈希
    content JSONB NOT NULL,               -- Prompt 记录内容
    metadata JSONB,                       -- 可选元数据
    author_identity TEXT,                 -- X-Author-Identity
    user_id TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE cas_ownership (
    hash TEXT NOT NULL REFERENCES cas_objects(hash),
    user_id TEXT NOT NULL,
    org_id TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (hash, user_id)
);
```

### 4.3 Reports 数据库

```sql
CREATE TABLE projects (
    id BIGSERIAL PRIMARY KEY,
    remote_url_hash TEXT UNIQUE NOT NULL,  -- 仓库 URL 的哈希
    branch TEXT,
    head_commit TEXT,
    organization TEXT,                     -- 所属组织
    department TEXT,                       -- 所属部门
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE report_uploads (
    id BIGSERIAL PRIMARY KEY,             -- 即 upload_id
    project_id BIGINT NOT NULL REFERENCES projects(id),
    schema_version TEXT NOT NULL,
    generated_at TEXT NOT NULL,
    commit_count INTEGER NOT NULL,
    uploaded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE commit_stats (
    project_id BIGINT NOT NULL REFERENCES projects(id),
    sha TEXT NOT NULL,
    author TEXT NOT NULL,
    author_time TEXT NOT NULL,
    subject TEXT NOT NULL,
    has_authorship_note BOOLEAN NOT NULL,
    git_diff_added_lines INTEGER NOT NULL,
    git_diff_deleted_lines INTEGER NOT NULL,
    ai_additions INTEGER NOT NULL,
    human_additions INTEGER NOT NULL,
    mixed_additions INTEGER NOT NULL,
    unknown_additions INTEGER NOT NULL,
    ai_accepted INTEGER NOT NULL,
    total_ai_additions INTEGER NOT NULL,
    total_ai_deletions INTEGER NOT NULL,
    time_waiting_for_ai INTEGER NOT NULL,
    PRIMARY KEY (project_id, sha)
);

CREATE TABLE tool_model_stats (
    project_id BIGINT NOT NULL REFERENCES projects(id),
    tool_model TEXT NOT NULL,
    ai_additions INTEGER NOT NULL,
    mixed_additions INTEGER NOT NULL,
    ai_accepted INTEGER NOT NULL,
    total_ai_additions INTEGER NOT NULL,
    total_ai_deletions INTEGER NOT NULL,
    time_waiting_for_ai INTEGER NOT NULL,
    PRIMARY KEY (project_id, tool_model)
);

-- Summaries (ProjectSummaryReport)
CREATE TABLE summary_uploads (
    id BIGSERIAL PRIMARY KEY,
    project_name TEXT NOT NULL,
    git_url TEXT,
    branch TEXT,
    total_commits INTEGER NOT NULL,
    organization TEXT,
    department TEXT,
    reporter_name TEXT,
    reporter_email TEXT,
    report_period TEXT,
    project_ratios JSONB NOT NULL,
    developers JSONB NOT NULL,
    uploaded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_projects_org ON projects(organization);
CREATE INDEX idx_projects_dept ON projects(department);
CREATE INDEX idx_commit_stats_author ON commit_stats(author);
CREATE INDEX idx_summary_uploads_org ON summary_uploads(organization);
```

### 4.4 Bundles 数据库

```sql
CREATE TABLE bundles (
    id TEXT PRIMARY KEY,                  -- Bundle UUID
    user_id TEXT NOT NULL,
    title TEXT NOT NULL,
    data JSONB NOT NULL,                  -- BundleData: { prompts, files }
    share_url TEXT UNIQUE NOT NULL,       -- 分享链接
    view_count INTEGER DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP
);

CREATE TABLE bundle_prompts (
    bundle_id TEXT NOT NULL REFERENCES bundles(id),
    hash TEXT NOT NULL,                    -- Prompt hash (key in prompts map)
    content JSONB NOT NULL,               -- PromptRecord JSON
    PRIMARY KEY (bundle_id, hash)
);

CREATE TABLE bundle_files (
    bundle_id TEXT NOT NULL REFERENCES bundles(id),
    file_path TEXT NOT NULL,              -- 文件路径 (key in files map)
    annotations JSONB,                    -- prompt_hash → 行号映射
    diff TEXT,                            -- Git diff
    base_content TEXT,                    -- 原始文件内容
    PRIMARY KEY (bundle_id, file_path)
);

CREATE INDEX idx_bundles_user_id ON bundles(user_id);
CREATE INDEX idx_bundles_share_url ON bundles(share_url);
```

---

## 五、统计看板设计

### 5.1 看板页面

客户端已有的 Report Server（`src/report/server.rs`）实现了完整的本地看板，包含以下页面和 API：

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/v1/reports` | POST | 接收报告数据 |
| `/api/v1/summaries` | POST | 接收摘要数据 |
| `/api/v1/projects` | GET | 项目列表 |
| `/api/v1/summaries` | GET | 摘要列表 |
| `/api/v1/aggregate/summary` | GET | 全局汇总统计 |
| `/api/v1/aggregate/organizations?limit={n}&cursor={cursor}` | GET | 按组织聚合 |
| `/api/v1/aggregate/departments?org={name}&limit={n}&cursor={cursor}` | GET | 按部门聚合 |
| `/api/v1/aggregate/projects?limit={n}&cursor={cursor}` | GET | 按项目聚合 |
| `/api/v1/aggregate/developers?limit={n}&cursor={cursor}` | GET | 按开发者聚合 |
| `/api/v1/projects/{id}/summary` | GET | 单项目摘要 |
| `/api/v1/projects/{id}/commits` | GET | 单项目提交列表 |
| `/api/v1/summaries/{id}` | GET | 单条摘要详情 |

已分页的 Enterprise 列表接口统一使用 `limit` 和不透明 `cursor` 参数。客户端应从响应的 `pagination.next_cursor` 请求下一页，不应解析或长期保存 cursor；旧客户端可以继续读取原有顶层列表字段并忽略新增的 `pagination`。

### 5.2 核心统计指标

#### 全局汇总

```json
{
  "total_commits": 1000,
  "total_ai_lines": 15000,
  "total_human_lines": 5000,
  "pct_ai_lines": 75.0,
  "total_developers": 25,
  "total_projects": 10,
  "total_organizations": 3
}
```

#### 组织级统计

```json
{
  "organization": "ACME Corp",
  "total_commits": 500,
  "w_ai": 12000,         // AI 加权行数
  "w_human": 3000,       // 人工加权行数
  "pct_ai": 80.0,
  "departments": [ ... ],
  "projects": [ ... ]
}
```

#### 项目级统计

```json
{
  "project_name": "my-project",
  "git_url": "https://github.com/org/repo",
  "branch": "main",
  "organization": "ACME",
  "department": "Engineering",
  "total_commits": 200,
  "total_ai": 5000,
  "total_human": 1000,
  "pct_ai": 83.3,
  "tool_breakdown": { ... }
}
```

#### 开发者级统计

```json
{
  "name": "Developer Name",
  "email": "dev@example.com",
  "organization": "ACME",
  "department": "Engineering",
  "total_commits": 50,
  "total_added_lines": 2000,
  "ai_added_lines": 1500,
  "human_added_lines": 500,
  "pct_ai": 75.0,
  "tool_usage": { ... }
}
```

### 5.3 Dashboard URL 生成

客户端通过 `git-ai dashboard` 命令（实际命令名为 `personal-dashboard`）生成 Dashboard URL：

- **端点逻辑**：`{api_base_url}/me`
- 需要用户已登录
- 从 JWT 中提取 `personal_org_id` 和 `orgs` 信息
- 客户端自动使用系统默认浏览器打开该 URL

> **注意**：Dashboard URL 路径为 `/me`（不是 `/dashboard`），源码 `src/commands/personal_dashboard.rs:9` 中 `format!("{}/me", api_base_url)`

---

## 六、企业配置管理

### 6.1 客户端配置文件

路径：`~/.git-ai/config.json`

| 字段 | 类型 | 说明 | 企业相关 |
|------|------|------|----------|
| `api_base_url` | String | API 基础 URL | **是** — 替换为企业服务端地址 |
| `api_key` | String | API 密钥 | **是** — 企业 API Key |
| `telemetry_enterprise_dsn` | String | 企业 Sentry DSN | **是** — 企业错误追踪 |
| `telemetry_oss` | String | OSS 遥测开关 | 否 |
| `disable_version_checks` | Boolean | 禁用版本检查 | 否 |
| `disable_auto_updates` | Boolean | 禁用自动更新 | 否 |
| `update_channel` | String | 更新频道 | **是** — `enterprise-latest` / `enterprise-next` |
| `allow_repositories` | Array | 仓库白名单 | **是** — 限制上报范围 |
| `exclude_repositories` | Array | 仓库黑名单 | **是** — 排除敏感仓库 |
| `exclude_prompts_in_repositories` | Array | 排除 Prompt 上报的仓库 | **是** — 保护代码隐私 |
| `include_prompts_in_repositories` | Array | 包含 Prompt 上报的仓库 | **是** |
| `prompt_storage` | String | Prompt 存储模式 | **是** — `default`/`notes`/`local` |
| `default_prompt_storage` | String | 默认 Prompt 存储模式 | **是** |
| `feature_flags` | Object | 功能标志 | 否 |
| `custom_attributes` | Object | 自定义属性 | **是** — 可附加组织/部门信息 |
| `git_ai_hooks` | Object | 自定义 Hook 脚本 | 否 |
| `quiet` | Boolean | 静默模式 | 否 |

### 6.2 环境变量覆盖

> **优先级**：环境变量 > 配置文件 > 编译时默认值（通过 `envy` crate 以 `GIT_AI_` 前缀加载）

| 环境变量 | 说明 | 对应配置字段 |
|---------|------|------------|
| `GIT_AI_API_BASE_URL` | 覆盖 API 基础 URL | `api_base_url` |
| `GIT_AI_API_KEY` | 覆盖 API 密钥 | `api_key` |
| `GIT_AI_TELEMETRY_ENTERPRISE_DSN` | 覆盖企业 Sentry DSN | `telemetry_enterprise_dsn` |
| `GIT_AI_UPDATE_CHANNEL` | 覆盖更新频道 | `update_channel` |
| `GIT_AI_AUTH_KEYRING` | 使用系统密钥链 | `auth_keyring` (feature flag) |
| `GIT_AI_REWRITE_STASH` | Stash 重写开关 | `rewrite_stash` (feature flag) |
| `GIT_AI_CHECKPOINT_INTER_COMMIT_MOVE` | 跨提交归属移动 | `inter_commit_move` (feature flag) |
| `GIT_AI_ASYNC_MODE` | 异步守护进程模式 | `async_mode` (feature flag) |
| `GIT_AI_GIT_HOOKS_ENABLED` | Git hooks 模式（已废弃） | `git_hooks_enabled` (feature flag) |
| `GIT_AI_GIT_HOOKS_EXTERNALLY_MANAGED` | 外部管理 hooks | `git_hooks_externally_managed` (feature flag) |
| `GIT_AI_DEBUG` | 调试输出 (`0`/`1`) | — |
| `GIT_AI_DEBUG_PERFORMANCE` | 性能计时 (`1`=文本, `2`=JSON) | — |
| `INSTALL_NONCE` | 安装 Nonce（自动登录） | — |
| `API_BASE` | 安装脚本中指定 API 地址 | — |
| `SENTRY_OSS` | OSS Sentry DSN（编译时嵌入） | — |
| `SENTRY_ENTERPRISE` | 企业 Sentry DSN（编译时嵌入） | — |
| `POSTHOG_API_KEY` | PostHog API Key（编译时嵌入） | — |
| `POSTHOG_HOST` | PostHog Host（编译时嵌入） | — |
| `OSS_BUILD` | 控制 auto-update 默认行为 | — |

### 6.3 服务端配置管理 API（建议新增）

虽然客户端当前不支持远程配置推送，但服务端可以设计配置管理 API，用于：

- 批量下发企业配置（通过安装脚本注入）
- 管理 API Key 和权限
- 管理仓库白/黑名单
- 管理功能标志

---

## 七、企业级增强功能规划

> 以下功能基于 usegitai.com 企业版公开信息和行业最佳实践规划，属于企业服务端应提供但当前客户端可能尚未完整支持的增强功能。

### 7.1 PR 级别聚合 API

当前文档的聚合维度仅覆盖组织/部门/项目/开发者，缺少 Pull Request 维度的统计。

**建议新增端点**：

```
GET /api/v1/aggregate/pull-requests?org={org_slug}&repo={repo_url}&since={date}&until={date}&limit={n}&cursor={cursor}
```

**响应结构**：

```json
{
  "pull_requests": [
    {
      "pr_id": "123",
      "pr_url": "https://github.com/org/repo/pull/123",
      "title": "Add feature X",
      "author": "developer@example.com",
      "merged_at": "2026-05-15T10:00:00Z",
      "total_lines": 500,
      "ai_lines": 350,
      "human_lines": 150,
      "pct_ai": 70.0,
      "tools_used": ["claude-code::claude-3.5", "cursor::gpt-4"],
      "files_changed": 12,
      "ai_files": 8
    }
  ],
  "summary": {
    "total_prs": 50,
    "avg_pct_ai": 65.0,
    "pr_size_distribution": { "small": 20, "medium": 20, "large": 10 }
  },
  "pagination": {
    "limit": 25,
    "has_more": true,
    "next_cursor": "opaque-cursor"
  }
}
```

**实现方式**：
- 从 Git commit messages 或 CI webhook 事件中提取 PR 关联信息
- 聚合 PR 关联的提交中的 AI/human 行数统计
- 支持按仓库、时间范围过滤

### 7.2 代码持久性追踪

追踪 AI 生成的代码在后续提交中是否被保留、修改或删除，评估 AI 代码的长期价值。

**建议新增端点**：

```
GET /api/v1/ai-code-persistence?org={org_slug}&repo={repo_url}&since={date}&until={date}
```

**响应结构**：

```json
{
  "period": { "since": "2026-04-01", "until": "2026-06-01" },
  "ai_code_snapshot": {
    "total_ai_lines_introduced": 5000,
    "lines_still_present": 3500,
    "lines_modified": 800,
    "lines_deleted": 700,
    "survival_rate": 70.0
  },
  "by_tool": {
    "claude-code::claude-3.5": {
      "introduced": 3000,
      "survival_rate": 75.0
    },
    "cursor::gpt-4": {
      "introduced": 2000,
      "survival_rate": 62.5
    }
  },
  "trend": [
    { "week": "2026-W18", "survival_rate": 72.0 },
    { "week": "2026-W19", "survival_rate": 70.0 }
  ]
}
```

**实现方式**：
- 基于 Authorship Notes 中的 line_ranges attestation，在后续提交中追踪归属变化
- 利用 git blame 和 authorship note 的对比分析
- 定期快照计算存活率

### 7.3 Agent 就绪度评估

衡量 AI coding agent 的 skills、rules、MCPs、AGENTS.md 配置变更对代码质量的影响。

**建议新增端点**：

```
GET /api/v1/agent-readiness?org={org_slug}
```

**响应结构**：

```json
{
  "agents": [
    {
      "tool": "claude-code",
      "model": "claude-3.5-sonnet",
      "config_changes": [
        {
          "type": "AGENTS.md",
          "changed_at": "2026-05-10",
          "ai_acceptance_before": 0.65,
          "ai_acceptance_after": 0.78
        },
        {
          "type": "mcp_config",
          "changed_at": "2026-05-12",
          "ai_acceptance_before": 0.78,
          "ai_acceptance_after": 0.82
        }
      ],
      "overall_score": 82,
      "trend": "improving"
    }
  ]
}
```

### 7.4 CI/CD 集成与全生命周期追踪

追踪 AI 代码从编写→提交→审查→部署→线上告警的完整生命周期。

**建议新增端点**：

```
POST /api/v1/ci-events — 接收 CI/CD 事件
```

```json
{
  "event_type": "deployment",
  "timestamp": "2026-05-15T12:00:00Z",
  "org_slug": "acme",
  "repo_url": "https://github.com/acme/api",
  "commit_sha": "abc123",
  "deployment_env": "production",
  "status": "success",
  "deployer": "ci-bot@example.com"
}
```

```
POST /api/v1/alert-events — 接收告警事件
```

```json
{
  "event_type": "alert",
  "alert_source": "pagerduty",
  "timestamp": "2026-05-15T14:30:00Z",
  "org_slug": "acme",
  "repo_url": "https://github.com/acme/api",
  "commit_sha": "abc123",
  "severity": "critical",
  "description": "API error rate exceeded threshold"
}
```

**关联分析**：

```
GET /api/v1/ai-code-lifecycle?org={org_slug}&commit_sha={sha}
```

```json
{
  "commit_sha": "abc123",
  "author": "developer@example.com",
  "ai_lines": 350,
  "lifecycle": [
    { "stage": "written", "timestamp": "2026-05-14T10:00:00Z", "detail": "claude-code session" },
    { "stage": "committed", "timestamp": "2026-05-14T11:00:00Z" },
    { "stage": "pr_reviewed", "timestamp": "2026-05-14T15:00:00Z", "reviewer": "senior@example.com" },
    { "stage": "deployed", "timestamp": "2026-05-15T12:00:00Z", "env": "production" },
    { "stage": "alert", "timestamp": "2026-05-15T14:30:00Z", "severity": "critical" }
  ],
  "ai_code_involved_in_alert": true,
  "truncated": false,
  "truncation": {
    "limit_per_event_type": 100,
    "ci_events": false,
    "alert_events": false
  }
}
```

### 7.5 Prompt 访问控制

CAS 存储的 AI Prompt 记录可能包含敏感信息（代码上下文、业务逻辑），企业服务端需要实现访问控制。

**建议实现**：

1. **组织级隔离**：Prompt 只能被同一组织的成员读取
2. **PII 过滤增强**：虽然客户端已做 `secrets.rs` 中的密钥脱敏，服务端应做二次验证
3. **访问审计**：记录每次 CAS 读取的访问者、时间、目的
4. **数据保留策略**：支持按组织配置 Prompt 数据的保留期限
5. **选择性同步**：通过 `exclude_prompts_in_repositories` 和 `include_prompts_in_repositories` 控制哪些仓库的 Prompt 允许上传

### 7.6 高级仪表盘增强

在现有聚合 API 基础上增加以下维度：

| 增强功能 | 说明 | 对应端点 |
|---------|------|---------|
| 趋势分析 | AI 代码占比随时间的变化趋势 | `GET /api/v1/aggregate/trends?metric=ai_ratio&granularity=week` |
| 跨 Agent 对比 | 不同 AI 工具/模型的产出质量对比 | `GET /api/v1/aggregate/agent-comparison?org={slug}` |
| 跨团队对比 | 不同团队/部门的 AI 采用率对比 | `GET /api/v1/aggregate/team-comparison?org={slug}` |
| 代码审查关联 | PR review 中的 AI 代码修改率 | `GET /api/v1/aggregate/review-impact?org={slug}` |

---

## 八、客户端数据结构参考

### 8.1 StoredCredentials

```rust
pub struct StoredCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub access_token_expires_at: i64,   // Unix 时间戳
    pub refresh_token_expires_at: i64,  // Unix 时间戳
}
```

存储路径：
- Keyring 后端：系统密钥链，服务名 `"git-ai"`，用户名 `"oauth-tokens"`
- 文件后端：`~/.git-ai/internal/credentials`（Unix: 0o600 权限）

### 8.2 TokenIdentity（从 JWT 解析）

```rust
pub struct TokenIdentity {
    pub user_id: Option<String>,        // JWT sub
    pub email: Option<String>,
    pub name: Option<String>,
    pub personal_org_id: Option<String>,
    pub orgs: Vec<TokenOrg>,
}

pub struct TokenOrg {
    pub org_id: Option<String>,
    pub org_name: Option<String>,
    pub org_slug: Option<String>,
    pub role: Option<String>,
}
```

### 8.3 PromptRecord（CAS 存储内容）

```rust
pub struct PromptRecord {
    pub agent_id: AgentId,
    pub human_author: Option<String>,
    pub messages: Vec<AiTranscriptMessage>,
    pub total_additions: u32,
    pub total_deletions: u32,
}

pub struct AgentId {
    pub tool: String,     // e.g., "cursor", "claude-code"
    pub id: String,       // 会话 ID
    pub model: String,    // e.g., "claude-3.5-sonnet"
}
```

> **CAS 存储的两种形态**：
> - `CasObject`（上传用）：`{ content: PromptRecord, hash: String, metadata: HashMap<String, String> }`
> - `CasMessagesObject`（读取用）：`{ messages: Vec<Message> }` — 仅包含对话消息

### 8.4 Internal DB（客户端本地数据库）

路径：`~/.git-ai/internal/prompts.db`

| 表 | 说明 |
|----|------|
| `prompts` | AI Prompt 记录（id, tool, model, messages, commit_sha, ...） |
| `cas_sync_queue` | CAS 同步队列（hash, data, status, attempts, ...） |
| `cas_cache` | CAS 缓存（hash, messages, cached_at） |

### 8.5 Metrics DB（客户端本地数据库）

路径：`~/.git-ai/internal/metrics-db`

| 表 | 说明 |
|----|------|
| `metrics` | 待上传事件（id, event_json） |
| `agent_usage_throttle` | AgentUsage 节流（prompt_id, last_sent_ts） |

---

## 九、技术架构建议

### 9.1 推荐技术栈

| 组件 | 推荐方案 | 说明 |
|------|---------|------|
| Web 框架 | Axum / Actix-web | Rust 原生，高性能 |
| 数据库 | PostgreSQL | 主数据存储 |
| 缓存 | Redis | Session、速率限制 |
| 对象存储 | MinIO / S3 | CAS 大对象存储 |
| 遥测 | Self-hosted Sentry | 错误追踪 |
| 分析 | Self-hosted PostHog | 用户行为分析 |
| 看板前端 | React + TypeScript | 参考 `src/report/server.rs` 中的 HTML/JS |
| 认证 | 自实现 OAuth Device Flow | 轻量级，无外部依赖 |
| 容器化 | Docker + docker-compose | 参考 `Dockerfile.server-cn` |

### 9.2 服务架构

```
                    ┌─────────────────┐
                    │   Nginx/Caddy   │ ← TLS 终止、反向代理
                    └────────┬────────┘
                             │
         ┌───────────────────┼───────────────────┐
         │                   │                   │
┌────────▼──────┐  ┌────────▼──────┐  ┌────────▼──────┐
│  API Server   │  │  Dashboard UI │  │  Release CDN  │
│  (Rust/Go)    │  │  (React)      │  │  (二进制/脚本) │
│  /worker/*    │  │  /me          │  │  /worker/      │
│  /api/*       │  │               │  │  releases/*   │
└────────┬──────┘  └───────────────┘  └───────────────┘
         │
┌────────┼────────────────────────┐
│        │           │            │
│  ┌─────▼────┐ ┌───▼────┐ ┌─────▼──────┐
│  │  PG DB   │ │ Redis  │ │ MinIO/S3   │
│  │  (主存储) │ │ (缓存) │ │ (CAS Store)│
│  └──────────┘ └────────┘ └────────────┘
│
│  ┌──────────────────────────────────────┐
│  │  External Integrations               │
│  │  ┌─────────┐ ┌─────────┐ ┌────────┐ │
│  │  │ Sentry  │ │ PostHog │ │ JetBrains│ │
│  │  │(自部署) │ │(自部署) │ │Proxy   │ │
│  │  └─────────┘ └─────────┘ └────────┘ │
│  └──────────────────────────────────────┘
│
│  ┌──────────────────────────────────────┐
│  │  Git Server Integration              │
│  │  refs/notes/ai push/fetch 支持       │
│  │  (GitLab / Gitea / Gogs 等)          │
│  └──────────────────────────────────────┘
```

### 9.3 关键设计决策

1. **OAuth 简化**：不依赖外部 IdP，自实现 Device Authorization Flow。用户管理通过管理后台手动创建或 LDAP/SSO 集成。

2. **PosEncoded 解码**：Metrics 事件使用位置编码（字段名为数字索引），服务端需要实现完整的解码器。建议直接移植 `src/metrics/pos_encoded.rs` 和 `src/metrics/attrs.rs` 的 Rust 代码。

3. **CAS 双重功能**：CAS 存储既是去重上传目标，也是 IDE 插件读取 Prompt 内容的数据源。读取端点返回完整 content，不仅用于存在性检查。

4. **CAS 去重**：基于 SHA256 hash 去重，相同的 Prompt 内容只存一份。

5. **数据隔离**：多租户通过 `org_id` 实现数据隔离。API Key 绑定到特定用户和组织。CAS 读取需组织级访问控制。

6. **密钥安全**：
   - API Key 存储哈希值而非原文
   - Access Token 为 JWT，无状态验证
   - Refresh Token 存储哈希值
   - CAS hash 值需验证仅含十六进制字符（防注入）

7. **速率限制**：
   - Metrics 上传：每客户端每分钟不超过 60 请求
   - CAS 上传：每客户端每分钟不超过 30 请求
   - CAS 读取：每客户端每分钟不超过 100 请求（IDE 插件频繁调用）
   - OAuth 轮询：按 RFC 8628 `interval` 控制

8. **Git Notes 同步**：服务端不直接处理 notes，但需确保 Git 服务器配置允许 `refs/notes/ai` 命名空间的 push/fetch。提供配置指南和验证工具。

9. **Feature Flags 管理**：当前通过配置文件和环境变量控制，未来可扩展为远程拉取模式。设计 API 端点预留。

---

## 十、部署方案

### 10.1 Docker Compose 部署

参考客户端已有的 `docker-compose.yml` 和 `Dockerfile.server-cn`：

```yaml
services:
  api:
    build: .
    ports:
      - "8080:8080"
    environment:
      - DATABASE_URL=postgresql://...
      - REDIS_URL=redis://redis:6379
      - S3_ENDPOINT=http://minio:9000
      - JWT_SECRET=...
    depends_on:
      - postgres
      - redis
      - minio

  web:
    build: ./web
    ports:
      - "3000:3000"

  postgres:
    image: postgres:16
    volumes:
      - pgdata:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine

  minio:
    image: minio/minio
    command: server /data

  sentry:
    image: getsentry/sentry:latest
    # ... Sentry 自部署配置

  posthog:
    image: posthog/posthog:latest
    # ... PostHog 自部署配置

  # Nginx 反向代理（TLS 终止 + 路由）
  nginx:
    image: nginx:alpine
    ports:
      - "443:443"
      - "80:80"
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf
      - ./certs:/etc/nginx/certs
    depends_on:
      - api
      - web
```

### 10.2 客户端配置指引

安装后配置企业服务端：

```bash
# 方法 1: 修改配置文件
git-ai config set api_base_url https://your-enterprise-server.com
git-ai config set api_key your-api-key
git-ai config set telemetry_enterprise_dsn https://sentry-key@your-sentry/project-id
git-ai config set update_channel enterprise-latest

# 方法 2: 环境变量（优先级高于配置文件）
export GIT_AI_API_BASE_URL=https://your-enterprise-server.com
export GIT_AI_API_KEY=your-api-key

# 方法 3: 安装脚本自动配置（使用 install_nonce）
# 安装脚本可自动调用 git-ai exchange-nonce
# 安装脚本也支持 API_BASE 环境变量指定 API 地址

# Feature Flags 配置（可选）
export GIT_AI_AUTH_KEYRING=true           # 使用系统密钥链
export GIT_AI_REWRITE_STASH=true          # Stash 重写 authorship notes
export GIT_AI_ASYNC_MODE=true             # 异步守护进程模式
```

### 10.3 Git 服务器 Notes 同步配置

确保 Git 服务器允许 `refs/notes/ai` 命名空间的 push：

**GitLab**：默认允许，无需特殊配置

**Gitea / Gogs**：需确认 `refs/notes/*` 不在 push 白名单限制中

**自建 Git 服务器**：确认 `receive.denyRefs` 配置不限制 `refs/notes/ai`

验证命令：
```bash
# 测试 notes push 是否成功
git push origin refs/notes/ai:refs/notes/ai

# 测试 notes fetch 是否成功
git fetch origin +refs/notes/ai:refs/notes/ai-remotes/origin
```

---

## 十一、开发路线图

### Phase 1: 核心认证与数据接收（2-3 周）

- [ ] OAuth Device Authorization Flow 完整实现（3 种 grant_type）
- [ ] Nonce 交换自动登录支持（企业安装脚本依赖）
- [ ] 用户/组织/部门 API Key 数据模型
- [ ] JWT 签发与验证（Payload 包含 sub, email, name, orgs, personal_org_id）
- [ ] Token 自动刷新机制（refresh_token 端点）
- [ ] Metrics 上传端点（`/worker/metrics/upload`）含部分成功响应
- [ ] PosEncoded 解码器（移植 `src/metrics/pos_encoded.rs`）
- [ ] 基础数据存储
- [ ] 认证门控逻辑：非默认 API 地址时始终上传，默认地址需登录/API Key

### Phase 2: CAS 与高级功能（2-3 周）

- [ ] CAS 上传端点（`/worker/cas/upload`）— 支持多种 Agent PromptRecord 格式
- [ ] CAS 读取端点（`GET /worker/cas/?hashes=`）— 完整内容读取，支持 IDE 插件
- [ ] CAS 安全校验（hash 十六进制验证）
- [ ] CAS Secrets 二次验证（客户端已做熵检测脱敏，服务端需二次验证）
- [ ] Bundle/Share 功能（含正确的 `CreateBundleRequest` 结构）
- [ ] Report 接收端点（含 `project_id` / `upload_id` 响应）
- [ ] Summary 接收端点（无认证，注意安全策略）
- [ ] 离线数据恢复支持（幂等处理、时间戳排序、去重）

### Phase 3: 版本分发与遥测（1-2 周）

- [ ] 版本发布端点（`/worker/releases`）— 支持 4 个频道（latest/next/enterprise-*）
- [ ] 下载端点（SHA256SUMS、安装脚本、**二进制文件**）— 需托管所有平台二进制
- [ ] JetBrains 插件代理端点
- [ ] Sentry 事件接收（双 DSN 支持：OSS + Enterprise）
- [ ] PostHog 集成
- [ ] Feature Flags 远程管理 API

### Phase 4: 统计看板（2-3 周）

- [ ] 聚合查询 API（全局、组织、部门、项目、开发者、**按 Agent 工具**）
- [ ] Dashboard Web UI（`/me` 路径）
- [ ] 个人 Dashboard URL 生成
- [ ] 数据导出功能
- [ ] Git Authorship Notes 同步配置指南
- [ ] 14 个 AI Agent 分类统计展示

### Phase 5: 企业管理与安装配置（2-3 周）

- [ ] 管理后台
- [ ] 用户管理（CRUD、角色）
- [ ] 部门管理
- [ ] API Key 管理（支持 CI/CD 场景的非交互式认证）
- [ ] 仓库白/黑名单管理
- [ ] 配置下发（通过安装脚本注入 `API_BASE` + `INSTALL_NONCE`）
- [ ] 审计日志
- [ ] Prompt 访问控制
- [ ] 跨平台安装配置指南（Windows Named Pipe vs Unix Socket）
- [ ] Daemon 模式部署指南（control.sock / named pipe 配置）
- [ ] 离线缓冲容量监控和恢复指南

### Phase 6: 企业级增强功能（2-3 周）

- [ ] PR 级别聚合 API
- [ ] 代码持久性追踪（基于 Authorship Note 的 line_ranges attestation）
- [ ] Agent 就绪度评估
- [ ] CI/CD 集成与全生命周期追踪
- [ ] 高级仪表盘增强（趋势分析、Agent 对比、团队对比）
- [ ] LDAP/SSO 集成
- [ ] 数据保留策略
- [ ] 告警与通知
- [ ] 多集群部署
- [ ] 数据脱敏增强
- [ ] RewriteLog 事件审计（追踪 rebase/cherry-pick/reset 等操作的归属变化）
- [ ] CI 环境专用 API Key 和权限隔离

---

## 十二、风险与注意事项

### 12.1 兼容性风险

1. **PosEncoded 编码**：Metrics 事件的编码方式复杂（位置编码 + 稀疏属性），解码器实现需要严格对齐客户端代码。建议直接移植 Rust 代码或编写对应的解码库。

2. **JWT Payload 格式**：客户端从 JWT 中提取用户身份（`sub`、`email`、`name`、`orgs`、`personal_org_id`），字段名和结构必须完全一致。客户端仅做 Base64 解码，不做签名验证。

3. **OAuth 错误码**：必须严格遵循 RFC 8628 标准错误码，客户端硬编码了这些错误码的处理逻辑。

4. **请求头**：客户端使用 `X-Distinct-ID`、`X-Author-Identity`、`X-API-Key` 等自定义请求头，服务端必须正确处理。

5. **CAS 读取端点**：此端点返回完整 Prompt 内容（不仅仅是存在性检查），VS Code 扩展依赖此端点获取 AI 对话记录。缺少此端点会导致 IDE 插件无法显示 AI 归属信息。

6. **Bundle 数据结构**：客户端发送的是 `{ title, data: { prompts, files? } }`，而非 `{ title, commit_shas, repository_url }`。错误的实现将导致 Bundle 功能不兼容。

7. **Metrics 部分成功**：HTTP 200 + `errors` 数组表示部分成功，服务端不应将验证失败的事件视为完全失败。

8. **Dashboard URL**：客户端生成 `{base_url}/me`，而非 `/dashboard?org=...`。

9. **Git Notes 同步**：需确保 Git 服务器允许 `refs/notes/ai` 的 push/fetch，部分 Git 服务器默认限制 notes 命名空间。

10. **Feature Flags 差异**：debug/release 构建的 feature flag 默认值不同（如 `async_mode`），测试环境与生产环境行为可能不一致。

11. **Daemon 模式数据上传时机**：Release 构建默认启用 `async_mode`，数据通过 daemon 的 3 秒 flush 循环上传。如果 daemon 未运行或无法连接服务端，数据退化为本地 SQLite 存储。企业服务端必须能处理延迟到达的批量数据。

12. **离线数据积压**：长时间离线的客户端可能积压大量 Metrics/CAS 数据，服务端需要支持大批量上传并做去重处理。

13. **跨平台路径差异**：客户端在 authorship log 中使用 POSIX 格式路径（Windows 反斜杠自动转换），服务端存储和查询时应统一使用 POSIX 格式。

14. **Agent Preset 格式差异**：14 种 AI Agent 的 hook input 格式各不相同，产生的 PromptRecord 结构有差异。服务端不应假设 PromptRecord 有统一结构。

15. **CI 环境认证**：CI 场景使用 `X-API-Key` 认证，需确保 API Key 有适当的权限隔离，防止 CI 凭据被用于访问非 CI 数据。

### 12.2 安全注意事项

1. **HTTPS 强制**：Release 模式下客户端强制要求 HTTPS，调试模式允许 HTTP。
2. **令牌安全**：客户端存储凭据时有严格的文件权限控制（Unix: 0o600）。
3. **密钥脱敏**：客户端上传 CAS 前已做密钥脱敏（`src/authorship/secrets.rs`），服务端仍需做二次验证。
4. **数据隐私**：`exclude_prompts_in_repositories` 和 `prompt_storage=local` 模式可完全阻止 Prompt 上传，企业部署时应尊重这些配置。
5. **CAS Hash 注入**：客户端在 CAS 读取前验证 hash 值仅含十六进制字符，服务端也应做相同校验。
6. **Prompt 访问控制**：CAS 内容包含完整的 AI 对话记录，可能包含代码上下文和敏感信息，必须实现组织级访问隔离。

### 12.3 已有参考实现

客户端自带了多个可参考的完整实现：

1. **本地 Report Server**（`src/report/server.rs`，约 2100 行），包含：
   - SQLite 存储层（`IngestResponse` 含 `project_id` / `upload_id`）
   - 完整的聚合查询 API（按组织、部门、项目、开发者维度）
   - 内嵌 HTML/CSS/JS 的 Web 看板
   - 数据接收和查询端点
   - 这个实现可以直接作为企业服务端看板的参考或基础

2. **Git Authorship Notes 同步**（`src/git/sync_authorship.rs`），包含：
   - Fetch + merge tracking ref 安全模式
   - 非快进 push 重试机制（3 次）
   - Fallback merge 逻辑
   - 可直接参考实现 Git 服务器端的 notes 同步支持

3. **PosEncoded 解码器**（`src/metrics/pos_encoded.rs` + `src/metrics/attrs.rs`），包含：
   - 完整的位置编码/解码逻辑
   - 稀疏属性序列化

4. **Secrets 脱敏**（`src/authorship/secrets.rs`），包含：
   - 基于 Shannon 熵的密钥检测算法（受 ripsecrets 启发）
   - 检测长度 15-90 的高熵字符串（API Key、Token 等）
   - 脱敏格式：保留首尾各 4 字符，中间替换为 `***`
   - 在 CAS 上传前自动执行，服务端接收的数据已脱敏
   - 使用 bigram 频率表区分随机字符串与自然代码文本
   - 服务端仍需做二次验证（参见 7.5 节）

5. **Daemon 架构**（`src/daemon/`），包含：
   - Coordinator 命令路由（Global + Family actors）
   - Control API 完整协议定义（JSON-RPC 风格）
   - Telemetry Worker flush 循环逻辑
   - Git Trace2 事件分析器（4 类）
   - 可参考实现企业服务端的批量数据接收逻辑

6. **AttributionTracker**（`src/authorship/attribution_tracker.rs`），包含：
   - 字符级归属追踪算法
   - imara-diff 行级变化检测
   - Move Detection（代码移动保持归属）
   - 大文件 token diff 快速路径优化

7. **RewriteLog**（`src/git/rewrite_log.rs`），包含：
   - 14 种历史改写事件类型定义
   - 事件序列化和反序列化
   - 可参考实现服务端的 CI 归属重建逻辑

8. **CI 集成**（`src/ci/`），包含：
   - GitHub Actions / GitLab CI 的完整集成实现
   - CiContext 运行时上下文
   - CiRunResult 结果格式

---

## 十三、客户端归属追踪数据管道

> 本章详细描述 git-ai 客户端的核心数据管道：Checkpoint → Working Log → Authorship Note → Git Note。
> 企业服务端需要理解此管道，因为 Metrics/CAS 数据的产生时机和内容格式直接取决于管道中的各个阶段。

### 13.1 完整数据流概览

```
AI Agent 编辑文件
    │
    ├─→ 1. Pre-edit Checkpoint (human/untracked)
    │     └─→ git-ai checkpoint human <agent> / <file>
    │          → 捕获编辑前文件状态（排除 AI 自身之前的修改）
    │
    ├─→ 2. Post-edit Checkpoint (ai_agent)
    │     └─→ git-ai checkpoint <agent_preset>
    │          → Agent preset 解析 hook input (JSON stdin)
    │          → 提取 edited files, transcript, model info
    │          → Diff against HEAD/last-checkpoint
    │          → 计算字符级 attributions (imara-diff)
    │          → 生成 WorkingLogEntry
    │               → 写入 .git/ai/working_logs/<base_commit>/
    │
    ├─→ 3. git commit (pre-commit hook)
    │     └─→ pre_commit(): 捕获虚拟归属 (virtual attributions)
    │
    ├─→ 4. git commit (post-commit hook)
    │     └─→ handle_rewrite_log_event(RewriteLogEvent::Commit)
    │          → 读取 working log entries
    │          → 生成 AuthorshipLog (schema: authorship/3.0.0)
    │          → 存储 Git Note (refs/notes/ai)
    │          → 触发 CAS upload (PromptRecord 去重上传)
    │          → 触发 Metrics 事件 (Committed)
    │          → 触发 Authorship Notes sync (push)
    │
    └─→ 5. 数据上传 (Daemon 模式)
         └─→ telemetry_worker flush 循环 (3秒间隔)
              → Metrics: POST /worker/metrics/upload
              → CAS: POST /worker/cas/upload
              → Sentry/PostHog: 发送到遥测端点
```

### 13.2 Checkpoint 类型与行为

| Checkpoint 类型 | 命令 | 触发场景 | 归属标记 |
|----------------|------|---------|---------|
| `human` (legacy) | `git-ai checkpoint human [/file]` | AI Agent pre-edit，捕获未追踪修改 | `Untracked` — 无法确认归属的变更 |
| `known_human` | `git-ai checkpoint mock_known_human [/file]` | IDE 扩展检测到人类手动编辑 | `KnownHuman` — 确认的人类编辑 |
| `ai_agent` | `git-ai checkpoint mock_ai [/file]` | AI Agent post-edit，捕获 AI 修改 | `AI` — 确认的 AI 编辑，附带 session 元数据 |

> **关键理解**：AI Agent Preset 的标准流程是：先调用 `human` checkpoint（排除 AI 之前未追踪的修改），再调用 `ai_agent` checkpoint（捕获 AI 自身的修改）。这种双 checkpoint 机制是实现精确归属追踪的基础。

### 13.3 Working Log 结构

Working Log 是归属追踪的中间状态，存储在 `.git/ai/working_logs/<base_commit>/` 目录下：

- **`base_commit`**：当前 HEAD 的 commit SHA，working log 按此分组
- **`WorkingLogEntry`**：每个 entry 记录：
  - 文件路径（POSIX 格式）
  - 行级归属（`LineAttribution`：起始行、结束行、作者 ID、覆盖作者）
  - Checkpoint 类型（AI / KnownHuman / Untracked）
  - Session 元数据（agent_id、model、tool）

Working Log 在 commit 时被消费，转化为 Authorship Note。Commit 后 working log 文件被清理。

### 13.4 AttributionTracker 算法核心

`src/authorship/attribution_tracker.rs` 是归属计算的核心：

| 组件 | 说明 |
|------|------|
| `Attribution` | 字符级归属范围（start, end, author_id, ts） |
| `LineAttribution` | 行级归属（start_line, end_line, author_id, overrode） |
| `compute_line_changes()` | 使用 imara-diff 做行级变化检测 |
| Move Detection | 检测代码移动（删除+插入同一内容），保持原归属 |
| Token Diff Fast Path | 大文件优化：超过 32KB 时使用 token 级 diff |

**归属类型**：

| 类型 | author_id 格式 | 说明 |
|------|---------------|------|
| AI | `ai::<tool>::<session_id>` | AI 生成，关联到具体 agent 和 session |
| KnownHuman | `human::<email>` | 确认的人类编辑（IDE 扩展触发） |
| Untracked | `human` (无 email) | 未追踪的变更（legacy human checkpoint） |

### 13.5 Authorship Note 格式（authorship/3.0.0）

Commit 后生成的 Authorship Log 存储为 Git Note（`refs/notes/ai`），Schema 版本 `authorship/3.0.0`：

```json
{
  "version": "authorship/3.0.0",
  "commit_sha": "abc123...",
  "attestations": [
    {
      "file": "src/main.rs",
      "hash": "sha256-of-file-content",
      "line_ranges": [
        { "start": 1, "end": 10, "author_type": "human", "author_id": "human::dev@example.com" },
        { "start": 11, "end": 25, "author_type": "ai", "author_id": "ai::claude-code::session-123" }
      ]
    }
  ],
  "metadata": {
    "prompt_records": [
      { "hash": "sha256-of-prompt", "tool": "claude-code", "model": "claude-3.5-sonnet" }
    ]
  }
}
```

> **企业服务端影响**：Authorship Note 中的 `prompt_records` 条目直接驱动 CAS 上传。每个 prompt hash 对应一个 `PromptRecord`，需要上传到 `/worker/cas/upload`。如果 CAS 上传失败，数据进入本地 `prompts.db` 的 `cas_sync_queue` 等待后续上传。

---

## 十四、Daemon 架构与通信协议

> 当 `async_mode` feature flag 启用时（Release 构建默认启用），git-ai 以 daemon 进程模式运行。
> Daemon 是 Metrics/CAS/Sentry 数据上传的实际执行者，企业服务端必须理解其工作机制。

### 14.1 Daemon 进程架构

```
┌──────────────────────────────────────────────────────┐
│                    Daemon Process                     │
│                                                      │
│  ┌──────────────┐    ┌────────────────────────────┐  │
│  │  Coordinator  │    │    Telemetry Worker        │  │
│  │  (命令路由)    │    │    (3秒 flush 循环)        │  │
│  │              │    │                            │  │
│  │  ┌────────┐  │    │  ┌──────────────────────┐  │  │
│  │  │ Global │  │    │  │ TelemetryBuffer      │  │  │
│  │  │ Actor  │  │    │  │ (内存缓冲区)          │  │  │
│  │  └────────┘  │    │  │ - errors              │  │  │
│  │              │    │  │ - performances        │  │  │
│  │  ┌────────┐  │    │  │ - messages            │  │  │
│  │  │Family  │  │    │  │ - metrics             │  │  │
│  │  │Actor 1 │  │    │  │ - cas_records         │  │  │
│  │  └────────┘  │    │  └──────────────────────┘  │  │
│  │  ┌────────┐  │    │                            │  │
│  │  │Family  │  │    │  Flush 目标:               │  │
│  │  │Actor 2 │  │    │  → /worker/metrics/upload  │  │
│  │  └────────┘  │    │  → /worker/cas/upload      │  │
│  └──────────────┘    │  → Sentry DSN              │  │
│                      │  → PostHog                 │  │
│  ┌──────────────┐    └────────────────────────────┘  │
│  │ Control API  │                                     │
│  │ (Socket/Pipe)│◄──── git-ai 客户端命令              │
│  └──────────────┘                                     │
│                                                      │
│  ┌──────────────┐    ┌────────────────────────────┐  │
│  │  Analyzers   │    │   Trace2 Normalizer        │  │
│  │ - generic    │    │   (Git Trace2 事件解析)      │  │
│  │ - history    │    └────────────────────────────┘  │
│  │ - transport  │                                     │
│  │ - workspace  │    ┌────────────────────────────┐  │
│  └──────────────┘    │   Reducer                  │  │
│                      │   (命令归约/分类)            │  │
│  ┌──────────────┐    └────────────────────────────┘  │
│  │ Sentry Layer │                                     │
│  └──────────────┘                                     │
└──────────────────────────────────────────────────────┘
```

### 14.2 控制通道协议

Daemon 通过 IPC 通道接收命令：

| 平台 | 通信方式 | 路径 |
|------|---------|------|
| macOS/Linux | Unix Domain Socket | `~/.git-ai/internal/daemon/control.sock` |
| Windows | Named Pipe | `\\.\pipe\git-ai-{hash}-control` |

**ControlRequest 消息类型**（JSON-RPC 风格）：

| method | 说明 | 服务端影响 |
|--------|------|-----------|
| `checkpoint.run` | 执行 checkpoint | 生成 WorkingLog → 最终触发 Metrics/CAS 上传 |
| `status.family` | 查询仓库族状态 | 无 |
| `telemetry.submit` | 提交遥测信封 | 直接进入 TelemetryBuffer → 3秒后 flush 到服务端 |
| `cas.submit` | 提交 CAS 记录 | 直接进入 TelemetryBuffer → 3秒后 flush 到服务端 |
| `wrapper.pre_state` | Git 命令执行前状态 | 记录 pre-hook 上下文 |
| `wrapper.post_state` | Git 命令执行后状态 | 触发 post-hook 处理 → 可能触发 Authorship 生成 |
| `snapshot.watermarks` | 查询/更新水印状态 | 无 |
| `shutdown` | 停止 daemon | 无 |

### 14.3 Git Trace2 事件处理

Daemon 通过 Git Trace2 协议接收 Git 操作事件：

- **通道**：`trace2.eventTarget` 配置指向 daemon 的 Unix Socket / Named Pipe
- **协议**：`af_unix:stream:{path}` (Unix) 或 named pipe (Windows)
- **Trace2 事件**：Git 操作的开始/结束、子命令、性能数据等

**分析器分类**：

| 分析器 | 文件 | 功能 |
|--------|------|------|
| `generic` | `analyzers/generic.rs` | 通用 Git 事件分析 |
| `history` | `analyzers/history.rs` | 历史改写操作检测（rebase, cherry-pick 等） |
| `transport` | `analyzers/transport.rs` | 网络传输事件（fetch, push, clone） |
| `workspace` | `analyzers/workspace.rs` | 工作区变更事件（checkout, switch, stash） |

### 14.4 Telemetry Worker Flush 循环

Daemon 的核心上传逻辑：

```
每 3 秒执行一次:
    1. 取出 TelemetryBuffer 中的所有积压事件
    2. flush_metrics():
       - 构造 ApiContext (认证门控检查)
       - should_upload=true → upload_metrics_with_retry()
         → POST /worker/metrics/upload (最多 2 次尝试: 首次 + 60秒后重试)
       - should_upload=false 或上传失败 → 存入本地 metrics-db (SQLite)
    3. flush_cas():
       - 构造 ApiContext
       - should_upload=true → POST /worker/cas/upload
       - 失败 → 存入 cas_sync_queue
    4. flush_sentry_and_posthog():
       - 构造 Sentry Envelope
       - 双 DSN 发送: SENTRY_OSS + SENTRY_ENTERPRISE
       - PostHog: POST https://us.i.posthog.com/capture/
```

**TelemetryEnvelope 类型**（`control_api.rs`）：

| 类型 | 字段 | 说明 |
|------|------|------|
| `Error` | timestamp, message, context | 错误事件 → Sentry |
| `Performance` | timestamp, operation, duration_ms, context, tags | 性能事件 → Sentry |
| `Message` | timestamp, message, level, context | 日志消息 → Sentry |
| `Metrics` | events: Vec\<MetricEvent\> | Metrics 事件 → /worker/metrics/upload |

> **企业部署关键点**：当 daemon 无法连接企业服务端时，所有遥测数据退化为本地 SQLite 存储。服务端恢复后，需通过 `flush-metrics-db` / `flush-cas` 命令手动触发上传，或等待 daemon 下次 flush 时自动重试。

---

## 十五、AI Agent 生态与 Checkpoint Preset

> 客户端支持 14 种 AI Agent，每种有独立的 Checkpoint Preset 解析其特定格式的 hook input。
> CAS 和 Metrics 数据格式直接取决于这些 preset 的行为。

### 15.1 支持的 AI Agent 列表

| Agent | 源码文件 | Agent Preset ID | 说明 |
|-------|---------|----------------|------|
| Claude Code | `claude_code.rs` | `claude-code` | Anthropic 的 CLI 编码助手 |
| Cursor | `cursor.rs` | `cursor` | AI-first 代码编辑器 |
| GitHub Copilot | `github_copilot.rs` | `github-copilot` | GitHub 的 AI 编码助手 |
| Codex | `codex.rs` | `codex` | OpenAI Codex CLI |
| Gemini | `gemini.rs` | `gemini` | Google Gemini CLI |
| Windsurf | `windsurf.rs` | `windsurf` | Codeium 的 AI 编辑器 |
| AMP | `amp.rs` | `amp` | Sourcegraph 的 AI 编码助手 |
| Droid | `droid.rs` | `droid` | AI 编码助手 |
| Firebender | `firebender.rs` | `firebender` | AI 编码助手 |
| JetBrains AI | `jetbrains.rs` | `jetbrains` | JetBrains IDE 内置 AI |
| OpenCode | `opencode.rs` | `opencode` | 开源 AI 编码 CLI |
| Pi | `pi.rs` | `pi` | AI 编码助手 |
| VS Code (ai_tab) | `vscode.rs` | `vscode` | VS Code AI 标签页 |
| Continue | (via `continue_session.rs`) | `continue` | 开源 AI 编码扩展 |

### 15.2 Agent Preset 行为

每个 Agent Preset（`AgentCheckpointPreset`）定义了：

| 属性 | 说明 |
|------|------|
| `agent_id` | Agent 标识符（如 `claude-code`） |
| Hook Input 解析 | 从 stdin 读取 JSON，提取编辑文件列表、对话记录、模型信息 |
| Pre-edit Checkpoint | 调用 `human` checkpoint 捕获未追踪修改 |
| Post-edit Checkpoint | 调用 `ai_agent` checkpoint 捕获 AI 修改 |

**AgentRunResult**（checkpoint 执行结果）：

```rust
pub struct AgentRunResult {
    pub agent_id: String,        // e.g., "claude-code"
    pub session_id: String,      // 会话唯一标识
    pub model: String,           // e.g., "claude-3.5-sonnet"
    pub edited_files: Vec<String>, // 编辑的文件列表
    pub transcript: Vec<AiTranscriptMessage>, // AI 对话记录
}
```

### 15.3 对 CAS/Metrics 数据的影响

| Agent 行为 | CAS 影响 | Metrics 影响 |
|-----------|---------|-------------|
| Hook Input 格式不同 | `PromptRecord.messages` 结构因 agent 而异 | `tool` 和 `tool_model_pairs` 字段值取决于 agent |
| 编辑文件提取方式不同 | 影响关联的 `PromptRecord` 中文件上下文 | 影响提交的 `ai_additions` 统计 |
| 模型信息提取方式不同 | `AgentId.model` 值来源不同 | `tool_model_pairs` 格式为 `<tool>::<model>` |

> **企业服务端建议**：Metrics 查询应支持按 `tool` 过滤；CAS 存储应能处理不同 agent 产生的不同 Prompt 格式；Dashboard 应展示按 Agent 分类的使用统计。

---

## 十六、客户端完整命令清单

> 以下列出 git-ai 客户端的所有子命令及其与服务端的交互关系。

### 16.1 命令清单

| 命令 | 说明 | 服务端依赖 | 源码位置 |
|------|------|-----------|---------|
| `checkpoint` | 执行归属追踪 checkpoint | 间接（通过 daemon 上传 metrics/CAS） | `src/commands/checkpoint.rs` |
| `login` | OAuth 设备授权登录 | `/worker/oauth/device/code`, `/worker/oauth/token` | `src/commands/login.rs` |
| `logout` | 清除本地凭据 | 无 | `src/commands/login.rs` |
| `whoami` | 显示当前用户身份 | JWT 本地解析（无网络请求） | `src/commands/login.rs` |
| `exchange-nonce` | Nonce 自动登录 | `/worker/oauth/token` (grant_type=install_nonce) | `src/commands/exchange_nonce.rs` |
| `install-hooks` | 安装所有 hooks + 扩展 | 无 | `src/commands/install_hooks.rs` |
| `upgrade` | 检查并执行升级 | `/worker/releases` + 下载端点 | `src/commands/upgrade.rs` |
| `flush-cas` | 手动 CAS 同步 | `/worker/cas/upload` | `src/commands/flush_cas.rs` |
| `flush-metrics-db` | 手动 Metrics 上传 | `/worker/metrics/upload` | `src/commands/flush_metrics_db.rs` |
| `share` | 创建分享 Bundle (TUI) | `/api/bundles` | `src/commands/share.rs` |
| `sync-prompts` | 同步 Prompt 记录 | 本地数据库操作 | `src/commands/sync_prompts.rs` |
| `report` | 生成/上传报告 | `/api/v1/reports`, `/api/v1/summaries` | `src/commands/report.rs` |
| `dashboard` | 打开 Web 看板 | `{api_base_url}/me` | `src/commands/personal_dashboard.rs` |
| `ci github run` | GitHub Actions CI 运行 | 本地 git 操作 | `src/commands/ci_handlers.rs` |
| `ci github install` | 安装 GitHub Actions workflow | 无 | `src/commands/ci_handlers.rs` |
| `ci gitlab run` | GitLab CI 运行 | 本地 git 操作 | `src/commands/ci_handlers.rs` |
| `ci gitlab install` | 生成 GitLab CI YAML | 无 | `src/commands/ci_handlers.rs` |
| `ci local` | 本地 CI 模拟 | 本地 git 操作 | `src/commands/ci_handlers.rs` |
| `fetch-notes` | 手动拉取 authorship notes | Git 服务器 | `src/commands/git_ai_handlers.rs` |
| `squash-authorship` | 压缩归属 | 本地 git 操作 | `src/commands/git_ai_handlers.rs` |
| `blame` | 归属 blame | 本地 git notes 读取 | `src/commands/blame.rs` |
| `diff` | 归属 diff | 本地 git notes 读取 | `src/commands/diff.rs` |
| `status` | 归属状态 | 本地 git notes 读取 | `src/commands/status.rs` |
| `log` | 归属日志 | 本地 git notes 读取 | `src/commands/git_ai_handlers.rs` |
| `show` | 显示归属详情 | 本地 git notes 读取 | `src/commands/git_ai_handlers.rs` |
| `search` | 搜索归属 | 本地 git notes 读取 | `src/commands/search.rs` |
| `continue` | 继续上一个 AI 会话 | 本地 prompts.db 查询 | `src/commands/continue_session.rs` |
| `prompts` | Prompt 数据库管理 | 本地 prompts.db 操作 | `src/commands/prompts_db.rs` |

### 16.2 命令分类与服务端交互

```
需要服务端交互的命令:
├── 认证类: login, exchange-nonce → /worker/oauth/*
├── 数据上传类: checkpoint(间接), report, share → /worker/*, /api/*
├── 手动同步类: flush-cas, flush-metrics-db → /worker/*
├── 版本更新类: upgrade → /worker/releases/*
└── 看板类: dashboard → /me (仅打开浏览器)

纯本地命令（无服务端依赖）:
├── 归属查询: blame, diff, status, log, show, search
├── 本地管理: install-hooks, logout, whoami, sync-prompts
├── CI 命令: ci github/gitlab/local
├── 本地操作: squash-authorship, fetch-notes
└── 数据管理: continue, prompts
```

---

## 十七、RewriteLog 与历史改写事件体系

> RewriteLog 是保证归属数据在 rebase/cherry-pick/reset 等操作后正确追踪的关键中间层。
> 企业服务端需要理解这些事件，因为 CI 集成和 Authorship Note 重建都依赖此机制。

### 17.1 RewriteLogEvent 完整类型

```rust
pub enum RewriteLogEvent {
    Commit { commit: CommitEvent },
    CommitAmend { commit_amend: CommitAmendEvent },
    RebaseStart { rebase_start: RebaseStartEvent },
    RebaseComplete { rebase_complete: RebaseCompleteEvent },
    RebaseAbort { rebase_abort: RebaseAbortEvent },
    CherryPickStart { cherry_pick_start: CherryPickStartEvent },
    CherryPickComplete { cherry_pick_complete: CherryPickCompleteEvent },
    CherryPickAbort { cherry_pick_abort: CherryPickAbortEvent },
    RevertMixed { revert_mixed: RevertMixedEvent },
    Reset { reset: ResetEvent },
    Merge { merge: MergeEvent },
    MergeSquash { merge_squash: MergeSquashEvent },
    Stash { stash: StashEvent },
    AuthorshipLogsSynced { authorship_logs_synced: AuthorshipLogsSyncedEvent },
}
```

### 17.2 Post-Hook 处理逻辑

| RewriteLogEvent | 触发的 Post-Hook | 归属处理 |
|----------------|-----------------|---------|
| `Commit` | commit post-hook | 生成新的 Authorship Note |
| `CommitAmend` | commit amend post-hook | 重写 Authorship Note（对比 amend 前后快照） |
| `RebaseComplete` | rebase post-hook | 逐 commit 重写 Authorship Notes |
| `RebaseAbort` | rebase post-hook | 恢复原始 Authorship Notes |
| `CherryPickComplete` | cherry-pick post-hook | 复制/适配源 commit 的归属 |
| `CherryPickAbort` | cherry-pick post-hook | 无需处理 |
| `Reset` | reset post-hook | 重建 working log（恢复被 reset 的归属） |
| `Merge` | merge post-hook | 保留双方归属 |
| `MergeSquash` | merge --squash post-hook | 合并所有源 commit 归属 |
| `Stash` | stash post-hook | 保存/恢复未提交归属 |
| `RevertMixed` | revert post-hook | 调整归属（反转修改的行） |

### 17.3 对企业服务端的影响

1. **CI/CD 中的 Authorship 重建**：`git-ai ci run` 命令在 CI 环境中执行 authorship 重建，需要读取 rewrite log 和 authorship notes
2. **数据一致性**：rebase 等操作会改变 commit SHA，导致 Metrics 中的 `commit_sha` 与实际不一致。服务端应支持通过 authorship note 内容（而非仅 SHA）关联数据
3. **AuthorshipLogsSynced 事件**：记录 notes 同步操作，可用于审计追踪

---

## 十八、离线缓冲与恢复机制

> 客户端具有完整的离线缓冲层，当无法连接服务端时，所有数据存入本地队列。
> 企业服务端必须支持"先存后传"模式，正确处理延迟到达的数据。

### 18.1 本地数据库体系

| 数据库 | 路径 | 角色 | 数据流 |
|--------|------|------|--------|
| `prompts.db` | `~/.git-ai/internal/prompts.db` | CAS 上传队列 + Prompt 缓存 | CAS 上传失败 → `cas_sync_queue` → 下次 flush 重试 |
| `metrics-db` | `~/.git-ai/internal/metrics-db` | Metrics 上传队列 + 节流 | Metrics 上传失败 → `metrics` 表 → 下次 flush 重试 |
| `credentials` | `~/.git-ai/internal/credentials` | OAuth tokens | 本地读写，不上传 |

### 18.2 prompts.db 表结构

| 表 | 说明 | 关键字段 |
|----|------|---------|
| `prompts` | AI Prompt 记录 | id, tool, model, messages, commit_sha, timestamp |
| `cas_sync_queue` | CAS 上传同步队列 | hash, data (PromptRecord JSON), status, attempts, created_at |
| `cas_cache` | CAS 读取缓存 | hash, messages, cached_at |

### 18.3 metrics-db 表结构

| 表 | 说明 | 关键字段 |
|----|------|---------|
| `metrics` | 待上传 Metrics 事件 | id, event_json (序列化的 MetricEvent) |
| `agent_usage_throttle` | AgentUsage 事件节流 | prompt_id, last_sent_ts |

> **AgentUsage 节流**：同一 `prompt_id` 的 AgentUsage 事件在 60 秒内不会重复发送，避免频繁 checkpoint 导致的重复 Metrics。

### 18.4 离线→在线恢复流程

```
客户端离线时:
    checkpoint/commit → Working Log → Authorship Note (本地)
    Metrics → metrics-db (SQLite)
    CAS → cas_sync_queue (SQLite)

客户端恢复在线时:
    Daemon 模式:
      telemetry_worker 每 3 秒 flush 循环自动重试
      → flush_metrics(): 从 metrics-db 读取积压事件 + 新事件一起上传
      → flush_cas(): 从 cas_sync_queue 读取积压记录 + 新记录一起上传

    非 Daemon 模式:
      手动执行: git-ai flush-metrics-db
      手动执行: git-ai flush-cas
```

### 18.5 企业服务端实现建议

1. **幂等处理**：Metrics 和 CAS 上传必须支持幂等。客户端可能重传已接收的数据（重试机制导致）
2. **时间戳排序**：离线期间积压的事件到达服务端时顺序可能乱序，服务端应按事件时间戳（而非接收时间）排序
3. **去重**：CAS 按 hash 天然去重；Metrics 事件应按 (distinct_id, timestamp, event_type) 组合去重
4. **容量规划**：长时间离线的客户端可能积压大量数据，服务端应支持大批量上传（当前无批次大小限制）

---

## 十九、跨平台差异

> 企业部署涉及 Windows 和 Unix (macOS/Linux) 两种环境，差异显著。

### 19.1 Daemon 通信差异

| 项目 | Windows | macOS/Linux |
|------|---------|-------------|
| IPC 通道 | Named Pipe: `\\.\pipe\git-ai-{hash}-control` | Unix Socket: `~/.git-ai/internal/daemon/control.sock` |
| Trace2 通道 | Named Pipe: `\\.\pipe\git-ai-{hash}-trace2` | Unix Socket: `af_unix:stream:{path}` |
| 连接方式 | `tokio::net::windows::named_pipe` | `tokio::net::UnixListener` |

### 19.2 进程管理差异

| 项目 | Windows | macOS/Linux |
|------|---------|-------------|
| 进程创建标志 | `CREATE_NO_WINDOW`, `CREATE_BREAKAWAY_FROM_JOB` | 无特殊标志 |
| 信号转发 | 不支持 | SIGTERM, SIGINT, SIGHUP, SIGQUIT 转发给子进程组 |
| Hooks 禁用 | `core.hooksPath=NUL` | `core.hooksPath=/dev/null` |

### 19.3 凭据存储差异

| 后端 | Windows | macOS/Linux |
|------|---------|-------------|
| 文件存储 | `~/.git-ai/internal/credentials` | `~/.git-ai/internal/credentials` (0o600 权限) |
| Keyring | Windows Credential Manager | macOS Keychain / Linux Secret Service |

> `auth_keyring` feature flag 控制是否使用系统密钥链，默认关闭。

### 19.4 路径处理

客户端在所有 authorship log 和 working log 中使用 POSIX 格式路径（`normalize_to_posix()`），Windows 的反斜杠会自动转换为正斜杠。企业服务端存储和查询时应统一使用 POSIX 格式。

---

## 二十、安装与初始化完整流程

> 企业部署时，客户端的安装和初始化流程需要特别关注。

### 20.1 install-hooks 完整编排

`git-ai install-hooks` 命令执行以下步骤（按顺序）：

| 步骤 | 说明 | 源码 |
|------|------|------|
| 1. Git shim 安装 | 将 git-ai 二进制设置为 git 命令的代理 | `src/mdm/ensure_git_symlinks.rs` |
| 2. core.hooksPath 配置 | 设置全局 hooks 路径 | `src/mdm/hook_installer.rs` |
| 3. trace2.eventTarget 配置 | 设置 Trace2 事件目标（指向 daemon） | `src/commands/install_hooks.rs` |
| 4. AI Agent 扩展检测+安装 | 自动检测已安装的 AI agent 并安装对应扩展 | `src/mdm/agents/mod.rs` → `get_all_installers()` |
| 5. JetBrains 插件检测+安装 | 检测已安装的 JetBrains IDE 并下载插件 | `src/mdm/jetbrains/` |
| 6. Skills 文件安装 | 安装 agent skills 配置文件 | `src/mdm/skills_installer.rs` |
| 7. Git 客户端集成 | 配置 Fork、Sublime Merge 等 Git GUI | `src/mdm/git_clients/` |

### 20.2 安装脚本行为

`install.sh` (Unix) / `install.ps1` (Windows) 执行：

1. 下载二进制文件 → `/worker/releases/{channel}/download/{filename}`
2. 下载 SHA256SUMS → `/worker/releases/{channel}/download/SHA256SUMS`
3. 验证 SHA256SUMS 完整性（与 `/worker/releases` 返回的 `checksum` 比对）
4. 验证二进制文件 hash（与 SHA256SUMS 中的条目比对）
5. 安装二进制到系统路径
6. 如果设置了 `INSTALL_NONCE` 环境变量：
   - 调用 `git-ai exchange-nonce` 自动登录
7. 如果设置了 `API_BASE` 环境变量：
   - 配置 `api_base_url` 指向企业服务端
8. 执行 `git-ai install` 初始化

> **企业部署建议**：企业安装脚本应设置 `API_BASE` 和 `INSTALL_NONCE` 环境变量，实现一键安装+自动登录+指向企业服务端。

### 20.3 MDM 配置分发

`src/mdm/` 模块负责企业配置管理：

| 组件 | 说明 |
|------|------|
| `agents/` | 14 个 AI agent 扩展安装器 |
| `jetbrains/` | JetBrains 插件下载+安装 |
| `git_clients/` | Git GUI 客户端集成（Fork, Sublime Merge 等） |
| `ensure_git_symlinks.rs` | 确保 git 命令指向 git-ai |
| `hook_installer.rs` | Git hooks 安装 |
| `skills_installer.rs` | Skills 文件安装 |
| `git_client_installer.rs` | Git 客户端安装器框架 |

---

## 二十一、CI/CD 客户端兼容性

> 客户端已内置 CI/CD 集成命令，企业服务端需要了解这些命令的行为以正确支持 CI 场景。

### 21.1 CI 命令体系

| 命令 | 说明 | 源码 |
|------|------|------|
| `git-ai ci github run` | 在 GitHub Actions 中运行 authorship 重写 | `src/ci/github.rs` |
| `git-ai ci github install` | 安装 GitHub Actions workflow 文件 | `src/ci/github.rs` |
| `git-ai ci gitlab run` | 在 GitLab CI 中运行 authorship 重写 | `src/ci/gitlab.rs` |
| `git-ai ci gitlab install` | 生成 GitLab CI YAML 配置 | `src/ci/gitlab.rs` |
| `git-ai ci local` | 本地 CI 模拟（测试用） | `src/ci/ci_context.rs` |

### 21.2 CI Run 行为

`git-ai ci <platform> run` 执行的流程：

1. 读取环境变量获取 CI 上下文（commit range, PR info 等）
2. 遍历 commit range 中的每个 commit
3. 读取/重建 authorship notes
4. 生成 `CiRunResult`（包含 AI/Human 行数统计）
5. 可选：上传结果到企业服务端

### 21.3 企业服务端 CI 支持

1. **Authorship Notes 可用性**：CI runner 必须能 fetch `refs/notes/ai`，服务端需确保 Git 服务器允许 notes 命名空间
2. **API Key 认证**：CI 环境使用 `X-API-Key` 认证（非交互式），服务端需支持此认证方式
3. **CI 事件上报**：建议扩展 Metrics 事件类型，新增 CI 相关事件（参见 7.4 节 CI/CD 集成）
4. **结果存储**：`CiRunResult` 可通过 `/api/v1/reports` 端点上传，与手动 report 上传使用相同端点
   - 可直接移植为服务端解码库
