# Repository Layout

本仓库是一个产品型 monorepo：根目录是 `git-ai` CLI/daemon，旁边包含企业服务、编辑器插件、agent 集成脚本、安装/发布脚本和测试资产。

## 项目单元

| 路径 | 类型 | 说明 |
| --- | --- | --- |
| `/` | Rust crate | `git-ai` CLI、Git 代理、daemon 和核心 authorship 逻辑。 |
| `enterprise-server/` | Rust crate | 企业服务端，包含 API、数据库迁移、内嵌 dashboard 页面、Docker 和部署文件。 |
| `agent-support/vscode/` | TypeScript 项目 | VS Code/Cursor/Windsurf 扩展。 |
| `agent-support/opencode/` | TypeScript 项目 | OpenCode 插件。 |
| `agent-support/intellij/` | Gradle/Kotlin 项目 | IntelliJ 平台插件。 |
| `agent-support/amp/`, `agent-support/pi/` | 集成脚本 | 轻量 agent preset 脚本，不是独立 package。 |

## 根目录职责

- `src/`：主 Rust 源码。`main.rs` 根据调用名分派到 `git-ai` 命令模式或 `git` 代理模式。
- `tests/integration/`：主集成测试套件，创建真实 Git 仓库验证 attribution、rewrite、daemon 和 agent preset。
- `tests/fixtures/`：测试夹具。新增 fixture 时应能解释其稳定性和用途。
- `scripts/`：本地开发、打包、demo 数据和 benchmark 脚本。
- `docs/`：开发和运维文档。
- `specs/`：Git AI 数据格式和长期设计规范。
- `assets/`：README、文档和插件使用的静态资源。
- `skills/`：安装给 agent 使用的技能说明。
- `data/`：本地 report server 运行数据目录，只保留占位和说明文件。

## 前端代码位置

本仓库没有独立的 Web 前端项目，也没有 React/Vue/Vite/Next 这类单独构建入口。前端页面和后端代码放在一起：

- `enterprise-server/src/handlers/dashboard.rs`：企业仪表盘页面，HTML/CSS/JavaScript 由 Rust handler 返回。
- `enterprise-server/src/handlers/login.rs`、`verify.rs`、`bundle_view.rs`：登录、设备验证和分享页面。
- `src/report/server.rs`：本地 report server 的 dashboard HTML、CSS 和浏览器端 JavaScript。
- `agent-support/vscode/`、`agent-support/intellij/`：编辑器插件 UI/交互代码，不是 Web 前端应用。

修改这些页面时，需要同时考虑后端路由、认证、API 响应结构和页面脚本；不要按独立前端项目查找构建入口。

## 不应提交的内容

以下内容是本地生成物或发布产物，应由 `.gitignore` 排除：

- Cargo/Gradle/TypeScript 构建目录：`target/`、`build/`、`dist/`、`node_modules/`。
- 本地数据库和日志：`*.sqlite`、`*.log`、`dev_stdout.txt`、`test-output.txt`。
- 打包产物：`enterprise-server/deploy/images/`、`enterprise-server/*-deploy.tar.gz`。
- OS/IDE 临时文件：`.DS_Store`、`.idea/`、大部分 `.vscode/*`。

## 组织原则

新增代码优先放入已有边界：Git 子命令放 `src/commands/`，Git 行为封装放 `src/git/`，归属计算放 `src/authorship/`，后台处理放 `src/daemon/`。新增子项目需要有独立清单文件和 README；只有单个 agent preset 脚本时，放在 `agent-support/<agent>/` 并在 `agent-support/README.md` 中登记。
