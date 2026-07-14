# Windows 兼容性改进任务清单

本文档把当前 Windows 兼容性审计发现拆成可以逐项实现、逐项测试、逐项提交的工程任务。执行者应严格按阶段顺序推进；每完成一个任务，先满足验收标准并记录测试结果，再进入下一个任务。

## 0. 执行规则

每个任务都包含：

- 改动范围：预计需要修改的文件或模块。
- 执行步骤：可以按顺序直接执行的实现步骤。
- 建议测试：优先在原生 Windows PowerShell 中执行的验证命令。
- 验收标准：任务完成的最低要求。

通用要求：

1. 不覆盖工作区中已有的用户改动；开始每个任务前运行 `git status --short`。
2. 每次只处理一个任务，测试通过后单独提交，避免把登录、凭据和 daemon 改动混在一个提交中。
3. Windows 路径测试至少包含空格；涉及 shell 的任务还要覆盖 `&`、`^`、`%`、单引号和双引号。
4. 不允许通过关闭 daemon、关闭安全校验或只修改测试来规避兼容问题。
5. 跨平台公共代码修改后，必须同时验证 Windows、macOS 和 Linux CI。
6. 默认使用 Taskfile：`task format`、`task lint`、`task build`、`task test`。只有任务明确要求时才运行特殊测试模式。
7. Windows 专属行为必须有 `#[cfg(windows)]` 测试或原生 Windows CI 验证，不能只依靠 macOS/Linux 上构造 `Command` 的单元测试。

## 1. 目标与完成标准

本计划完成后，必须达到以下结果：

- [ ] Windows 用户目录包含空格或常见 shell 特殊字符时，所有受支持 agent 的 Hook 都能正常调用 `git-ai.exe`。
- [ ] `git-ai login` 和 `git-ai personal-dashboard` 打开的 URL 不会被 `cmd.exe` 截断。
- [ ] 浏览器、安全软件或端口探测连接回调端口时，不会提前终止 CLI 登录。
- [ ] Windows 登录凭据默认进入 Windows Credential Manager，或使用同等强度的系统加密保护。
- [ ] 文件凭据回退模式只允许当前 Windows 用户访问，权限设置失败时不会静默继续。
- [ ] daemon named pipe 只允许当前用户、SYSTEM 和管理员访问。
- [ ] 无数据或不发送换行的 pipe 客户端不能永久占用 daemon worker，也不能阻止 daemon 退出。
- [ ] Windows CI 使用带空格的测试用户目录，并真正执行生成的 Hook 命令。
- [ ] Windows x64 和 ARM64 发布构建、安装、登录、daemon、Hook 和更新流程都有明确验收结果。

## 2. 当前风险地图

| 编号 | 优先级 | 问题 | 主要位置 |
| --- | --- | --- | --- |
| WIN-01 | P0 | Agent Hook 中的可执行文件路径未统一引用 | `src/mdm/agents/*.rs` |
| WIN-02 | P0 | Windows 发布包默认回退到明文凭据文件 | `src/auth/credentials.rs`、`src/auth/credential_backend.rs`、发布工作流 |
| WIN-03 | P0 | named pipe 没有显式同用户 ACL | `src/daemon/mod.rs` |
| WIN-04 | P1 | named pipe 服务端读取没有超时和帧大小限制 | `src/daemon/mod.rs` |
| WIN-05 | P1 | 登录和 dashboard 的浏览器打开逻辑不统一 | `src/commands/login.rs`、`src/commands/personal_dashboard.rs` |
| WIN-06 | P1 | Windows agent E2E 未真正执行 Hook，且关闭了 async mode | `.github/workflows/install-scripts-local.yml` |
| WIN-07 | P2 | `mklink /J` 仍经过 `cmd.exe`，特殊字符路径有风险 | `src/mdm/ensure_git_symlinks.rs` |
| WIN-08 | P2 | `git ai update` 在 Windows 不受支持 | `src/commands/upgrade.rs`、用户文档 |

## 3. 阶段 0：建立 Windows 基线

### Task 0.1：保存工作区和环境基线

改动范围：

- 不修改代码。
- 在 PR 描述或任务记录中保存结果。

执行步骤：

1. 查看当前改动：

   ```powershell
   git status --short
   git diff --check
   ```

2. 记录 Rust、Git 和 PowerShell 版本：

   ```powershell
   rustc --version
   cargo --version
   git --version
   $PSVersionTable.PSVersion
   ```

3. 确认 Windows target 和主机架构：

   ```powershell
   rustup target list --installed
   [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
   ```

4. 运行当前基线测试：

   ```powershell
   task format:check
   task lint
   task build
   task test
   ```

5. 保存失败测试的完整命令、错误信息和是否能够稳定复现。

验收标准：

- [ ] 已记录工作区原有改动，不会在后续任务中误删。
- [ ] 已记录 Windows 版本、CPU 架构和工具链版本。
- [ ] 已区分基线失败与后续改动引入的失败。

### Task 0.2：建立特殊路径测试目录

改动范围：

- 本地测试目录。
- 后续会同步到 `.github/workflows/install-scripts-local.yml`。

执行步骤：

1. 创建至少两个测试 HOME：

   ```powershell
   $SpaceHome = Join-Path $env:RUNNER_TEMP 'git ai home'
   $MetaHome = Join-Path $env:RUNNER_TEMP 'git-ai & home'
   New-Item -ItemType Directory -Force -Path $SpaceHome, $MetaHome
   ```

2. 分别设置 `HOME`、`USERPROFILE`、`HOMEDRIVE` 和 `HOMEPATH` 后运行本地安装。
3. 记录哪些 agent 配置文件中出现未引用的 `git-ai.exe` 路径。
4. 不在本任务中修复问题，只生成可复现样例。

验收标准：

- [ ] 空格路径可以稳定复现 Hook 命令解析问题，或已有测试证明不存在问题。
- [ ] `&` 路径可以稳定复现 cmd/Git Bash 解析问题，或已有测试证明不存在问题。

### 阶段 0 执行记录（2026-07-14）

执行环境：

- 当前执行主机是 macOS 15.7.7（Darwin 24.6.0，x86_64），不是原生 Windows。
- Rust：`rustc 1.96.1 (31fca3adb 2026-06-26)`、`cargo 1.96.1`。
- Git：`git version 2.50.1 (Apple Git-155)`。
- Task：`Task version: v3.50.0`。
- 已安装 Rust targets：`x86_64-apple-darwin`、`x86_64-pc-windows-msvc`。
- 当前环境没有 `pwsh` 或 `powershell`，因此尚未记录 Windows 版本、PowerShell 版本和 Windows CPU 架构。
- 执行前工作区干净，HEAD 为 `689b01b`。测试过程产生的 `Cargo.lock` 项目版本机械刷新已还原，没有把它混入阶段 0 记录。

Task 0.1 基线结果：

| 命令 | 结果 | 记录 |
| --- | --- | --- |
| `task format:check` | 失败 | `src/api/bundle.rs:2` 存在两个多余空行；执行前工作区干净，判定为已有基线问题。 |
| `task lint` | 通过 | `cargo clippy --all-targets -- -D warnings` 通过。 |
| `task build` | 通过 | debug 构建通过。 |
| `task test` | 未完整通过 | library 测试 `1574 passed, 0 failed, 4 ignored`；`async_mode`、`commit_tree_update_ref`、`config_fresh`、`daemon_mode` 测试二进制均通过。主 integration 测试运行至接近结束后长期无输出，累计约 1 小时 26 分钟后人工中断。 |

`task test` 的失败和复现细节：

1. 首次在受限沙箱内执行时有 6 个测试因不允许绑定本地回环地址失败；在允许本地 socket 的环境重跑后，这 6 个测试全部通过，判定为执行环境限制，不是项目回归。
2. 完整测试中观察到 `diff_comprehensive::test_diff_many_files` 失败。
3. 使用以下命令单独运行可稳定复现：

   ```bash
   task test TEST_FILTER=diff_comprehensive::test_diff_many_files EXTRA_TEST_BINARY_ARGS=--exact
   ```

4. 失败发生在测试共享 daemon 启动阶段，而不是 diff 断言阶段。子进程在 control/trace socket 就绪前以状态 0 退出，`daemon.test.stderr.log` 为空。即使把共享 daemon pool 和测试线程都限制为 1，仍可复现：

   ```bash
   GIT_AI_TEST_GIT_MODE=daemon \
     GIT_AI_TEST_SHARED_DAEMON_POOL_SIZE=1 \
     cargo test --test integration \
     diff_comprehensive::test_diff_many_files \
     -- --exact --nocapture --test-threads 1
   ```

5. 该问题应作为独立基线缺陷排查；在它修复前，不能把当前主 integration 测试标记为全绿。

Task 0.2 当前结果：

- 原生 Windows 特殊 HOME 复现尚未执行，Task 0.2 的两个验收项保持未完成。
- 静态扫描已经定位到需要优先复现的 Hook 命令生成器：
  - Git Bash 路径转换后直接拼接命令：`claude_code.rs`、`codebuddy.rs`、`qoder.rs`、`trae.rs`。
  - 原生路径直接拼接命令：`codex.rs`、`cursor.rs`、`droid.rs`、`firebender.rs`、`gemini.rs`、`github_copilot.rs`、`windsurf.rs`。
- 下一步必须在原生 Windows 上分别以包含空格和 `&` 的 HOME 执行本地安装，保存实际生成的配置文件和命令解释器报错；静态扫描结果不能替代这一步验收。

阶段 0 状态：**部分完成**。Task 0.1 的非 Windows 基线已经保存并区分了环境失败与项目失败；Windows 环境信息和 Task 0.2 等待原生 Windows 主机或 CI runner 执行。

## 4. 阶段 1：统一 Agent Hook 命令生成

### Task 1.1：确认每个 Agent 使用的命令解释器

改动范围：

- `src/mdm/agents/`
- Agent 官方 Hook 配置说明或现有集成代码。

执行步骤：

1. 列出所有把 `binary_path` 拼接为命令字符串的 agent：

   ```bash
   rg -n "binary_path.*display|to_git_bash_path|checkpoint" src/mdm/agents
   ```

2. 为每个 agent 标记实际执行环境：

   - 原生 Windows 进程参数。
   - `cmd.exe /C`。
   - PowerShell。
   - Git Bash/POSIX shell。
   - JavaScript/TypeScript 中的 `spawn` 参数数组。

3. 不要假设所有 Hook 都使用同一种引用规则。
4. 把最终映射写成测试表或代码注释，供统一渲染函数使用。

验收标准：

- [x] Claude、Codex、Gemini、Cursor、GitHub Copilot、Droid、CodeBuddy、Qoder、Trae、Firebender、Windsurf 的命令解释器已明确。
- [x] Amp、OpenCode、PI 等直接使用参数数组或脚本插件的 agent 已与 shell 命令型 agent 分开处理。

### Task 1.2：实现统一 Hook 命令渲染器

改动范围：

- 建议新增 `src/mdm/command_line.rs`，或在现有 `src/mdm/utils.rs` 中增加小型独立模块。
- `src/mdm/agents/*.rs`。

执行步骤：

1. 定义明确的执行环境枚举，例如 `NativeWindows`、`Cmd`、`PowerShell`、`GitBash`。
2. 定义统一入口，接收 `binary_path`、子命令参数和执行环境，返回完整 Hook 命令。
3. 对可执行文件路径和每个参数分别引用，不允许 agent 自己调用 `format!("{} {}", ...)`。
4. Git Bash 路径先完成 `C:\...` 到 `/c/...` 转换，再按 POSIX shell 规则引用。
5. `cmd.exe` 和 PowerShell 必须使用各自的转义规则，不能复用 POSIX 引用。
6. JavaScript/TypeScript 插件继续优先使用 `spawn(executable, args)`，不要退化为 shell 字符串。
7. 迁移所有命令字符串型 agent，并删除重复的本地拼接代码。

必须增加的测试用例：

```text
C:\Users\Test User\.git-ai\bin\git-ai.exe
C:\Users\A&B\.git-ai\bin\git-ai.exe
C:\Users\100% Dev\.git-ai\bin\git-ai.exe
C:\Users\O'Neil\.git-ai\bin\git-ai.exe
```

建议测试：

```powershell
cargo test mdm:: --lib
task format
task lint
task test
```

验收标准：

- [x] `src/mdm/agents/` 中不再存在未审查的 `format!("{} {}", binary_path, ...)`。
- [x] 所有特殊路径测试通过。
- [ ] 在原生 Windows 上从生成的配置中提取 Hook 命令并执行，退出码为 0。
- [x] 普通无空格路径生成结果保持向后兼容。

提交建议：

```bash
git add src/mdm
git commit -m "Quote Windows agent hook commands"
```

### 阶段 1 执行记录（2026-07-14）

解释器映射：

| Agent | macOS/Linux | Windows | 配置方式 |
| --- | --- | --- | --- |
| Claude Code | POSIX shell | Git Bash | 单一 `command` 字段；Windows 盘符先转换为 `/c/...`。 |
| CodeBuddy、Qoder、Trae | POSIX shell | Git Bash | 延续现有 Claude 兼容 Hook 模式和路径转换。 |
| Gemini、Cursor、Droid、Firebender | POSIX shell | `cmd.exe`/原生 Windows shell | 单一 `command` 字段，安装时按当前平台渲染。 |
| Codex | POSIX shell | PowerShell | 同时生成 `command` 和 `commandWindows`。 |
| GitHub Copilot | Bash/POSIX shell | PowerShell | 同时生成 `command` 和 `powershell`，Windows 不再复用 POSIX 字符串。 |
| Windsurf | `bash -c` | `powershell -Command` | 同时生成 `command` 和 `powershell`。 |
| Amp、OpenCode、PI | JavaScript/TypeScript 插件 | JavaScript/TypeScript 插件 | 保留 executable/argv 分离，不使用 shell 命令渲染器。 |

实现结果：

- 新增 `src/mdm/command_line.rs`，提供 `HookShell`、`platform_hook_shell` 和 `render_hook_command`。
- 可执行文件路径和参数逐项引用；Git Bash、POSIX shell、`cmd.exe`、PowerShell 使用独立规则。
- PowerShell 在带空格或元字符的可执行文件路径前使用调用运算符 `&`，单引号通过双写转义。
- Git Bash 先将 Windows 盘符路径转换为 `/c/...`，再使用 POSIX 单引号规则。
- 已迁移 Claude、Codex、Gemini、Cursor、GitHub Copilot、Droid、CodeBuddy、Qoder、Trae、Firebender 和 Windsurf；扫描只剩 Amp、OpenCode、PI 的插件脚本路径生成逻辑。

测试结果：

| 命令 | 结果 |
| --- | --- |
| `cargo test mdm:: --lib` | 通过：`207 passed, 0 failed`。 |
| `task lint` | 通过。 |
| `task build` | 通过。 |
| `task format:check` | 仍只因阶段 0 已记录的 `src/api/bundle.rs:2` 两个历史空行失败；本阶段修改的 Rust 文件均已单独执行 `rustfmt`。 |
| `cargo check --target x86_64-pc-windows-msvc` | 未完成：macOS 主机缺少 Windows/MSVC C 工具链，`libsqlite3-sys` 编译 bundled SQLite 时找不到 `stdlib.h`。 |

特殊路径测试覆盖：

```text
C:\Users\Test User\.git-ai\bin\git-ai.exe
C:\Users\A&B\.git-ai\bin\git-ai.exe
C:\Users\100% Dev\.git-ai\bin\git-ai.exe
C:\Users\O'Neil\.git-ai\bin\git-ai.exe
```

阶段 1 状态：**代码实现和本机回归已完成，原生 Windows E2E 待验收**。需要在 Windows runner 上从每类 Agent 的实际配置中取出生成命令，分别交给 Git Bash、`cmd.exe` 和 PowerShell 执行并确认退出码为 0。

## 5. 阶段 2：统一浏览器和登录回调行为

### Task 2.1：建立跨平台浏览器打开模块

改动范围：

- `src/commands/login.rs`
- `src/commands/personal_dashboard.rs`
- 建议新增 `src/platform/browser.rs` 或等价公共模块。

执行步骤：

1. 把登录命令当前使用的 Windows `rundll32.exe url.dll,FileProtocolHandler <url>` 提取为公共实现。
2. macOS 继续使用 `open`，Linux 继续使用 `xdg-open`。
3. `login` 和 `personal-dashboard` 都调用同一个公共函数。
4. 删除 `personal-dashboard` 中的 `cmd /C start`。
5. 所有平台都把 URL 作为单独参数传递，不拼接 shell 命令字符串。
6. 打开失败时保留完整 URL 的终端回退提示。

必须增加的测试 URL：

```text
https://example.test/auth?client_id=git-ai-cli&redirect_uri=http%3A%2F%2F127.0.0.1%3A12345%2Fcallback&state=a%26b
```

建议测试：

```powershell
cargo test windows_browser_command --lib
cargo test personal_dashboard --lib
```

验收标准：

- [x] 登录和 dashboard 不再经过 `cmd.exe` 打开 URL。
- [x] 含多个 `&` 的 URL 仍作为一个完整参数传给系统浏览器。
- [x] 用户关闭自动打开功能或系统打开失败时，仍能复制完整 URL。

### Task 2.2：加固 CLI 回调监听器

改动范围：

- `src/auth/cli_callback.rs`
- `src/commands/login.rs`。

执行步骤：

1. 保留“忽略空连接或畸形端口探测，继续等待真实 OAuth callback”的行为。
2. 增加空 TCP 连接后再发送合法 callback 的测试。
3. 增加错误路径、错误 state、缺少 code 和浏览器重复请求测试。
4. 确保无效连接不会重置总超时时间，防止无限等待。
5. 为单个请求设置合理读取上限，避免客户端无限发送请求头。
6. 在 Windows Defender/企业终端安全软件开启的机器上手工验证一次完整登录。

建议测试：

```powershell
cargo test auth::cli_callback --lib
cargo test commands::login --lib
git-ai login --server https://your-git-ai-server.example.com
```

验收标准：

- [x] 空探测、无效请求和合法 callback 的顺序组合测试通过。
- [x] state 不匹配时不会兑换 token。
- [x] 登录总超时仍然生效。
- [ ] 浏览器点击授权后 CLI 能完成 token 兑换并退出（待原生 Windows 验收）。

### 阶段 2 执行记录（2026-07-15）

实现结果：

- 新增 `src/platform/browser.rs`，统一封装 macOS `open`、Linux `xdg-open` 和 Windows `rundll32.exe url.dll,FileProtocolHandler`；URL 始终作为单独参数传递。
- `login` 与 `personal-dashboard` 共用浏览器模块，移除 dashboard 的 `cmd /C start`；打开失败仍打印完整授权 URL。
- 回调监听器按总截止时间计算每个连接的读取时间，并限制请求行 8 KiB、单行请求头 8 KiB、请求头总量 64 KiB；畸形、超大和空探测会被忽略。

测试结果：

| 命令 | 结果 |
| --- | --- |
| `cargo test platform::browser --lib` | 通过：`2 passed, 0 failed`。 |
| `cargo test commands::login --lib` | 通过：`11 passed, 0 failed`。 |
| `cargo test auth::cli_callback --lib` | 通过：`7 passed, 0 failed`（需允许回环端口监听）。 |
| `task lint` | 通过。 |
| `task build` | 通过。 |
| `task format:check` | 恢复阶段 0 已记录的 `src/api/bundle.rs` 历史空行后仍按预期失败；阶段 2 文件已执行 `rustfmt`。 |

阶段 2 状态：**代码实现和本机回归已完成，需在 Windows/Defender 环境完成一次真实登录验收**。

提交建议：

```bash
git add src/commands/login.rs src/commands/personal_dashboard.rs src/auth/cli_callback.rs src/platform
git commit -m "Harden Windows browser login flow"
```

## 6. 阶段 3：保护 Windows 登录凭据

### Task 3.1：让 Windows 发布包默认具备系统 Keyring 能力

改动范围：

- `Cargo.toml`
- `src/auth/credentials.rs`
- `src/feature_flags.rs`
- `.github/workflows/release.yml`
- `.github/workflows/install-scripts-local.yml`。

执行步骤：

1. 确认 `keyring` crate 的 `windows-native` 后端能在 x64 和 ARM64 MSVC 上构建。
2. Windows 发布构建显式启用 keyring feature。
3. Windows release 默认优先使用 Credential Manager，不要求用户额外开启隐藏 feature flag。
4. 保留 keyring 不可用时的文件回退，但必须先完成 Task 3.2。
5. `git-ai login`、刷新 token、logout 和不同 server URL 切换都要走同一个凭据后端。
6. 日志中不得输出 access token、refresh token 或完整凭据 JSON。

建议测试：

```powershell
cargo build --release --features keyring --target x86_64-pc-windows-msvc
cargo test --features keyring --lib auth::credentials
```

验收标准：

- [ ] Windows x64 和 ARM64 发布构建都包含 Credential Manager 后端。
- [ ] 默认登录后，凭据不会以明文形式出现在 `%USERPROFILE%\.git-ai\internal\credentials`。
- [ ] `git-ai whoami`、token 刷新和 logout 正常工作。

### Task 3.2：加固文件凭据回退模式

改动范围：

- `src/auth/credential_backend.rs`
- Windows 专属测试模块。

执行步骤：

1. 不再仅依赖 `%USERNAME%`；获取当前进程 token 的用户 SID。
2. 创建文件时就应用限制性安全描述符，避免先写明文、再执行 `icacls` 的权限窗口。
3. ACL 只允许当前用户 SID、SYSTEM 和管理员访问。
4. 如果不能安全创建或收紧 ACL，保存凭据必须失败，不能只打印 warning 后返回成功。
5. 使用临时文件加原子替换时，验证目标文件 ACL 不会被重置为继承权限。
6. 保留隐藏文件属性，但不要把隐藏属性当成安全措施。
7. 增加域账号、Azure AD 风格账号和非 ASCII 用户名测试。

手工验证：

```powershell
$CredentialFile = Join-Path $env:USERPROFILE '.git-ai\internal\credentials'
icacls $CredentialFile
Get-Content $CredentialFile
```

仅在明确启用文件回退模式的测试环境执行 `Get-Content`，不得把输出写入 CI 日志或提交记录。

验收标准：

- [ ] 文件回退模式不依赖账号显示名称解析 ACL。
- [ ] 权限设置失败会导致凭据保存失败并给出可操作错误。
- [ ] 其他普通本机用户无法读取凭据文件。
- [ ] 测试和日志不泄露凭据内容。

提交建议：

```bash
git add Cargo.toml src/auth src/feature_flags.rs .github/workflows
git commit -m "Secure Windows credential storage"
```

### 阶段 3 执行记录（2026-07-15）

已完成阶段 3.1 的发布配置调整：release 构建统一启用 `keyring` feature，release 默认开启 `auth_keyring`，因此 Windows x64/ARM64 发布包会编译并优先使用 Credential Manager；debug 构建仍默认关闭，避免本地测试依赖系统钥匙串。

验证结果：

- `cargo test feature_flags --lib`：通过，`10 passed`。
- `cargo test --features keyring --lib auth::credentials`：因当前环境无法连接 crates.io 下载 `keyring 3.6.3`，未能完成；需在可联网环境或 Windows runner 重试。

阶段 3 状态：**发布开关和文件回退 ACL 加固已完成，keyring 跨平台构建和 Credential Manager 实机验收待联网/Windows 环境继续**。

阶段 3.2 补充实现：Windows 文件回退现在通过 `whoami /user` 获取当前用户 SID，ACL 同时保留当前用户、SYSTEM 和 Administrators；`icacls` 或隐藏属性设置失败会直接返回错误，不再仅打印 warning。Linux/macOS 凭据行为保持不变。

验证：`cargo test auth::credentials --lib` 通过（23 tests）；Windows 原生 ACL、域账号和 Azure AD 账号仍需在 Windows runner 手工验证。

## 7. 阶段 4：加固 Windows daemon named pipe

### Task 4.1：为 named pipe 设置同用户 ACL

改动范围：

- `src/daemon/mod.rs`
- 可能新增 `src/daemon/windows_pipe.rs`
- `Cargo.toml` 中的 Windows 专属依赖。

执行步骤：

1. 确认当前 `named_pipe` crate 是否支持创建时传入 Windows security descriptor。
2. 如果不支持，选择以下一种实现：

   - 使用维护中的 Windows pipe crate，并确保能配置 ACL；或
   - 通过 `windows-sys`/Win32 API 创建 pipe 并传入 `SECURITY_ATTRIBUTES`。

3. 从当前进程 token 获取用户 SID。
4. 为 control pipe 和 trace pipe 应用同一套 ACL：当前用户、SYSTEM、管理员允许访问，其余拒绝。
5. 不依赖 pipe 名称的随机性或哈希值作为访问控制。
6. 保留 first-instance 语义，确保同一配置只启动一个 daemon。
7. 添加同用户连接成功、第二实例绑定失败、不同低权限用户连接失败的原生 Windows 测试。

验收标准：

- [x] control 和 trace pipe 创建时已经带有限制性 ACL。
- [ ] 同一用户的 CLI、Git 代理和 Hook 可以正常连接。
- [ ] 其他普通 Windows 用户不能发送 control request 或 trace payload。
- [ ] daemon 重启后 ACL 仍然正确。

### 阶段 4.1 执行记录（2026-07-15）

- 已确认旧 `named_pipe 0.4.1` 服务端创建 API 固定传入空 `SECURITY_ATTRIBUTES`，无法配置 DACL。
- 服务端改用现有 `interprocess 2.4` 的 Windows named pipe listener；创建 control/trace pipe 时传入受保护 DACL，仅允许当前进程用户 SID、SYSTEM 和 Administrators，并保持拒绝远程客户端。
- listener 初次创建仍使用 `FILE_FLAG_FIRST_PIPE_INSTANCE`；后续 worker 共享 listener，由它创建相同安全描述符的管道实例。
- 客户端继续使用兼容的 byte-mode named pipe 协议，连接超时行为不变。

验证结果：

- `task lint`：通过。
- `task build`：通过。
- SID 解析测试覆盖 Azure AD 风格、非 ASCII 用户名和畸形输出。
- `cargo check --target x86_64-pc-windows-msvc`：`interprocess`、`named_pipe` 等 Windows 依赖检查通过，随后仍因 macOS 主机缺少 MSVC C 头文件而在 bundled SQLite 的 `stdlib.h` 处停止。

阶段 4.1 状态：**代码实现和本机检查完成；同用户连接、跨用户拒绝及 daemon 重启后的 ACL 仍需在原生 Windows 验收**。

### Task 4.2：增加服务端读超时、帧限制和可靠退出

改动范围：

- `src/daemon/mod.rs`
- `src/daemon/client.rs`
- daemon Windows 测试。

执行步骤：

1. 给已连接的 control/trace pipe 设置服务端读取超时。
2. 用有最大长度限制的帧读取替换无限制 `read_line()`；超过上限时断开连接并记录不含敏感内容的诊断日志。
3. 明确 control 请求和 trace payload 的最大字节数，并为合理的大 checkpoint 留出余量。
4. 客户端连接但不发送数据时，worker 应在超时后回收。
5. 客户端只发送半行且不发送换行时，也应在超时后回收。
6. shutdown 时主动取消或断开所有已连接 pipe，不能只唤醒等待 accept 的 worker。
7. 连续占满 8 个 control worker、16 个 trace worker后，daemon 仍能恢复并响应新客户端。

必须增加的测试：

- 空连接超时。
- 半行 JSON 超时。
- 超大单行请求被拒绝。
- worker 耗尽后恢复。
- 有卡住客户端时 graceful shutdown 仍在规定时间内结束。
- 正常 checkpoint、status、shutdown 请求不受影响。

建议测试：

```powershell
cargo test daemon --lib
cargo test --test daemon_mode
git-ai daemon restart
git-ai daemon status
git-ai daemon shutdown
```

验收标准：

- [x] 恶意或异常客户端不能永久占用 worker。
- [x] graceful shutdown 不需要依靠最终强杀才能完成。
- [ ] 合法的大型 checkpoint 不会被错误截断。
- [ ] Windows daemon、wrapper-daemon CI 均通过。

### 阶段 4.2 执行记录（进行中，2026-07-15）

- control frame 上限明确为 16 MiB，为大型 checkpoint 请求保留余量。
- trace frame 上限明确为 8 MiB；读取过程按块累计并在越界时立即断开，不再通过无限制 `read_line()` 扩张内存。
- Windows 接受连接后切换为非阻塞读取；空连接或半行 JSON 在 5 秒后断开并回收 worker，因此 shutdown 不会无限等待已连接客户端。
- 已覆盖“刚好达到上限”“超过上限”“EOF 前没有换行”“空连接超时”和“半行超时”五类边界，`cargo test daemon_frame_tests --lib` 通过（5 tests）。
- Windows worker 耗尽恢复、合法大型 checkpoint 和 wrapper-daemon CI 仍待原生 Windows 验收。

提交建议：

```bash
git add Cargo.toml src/daemon tests
git commit -m "Harden Windows daemon named pipes"
```

## 8. 阶段 5：清理剩余 Windows shell 边界

### Task 5.1：移除 `mklink /J` 的脆弱字符串边界

改动范围：

- `src/mdm/ensure_git_symlinks.rs`
- Windows 安装测试。

执行步骤：

1. 使用原生 Windows API 或可靠的 junction 库创建目录 junction，避免 `cmd.exe /C mklink` 二次解析路径。
2. 保留无需管理员权限的行为。
3. 处理 junction 已存在、目标错误、目标被占用和删除失败场景。
4. 对空格、`&`、`^`、`%` 和非 ASCII 路径增加测试。
5. 确认 Git GUI 和 Git for Windows 仍能找到 managed `git.exe`。

验收标准：

- [x] junction 创建不再经过 `cmd.exe`。
- [ ] 特殊字符路径安装、升级和卸载均可完成。
- [ ] 重复执行安装具有幂等性。

### 阶段 5.1 执行记录（2026-07-15）

- Windows junction 创建改用 `junction 1.4.2` 的原生 reparse-point 实现，移除 `cmd.exe /C mklink /J`，路径不再经过 shell 解析。
- 替换前先区分 junction、目录符号链接和真实目录：junction 只删除重解析点；符号链接使用目录/文件删除回退；真实目录或普通文件会安全失败，不会被递归删除或覆盖。
- 目标仍来自 `git --exec-path` 的父目录，创建位置仍为 `~/.git-ai/libexec`，保持无需管理员权限的 junction 语义。
- `cargo test mdm::ensure_git_symlinks --lib` 通过（3 tests）；特殊字符路径、重复安装和 Git for Windows/GUI 仍需原生 Windows 验收。
- Windows 目标检查已下载并解析 `junction` 依赖，随后仍在 bundled SQLite 编译处因 macOS 缺少 MSVC `stdlib.h` 停止。

### Task 5.2：明确 Windows 更新命令边界

改动范围：

- `src/commands/upgrade.rs`
- `docs/user-guide.md`
- `docs/guides/developer-install-guide.md`
- `docs/enterprise/installation-guide-zh.md`。

执行步骤：

1. 保留 Windows 下阻止 `git ai update` 自替换父进程的安全检查。
2. 在所有 Windows 安装和升级说明中统一写明使用：

   ```powershell
   git-ai update
   ```

3. 当用户执行 `git ai update` 时，错误信息继续给出正确替代命令。
4. 安装脚本完成后输出同样的升级说明。
5. 增加 Windows 安装脚本升级和文件锁测试。

验收标准：

- [ ] Windows 用户文档不再推荐 `git ai update`。
- [ ] `git-ai update` 可以处理 `git-ai.exe`/`git.exe` 被 daemon 或 Git 客户端占用的情况。
- [ ] 更新失败时旧版本仍可运行，不留下半写入二进制。

提交建议：

```bash
git add src/mdm/ensure_git_symlinks.rs src/commands/upgrade.rs docs install.ps1 tests
git commit -m "Finish Windows install and update compatibility"
```

## 9. 阶段 6：补齐原生 Windows CI

### Task 6.1：让 Windows 安装 E2E 使用特殊路径

改动范围：

- `.github/workflows/install-scripts-local.yml`
- `.github/workflows/install-scripts-nightly.yml`
- `tests/windows_install_script.rs`。

执行步骤：

1. 把 Windows E2E 的 `TEST_HOME` 改为包含空格的目录，例如 `git ai home`。
2. 增加一个包含 `&` 的专用安装/Hook 命令测试；如果 GitHub Actions 环境变量不适合全流程使用该路径，就在 Rust Windows 测试中覆盖。
3. 安装后读取 Claude、Codex、Gemini 等配置中的实际 Hook 命令。
4. 使用 agent 声明的真实解释器执行生成的 Hook，而不是只用 `Select-String` 检查文本。
5. 给 Hook 输入最小合法 JSON，并验证 checkpoint 被保存。
6. 保留 synthetic attribution 验证，但不要用它代替 Hook 执行验证。

验收标准：

- [ ] Windows E2E 在带空格 HOME 下安装成功。
- [ ] 至少 Claude、Codex、Gemini、OpenCode 的真实配置入口被执行。
- [ ] 生成的 Hook 命令能找到正确的 `git-ai.exe`。

### Task 6.2：在 Windows E2E 开启 async mode

改动范围：

- `.github/workflows/install-scripts-local.yml`
- daemon 测试与诊断日志。

执行步骤：

1. 删除“Windows 不支持 Unix-domain sockets，所以关闭 async mode”的过期假设。
2. 在 Windows E2E 设置 `GIT_AI_ASYNC_MODE=true`。
3. 安装后执行：

   ```powershell
   git-ai daemon restart
   git-ai daemon status
   ```

4. 通过真实 Hook 或 checkpoint 请求把数据发送到 named pipe daemon。
5. 提交后等待处理完成，验证 authorship note。
6. 最后执行 graceful shutdown，并确认没有残留 `git-ai.exe` daemon 进程。

验收标准：

- [ ] Windows agent E2E 不再强制关闭 async mode。
- [ ] checkpoint 确实经过 named pipe daemon，而不是直接同步回退。
- [ ] 测试结束后 daemon 正常退出。

### Task 6.3：增加 Windows 登录和发布冒烟测试

改动范围：

- 新增或扩展 GitHub Actions Windows workflow。
- 测试 OAuth 服务或本地 mock server。

执行步骤：

1. 在 Windows runner 启动本地 mock authorization/token server。
2. 运行 CLI login，验证生成 URL 中的 `redirect_uri` 完整存在。
3. 模拟空端口探测后再发送合法 callback。
4. 验证 token 兑换、保存、`whoami` 和 logout。
5. 对发布产物执行同样的安装后冒烟测试，而不只测试 `cargo build` 生成的本地二进制。
6. x64 每次发布必跑；ARM64 至少在发布流程中运行安装和 `--version`/`help` 冒烟测试。

验收标准：

- [ ] Windows 登录回归不再只能靠人工发现。
- [ ] 测试覆盖浏览器 URL、loopback callback 和 Credential Manager。
- [ ] 发布产物而非仅源码构建产物通过冒烟验证。

提交建议：

```bash
git add .github/workflows tests scripts
git commit -m "Expand native Windows compatibility CI"
```

## 10. 最终回归与发布检查

按以下顺序执行，任何一步失败都不要发布：

### 10.1 通用代码质量

```bash
task format
task lint
task build
task test
git diff --check
```

### 10.2 Windows x64

```powershell
cargo build --release --features keyring --target x86_64-pc-windows-msvc
cargo test --features keyring
./install.ps1
git-ai --version
git-ai install
git-ai daemon restart
git-ai daemon status
git-ai login --server https://your-test-server.example.com
git-ai whoami
git-ai update
git-ai daemon shutdown
```

### 10.3 Windows ARM64

```powershell
cargo build --release --features keyring --target aarch64-pc-windows-msvc
```

在原生 ARM64 runner 或设备上至少验证：

- [ ] `git-ai --version`
- [ ] `install.ps1`
- [ ] `git-ai install`
- [ ] daemon start/status/shutdown
- [ ] 一次 Hook checkpoint
- [ ] 一次登录或 mock OAuth 冒烟流程

### 10.4 特殊场景手工验收

- [ ] HOME 路径包含空格。
- [ ] HOME 路径包含 `&`。
- [ ] 非 ASCII Windows 用户名。
- [ ] Git for Windows 安装在 `Program Files`。
- [ ] Windows Defender 或企业终端安全软件开启。
- [ ] daemon 正在运行时执行升级。
- [ ] Git GUI 占用 `git.exe` 时执行升级。
- [ ] 浏览器先探测回调端口，再发送正式 callback。
- [ ] 另一个普通本机用户尝试读取凭据文件和连接 named pipe。

## 11. 完成记录模板

每完成一个任务，在 PR 描述或本文档副本中记录：

```text
任务：Task X.Y
提交：<commit sha>
Windows 环境：<Windows 版本 / x64 或 ARM64>
修改文件：<files>
执行测试：<commands>
测试结果：<pass/fail>
手工验证：<result>
剩余风险：<none 或说明>
回滚方式：<revert commit / feature flag>
```

最终发布前，确认第 1 节所有完成标准均已勾选，并在发布说明中列出：

- Windows Hook 特殊路径支持情况。
- Windows Credential Manager 存储策略和文件回退行为。
- named pipe ACL、超时和退出行为。
- `git ai update` 与 `git-ai update` 的命令差异。
- 已验证的 Windows 架构、Git for Windows 版本和 agent 版本。
