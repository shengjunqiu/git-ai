# 系统角色与使用方式

本文档从实际实现角度说明 `git-ai` 系统里的角色、各角色如何使用系统，以及这些角色之间的数据如何流转。它补充 `docs/architecture/data-flow.md` 的数据流视角，重点放在“谁在用系统、用系统做什么、系统内部如何响应”。

## 1. 系统定位

`git-ai` 不是一个单纯的 CLI，也不是一个单纯的 dashboard。它由几部分共同组成：

- 本地 `git-ai` CLI。
- 作为 `git` 调用时工作的 Git 代理。
- 本地 daemon 和本地 SQLite 队列。
- AI agent 和编辑器插件。
- Git Notes 中的 `authorship/3.0.0` 归属记录。
- 可选的企业服务端、CAS、metrics、report dashboard。

系统的核心目标是记录“代码行来自谁、由哪个 AI session 产生、对应哪段 prompt 或 transcript”，并在 commit、rebase、amend、stash、reset 等 Git 操作后尽量保持这些归属可追溯。

一个简化链路如下：

```text
Human / Agent / IDE
  -> git-ai checkpoint
  -> .git/ai/working_logs/<base_commit>/
  -> git commit
  -> refs/notes/ai
  -> blame/search/stats/continue/share/report
  -> optional enterprise server
```

## 2. 角色总览

| 角色 | 在系统中的身份 | 主要入口 | 主要产出或消费 |
| --- | --- | --- | --- |
| 普通开发者 | 人类作者、AI 使用者、代码消费者 | `git commit`、`git-ai blame`、`git-ai search`、`git-ai stats`、`/ask` | 人类修改、提交、查询归属和原始上下文 |
| AI agent | AI 代码作者 session | `git-ai checkpoint <agent>` | AI checkpoint、agent id、model、transcript、编辑文件 |
| 编辑器/agent 插件 | 事件桥接层 | VS Code extension、OpenCode plugin、Pi/Amp preset | 在保存、tool call 前后自动创建 checkpoint |
| Git 代理 | Git 命令拦截和归属维护层 | 作为 `git` shim 被调用 | pre/post hook、commit note、rewrite log、stats 输出 |
| Daemon | 异步处理和上传层 | `git-ai bg`、async mode control socket | 异步 checkpoint、telemetry、CAS/metrics flush |
| 代码审查者/维护者 | 归属和 intent 消费者 | `blame`、`search`、`show-prompt`、`continue`、`share` | 查看 AI 行、prompt、模型、决策背景 |
| 团队管理员 | 企业数据查看者和配置者 | 企业 dashboard、admin API | 用户、API key、组织级聚合、工具使用统计 |
| CI/自动化 | 非交互式上传者 | API key、`report upload`、CI helpers | 上传 report、metrics、CAS，支持组织级统计 |

## 3. 本地创作角色

### 3.1 普通开发者

普通开发者是系统中最重要的使用者。对他们而言，理想体验是“不改变 Git 工作流”：

```bash
git status
git add .
git commit -m "Implement feature"
```

当本机安装并启用 `git-ai` 后，普通 `git` 命令会通过 Git 代理执行。代理仍然调用真实 Git，只是在关键命令前后补充归属逻辑。

开发者实际会做几类事情：

1. 使用 AI agent 生成或修改代码。
2. 自己手动修改代码。
3. 保存文件、提交 commit。
4. 在提交后查看 AI/人类占比。
5. 在审查或维护时查询某行代码背后的 prompt 和 session。

常用命令：

```bash
git-ai blame src/foo.rs
git-ai blame src/foo.rs -L 20,60 --show-prompt
git-ai search --file src/foo.rs --lines 20-60 --verbose
git-ai stats HEAD
git-ai diff HEAD --include-stats --all-prompts
git-ai show-prompt <prompt_id>
git-ai continue --file src/foo.rs --lines 20-60 --agent claude --launch
```

普通开发者在归属模型里通常对应：

- `human_author`：Git author，例如 `Name <email>`。
- `CheckpointKind::Human`：系统无法确认是 AI 的普通工作区修改。
- `CheckpointKind::KnownHuman`：编辑器保存事件明确标记的人类修改。

`KnownHuman` 很关键。它用于把“AI 生成后又被人改过”的行从纯 AI 归属里切出来，避免把人类覆盖和修正仍然算作完整 AI 代码。

### 3.2 AI agent

AI agent 是代码生成方，但它不会直接写 Git Notes。它通过 preset 或插件触发 checkpoint，把一次 AI 操作的上下文交给 `git-ai`。

系统支持的 agent preset 包括：

- `claude`
- `codex`
- `continue-cli`
- `cursor`
- `gemini`
- `github-copilot`
- `amp`
- `windsurf`
- `opencode`
- `pi`
- `ai_tab`
- `firebender`
- `droid`

典型调用形态：

```bash
git-ai checkpoint codex --hook-input stdin
git-ai checkpoint opencode --hook-input stdin
git-ai checkpoint github-copilot --hook-input stdin
git-ai checkpoint ai_tab --hook-input stdin
```

agent 提交的核心字段包括：

| 字段 | 说明 |
| --- | --- |
| `agent_id.tool` | 工具名，例如 `codex`、`cursor`、`opencode`。 |
| `agent_id.id` | 外部 session/conversation id。 |
| `agent_id.model` | 模型名。 |
| `checkpoint_kind` | 通常是 `AiAgent` 或 `AiTab`。 |
| `transcript` | 可选 AI 对话内容。 |
| `edited_filepaths` | AI 已经编辑的文件。 |
| `will_edit_filepaths` | AI 即将编辑的文件，用于前置 human checkpoint。 |
| `dirty_files` | 插件捕获的文件内容，解决远程编辑器或文件系统延迟。 |

AI agent 的归属并不是根据最终 diff 猜测出来的，而是通过 agent/IDE hook 提供的编辑事件和 session 信息记录出来的。这也是系统与“AI 检测器”的核心区别。

### 3.3 AI tab completion

AI tab completion 是一个单独类型：`CheckpointKind::AiTab`。它适合 Cursor、VS Code、Copilot 一类内联补全场景。

它与普通 AI agent 的区别是：

- 粒度更小，通常是局部补全。
- 没有完整长对话 transcript。
- 仍然应该作为 AI 生成内容纳入 stats 和 blame。

VS Code/Cursor 扩展里的实验性 `aiTabTracking` 就是为这类场景准备的。

## 4. 插件角色

插件的职责是把外部工具事件翻译成 `git-ai checkpoint`。

### 4.1 编辑器插件

VS Code/Cursor/Windsurf 兼容扩展主要做三件事：

1. 监听保存事件，触发 `known_human` checkpoint。
2. 监听 AI 编辑或补全事件，触发 AI checkpoint。
3. 调用 `git-ai blame --json --contents -`，在编辑器里展示 gutter、hover 和 prompt。

保存事件的典型 payload：

```json
{
  "editor": "vscode",
  "editor_version": "...",
  "extension_version": "...",
  "cwd": "/path/to/repo",
  "edited_filepaths": ["/path/to/repo/src/foo.rs"],
  "dirty_files": {
    "/path/to/repo/src/foo.rs": "current file content"
  }
}
```

插件会调用：

```bash
git-ai checkpoint known_human --hook-input stdin
```

这类 checkpoint 的意义是明确告诉系统：“这次保存来自人类编辑器操作，不是 AI agent 的 tool call。”

### 4.2 Agent 插件

OpenCode、Pi、Amp 等 agent 插件通常围绕 tool call 生命周期工作：

```text
tool.execute.before
  -> human checkpoint 或 pre-snapshot
tool.execute.after
  -> AI checkpoint
```

对 edit/write/patch/multiedit/apply_patch 等工具，插件可以从 tool input 里直接提取文件路径。对 bash/shell 工具，文件路径不一定出现在输入里，系统使用 pre/post 文件快照推断哪些文件发生了变化。

OpenCode 的典型流程：

```text
PreToolUse
  -> git-ai checkpoint opencode --hook-input stdin
  -> 标记 AI 操作前已有改动为人类上下文

PostToolUse
  -> git-ai checkpoint opencode --hook-input stdin
  -> 标记 tool call 实际产生的文件改动为 AI
```

这种设计解决了一个常见问题：如果 AI agent 在已有脏工作区上继续修改，系统需要先把 AI 操作前的脏改动归到人类或上一轮上下文，再把 AI 操作后的增量归到本轮 AI session。

## 5. Git 代理角色

`src/main.rs` 根据进程名分派：

| 调用名 | 行为 |
| --- | --- |
| `git-ai` | 进入 `handle_git_ai`，处理 CLI 子命令。 |
| `git` 或其他 shim 名 | 进入 `handle_git`，代理真实 Git。 |
| deprecated hook binary name | 打印迁移提示。 |

Git 代理的原则是：真实 Git 行为仍由真实 Git 执行，`git-ai` 只在前后补充记录和同步。

### 5.1 Git 命令前后处理

代理会解析 Git 子命令，并对关键命令运行 pre/post 逻辑：

| Git 命令 | pre 逻辑 | post 逻辑 |
| --- | --- | --- |
| `commit` | 运行 pre-commit checkpoint | 生成 authorship note、stats、metrics |
| `rebase` | 记录原始 HEAD、onto 等上下文 | 重写或迁移 authorship |
| `reset` | 捕获 reset 前状态 | 更新 working log 或 attribution |
| `cherry-pick` | 记录源 commit 和原始 HEAD | 映射新旧 commit 归属 |
| `push` | 准备推送 authorship notes | 推送后收尾 |
| `pull`/`fetch` | 拉取 authorship notes 或捕获上下文 | fast-forward/rebase 后维护归属 |
| `stash` | 捕获未提交 attribution | stash 后恢复或迁移 attribution |
| `checkout`/`switch` | 捕获工作区状态 | 分支切换后迁移工作日志 |
| `update-ref` | 捕获 ref 更新上下文 | 处理底层 ref rewrite |

### 5.2 commit 期间发生什么

一次 `git commit` 的关键步骤：

```text
git commit
  -> git-ai wrapper
  -> commit_pre_command_hook
  -> authorship::pre_commit
  -> git-ai checkpoint
  -> real git commit
  -> commit_post_command_hook
  -> rewrite log event
  -> authorship::post_commit
  -> refs/notes/ai
  -> stats output
```

`post_commit` 会：

1. 读取 `.git/ai/working_logs/<base_commit>/`。
2. 更新 checkpoint 中最新 transcript。
3. 将 prompt 写入本地 prompts DB。
4. 通过 `VirtualAttributions` 计算最终提交中的行级归属。
5. 生成 `AuthorshipLog`。
6. 按配置处理 prompt messages：
   - `local`：messages 只留本地 SQLite，notes 中不写完整 transcript。
   - `notes`：脱敏后把 messages 写入 Git Notes。
   - `default`：尝试上传到 CAS，notes 中保留引用。
7. 写入 `refs/notes/ai`。
8. 计算并输出 commit stats。
9. 如果仍有未提交 AI attribution，写入新 commit 的 `INITIAL`。
10. 归档旧 working log。

## 6. 本地存储角色

### 6.1 Working Log

Working Log 是提交前的临时事实来源：

```text
.git/ai/
  working_logs/
    <base_commit>/
      checkpoints.jsonl
      blobs/
      INITIAL
  rewrite_log
  logs/
```

它记录“从 base commit 到当前工作区之间发生过什么”。commit 成功后，working log 被压缩成 Git Note。

### 6.2 Git Notes

最终固化结果在：

```text
refs/notes/ai
```

标准格式是 `authorship/3.0.0`，分为两段：

```text
<attestation-section>
---
<metadata-section>
```

attestation section 记录：

```text
src/foo.rs
  <prompt_hash> 1-3,8,10-12
```

metadata section 记录：

- `schema_version`
- `base_commit_sha`
- `prompts`
- 每个 prompt 的 `agent_id`
- `human_author`
- `messages`
- `total_additions`
- `total_deletions`
- `accepted_lines`
- `overridden_lines`

Git Notes 的好处是不会改写 commit 本身，也不会污染代码 diff。

### 6.3 Prompt DB 和 CAS

本地 prompt 数据主要存在 `~/.git-ai/internal/prompts.db`。在默认 prompt storage 模式下，完整 messages 不直接写入 Git Notes，而是：

```text
prompt messages
  -> secret redaction
  -> CAS hash / upload queue
  -> enterprise CAS or cloud CAS
  -> notes 中保留 messages_url
```

这让 repo 保持轻量，同时避免把敏感 prompt 原文默认写进 Git 历史可传播对象。

## 7. 查询和维护角色

### 7.1 查看每行是谁写的

代码审查者或维护者使用：

```bash
git-ai blame src/foo.rs
```

它是带 AI overlay 的 `git blame`。普通人类行显示 Git author；AI 行显示 tool/model/session 相关信息。加 `--show-prompt` 可以把 prompt hash 和 prompt dump 一起输出：

```bash
git-ai blame src/foo.rs -L 20,60 --show-prompt
```

### 7.2 搜索 prompt 历史

按 commit：

```bash
git-ai search --commit HEAD --verbose
```

按文件和行：

```bash
git-ai search --file src/foo.rs --lines 20-60 --verbose
```

按关键词：

```bash
git-ai search --pattern "retry logic" --verbose
```

按 prompt id：

```bash
git-ai search --prompt-id <id> --json
```

`search` 会优先从 Git Notes 找 commit/file/line 的归属，再用本地 DB 补齐可能被剥离的 messages。

### 7.3 继续历史 AI 会话

`continue` 用于把历史 prompt、commit diff、项目上下文和 Git 状态组合成新 agent 可读的上下文：

```bash
git-ai continue --commit HEAD --agent claude --launch
git-ai continue --file src/foo.rs --lines 20-60 --clipboard
git-ai continue --prompt-id <id> --json
```

这个角色面向“接手已有 AI 代码继续开发”的场景。

### 7.4 分享 prompt bundle

分享一个 prompt：

```bash
git-ai share <prompt_id> --title "Explain retry implementation"
```

分享前会脱敏 prompt 内容。可选地包含同 commit 的其他 prompt 或相关 diff。企业端通过 bundle API 创建可访问页面。

### 7.5 使用 `/ask`

安装时系统会把 `/ask` skill 放到 agent 可读取的 skills 目录。它的用法是：

```text
/ask Why is this function implemented this way?
```

当用户选中代码或指定文件行时，agent 会用 `git-ai search` 或 `git-ai blame --show-prompt` 找到原始 prompt，再结合源码回答“为什么这样写”，而不只是解释代码现在做什么。

## 8. 企业端角色

企业服务端引入了用户、组织、部门、成员角色和 API key scopes。它和本地 Git Notes 不是同一层：

- Git Notes 解决代码仓库中的行级归属。
- 企业服务端解决团队级数据上传、权限隔离、CAS、dashboard 聚合。

### 8.1 User

`User` 是登录主体，字段包括：

- `id`
- `email`
- `name`
- `personal_org_id`

一个用户可以属于多个组织。每个用户也可以有一个个人组织，用来承载个人数据。

### 8.2 Organization

`Organization` 是企业数据隔离的主边界。dashboard 聚合、API key、report、metrics 通常都会归属到某个组织。

### 8.3 Department

`Department` 属于组织，用于团队、部门维度聚合。dashboard 可以按部门比较 AI 使用率、提交量、AI 行数、人类行数等。

### 8.4 OrgMember

`OrgMember` 连接 user 和 organization，并带有角色：

| role | 含义 |
| --- | --- |
| `owner` | 组织所有者，拥有管理权限。 |
| `admin` | 管理员，可看组织范围数据，可管理用户/API key。 |
| `member` | 普通成员，只看自己的数据。 |

服务端判断管理员的核心逻辑是：

```text
role == "owner" or role == "admin" or API key scopes contains "admin"
```

### 8.5 API Key

API key 用于非交互式场景，例如 CI、自动化上传、企业 agent。它关联：

- `user_id`
- 可选 `org_id`
- `scopes`
- `expires_at`
- `revoked_at`

常见 scopes：

- `metrics:write`
- `cas:write`
- `cas:read`
- `reports:write`
- `admin`

客户端配置方式：

```bash
git-ai config set api_base_url https://your-enterprise-server.com
git-ai config set api_key your-api-key
```

也可以使用环境变量：

```bash
export GIT_AI_API_BASE_URL=https://your-enterprise-server.com
export GIT_AI_API_KEY=your-api-key
```

### 8.6 Dashboard 权限

dashboard 通过 Bearer token、cookie 或 API key 识别身份。

数据可见范围：

| 身份 | 可见范围 |
| --- | --- |
| admin/owner | 当前组织内所有用户数据。 |
| member | 当前组织内自己的数据。 |
| 带 `admin` scope 的 API key | 对应组织内所有用户数据。 |
| 默认 API key | 对应 user/org 的数据，按 scope 限制写入和读取。 |

UI 层也会隐藏非管理员的 admin sections，例如用户管理和 API key 管理入口。

## 9. CI 和自动化角色

CI 不需要交互式登录，通常使用 API key。

常见用途：

1. 上传报告：

```bash
git-ai report upload . \
  --range main..HEAD \
  --server https://your-enterprise-server.com
```

2. 上传 summary：

```bash
git-ai report summary . \
  --server https://your-enterprise-server.com \
  --organization "Example Org" \
  --department "Platform" \
  --reporter "CI" \
  --reporter-email "ci@example.com"
```

3. 在 GitHub Actions 或其他 CI 中检查/补齐企业可见数据。

CI 角色一般不会生成 line attribution。line attribution 仍然来自开发者本地的 checkpoint 和 Git Notes。CI 更多是消费 Git Notes、扫描 repo、上传聚合数据。

## 10. 典型使用场景

### 10.1 单人本地开发

```text
开发者安装 git-ai
  -> 使用 Codex/Cursor/Claude 修改代码
  -> 插件自动 checkpoint
  -> 开发者手动修正并保存
  -> known_human checkpoint
  -> git commit
  -> refs/notes/ai
  -> git-ai stats/blame/search
```

适合目标：

- 看 AI 生成代码比例。
- 保留 prompt 和意图。
- 以后能通过 `/ask` 问原始 agent。

### 10.2 团队协作和 code review

```text
开发者 push commit + refs/notes/ai
  -> reviewer fetch notes
  -> git-ai blame/search
  -> 查看 AI 行背后的 prompt
  -> 需要时 share bundle
```

适合目标：

- 审查 AI 生成代码的上下文。
- 判断某段代码是纯 AI、AI 后人改、还是人类写的。
- 复盘某个实现选择来自哪条需求或 prompt。

### 10.3 企业统计

```text
本地 commit/checkpoint 产生 metrics
  -> daemon flush metrics/CAS
  -> enterprise-server
  -> dashboard 聚合组织、部门、项目、开发者、工具和模型
```

适合目标：

- 衡量 AI adoption。
- 比较 agent 和 model。
- 看 AI 代码被接受、被改写、进入提交后的保留情况。
- 识别团队使用 AI 的模式。

### 10.4 历史代码接手

```text
维护者看到一段陌生代码
  -> git-ai blame file -L start,end --show-prompt
  -> git-ai search --file file --lines start-end --verbose
  -> /ask 询问原始上下文
  -> git-ai continue 继续 session 或复制上下文给新 agent
```

适合目标：

- 不只知道代码“做什么”，还知道“为什么这么做”。
- 把历史 prompt 和 commit diff 转成新 agent 的上下文。

## 11. 角色之间的边界

### 11.1 AI 归属不是根据内容猜测

系统依赖 agent/editor hook 和 checkpoint，不靠文本特征判断一行是不是 AI 写的。没有 checkpoint 的行不会被强行判定为 AI。

### 11.2 Git Notes 是权威提交归属

提交前的 working log 是临时事实来源。提交后，`refs/notes/ai` 是 commit 级查询的主要来源。

### 11.3 Transcript 不一定在 Git Notes 里

根据配置不同，prompt messages 可能：

- 只在本地 SQLite。
- 脱敏后写入 Git Notes。
- 上传到 CAS，Git Notes 只保留引用。

所以看到 AI 行但看不到完整 prompt，不一定代表没有归属，可能只是 prompt storage 策略或权限问题。

### 11.4 企业服务不是本地归属的前置依赖

本地 `blame`、`stats`、`search` 可以只靠 Git Notes 和本地 DB 工作。企业服务提供的是跨用户、跨项目、跨组织的聚合、CAS 和 dashboard 能力。

## 12. 常用命令速查

### 开发者

```bash
git-ai install-hooks
git-ai status
git-ai blame src/foo.rs
git-ai stats HEAD
git-ai search --file src/foo.rs --lines 20-60 --verbose
git-ai show-prompt <prompt_id>
git-ai continue --prompt-id <prompt_id> --agent claude --launch
```

### Agent 或插件

```bash
git-ai checkpoint codex --hook-input stdin
git-ai checkpoint opencode --hook-input stdin
git-ai checkpoint known_human --hook-input stdin
git-ai checkpoint ai_tab --hook-input stdin
```

### 团队和企业

```bash
git-ai login
git-ai whoami
git-ai report scan .
git-ai report export . --format json --output report.json
git-ai report upload . --server https://your-enterprise-server.com
git-ai flush-cas
git-ai flush-metrics-db
```

### Git Notes 同步

```bash
git-ai fetch-notes
git-ai fetch-authorship-notes
git-ai push-authorship-notes
```

## 13. 代码位置索引

| 主题 | 主要文件 |
| --- | --- |
| 入口分派 | `src/main.rs` |
| `git-ai` CLI 子命令 | `src/commands/git_ai_handlers.rs` |
| Git 代理 | `src/commands/git_handlers.rs` |
| commit hook | `src/commands/hooks/commit_hooks.rs` |
| checkpoint 主实现 | `src/commands/checkpoint.rs` |
| agent preset | `src/commands/checkpoint_agent/` |
| working log 数据结构 | `src/authorship/working_log.rs` |
| pre-commit 归属 | `src/authorship/pre_commit.rs` |
| post-commit note 生成 | `src/authorship/post_commit.rs` |
| Git Notes 读写 | `src/git/refs.rs` |
| rewrite log | `src/git/rewrite_log.rs` |
| blame | `src/commands/blame.rs` |
| search | `src/commands/search.rs` |
| continue | `src/commands/continue_session.rs` |
| share | `src/commands/share.rs` |
| report | `src/commands/report.rs`、`src/report/` |
| VS Code/Cursor 插件 | `agent-support/vscode/` |
| OpenCode 插件 | `agent-support/opencode/` |
| Pi 插件 | `agent-support/pi/` |
| 企业用户模型 | `enterprise-server/src/models/user.rs` |
| 企业认证中间件 | `enterprise-server/src/auth/middleware.rs` |
| 企业 dashboard | `enterprise-server/src/handlers/dashboard.rs` |
| 数据格式标准 | `specs/git_ai_standard_v3.0.0.md` |

## 14. 阅读顺序建议

如果要继续理解或改造系统，建议按这个顺序读：

1. `docs/architecture/repository-layout.md`：先看仓库边界。
2. `docs/architecture/data-flow.md`：理解数据从 checkpoint 到 Git Notes 再到企业服务的流向。
3. 本文档：理解系统中每个角色如何参与。
4. `src/main.rs`、`src/commands/git_handlers.rs`：理解入口和 Git 代理。
5. `src/commands/checkpoint.rs`、`src/authorship/post_commit.rs`：理解归属生成。
6. `agent-support/vscode/` 和 `agent-support/opencode/`：理解外部工具如何接入。
7. `enterprise-server/src/models/user.rs` 和 `enterprise-server/src/handlers/dashboard.rs`：理解企业权限和 dashboard。
