# Windows Trae、CodeBuddy、Qoder Hook 归属修复任务计划

## 1. 文档目的

本文档把 Windows 上 Trae、CodeBuddy 和 Qoder 将 AI 代码归属为人工代码的问题，拆分为可按顺序执行、可独立验证的修复任务。

执行者必须逐项完成任务并记录测试结果。只有真实的 `PostToolUse` Hook 成功写入 `AiAgent` checkpoint，并且提交后的行级归属正确，才能认为问题已修复。仅验证配置文件中出现了 Hook 命令，或仅验证命令字符串格式，不算完成。

## 2. 已确认的问题链路

当前 Windows 故障链路如下：

```text
Agent 修改文件
  -> PostToolUse Hook 应调用 git-ai checkpoint
  -> Hook 命令启动失败、未安装或 payload 被跳过
  -> 没有写入 AiAgent checkpoint
  -> 编辑器保存事件继续写入 KnownHuman checkpoint
  -> AI 修改最终被归属为人工代码
```

已确认的三个直接问题：

| Agent | 当前故障 | 证据 | 优先级 |
| --- | --- | --- | --- |
| Trae | Windows Hook 被渲染为 Git Bash 的 `/c/...` 路径，但 Trae 实际使用 PowerShell | `src/mdm/agents/trae.rs` 使用 `HookShell::GitBash`；Trae Windows 运行日志出现 `CommandNotFoundException` | P0 |
| CodeBuddy | IDE Hook 收到 Git Bash 的 `/c/...` 路径，Windows IDE 启动命令失败 | `src/mdm/agents/codebuddy.rs` 使用 `HookShell::GitBash`；IDE 日志返回错误码 1 和“系统找不到指定的路径” | P0 |
| CodeBuddy | IDE payload 使用 `write_to_file`、`replace_in_file`、`execute_command`，分类器只识别 CLI 名称 | `src/commands/checkpoint_agent/bash_tool.rs` 只识别 `Write`、`Edit`、`Bash` 等 | P0 |
| Qoder | Qoder CN 的 `QoderCN.exe`、`QoderCN` 进程和自定义安装目录未被检测，Hook 没有安装 | `src/mdm/agents/qoder.rs` 只检测 `Qoder.exe` 和 `qoder`；用户级 `.qoder/settings.json` 未生成 | P0 |
| 公共错误处理 | preset 解析失败后打印错误并 `exit(0)`，调用方容易把真实失败视为成功 | `src/commands/git_ai_handlers.rs` | P1 |

## 3. 最终完成标准

所有任务完成后必须满足：

- [ ] Trae 在 Windows PowerShell 环境中可以从 Hook 配置成功启动 `git-ai.exe`。
- [ ] CodeBuddy IDE 和 CodeBuddy CLI 在 Windows 上都可以成功启动同一版本的 `git-ai.exe`，不能修好一个而破坏另一个。
- [ ] CodeBuddy IDE 的文件创建、文件编辑和终端工具都能被正确分类。
- [ ] Qoder 和 Qoder CN 均可被检测；默认安装、自定义盘符安装和运行中进程至少有一种稳定检测路径。
- [ ] 三个 Agent 的 `PostToolUse` 均能写入 `AiAgent` checkpoint。
- [ ] 即使编辑器随后写入 `KnownHuman` checkpoint，AI 修改行仍保持 AI 归属。
- [ ] 提交后 `authorship/3.0.0` Git Note 中的行级归属正确。
- [ ] macOS、Linux 和 Windows 的现有 Agent 集成没有回归。
- [ ] Windows 测试真实执行生成后的 Hook 命令，不只比较字符串。

## 4. 执行规则

1. 开始每个任务前运行 `git status --short`，保留用户已有修改。
2. 每个任务只处理本文档指定范围；测试通过后再进入下一任务。
3. 路径写入 working log、authorship 或 Git Note 前继续保持 POSIX 化；Hook 命令的 shell 转义与归属数据路径格式是两个不同问题，不要混合修改。
4. Windows 专属代码使用明确的 `#[cfg(windows)]`；非 Windows 分支必须保留。
5. 优先使用 `task format`、`task lint`、`task build` 和 `task test`。若开发环境没有 Task，只能临时使用等价 Cargo 命令，并在执行记录中说明。
6. 不使用 `task test:wrapper-daemon` 或 `task test:wrapper`，除非后续任务明确要求或用户单独批准。
7. 每个 Agent 的修复建议独立提交，便于回滚和定位回归。


## 5. 阶段 0：保存基线和可复现证据

### Task 0.1：记录 Windows 环境基线

改动范围：无代码修改。


执行步骤：

1. 记录工具链和 shell：

   ```powershell
   git status --short
   rustc --version
   cargo --version
   task --version
   git --version
   $PSVersionTable.PSVersion
   Get-Command bash.exe -ErrorAction SilentlyContinue
   ```

2. 记录三个 Agent 的版本、安装路径和进程名。
3. 备份以下配置；不存在时记录为“未生成”：

   ```text
   %USERPROFILE%\.trae\hooks.json
   %USERPROFILE%\.trae-cn\hooks.json
   %USERPROFILE%\.codebuddy\settings.json
   %USERPROFILE%\.qoder\settings.json
   ```

4. 在临时 Git 仓库中分别让三个 Agent 修改一个独立文件。
5. 保存 Agent Hook 日志、`.git/ai/working_logs/**/checkpoints.jsonl` 和提交后的 `refs/notes/ai`。

验收标准：

- [x] 每个 Agent 都有一份包含时间、Agent 版本、命令、退出码和工作目录的复现记录。
- [x] 能明确区分“Hook 未安装”“Hook 未启动”“payload 被跳过”和“checkpoint 写入后归属错误”。

### Task 0.2：固化最小 Hook payload 夹具

改动范围：

- `tests/fixtures/`，建议新增 `tests/fixtures/agent-hooks/`。
- 必要时更新 `tests/integration/trae.rs`、`tests/integration/codebuddy.rs`、`tests/integration/qoder.rs`。

执行步骤：

1. 从日志中脱敏保存三个 Agent 的最小 `PreToolUse` 和 `PostToolUse` JSON。
2. 每份夹具保留以下字段：
   - `session_id`
   - `cwd`
   - `hook_event_name`
   - `tool_name`
   - `tool_input`
   - `tool_response` 或实际 Agent 提供的等价路径字段
3. 至少保存这些工具名：
   - Trae：`Write`、`Edit`、`RunCommand`
   - CodeBuddy CLI：`Write`、`Edit`、`Bash`
   - CodeBuddy IDE：`write_to_file`、`replace_in_file`、`execute_command`
   - Qoder：`create_file`、`search_replace`、`run_in_terminal`
4. 删除用户邮箱、提示词正文、真实仓库名称和不必要的绝对路径。

验收标准：

- [x] 夹具可以稳定复现当前分类行为；三个 fixture 消费测试已在原生 Windows MSVC Rust 工具链上通过。
- [x] 测试不依赖开发者机器上的 Agent 日志。
- [x] 夹具使用 Windows 路径时，同时覆盖反斜杠、盘符和包含空格的目录。

## 6. 阶段 1：建立真实的 Windows Hook 命令测试

### Task 1.1：增加 shell 执行测试工具

改动范围：

- `src/mdm/command_line.rs` 的测试模块，或新的 Windows 专用测试模块。
- 必要时增加 `tests/integration/windows_hook_commands.rs`。

执行步骤：

1. 保留现有 `render_hook_command` 字符串单元测试。
2. 新增 `#[cfg(windows)]` 测试，把生成后的命令交给真实 shell：
   - PowerShell：`powershell.exe -NoProfile -NonInteractive -Command ...`
   - Cmd：`cmd.exe /D /S /C ...`
   - Git Bash：使用已检测到的 `bash.exe -lc ...`
3. 测试目标使用一个记录 argv、stdin、cwd 和退出码的小型测试二进制，不能只调用 `echo`。
4. 测试路径至少覆盖：

   ```text
   C:\Users\Test User\.git-ai\bin\git-ai.exe
   C:\Users\A&B\.git-ai\bin\git-ai.exe
   C:\Users\100% Dev\.git-ai\bin\git-ai.exe
   C:\Users\O'Neil\.git-ai\bin\git-ai.exe
   D:\Tools\git ai\git-ai.exe
   ```

5. stdin 使用真实 Hook JSON，验证没有被 shell 截断或改写。

验收标准：

- [x] 每种声明支持的 shell 都真实启动了测试二进制。
- [x] argv、stdin 和 cwd 与输入完全一致。
- [x] 测试能够在修复前捕获 Trae 和 CodeBuddy IDE 的 `/c/...` 启动失败。
- [x] Windows CI 缺少声明为必需的 shell 时直接失败，不静默跳过。

## 7. 阶段 2：修复 Trae Windows Hook

### Task 2.1：按平台选择 Trae Hook shell

改动范围：

- `src/mdm/agents/trae.rs`
- `src/mdm/command_line.rs` 中的 Agent runtime 注释或映射
- Trae installer 单元测试

执行步骤：

1. 将 Trae 的运行环境映射改为：
   - Windows：PowerShell
   - macOS/Linux：POSIX shell
2. 使用统一渲染入口生成命令；Windows 不再把盘符转换成 `/c/...`。
3. 确认包含空格的可执行文件使用 PowerShell call operator，例如：

   ```powershell
   & 'C:\Users\Test User\.git-ai\bin\git-ai.exe' checkpoint trae --hook-input stdin
   ```

4. 更新 installer 的 up-to-date 判断，使旧 Git Bash 命令被识别为需要升级。
5. 保证重复安装不会产生重复 Hook。
6. 保证卸载逻辑既能删除旧命令，也能删除新命令，但不删除用户自己的 Hook。

建议测试：

```powershell
task test TEST_FILTER=mdm::agents::trae
task test TEST_FILTER=mdm::command_line
```

验收标准：

- [x] Windows 生成 PowerShell/native Windows 路径，配置中不再出现 `/c/.../git-ai.exe`。
- [x] PowerShell 真实执行测试通过。
- [x] 旧配置升级后只保留一个 PreToolUse 和一个 PostToolUse Git AI Hook。
- [x] macOS/Linux 生成结果保持兼容。

### Task 2.2：验证 Trae 归属闭环

执行步骤：

1. 使用本地 debug 构建重新安装 Hook 并重启 Trae。
2. 让 Trae 创建文件并编辑已有文件。
3. 检查 Hook 日志退出码为 0，且没有 `CommandNotFoundException`。
4. 检查 working log 同时包含：
   - Agent 修改前的基线 checkpoint
   - `PostToolUse` 产生的 `AiAgent` checkpoint
5. 提交并检查行级归属。

验收标准：

- [ ] 新建行和编辑行被归属为 Trae AI。
- [ ] 未被 Trae 修改的人工行保持人工归属。

## 8. 阶段 3：修复 CodeBuddy Windows Hook

### Task 3.1：区分 CodeBuddy IDE 与 CLI 的执行环境

改动范围：

- `src/mdm/agents/codebuddy.rs`
- `src/mdm/command_line.rs`
- CodeBuddy Hook installer 测试

执行步骤：

1. 根据官方文档和实机日志建立明确矩阵：

   | 产品 | Windows Hook shell | 工具名风格 |
   | --- | --- | --- |
   | CodeBuddy IDE / Craft Agent | 以实机日志和安装包实现为准，不能假定 Git Bash | IDE 风格 |
   | CodeBuddy CLI | Git Bash | CLI 风格 |

2. 确认 IDE 与 CLI 是否读取同一个 `.codebuddy/settings.json`。
3. 如果配置文件不同，分别生成对应 shell 命令。
4. 如果共享同一个配置文件，选择并实现以下一种方案：
   - 生成可被 IDE 和 Git Bash 共同启动的 Windows launcher；或
   - 安装一个不依赖调用方 shell 路径语法的稳定包装器，并让两者调用该包装器。
5. 不允许继续用一个未经真实执行验证的 `HookShell::GitBash` 假设覆盖 IDE 和 CLI。
6. 更新旧命令识别、幂等安装和卸载测试。

验收标准：

- [ ] CodeBuddy IDE 的真实日志中 Hook 退出码为 0。
- [x] CodeBuddy CLI 的 Hook 仍能在 Git Bash 中执行。
- [x] 两者使用同一配置时，重装不会互相覆盖成不可执行命令。
- [x] 包含空格和特殊字符的 Windows 路径测试通过。

### Task 3.2：支持 CodeBuddy IDE 工具名

改动范围：

- `src/commands/checkpoint_agent/bash_tool.rs`
- `src/commands/checkpoint_agent/agent_presets/codebuddy.rs`
- `tests/integration/codebuddy.rs`

执行步骤：

1. 为 CodeBuddy 增加官方 IDE 别名：
   - `write_to_file` -> `FileEdit`
   - `replace_in_file` -> `FileEdit`
   - `execute_command` -> `Bash`
2. 保留现有 CLI 名称支持。
3. 使用 Task 0.2 的 IDE payload 夹具测试 PreToolUse 和 PostToolUse。
4. 对文件工具验证能提取正确路径；对终端工具验证 sidecar/dirty-file 流程。
5. 未知只读工具应正常跳过，不能误写 AI checkpoint。

建议测试：

```powershell
task test TEST_FILTER=test_tool_classification_all_agents
task test TEST_FILTER=codebuddy
```

验收标准：

- [x] CodeBuddy CLI 和 IDE 的写入、编辑、终端工具都有测试。
- [x] IDE `PostToolUse` 生成 `CheckpointKind::AiAgent`。
- [x] 只读工具仍为 Skip，人工编辑不会被扩大标记为 AI。

### Task 3.3：验证 CodeBuddy 归属闭环

按照 Task 2.2 的流程分别验证 CodeBuddy IDE 和 CLI。若当前产品只安装了 IDE，CLI 验证必须由 Windows CI 或另一台装有 CLI 的机器完成，不能标记为“默认通过”。

验收标准：

- [ ] IDE 创建和编辑的代码被归属为 AI。
- [ ] CLI 创建和编辑的代码被归属为 AI。
- [ ] IDE 与 CLI 的 Hook 日志均无路径启动错误。

## 9. 阶段 4：修复 Qoder CN 检测和 Hook 安装

### Task 4.1：扩展 Qoder Windows 安装检测

改动范围：

- `src/mdm/agents/qoder.rs`
- Qoder installer 单元测试

执行步骤：

1. 增加可执行文件和进程名：
   - `Qoder.exe`
   - `QoderCN.exe`
   - `qoder`
   - `Qoder`
   - `QoderCN`
2. 扩展默认候选目录，覆盖 Qoder 与 Qoder CN 的常见用户级和机器级安装。
3. 增加 PATH 检测；PATH 中命令或可执行文件存在时应视为已安装。
4. 不要把某个开发者的 `D:\QoderCN` 写死到生产代码。自定义盘符通过 PATH、进程可执行路径或可复用的注册信息检测。
5. 让 `tasklist_contains_qoder` 覆盖带 `.exe` 和不带 `.exe` 的 CN 名称。
6. 更新 `process_names()`，保证 Hook 更新流程能识别运行中的 Qoder CN。
7. 覆盖“应用已安装但 `.qoder` 目录尚不存在”的首次安装场景。

建议测试：

```powershell
task test TEST_FILTER=windows_app_candidates_cover_per_user_and_machine_installs
task test TEST_FILTER=tasklist_contains_qoder
task test TEST_FILTER=mdm::agents::qoder
```

验收标准：

- [x] `QoderCN.exe` 正在运行时 `tool_installed` 为 true。
- [x] Qoder CN 位于自定义盘符且安装目录在 PATH 时 `tool_installed` 为 true。
- [x] 未安装任何 Qoder 产品时不产生误报。
- [x] 首次安装成功创建 `%USERPROFILE%\.qoder\settings.json`。

### Task 4.2：确认并修复 Qoder Windows Hook shell

改动范围：

- `src/mdm/agents/qoder.rs`
- `src/mdm/command_line.rs` 的 runtime 映射
- Qoder installer 测试

执行步骤：

1. 在安装检测修复后，用最小 Hook 捕获 Qoder CN 实际执行环境和原始错误输出。
2. 不根据 CodeBuddy CLI 或 macOS 行为推断 Qoder shell。
3. 根据实机结果选择 Git Bash、PowerShell、Cmd 或稳定 launcher。
4. 用 Task 1.1 的真实 shell 测试执行最终命令。
5. 更新旧命令迁移、幂等安装和卸载测试。

验收标准：

- [ ] Qoder 和 Qoder CN 的 Hook 日志都证明 `git-ai.exe` 被实际启动。
- [x] 配置中的命令与实机 shell 一致。
- [x] `create_file`、`search_replace` 和 `run_in_terminal` payload 均可进入 preset。

### Task 4.3：验证 Qoder 归属闭环

按照 Task 2.2 的流程验证 Qoder CN；如同时支持国际版 Qoder，还需在 CI 或测试机上验证国际版。

验收标准：

- [ ] Qoder CN 的创建、编辑和终端修改均写入 AI checkpoint。
- [ ] 提交后的相应行归属为 Qoder AI。

## 10. 阶段 5：修复 Hook 错误语义和可观测性

### Task 5.1：区分正常跳过与真实失败

改动范围：

- `src/commands/git_ai_handlers.rs`
- `src/commands/checkpoint_agent/agent_presets/*.rs`
- `src/error.rs`，仅在现有 `GitAiError` 无法表达时扩展

执行步骤：

1. 列出 checkpoint handler 中所有 `process::exit(0)`。
2. 将结果划分为：
   - 正常跳过：只读工具、不支持的事件、不影响文件的工具调用。
   - 真实失败：stdin 读取失败、JSON 无效、必要字段缺失、路径解析失败、checkpoint 写入失败。
3. 正常跳过保持退出码 0，但输出可检索的 debug 日志。
4. 真实失败返回非零退出码，并包含 Agent、事件、工具名和失败阶段；不要输出完整提示词或敏感 payload。
5. 保证非零退出不会阻止 Agent 正常工作：三方 Hook 都应按“非阻塞错误”配置或由宿主自然降级。
6. 为错误码和 stderr 增加测试。

验收标准：

- [ ] 无效 JSON 和 checkpoint 写入失败不能再返回 0。
- [ ] 未知只读工具仍可正常跳过。
- [ ] 日志可以判断失败发生在“启动、解析、分类、路径提取、写入”中的哪一步。

### Task 5.2：增加 Hook 健康检查输出

改动范围：

- Hook installer/status 相关命令。
- 必要时更新用户文档。

执行步骤：

1. 状态输出至少区分：Agent 未检测、配置缺失、命令过期、命令不可执行、最近执行未知。
2. Windows 上显示已选择的 shell 类型和渲染后的可执行路径，但不要输出敏感 stdin。
3. 为 Trae、CodeBuddy、Qoder 增加可操作的修复提示。

验收标准：

- [ ] 用户不需要翻 IDE 内部日志就能发现 Hook 未安装或命令格式过期。
- [ ] 状态命令不会修改配置。

## 11. 阶段 6：增加归属端到端回归测试

### Task 6.1：增加 Agent Hook 到 Git Note 的闭环测试

改动范围：

- `tests/integration/trae.rs`
- `tests/integration/codebuddy.rs`
- `tests/integration/qoder.rs`
- 必要时增加共享测试工具到 `tests/integration/repos/`

执行步骤：

1. 为每个 Agent 创建真实 Git 仓库。
2. 手动写入初始人工内容并调用 `mock_known_human` 或等价测试工具。
3. 显式调用 PreToolUse payload。
4. 修改文件，模拟 Agent 的创建、替换和终端修改。
5. 显式调用 PostToolUse payload。
6. 在 PostToolUse 后再模拟一次编辑器 `KnownHuman` 保存事件，覆盖真实竞争顺序。
7. 提交文件。
8. 断言：
   - working log 中存在对应 Agent 的 `AiAgent` checkpoint；
   - AI 修改行是 AI；
   - 原始人工行是人工；
   - Git Note 使用 `authorship/3.0.0`；
   - 路径在持久化数据中为 POSIX 格式。

注意：不要依赖 `file.set_contents` 的简化归属流程；按仓库测试规范手动写文件，并显式调用 `mock_known_human`、`human`、`mock_ai` 或 Agent preset。

验收标准：

- [ ] 三个 Agent 都有创建文件和编辑已有文件的闭环测试。
- [ ] CodeBuddy IDE 别名有独立闭环测试。
- [ ] `KnownHuman` 在 AI Hook 之后触发时不会覆盖已确认的 AI 行。

### Task 6.2：增加原生 Windows CI 验证

改动范围：

- 现有 GitHub Actions Windows workflow，优先扩展已有安装或集成测试工作流。

执行步骤：

1. 使用 Windows runner 构建 `git-ai.exe`。
2. 把二进制复制到包含空格和特殊字符的临时 HOME。
3. 生成三种 Agent 配置。
4. 从配置 JSON 中读取最终 Hook 命令，并交给对应真实 shell 执行。
5. 通过 stdin 发送脱敏 fixture。
6. 检查退出码、working log 和最终行级归属。
7. 至少保留 PowerShell、CodeBuddy IDE 实际 runner、CodeBuddy CLI Git Bash 和 Qoder 实际 runner 的验证结果。

验收标准：

- [ ] CI 不是重新拼一条“预期命令”，而是执行 installer 实际写入配置的命令。
- [ ] Hook 未启动或没有生成 `AiAgent` checkpoint 时 CI 失败。
- [ ] Windows 路径至少覆盖空格和 `&`。

## 12. 阶段 7：完整回归与发布准备

### Task 7.1：运行完整质量检查

建议命令：

```powershell
task format
task lint
task build
task test
```

并在原生 Windows 上执行本文档新增的 Hook 命令测试和三 Agent 闭环测试。macOS、Linux CI 也必须通过。

验收标准：

- [ ] `task format` 无额外变更。
- [ ] `task lint` 通过且无 warning。
- [ ] `task build` 通过。
- [ ] `task test` 通过；任何已知基线失败均有独立记录，不能笼统标记为通过。
- [ ] Windows、macOS、Linux CI 全部通过。

### Task 7.2：验证升级和回滚

执行步骤：

1. 从包含旧 `/c/...` 命令的配置升级到新版本。
2. 确认安装器替换旧 Git AI Hook，不影响用户 Hook。
3. 连续运行两次安装，确认幂等。
4. 运行卸载，确认只删除 Git AI Hook。
5. 降级旧版本时记录已知行为；如果无法兼容，必须写入发布说明。

验收标准：

- [ ] 升级无需用户手工编辑 JSON。
- [ ] 不产生重复 PreToolUse/PostToolUse Hook。
- [ ] 用户自定义 Hook 完整保留。
- [ ] 发布说明明确要求重启哪些 IDE，以及何时重新安装 Hook。

## 13. 建议提交拆分

建议按以下顺序提交：

1. `Add Windows agent hook execution fixtures`
2. `Fix Trae PowerShell hook command`
3. `Fix CodeBuddy Windows hook launchers`
4. `Support CodeBuddy IDE tool names`
5. `Detect Qoder CN installations`
6. `Fix Qoder Windows hook command`
7. `Report agent checkpoint hook failures`
8. `Add Windows agent attribution end-to-end tests`

不要把三个 Agent、错误语义和 CI 全部压入一个提交。

## 14. 执行记录模板

每完成一个 Task，追加以下记录：

```markdown
### Task X.Y 执行记录：YYYY-MM-DD

- 执行人：
- 平台与版本：
- Agent 与版本：
- 修改文件：
- 执行命令：
- 测试结果：
- 生成的 Hook 命令：
- working log 是否包含 AiAgent：
- 提交后行级归属结果：
- 未解决风险：
- 对应提交：
```

只有“验收标准”全部勾选，并填写执行记录后，任务才可标记完成。

## 15. 阶段 0 执行记录：2026-07-16

阶段状态：**已完成**。Task 0.1 的基线和复现记录已保存；Task 0.2 的 fixture、测试代码、静态校验和原生 Windows 定向测试均已完成。

### Task 0.1 执行记录：2026-07-16

- 执行平台：Microsoft Windows NT 10.0.20348.0，x64。
- PowerShell：5.1.20348.2849。
- Git：2.55.0.windows.3，由 `C:\Users\admin\.git-ai\bin\git.exe` 提供。
- 仓库 HEAD：`746199a`。
- 阶段执行开始时的 Rust/Task 状态：`rustc`、`cargo`、`rustup`、`rustfmt` 和 `task` 均不在 PATH，常见的 `%USERPROFILE%\.cargo\bin` 也不存在。
- Git Bash：`C:\Program Files\Git\bin\bash.exe` 存在，但 `bash.exe` 不在 PowerShell PATH。
- 执行前工作区：只有本任务新增且尚未跟踪的计划文档。

Agent 基线：

| Agent | 版本 | 安装路径 | 运行进程 |
| --- | --- | --- | --- |
| Trae CN | 3.3.75 | `D:\Trae CN\Trae CN.exe` | `Trae CN` |
| CodeBuddy CN | 4.10.2 | `D:\CodeBuddy CN\CodeBuddy CN.exe` | `CodeBuddy CN` |
| Qoder CN | 1.7.0 | `D:\QoderCN\QoderCN.exe` | `QoderCN` |

配置基线：

| 配置 | 状态 | SHA-256 / Hook 命令 |
| --- | --- | --- |
| `%USERPROFILE%\.trae\hooks.json` | 不存在 | - |
| `%USERPROFILE%\.trae-cn\hooks.json` | 存在 | SHA-256 `6E9187F7F7FDEB633FCC5C90DBBD147F3F5A9127E2EDAF8A406E3B40A4CD0396`；Pre/Post 均为 `/c/Users/admin/.git-ai/bin/git-ai.exe checkpoint trae --hook-input stdin` |
| `%USERPROFILE%\.codebuddy\settings.json` | 存在 | SHA-256 `4965232FCD0546F033A9F9CE6C5E887DB714B9D094021B45C7792697DA8A229A`；Pre/Post 均为 `/c/Users/admin/.git-ai/bin/git-ai.exe checkpoint codebuddy --hook-input stdin` |
| `%USERPROFILE%\.qoder\settings.json` | 不存在 | - |

存在的配置已备份到当前用户临时目录：

```text
C:\Users\admin\AppData\Local\Temp\1\git-ai-20260716-stage0-agent-configs
```

没有修改任何 Agent 用户配置。

复现证据：

| Agent | 工作目录 | 复现分类 | 命令、退出状态和结果 |
| --- | --- | --- | --- |
| Trae | `D:\code\git-ai-test` | Hook 未启动 | 命令为 `/c/Users/admin/.git-ai/bin/git-ai.exe checkpoint trae --hook-input stdin`。`agent-hooks.log` 中有 18 次调用，18 次均为 `CommandNotFoundException`；首个命令在第 2 行，首个错误在第 15 行。 |
| CodeBuddy | `D:\code\git-ai-test` | Hook 未启动 | 命令为 `/c/Users/admin/.git-ai/bin/git-ai.exe checkpoint codebuddy --hook-input stdin`。IDE 日志中有 6 次调用，6 次均返回非阻塞错误码 1 和“系统找不到指定的路径”；首个命令和错误分别位于第 1324、1325 行。 |
| CodeBuddy IDE payload | `D:\code\git-ai-test` | payload 分类缺口 | 实机注册并调用 `write_to_file`、`replace_in_file`、`execute_command`，当前分类器只支持 CLI 名称。 |
| Qoder | `D:\code\git-ai-test` | Hook 未安装 | `%USERPROFILE%\.qoder\settings.json` 不存在，因此没有生成命令和退出码；检查到 5 个 `hooks.log`，全部为空。运行进程名为 `QoderCN`，当前检测器不识别。 |

归属结果：

- 复现仓库：`D:\code\git-ai-test`。
- `index.html` 有 12 个 `KnownHuman` checkpoint，0 个 `AiAgent` checkpoint。
- working log 中唯一的 `AiAgent` 属于独立的 `ai-test.txt` mock，不属于三个 Agent 对 `index.html` 的修改。
- 因此现有证据可以把问题定位在 Agent Hook 进入归属计算之前，而不是 Git Note 序列化之后。

环境副作用记录：

- 为检查 Git Bash 登录环境是否能找到 Cargo，首次运行 `bash.exe -lc` 时，Git Bash 自动创建了 `C:\Users\admin\.bash_profile`（99 字节）。本任务没有修改其内容，也没有擅自删除该用户文件。

### Task 0.2 执行记录：2026-07-16

新增 fixture：

- `tests/fixtures/agent-hooks/trae.json`：4 个 case，覆盖 `Write`、`Edit`、`RunCommand` 和 Pre/Post 事件。
- `tests/fixtures/agent-hooks/codebuddy.json`：6 个 case，分别覆盖 CLI 的 `Write`、`Edit`、`Bash`，以及 IDE 的 `write_to_file`、`replace_in_file`、`execute_command`。
- `tests/fixtures/agent-hooks/qoder.json`：4 个 case，覆盖 `create_file`、`search_replace`、`run_in_terminal` 和 Pre/Post 事件。
- `tests/fixtures/agent-hooks/README.md`：记录脱敏原则、fixture 结构和 CodeBuddy IDE 当前预期失败。

新增或更新的 fixture 消费测试：

- `tests/integration/trae.rs`
- `tests/integration/codebuddy.rs`
- `tests/integration/qoder.rs`

当前验证结果：

| 检查 | 结果 |
| --- | --- |
| 三份 JSON 使用 PowerShell `ConvertFrom-Json` 解析 | 通过 |
| 每个 Agent 同时包含 PreToolUse 和 PostToolUse | 通过 |
| 工具名覆盖任务要求 | 通过 |
| fixture 不读取开发者本机日志 | 通过 |
| `git diff --check` | 通过，仅有 Git 的 LF/CRLF 提示 |
| Rust 编译和集成测试 | 通过：3 passed，0 failed，3122 filtered out |
| 生产代码变更检查 | 通过：`cargo fmt` 触及的既有 `src/api/bundle.rs` 空行已恢复，本阶段没有生产代码修改 |

工具安装结果：

- Rustup stable：Rust 1.97.0，`x86_64-pc-windows-msvc`。
- Cargo：1.97.0。
- rustfmt：1.9.0-stable。
- Clippy：0.1.97。
- Task：3.46.4，通过 Chocolatey `go-task` 安装。
- `%USERPROFILE%\.cargo\bin` 已写入用户 PATH；当前自动化父进程未重载 PATH，因此执行命令时显式前置该目录。

执行命令与结果：

| 命令 | 结果 |
| --- | --- |
| `task format` | 失败于 Taskfile 全局 `TEST_THREADS` 计算：Windows 上找不到 `nproc`、`getconf` 和 `sysctl`，尚未进入 format task。该问题与 fixture 无关，应作为 Taskfile Windows 兼容问题单独修复。 |
| `cargo fmt` | 成功格式化本阶段 Rust 测试代码；对无关基线文件的机械修改已恢复。 |
| `cargo test --test integration windows_hook_fixtures -- --test-threads 1` | 通过；Trae、CodeBuddy 和 Qoder 三个测试全部成功。 |

编译阶段观察到两条既有 Windows warning：`src/config.rs` 的未使用 import，以及 `src/mdm/ensure_git_symlinks.rs` 的未使用函数；它们不由本阶段改动引入。

Task 0.2 和阶段 0 已满足验收条件。Taskfile 的 Windows CPU 数量探测问题不阻塞本阶段 fixture 验收，但会阻塞后续按 Taskfile 运行完整质量检查，必须在阶段 1 或独立任务中处理。

## 16. 阶段 1 执行记录：2026-07-16

阶段状态：**已完成**。Task 1.1 已建立原生 Windows PowerShell、Cmd 和 Git Bash 的 Hook 命令真实执行测试。

### Task 1.1 执行记录：2026-07-16

实现内容：

- 新增仅在 `test-support` feature 下构建的 `git-ai-hook-test-recorder`，普通发布构建不会构建该目标。
- recorder 记录实际收到的 argv、stdin 和 cwd；测试进程单独断言 shell 退出码。
- 新增 `command_line_test_support`，只在 `test-support` feature 下暴露现有 renderer，不扩大普通生产 API。
- 新增 Windows integration 模块，分别通过以下真实解释器执行 renderer 输出：
  - `powershell.exe -NoLogo -NoProfile -NonInteractive -Command`
  - `cmd.exe /D /S /C`
  - Git for Windows 的 `bash.exe -c`
- Cmd harness 使用 Windows `CommandExt::raw_arg` 传递 `/C` 后的完整命令，避免 Rust `Command` 为 Cmd 不兼容地二次转义引号。
- Git Bash 可以通过 `GIT_AI_TEST_GIT_BASH` 显式指定；否则检查标准 Git for Windows 安装目录。找不到时测试直接失败。

真实执行覆盖：

- 可执行文件目录：`Test User`、`A&B`、`100% Dev`、`O'Neil` 和 `Tools`。
- 静态 renderer 额外覆盖 `D:\Tools\git ai\git-ai.exe`，验证非 C 盘的 Git Bash 路径转换。
- argv：普通参数、空格、`&`、`%` 和单引号。
- cwd：包含空格和 `&`。
- stdin：完整的 `PostToolUse` JSON，并保留末尾换行。
- 负向场景：把 Git Bash `/c/...` 命令交给 PowerShell 和 Cmd，二者都返回失败且 recorder 没有运行，复现当前 Trae 和 CodeBuddy IDE 问题。

测试结果：

| 命令 | 结果 |
| --- | --- |
| `cargo test mdm::command_line::tests --lib -- --test-threads 1` | 通过：5 passed，0 failed。 |
| `cargo test --test integration windows_hook_commands:: -- --test-threads 1` | 通过：4 passed，0 failed，3125 filtered out。 |
| `cargo build --bin git-ai` | 通过；确认普通生产二进制不依赖 recorder。 |
| `cargo clippy --bin git-ai-hook-test-recorder --features test-support -- -D warnings` | 未通过：Clippy 同时检查 library，失败列表仅涉及仓库既有文件中的 11 条基线告警；recorder 目标没有产生诊断。 |
| `git diff --check` | 通过，仅有 Git 的 LF/CRLF 提示。 |

首次运行 Cmd 测试时，Rust `Command::arg` 把嵌套引号编码为 Cmd 无法识别的 `\"...\"`，导致一条 harness 假失败。改用 `raw_arg` 模拟宿主向 `/C` 传递完整命令后，Cmd 的所有特殊路径和参数均通过；生产 renderer 未因此修改。

普通构建仍显示阶段 0 已记录的两条既有 Windows warning，未新增 warning。严格 Clippy 检查还发现 11 条既有 library 基线告警，涉及 `config`、`authorship`、`commands`、`git` 和 `mdm` 中未由本阶段修改的文件；本阶段不扩大范围修复这些告警。

## 17. 阶段 2 执行记录：2026-07-16

阶段状态：**Task 2.1 已完成；Task 2.2 自动化闭环与本机迁移已完成，等待一次 Trae CN 交互式编辑作为最终实机抽查。**

### Task 2.1 执行记录：2026-07-16

实现内容：

- Trae Hook shell 改为 `platform_hook_shell(HookShell::PowerShell)`：Windows 使用 PowerShell，macOS/Linux 保持 POSIX shell。
- Windows 配置使用原生盘符路径；路径含空格或特殊字符时由统一 renderer 添加单引号与 PowerShell call operator `&`。
- up-to-date 判断现在要求每个事件恰好有一个位于 `matcher: "*"` 下且命令完全匹配当前平台的 Trae Hook；旧 Git Bash `/c/...` 命令不再被误判为最新。
- 安装迁移和卸载只匹配 `checkpoint trae`，不再删除用户的其他 `git-ai checkpoint` Hook。
- 旧命令迁移后每个事件只保留一个 Trae Hook；再次安装返回无改动。
- Trae 进程检测新增实机进程名 `Trae CN`，后续 Hook 更新能够正确提示重启。
- `command_line.rs` 的 Agent runtime 映射注释同步更新。

测试基础设施修复：

- `TestRepo` 的已知 checkpoint preset 列表原先漏掉 `codebuddy`、`qoder` 和 `trae`，导致测试命令被错误改写成普通文件参数，返回 0 但生成 0 个 checkpoint；现已补齐并增加回归断言。
- 行级归属测试的 AI author 列表补齐上述三个 Agent，为阶段 2～4 的闭环测试提供统一断言能力。

### Task 2.2 执行记录：2026-07-16

自动化闭环已验证：

- 通过真实 stdin 调用执行 Trae `PreToolUse` 和 `PostToolUse`。
- PreToolUse 在 working log 中建立 `active_agent_edits.json` 基线范围，包含 `src/main.rs` 和 Trae agent identity。
- PostToolUse 生成包含 `src/main.rs` 的 `AiAgent` checkpoint，并清除 active edit marker。
- 提交后新增行被 `git-ai blame` 识别为 AI，未被 Trae 修改的原人工行仍为人工。
- 最终安装器命令通过真实 PowerShell 启动 recorder，argv、stdin、cwd 和退出码均正确。

本机迁移结果：

- 迁移前 `C:\Users\admin\.trae-cn\hooks.json` 的 PreToolUse/PostToolUse 都使用 `/c/Users/admin/.git-ai/bin/git-ai.exe`。
- 使用仓库规定的 `scripts/dev.ps1` 安装本地 debug 构建后，两条命令均变为 `C:\Users\admin\.git-ai\bin\git-ai.exe checkpoint trae --hook-input stdin`，配置中不再包含 `/c/...`。
- 安装后的 `git-ai.exe` 与本地 debug 构建 SHA-256 一致。
- 再次执行 `scripts/dev.ps1` 时 Trae、CodeBuddy 和其他已安装 Agent 均报告 Hook 已是最新，实机重复安装保持幂等。
- CodeBuddy 配置 SHA-256 在迁移前后保持 `4965232FCD0546F033A9F9CE6C5E887DB714B9D094021B45C7792697DA8A229A`，没有被本阶段改动。
- 16 个 Trae CN 进程已正常关闭，随后从 `D:\Trae CN\Trae CN.exe` 重新启动；没有执行强制终止。
- 迁移前配置备份位于 `C:\Users\admin\AppData\Local\Temp\1\git-ai-20260716-stage2-live-configs`。

验证命令：

| 命令 | 结果 |
| --- | --- |
| `cargo test mdm::agents::trae::tests --lib -- --test-threads 1` | 通过：新增进程名测试后共 8 个 Trae installer 测试。 |
| `cargo test --test integration trae:: -- --test-threads 1` | 通过：5 passed，包含 checkpoint→commit→blame 闭环。 |
| `cargo test --test integration windows_hook_commands:: -- --test-threads 1` | 通过：5 passed，包含 Trae 最终命令真实 PowerShell 执行。 |
| `cargo test mdm::command_line::tests --lib -- --test-threads 1` | 通过：5 passed。 |
| `cargo build --bin git-ai` | 通过。 |
| `scripts/dev.ps1` | 通过；本地 debug 二进制和 Trae Hook 已安装。 |

已知非阻塞项：

- Taskfile 的 Windows CPU 探测仍无法解析，验证继续使用等价 Cargo 命令并直接执行 Taskfile 指定的 `scripts/dev.ps1`。
- 普通构建仍只有既有的 `src/config.rs` unused import 与 `src/mdm/ensure_git_symlinks.rs` dead code warning。
- dev 安装期间三个 skill 链接因用户目录权限失败；这不影响 git-ai 二进制、Trae Hook 或归属闭环。
- 第二次 dev 安装尝试创建已存在的 `~/.git-ai/libexec` junction 时收到 Windows `os error 183` warning；现有 junction 未被删除，Hook 安装和最终一致性检查仍通过。
- Trae CN 已加载新配置，但尚未由本次自动化会话在 GUI 中亲自触发一次创建/编辑操作；Task 2.2 的两个手工验收勾选项保留未完成，等待用户在已重启的 Trae CN 中执行一次编辑后核对真实 Hook 日志。

## 18. 阶段 3 执行记录：2026-07-16

阶段状态：**Task 3.1 的实现、双 runner 自动化和本机配置迁移已完成；Task 3.2 已完成；Task 3.3 自动化闭环已完成，等待 CodeBuddy CN 交互式重启验收和一套真实 CodeBuddy CLI 环境。**

### Task 3.1 执行记录：2026-07-16

执行环境矩阵已经由官方资料和本机安装包实现共同确认：

| 产品 | 配置 | Windows Hook runner | 证据 |
| --- | --- | --- | --- |
| CodeBuddy IDE / Craft Agent | `~/.codebuddy/settings.json` | Node `spawn(command, [], { shell: true })`，Windows 下进入 `cmd.exe` | CodeBuddy CN 4.10.2 安装包 `extensions/genie/out/extension/index.js` 的 `HookExecutorImpl`；本机旧 `/c/...` 命令在 IDE 日志中失败。 |
| CodeBuddy CLI | `~/.codebuddy/settings.json` | Git Bash | [官方 Hooks Guide](https://www.codebuddy.ai/docs/cli/hooks-guide) 和 [Hooks Reference](https://www.codebuddy.ai/docs/cli/hooks)。 |

实现内容：

- 增加 `CmdAndGitBash` renderer。Windows 盘符路径统一使用正斜杠，必要时使用两个 runner 都接受的双引号；macOS/Linux 仍使用 POSIX shell。
- CodeBuddy installer 写入一条 IDE Cmd 与 CLI Git Bash 共用的命令，不再写 `/c/...`。
- up-to-date 判断要求 PreToolUse/PostToolUse 各恰好存在一个位于 `matcher: "*"` 下且命令完全匹配当前平台的 CodeBuddy Hook。
- 安装迁移和卸载只匹配 `checkpoint codebuddy`，不会删除其他 Agent 的 `git-ai checkpoint` Hook。
- 旧 Git Bash Hook、重复 Hook、用户 Hook 保留、幂等重装和精确卸载均有单元测试。
- 进程检测增加实机名称 `CodeBuddy CN`。
- 最终 installer 命令通过真实 `cmd.exe /D /S /C` 和 Git for Windows `bash.exe -c` 执行；两侧收到相同 argv、stdin 和 cwd。
- 路径覆盖 `Test User`、`A&B`、`100% Dev`、`O'Neil` 和非系统盘，Cmd 与 Git Bash 均返回 0。

本机迁移结果：

- 迁移前配置 SHA-256 为 `4965232FCD0546F033A9F9CE6C5E887DB714B9D094021B45C7792697DA8A229A`，命令为 `/c/Users/admin/.git-ai/bin/git-ai.exe checkpoint codebuddy --hook-input stdin`。
- 使用 `scripts/dev.ps1` 安装后，PreToolUse/PostToolUse 均变为 `C:/Users/admin/.git-ai/bin/git-ai.exe checkpoint codebuddy --hook-input stdin`，配置中不再包含 `/c/...`。
- 迁移后配置 SHA-256 为 `F7588896FC09FC0BECC9A58231A376197292574C5936F3D5270899CE250D3274`。
- 安装后的 `git-ai.exe` 与本地 debug 构建 SHA-256 均为 `45CB7F3EAF3A393D5267C4E7344C318149F80959FA5D8347423317353844780F`。
- 再次执行已安装二进制的 `install` 后 CodeBuddy 报告 Hook 已是最新，配置 SHA-256 保持不变。
- 迁移前配置备份位于 `C:\Users\admin\AppData\Local\Temp\1\git-ai-20260716-stage3-live-configs`。

### Task 3.2 和 Task 3.3 自动化记录：2026-07-16

实现内容：

- CodeBuddy 分类器增加 IDE 别名：`write_to_file`、`replace_in_file` -> `FileEdit`，`execute_command` -> `Bash`；CLI 名称保持不变。
- `write_to_file` 可以从 `tool_input.content` 生成 dirty file 内容。
- IDE 终端 payload 没有 `tool_use_id` 时改用共享 bash handler 的 `bash` 回退键，通过 per-session sidecar 关联 Pre/Post 快照。
- fixture 测试同时覆盖 CLI 与 IDE 的写入、编辑和终端名称；未知 `Read` 仍返回 Skip。
- CLI 与 IDE 分别建立真实 checkpoint -> commit -> blame 闭环：
  - 创建文件行归属为 CodeBuddy AI；
  - 编辑已有文件的新增行归属为 CodeBuddy AI；
  - 未修改的人工基线行保持人工；
  - CLI `Bash` 与 IDE `execute_command` 都通过 sidecar/stat diff 把终端生成文件归属为 AI。

验证命令：

| 命令 | 结果 |
| --- | --- |
| `cargo test mdm::command_line::tests --lib -- --test-threads 1` | 通过：6 passed。 |
| `cargo test mdm::agents::codebuddy::tests --lib -- --test-threads 1` | 通过：7 passed。 |
| `cargo test test_tool_classification_claude --lib -- --test-threads 1` | 通过：1 passed，包含 CodeBuddy IDE 别名断言。 |
| `cargo test --test integration codebuddy:: -- --test-threads 1` | 通过：8 passed，包含 CLI/IDE 创建、编辑、终端和 blame 闭环。 |
| `cargo test --test integration windows_hook_commands:: -- --test-threads 1` | 通过：6 passed，包含 CodeBuddy 最终命令的 Cmd/Git Bash 双 runner 执行。 |
| `cargo build --bin git-ai` | 通过。 |
| `cargo clippy --lib --tests -- -D warnings` | 未通过；仍为阶段 1 已记录的 11 条仓库基线告警，本阶段文件没有诊断。 |
| `scripts/dev.ps1` | 通过；本地 debug 二进制已安装，CodeBuddy 配置已迁移。 |

待完成的实机验收：

- 本机存在 14 个 CodeBuddy CN 进程，但都没有可用的 `MainWindowHandle`，无法发送正常窗口关闭请求；本阶段没有强制终止进程，以免丢失未保存工作。用户需手工完全退出并重启 CodeBuddy CN，再触发一次创建和编辑，核对新日志退出码为 0。
- 本机没有可调用的 `codebuddy` CLI。真实 Git Bash runner 和 CLI payload 自动化均已通过，但按照 Task 3.3 要求，不把它替代为真实 CLI 产品验收；该项等待 Windows CI 或另一台装有 CodeBuddy CLI 的机器。
- `scripts/dev.ps1` 仍出现阶段 2 已记录的三条 skill 链接权限 warning；第二次安装仍出现现有 `libexec` junction 权限 warning，不影响 CodeBuddy Hook 更新与幂等检查。

## 19. 阶段 4 执行记录：2026-07-16

阶段状态：**Task 4.1 实现、单元测试和本机 D 盘 Qoder CN 检测已完成；Task 4.2 的 runner 证据、命令执行测试和配置迁移已完成；Task 4.3 自动化归属闭环已完成。等待手工重启 Qoder CN 后的 GUI Hook 日志验收，以及一套国际版 Qoder 环境。**

### Task 4.1 执行记录：Qoder CN 检测

实现内容：

- Windows 候选安装覆盖 `Qoder.exe` 和 `QoderCN.exe`，以及 `Qoder`、`Qoder IDE`、`QoderCN`、`Qoder CN` 常见用户级/机器级目录。
- PATH 检测覆盖 `qoder`、`Qoder`、`qoder-cn` 和 `QoderCN`；Windows `PATHEXT` 使实机 `qoder-cn.cmd` 能被识别。
- `tasklist` 严格匹配 Qoder/Qoder CN 带 `.exe` 和不带 `.exe` 的名称，不用宽泛子串避免误报。
- `process_names()` 增加 `QoderCN` 和 `Qoder CN`，Hook 更新后可识别需重启的 CN 进程。
- 首装时会创建缺失的 `.qoder` 目录与 `settings.json`；dry-run 不再提前创建目录。
- 生产代码没有写死 `D:\QoderCN`，自定义盘符由 PATH 和运行进程识别。

本机证据：

- Qoder CN 1.7.0 安装于 `D:\QoderCN\QoderCN.exe`，运行进程可见的可执行路径也是该 D 盘目录。
- PATH 包含 `D:\QoderCN\bin`，其中存在 `qoder-cn.cmd`。
- 在 `.qoder` 完全不存在时，新 debug 二进制的 `install-hooks --dry-run` 报告 `Qoder: Pending updates`，证明 `tool_installed=true`。
- 创建配置后再次 dry-run 报告 `Qoder: Hooks already up to date`。

### Task 4.2 执行记录：Windows Hook runner

执行环境由实机安装包而非其他 Agent 行为确认：

- Qoder CN 内置 Go 后端 `resources\app\resources\bin\x86_64_windows\QoderCN.exe` 包含 `cosy/extension/hook.buildCmdWindows`、`bashCommandCandidates`、`getInterpreter` 等 Hook 执行符号，并包含 `git-bash.exe`/Bash 候选字符串。
- 因此 Windows Qoder Hook 继续使用 Git Bash renderer，盘符路径生成 `/d/...`，macOS/Linux 使用 POSIX renderer。
- [Qoder 官方 Hooks 文档](https://docs.qoder.com/extensions/hooks) 确认 IDE/JetBrains/CLI 共享 Hook 配置，且配置变更后需重启 Qoder，不支持热加载。
- 最终 installer 命令已由真实 Git for Windows `bash.exe -c` 执行；包含空格、`&`、`%`、单引号和 D 盘路径的情况均能正确传递 argv、stdin 和 cwd。
- up-to-date 判断要求 PreToolUse/PostToolUse 各恰好一个命令完全匹配的 `matcher: "*"` Qoder Hook。
- 迁移与卸载只删除 `checkpoint qoder`，保留用户 Hook 和其他 Agent 的 checkpoint；重复安装保持幂等。

本机配置：

- 因 C 盘容量不足，本阶段不再复制 debug 二进制到 `%USERPROFILE%\.git-ai\bin`，直接使用 `D:\linewell-code\git-ai\target\debug\git-ai.exe`。
- `%USERPROFILE%\.qoder\settings.json` 大小约 1.4 KB，PreToolUse/PostToolUse 均为 `/d/linewell-code/git-ai/target/debug/git-ai.exe checkpoint qoder --hook-input stdin`。
- 被中止的 `task dev` 未更新 C 盘二进制、未留下临时副本，也未写入 Qoder 配置。

### Task 4.3 自动化闭环记录：2026-07-16

- `create_file` 和 `search_replace` 通过真实 stdin 调用生成 Qoder `AiAgent` checkpoint；创建行、编辑新增行提交后均被 blame 为 AI，未修改的人工基线行保持人工。
- `run_in_terminal` 缺少 `tool_use_id` 时使用共享 bash handler 的 `bash` 回退键，通过 per-session sidecar 关联 Pre/Post 快照。
- 终端生成文件写入 Qoder `AiAgent` checkpoint，提交后文件行归属为 AI。
- 原生工具名 `create_file`、`search_replace`、`run_in_terminal` 的 fixture 分类和 Pre/Post preset 解析全部通过。

验证命令：

| 命令 | 结果 |
| --- | --- |
| `cargo test qoder --lib -- --test-threads 1` | 通过：9 passed，包含 CN 检测、首装/dry-run、迁移、幂等与精确卸载。 |
| `cargo test --test integration qoder:: -- --test-threads 1` | 通过：9 passed，包含创建、编辑、终端 checkpoint -> commit -> blame 闭环。 |
| `cargo test --test integration windows_hook_commands:: -- --test-threads 1` | 通过：7 passed，包含 Qoder 最终命令的真实 Git Bash 执行。 |
| `cargo build --bin git-ai` | 通过。 |
| `cargo clippy --lib --tests -- -D warnings` | 未通过；仅余阶段 1 已记录的 11 条仓库基线告警，本阶段文件没有诊断。 |
| `cargo fmt -- --check` | 未通过；仅命中本阶段前已存在的 `src/api/bundle.rs` 多余空行，本阶段文件已通过 `rustfmt --edition 2024`。 |
| D 盘 debug `install-hooks --dry-run` | 迁移前报告 `Qoder: Pending updates`，迁移后报告 `Qoder: Hooks already up to date`。 |

待完成的实机验收：

- Qoder 配置不支持热加载。当前 Qoder CN 进程尚未由本次自动化会话强制结束，避免丢失未保存工作；用户需手工完全退出并重启 Qoder CN。
- 重启后需在 GUI 中各触发一次创建、编辑和终端修改，核对 `hooks.log` 中 `git-ai.exe` 启动成功且退出码为 0，因此 Task 4.2 第一个验收项仍保持未勾选。
- 本机只安装 Qoder CN，国际版 Qoder 日志验收等待 Windows CI 或另一台安装国际版的机器。
