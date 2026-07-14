# git-ai 文档索引

本目录按主题分类组织项目文档。文档内部相互引用时统一使用相对于仓库根目录的路径（如 `docs/architecture/data-flow.md`）。

## 架构与设计 `architecture/`

- [data-flow.md](architecture/data-flow.md) — 核心数据流：数据在本地/服务端的生产、存储与消费路径。
- [repository-layout.md](architecture/repository-layout.md) — 仓库目录边界与模块组织。
- [system-roles-and-usage.md](architecture/system-roles-and-usage.md) — 系统角色、各角色如何使用系统、角色间数据流转。
- [user-system-architecture.md](architecture/user-system-architecture.md) — 用户体系架构分析。

## 开发者指南 `guides/`

- [developer-install-guide.md](guides/developer-install-guide.md) — 开发者安装与卸载、打包指南。
- [developer-end-to-end-workflow.md](guides/developer-end-to-end-workflow.md) — 开发者端到端使用流程。
- [local-run-guide.md](guides/local-run-guide.md) — 在本机完整跑起 CLI/daemon、report dashboard、企业服务端及编辑器插件。
- [report-summary-guide.md](guides/report-summary-guide.md) — 编码分析上报与可视化分析使用指南。
- [server-deployment.md](guides/server-deployment.md) — 旧版 Git-AI Report Server（SQLite / 8787）部署指南。

## 企业服务端 `enterprise/`

- [enterprise-server-planning.md](enterprise/enterprise-server-planning.md) — 企业服务端功能规划文档。
- [enterprise-server-deployment.md](enterprise/enterprise-server-deployment.md) — 企业服务端（Postgres/Redis/MinIO/CAS）部署教程。
- [enterprise-server-performance-optimization-plan.md](enterprise/enterprise-server-performance-optimization-plan.md) — 性能优化执行计划（吞吐/延迟/大数据量查询）。
- [enterprise-server-performance-task-plan.md](enterprise/enterprise-server-performance-task-plan.md) — 性能优化任务清单。
- [enterprise-server-concurrency-optimization.md](enterprise/enterprise-server-concurrency-optimization.md) — 并发一致性优化计划。
- [enterprise-server-concurrency-task-plan.md](enterprise/enterprise-server-concurrency-task-plan.md) — 并发优化任务清单。
- [enterprise-server-next-concurrency-optimization-task-plan.md](enterprise/enterprise-server-next-concurrency-optimization-task-plan.md) — 下一轮并发优化任务清单。
- [enterprise-auth-login-performance-task-plan.md](enterprise/enterprise-auth-login-performance-task-plan.md) — 认证登录性能优化任务清单。

## 开发计划 `plans/`

- [developer-registration-and-cli-auth-plan.md](plans/developer-registration-and-cli-auth-plan.md) — 开发者注册登录与 CLI 授权任务文档。
- [windows-compatibility-task-plan.md](plans/windows-compatibility-task-plan.md) — Windows 兼容性改进的分阶段执行与验收清单。

## 规范与策略 `reference/`

- [COVERAGE.md](reference/COVERAGE.md) — 代码覆盖率策略。

## 子项目文档

- [ai-usage-reporting-tool/](ai-usage-reporting-tool/) — AI 用量上报工具：开发计划、任务计划、服务端 API、阶段完成报告。
- [superpowers/](superpowers/) — 功能规格与实施计划：`plans/`（实施计划）与 `specs/`（设计规格）。
