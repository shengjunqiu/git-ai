# Git-AI Data Flow

本文档梳理本仓库的核心数据流：数据在哪里产生、写入哪些本地/服务端存储，以及哪些命令会消费这些数据。


## 总览

`git-ai` 的数据流可以分成三层：

1. **本地归属层**：记录 AI、人类和 known-human 对工作区文件的修改，最终固化为 `authorship/3.0.0` Git Note。
2. **遥测与报表层**：从 checkpoint、commit stats 和 Git Notes 派生 metrics/report 数据，用于上传和 dashboard 聚合。
3. **企业服务层**：接收 OAuth/API key 认证后的 CAS、metrics、report 数据，写入 Postgres/对象存储，再由 dashboard API 聚合展示。

简化链路：

```text
Agent/IDE/Git wrapper
  -> checkpoint
  -> .git/ai/working_logs/<base_commit>/
  -> git commit post-processing
  -> refs/notes/ai
  -> local metrics/CAS queues
  -> enterprise-server
  -> dashboard aggregate APIs
```

## 入口

`src/main.rs` 根据进程名分派：

| 调用方式 | 入口 | 作用 |
| --- | --- | --- |
| `git-ai ...` | `commands::git_ai_handlers::handle_git_ai` | 处理 CLI 子命令，如 `checkpoint`、`report`、`login`、`flush-metrics-db`。 |
| `git ...`，且实际命中 git-ai shim | `commands::git_handlers::handle_git` | 代理真实 Git，并在关键 Git 命令前后运行归属逻辑。 |
| 已废弃 git core hook symlink | `commands::git_hook_handlers` | 只保留迁移/移除旧 hook 的能力。 |

Git wrapper 模式下，`git_handlers` 会解析真实 Git 命令，然后对 `commit`、`push`、`pull`、`rebase`、`reset`、`stash`、`checkout` 等命令执行对应 pre/post 逻辑。普通 Git 命令仍由真实 Git 执行，git-ai 只在前后补充归属处理。

## 本地归属采集

### Checkpoint 产生

checkpoint 的主实现位于 `src/commands/checkpoint.rs`。

数据来源包括：

- 手动或 agent preset 触发的 `git-ai checkpoint`。
- IDE/agent 集成提交的 agent run result。
- Git wrapper 在 `git commit` 前调用的 pre-commit checkpoint。
- async daemon 模式下，wrapper 捕获 Git 前后状态，daemon 再执行归属分析。

每个 checkpoint 会记录：

| 字段 | 说明 |
| --- | --- |
| `kind` | `human`、`ai_agent`、`ai_tab`、`known_human`。 |
| `author` | 人类 Git author、agent 名称或 known-human 来源。 |
| `entries` | 变更文件、blob hash、文件级/行级 attribution。 |
| `transcript` | 可选 AI 对话内容。 |
| `agent_id` | tool、外部会话 id、model。 |
| `line_stats` | checkpoint 粒度的新增/删除行数。 |

### Working Log

checkpoint 先写入仓库内的临时工作日志：

```text
.git/ai/
  working_logs/
    <base_commit>/
      checkpoints.jsonl
      blobs/<sha256>
      INITIAL
  rewrite_log
  logs/
```

对应实现：

- `src/git/repo_storage.rs`
- `src/authorship/working_log.rs`

关键规则：

- `<base_commit>` 是当前工作区基于的 commit；空仓库使用 `initial`。
- `checkpoints.jsonl` 是 checkpoint 追加日志。
- `blobs/` 保存 checkpoint 捕获的文件内容快照。
- `INITIAL` 保存跨 commit 保留下来的未提交 attribution。
- commit 成功后，旧 working log 会移动到 `old-<base_commit>`；release 构建会定期清理过期 old logs。

## Commit 固化流程

`git commit` 的主流程：

```text
git wrapper
  -> commit_pre_command_hook
  -> authorship::pre_commit
  -> real git commit
  -> commit_post_command_hook
  -> rewrite log event
  -> authorship::post_commit
```

主要源码：

- `src/commands/hooks/commit_hooks.rs`
- `src/authorship/pre_commit.rs`
- `src/authorship/post_commit.rs`
- `src/git/rewrite_log.rs`

`post_commit` 做几件关键事情：

1. 读取当前 base commit 的 working log。
2. 更新最新 AI transcript，避免最后一轮对话遗漏。
3. 将 prompt 摘要写入本地 prompts DB。
4. 用 `VirtualAttributions` 把 working log 转成提交后的行级 authorship。
5. 生成 `AuthorshipLog`。
6. 根据 prompt storage 配置处理 prompt messages：
   - `local`：messages 只留在本地 SQLite，不写入 notes。
   - `notes`：脱敏后写入 Git Notes。
   - `default`：尝试把 messages 入 CAS 队列，notes 中只保留 `messages_url`。
7. 写入 `refs/notes/ai`。
8. 计算 commit stats，记录 `committed` metrics。
9. 如还有未提交 AI attribution，写入新 commit 对应的 `INITIAL`。

Authorship Note 格式版本是 `authorship/3.0.0`，由 `src/authorship/authorship_log_serialization.rs` 维护。它由两部分组成：

```text
<file path>
  <prompt_or_human_hash> <line ranges>
---
{
  "schema_version": "authorship/3.0.0",
  "git_ai_version": "...",
  "base_commit_sha": "...",
  "prompts": {...},
  "humans": {...}
}
```

本地查询命令如 `git-ai blame`、`diff`、`show`、`search` 主要读取 Git Notes 和本地 prompt/cache 数据，不依赖 enterprise-server。

## 历史改写数据流

rebase、amend、cherry-pick、reset、merge、stash 等会改变 commit 或工作区状态。git-ai 用 rewrite log 记录这些事件，再重建或迁移 authorship。

```text
Git rewrite operation
  -> pre hook captures context
  -> real git command
  -> post hook appends RewriteLogEvent
  -> repository.handle_rewrite_log_event
  -> sync/rebuild authorship notes or working log
```

本地文件：

```text
.git/ai/rewrite_log
```

这条链路的目标是让 `refs/notes/ai` 跟随 Git 历史变化，而不是把 attribution 只绑定在旧 SHA 上。

## Metrics 数据流

metrics 是面向服务端 dashboard 的事件流，和 Git Notes 是两套不同用途的数据：

| 类型 | 何时产生 | 主要用途 |
| --- | --- | --- |
| `Checkpoint` | checkpoint 写入 working log 后 | 记录文件级 checkpoint 行数、tool/model、prompt id。 |
| `AgentUsage` | agent 使用事件，带节流 | 记录 AI 会话使用情况。 |
| `Committed` | commit stats 计算完成后 | dashboard 的提交、AI 行数、人类行数、工具模型统计。 |
| `InstallHooks` | 安装流程 | 安装/启用状态统计。 |

客户端 metrics 结构在 `src/metrics/`：

- `types.rs` 定义 `MetricEventId`、`MetricEvent`、`MetricsBatch`。
- `events.rs` 定义各事件的 PosEncoded 字段位置。
- `attrs.rs` 定义 repo、branch、author、tool、model 等属性。
- `db.rs` 是本地 SQLite 队列，路径为 `~/.git-ai/internal/metrics-db`。

调用链：

```text
metrics::record(...)
  -> observability::log_metrics(...)
  -> daemon telemetry envelope
  -> telemetry_worker flush loop
  -> /worker/metrics/upload
```

如果 daemon 或网络不可用，事件会进入本地 `metrics-db`。手动补传命令是：

```bash
git-ai flush-metrics-db
```

上传条件由 `ApiContext` 和配置决定：

```text
should_upload = api_base_url 不是默认云端
             或 已登录 Bearer token
             或 配置了 GIT_AI_API_KEY / api_key
```

注意：本地 `git commit` 会生成 Git Note 和 metrics 事件，但只有 telemetry worker 正常运行且满足上传条件时，metrics 才会自动进入 enterprise-server。否则需要等待后续 flush 或手动执行 `flush-metrics-db`。

## CAS / Prompt 数据流

Prompt messages 不默认直接写入 Git Notes。默认模式下，post-commit 会：

```text
PromptRecord.messages
  -> secret redaction
  -> CAS hash
  -> local cas_sync_queue / daemon CAS submit
  -> /worker/cas/upload
  -> enterprise CAS store
  -> Authorship Note 中保留 messages_url
```

相关源码：

- `src/authorship/post_commit.rs`
- `src/authorship/internal_db.rs`
- `src/api/cas.rs`
- `src/commands/flush_cas.rs`
- `enterprise-server/src/handlers/cas.rs`
- `enterprise-server/src/services/cas.rs`

本地 prompt/CAS 数据主要在：

```text
~/.git-ai/internal/prompts.db
```

如果 CAS 上传失败，数据会留在本地队列，后续 daemon flush 或手动命令可重试：

```bash
git-ai flush-cas
```

## Report 数据流

report 是从 Git 历史和 `refs/notes/ai` 扫描出来的快照，不等同于实时 metrics。

### Full Report

命令：

```bash
git-ai report scan .
git-ai report upload . --range <from>..<to> --server <url>
```

源码：

- `src/commands/report.rs`
- `src/report/scan.rs`
- `src/report/model.rs`
- `src/report/upload.rs`

扫描流程：

```text
resolve commits
  -> read commits with authorship notes
  -> stats_for_commit_stats for each commit
  -> ReportDocument git-ai-report/1.0.0
  -> sanitize repo.workdir
  -> POST /api/v1/reports
```

`ReportDocument` 包含 repo hash、range、summary、tool/model breakdown 和每个 commit 的 stats。上传前会移除本地绝对路径 `repo.workdir`。

### Summary Report

命令：

```bash
git-ai report summary
git-ai report summary --server <url>
```

它生成 `git-ai-summary/1.0.0`，按开发者聚合全历史统计，并可附加 org、dept、reporter、period 等元数据。

现有 `docs/guides/report-summary-guide.md` 和 `docs/guides/server-deployment.md` 主要讲的是这个旧版 `git-ai report server` / SQLite / 8787 流程；enterprise-server 当前还额外支持 `/api/v1/reports` 和认证后的 dashboard 数据隔离。

## Enterprise Server 数据流

enterprise-server 是独立 Rust crate，主要入口：

- `enterprise-server/src/main.rs`
- `enterprise-server/src/routes.rs`
- `enterprise-server/src/db/migrations.rs`

核心依赖：

| 组件 | 用途 |
| --- | --- |
| Postgres | 用户、组织、metrics、reports、dashboard 聚合数据。 |
| Redis | rate limit 等运行时服务。 |
| MinIO/S3 | CAS、release asset 等对象存储。 |

主要写入端点：

| Endpoint | 来源 | 落库/存储 |
| --- | --- | --- |
| `POST /worker/oauth/device/code` | `git-ai login` | `oauth_devices`。 |
| `POST /worker/oauth/token` | `git-ai login` 轮询/刷新 | `refresh_tokens`，返回 Bearer token。 |
| `POST /worker/metrics/upload` | daemon / `flush-metrics-db` | `metrics_events`。 |
| `POST /worker/cas/upload` | daemon / `flush-cas` | `cas_objects`、`cas_ownership`、对象存储。 |
| `POST /api/v1/reports` | `git-ai report upload` | `projects`、`report_uploads`、`commit_stats`、`tool_model_stats`。 |
| `POST /api/v1/summaries` | `git-ai report summary --server` | `summary_uploads`。 |
| `POST /api/bundles` | `git-ai share` | `bundles`、`bundle_prompts`、`bundle_files`。 |

认证由 `enterprise-server/src/auth/middleware.rs` 处理。常见方式：

- CLI 登录后的 `Authorization: Bearer <token>`。
- API key：`X-API-Key: gai_...`。

普通成员通常只能看自己的数据；管理员/owner 或带 admin scope 的 API key 可看组织范围数据。

## Dashboard 聚合数据流

dashboard 页面是 `GET /me`，HTML/JS 由 `enterprise-server/src/handlers/dashboard.rs` 返回。页面请求以下聚合 API：

| Endpoint | 数据来源 |
| --- | --- |
| `/api/v1/aggregate/summary` | `metrics_events` + `commit_stats`。 |
| `/api/v1/aggregate/projects` | `metrics_events.repo_url` + `projects/commit_stats`。 |
| `/api/v1/aggregate/developers` | `metrics_events.author_email` + `commit_stats.author`。 |
| `/api/v1/aggregate/tools` | `metrics_events.tool_model_pairs` + `tool_model_stats`。 |
| `/api/v1/aggregate/trends` | 按时间聚合 `metrics_events.timestamp` 和 `commit_stats.author_time`。 |
| `/api/v1/aggregate/organizations` | `organizations` + metrics 数据。 |
| `/api/v1/aggregate/departments` | `departments` + org member/metrics 数据。 |

当前 dashboard 聚合会合并两类来源：

1. **metrics_events**：客户端自动/手动 flush 的事件流，是更实时的来源。
2. **commit_stats**：`git-ai report upload` 扫描 Git Notes 后上传的报表来源。

为避免同一 commit 重复统计，report 聚合侧会排除已经存在同 SHA `metrics_events` 的 `commit_stats` 行。

## 本地 Report Server 数据流

根 crate 也保留了一个轻量 report server：

```bash
git-ai report server --addr 127.0.0.1:8787 --db git-ai-report-server.sqlite
```

实现位于 `src/report/server.rs`。它使用 SQLite，接收 `/api/v1/reports` 和 `/api/v1/summaries`，适合本地或 MVP 场景。它不是 enterprise-server，不包含 OAuth、组织数据隔离、Postgres、Redis、MinIO。

## 自动与手动边界

| 用户动作 | 本地 authorship note | metrics 自动上传 | report 自动上传 | dashboard 可见条件 |
| --- | --- | --- | --- | --- |
| `git-ai checkpoint` | 否，只写 working log | 可能，取决于 daemon/认证 | 否 | checkpoint metrics 成功上传后可见部分使用数据。 |
| `git commit` | 是，写 `refs/notes/ai` | 可能，取决于 daemon/认证 | 否 | metrics 成功上传，或之后执行 report upload。 |
| `git push` | 不新增本地 note，只推 Git 对象 | 不由 push 直接触发 | 否 | push 本身不会把数据写入 enterprise-server。 |
| `git-ai flush-metrics-db` | 不变 | 手动补传 metrics | 否 | metrics 落入 `metrics_events` 后可见。 |
| `git-ai flush-cas` | 不变 | 不涉及 metrics | 否 | prompt 内容可通过 CAS 获取，但不增加统计行。 |
| `git-ai report upload ...` | 读取已有 notes | 不涉及 metrics | 是 | `commit_stats` 落库后可见。 |
| CI/CD 运行 report upload | 读取仓库 notes | 不涉及 metrics | 是 | CI 上传完成后可见。 |

因此，“提交后 dashboard 自动出现”依赖 metrics 这条链路；“push 后 dashboard 出现”默认不会发生，除非 CI/CD 在 push 后执行 `git-ai report upload` 或其他上传命令。

## 常见排查路径

1. **确认 Git wrapper 生效**

   ```bash
   command -v git
   git-ai --version
   ```

2. **确认 commit 有 authorship note**

   ```bash
   git notes --ref=refs/notes/ai show HEAD
   ```

3. **确认本地 report 能扫描到 commit**

   ```bash
   git-ai report scan . --range HEAD^..HEAD --json
   ```

4. **手动上传 report 到 enterprise-server**

   ```bash
   git-ai report upload . --range HEAD^..HEAD --server http://localhost:8080
   ```

5. **补传 metrics 队列**

   ```bash
   git-ai flush-metrics-db
   ```

6. **补传 CAS 队列**

   ```bash
   git-ai flush-cas
   ```

7. **确认 dashboard 认证**

   `/me` 页面需要 Bearer token 或 API key。CLI 的 `git-ai login --server ...` 会把 token 存在本机，浏览器页面仍需要通过登录页或输入 token/API key 建立浏览器会话。

