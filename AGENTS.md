# Repository Guidelines

## 项目结构与模块组织

本仓库是 Rust 2024 项目，核心代码在 `src/`。`src/main.rs` 根据调用名分派：`git-ai` 进入 CLI 子命令，作为 `git` 调用时进入 Git 代理。`src/commands/` 放命令处理，`src/commands/hooks/` 放 commit、rebase、stash、reset 等 Git 钩子逻辑；`src/authorship/` 负责 checkpoint、working log、Git Notes 和行级归属；`src/git/` 封装 Git CLI 调用与仓库状态；`src/daemon/` 是后台服务路径。`tests/integration/` 是主要测试套件，`tests/fixtures/` 和 `tests/**/snapshots/` 保存夹具与 insta 快照。编辑器/agent 集成在 `agent-support/`，企业服务在 `enterprise-server/`，文档与规范在 `docs/`、`specs/`。更详细的目录边界见 `docs/repository-layout.md`。

## 构建、测试与本地开发

优先使用 Taskfile：

```bash
task dev       # 安装本地 debug 构建并重启 daemon，用于真实本地试跑
task build     # 只检查能否编译
task test      # 默认 daemon 模式运行测试
task lint      # cargo clippy --all-targets -- -D warnings
task format    # cargo fmt
```

可用 `task test TEST_FILTER=foo` 跑指定测试，`NO_CAPTURE=true` 查看测试输出。只有用户明确要求时才使用 `task test:wrapper-daemon` 或 `task test:wrapper`。

## 编码风格与命名约定

使用标准 `rustfmt` 格式，提交前运行 `task format` 和 `task lint`。生产代码通过 `std::process::Command` 调用真实 Git；`git2` 仅用于 `test-support`。路径写入 authorship/working log 前应 POSIX 化。错误统一走 `src/error.rs` 的 `GitAiError`，不要随意引入新的错误风格。跨平台逻辑用明确的 `#[cfg(unix)]` / `#[cfg(windows)]`。

## 测试指南

集成测试会创建真实 Git 仓库，并通过 `GIT_AI=git` 让 debug 二进制走代理路径。常用工具在 `tests/integration/repos/`：`TestRepo` 运行命令，`TestFile` 和 `lines![]` 断言内容与 AI/人类归属。涉及 checkpoint 顺序、部分暂存或 attribution 边界时，不要依赖 `file.set_contents` 的简化流程；手动写文件并显式调用 `mock_known_human`、`human`、`mock_ai`。每次提交后都断言行级归属。快照更新使用 `cargo insta review` 或 `cargo insta accept`。

## 提交与 Pull Request

当前检出的 Git 历史不可用，无法从本地历史推断固定提交格式；请使用简短、描述性的动宾短句，例如 `Fix reset attribution replay`。PR 应说明问题、实现思路、测试命令和结果，并链接相关 issue。涉及 UI、报告或截图输出时附前后对比；涉及 attribution、rewrite、daemon 或配置默认值时说明兼容性和迁移影响。

## 配置与架构注意事项

配置由 `Config::get()` 单例读取，测试可用 `GIT_AI_TEST_CONFIG_PATCH` 覆盖。功能开关遵循环境变量 `GIT_AI_*` > 配置文件 > 默认值。核心数据流是 checkpoint 写入 `.git/ai/working_logs/<base_commit>/`，commit 后生成 `authorship/3.0.0` Git Note 到 `refs/notes/ai`；改写历史时由 rewrite log 维护归属延续。
