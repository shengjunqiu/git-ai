# Agent Support

`agent-support/` 存放编辑器插件和 agent 集成脚本。这里既有完整子项目，也有只随主程序安装或复制的轻量脚本。

## 完整子项目

| 路径 | 构建系统 | 用途 |
| --- | --- | --- |
| `vscode/` | Yarn + TypeScript | VS Code、Cursor、Windsurf 等兼容 VS Code 扩展的编辑器支持。 |
| `opencode/` | Yarn + TypeScript | OpenCode 插件。 |
| `intellij/` | Gradle + Kotlin | IntelliJ 平台插件。 |

这些目录有自己的依赖清单、测试或构建命令，修改时优先查看各自 README。

## 轻量集成脚本

| 路径 | 说明 |
| --- | --- |
| `amp/` | Amp agent 的 `git-ai.ts` 集成脚本。 |
| `pi/` | Pi agent 的 `git-ai.ts` 集成脚本。 |

新增轻量 agent 支持时，使用 `agent-support/<agent-name>/git-ai.ts` 的结构；只有需要独立依赖、测试或发布流程时，才添加 `package.json` 或新的构建系统。
