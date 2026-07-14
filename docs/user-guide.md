# git-ai 用户使用指南

本文面向公司内部开发人员，介绍如何注册账号、安装 git-ai CLI、完成首次登录授权，以及配置常用编辑器和 AI 编程工具。

> 公司 git-ai 服务地址：`https://117.147.213.234:38080`
>
> 注册时必须使用公司企业邮箱。

## 1. 注册账号

在浏览器中打开：

<https://117.147.213.234:38080/auth/register>

按照页面提示填写姓名、企业邮箱和密码，并选择所属组织与部门。注册成功后请保持浏览器登录状态，后续 CLI 授权会直接使用当前账号。

## 2. 安装前准备

安装前请确认电脑已安装 Git：

```bash
git --version
```

如果命令无法执行，请先安装 Git：

- macOS：安装 Xcode Command Line Tools 或其他 Git 发行版。
- Linux：使用系统包管理器安装 `git`、`curl` 和 `bash`。
- Windows：安装 Git for Windows，并确认 PowerShell 中可以执行 `git --version`。

## 3. 安装 git-ai CLI

### 3.1 macOS

打开“终端”，执行：

```bash
curl -fsSL \
  https://117.147.213.234:38080/worker/releases/latest/download/install.sh |
  bash
```

安装器会自动识别 Intel 或 Apple Silicon 芯片，并将 git-ai 安装到 `~/.git-ai/bin/`。

### 3.2 Linux

打开终端，执行：

```bash
curl -fsSL \
  https://117.147.213.234:38080/worker/releases/latest/download/install.sh |
  bash
```

安装器会自动识别 x64 或 ARM64 架构，并将 git-ai 安装到 `~/.git-ai/bin/`。

### 3.3 Windows

打开 PowerShell，先临时允许当前窗口执行安装脚本：

```powershell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
```

然后执行安装：

```powershell
irm "https://117.147.213.234:38080/worker/releases/latest/download/install.ps1" | iex
```

git-ai 会安装到 `%USERPROFILE%\.git-ai\bin\`。

## 4. 首次登录与浏览器授权

安装完成后，CLI 需要通过浏览器授权获取当前账号的登录凭证。

macOS 和 Linux：

```bash
~/.git-ai/bin/git-ai login --server https://117.147.213.234:38080
```

Windows PowerShell：

```powershell
& "$HOME\.git-ai\bin\git-ai.exe" login --server "https://117.147.213.234:38080"
```

命令执行后：

1. 浏览器会自动打开 CLI 授权页面。
2. 如果浏览器尚未登录，请先使用注册时的账号登录。
3. 确认页面显示的是本人账号、组织和部门。
4. 点击“授权”，等待终端提示登录成功。

登录成功后，公司服务地址会自动保存到本机配置中，后续无需再次传入 `--server`。

## 5. 验证安装结果

建议关闭并重新打开终端以及正在运行的 IDE，然后执行以下命令。

macOS 和 Linux：

```bash
git-ai --version
git-ai whoami
which git
```

Windows PowerShell：

```powershell
git-ai --version
git-ai whoami
where.exe git
```

请确认：

- `git-ai --version` 能正常显示版本号。
- `git-ai whoami` 显示本人的企业邮箱、组织和角色。
- `git` 优先指向 `.git-ai/bin` 目录下的代理程序。

## 6. 编辑器与 AI 工具集成

安装脚本会自动检测已安装的编辑器和 AI 编程工具，并配置相应的插件、扩展或 hooks。通常不需要手动执行额外命令。

如果安装 CLI 后才安装新的编辑器或 AI 工具，或者自动配置失败，请重新执行：

```bash
git-ai install-hooks
```

完成后重新启动对应的终端、编辑器或 AI 工具。

当前安装器可自动检测或配置的工具包括：



### VS Code / Cursor 推荐设置

在 `settings.json` 中加入：

```json
{
  "gitai.enableCheckpointLogging": true,
  "gitai.experiments.aiTabTracking": true
}
```

其中 `gitai.experiments.aiTabTracking` 为实验性功能，用于追踪 AI Tab 补全内容。修改设置后请重新启动 VS Code 或 Cursor。

## 7. 日常使用

安装和授权完成后，无需为每个 Git 仓库单独初始化。正常使用支持的 AI 工具编辑代码并执行 Git 命令即可。

在 Git 仓库中可以运行：

```bash
git-ai status
git-ai stats
```

- `git-ai status`：查看当前仓库的 git-ai 状态。
- `git-ai stats`：查看 AI 与人工代码归属统计。

## 8. 退出或切换账号

退出当前 CLI 账号：

```bash
git-ai logout
```

重新登录：

```bash
git-ai login --server https://117.147.213.234:38080
```

如果浏览器中仍保留其他账号的登录状态，请先在网页端退出，再重新执行 CLI 登录。

## 9. 常见问题

### 安装后提示 `git-ai: command not found`

请先关闭并重新打开终端。macOS 或 Linux 也可以手动加载当前 shell 配置：

```bash
source ~/.zshrc  # zsh
source ~/.bashrc # bash
```

### Windows 中 `where.exe git` 没有优先显示 `.git-ai\bin\git.exe`

关闭并重新打开 PowerShell、Git Bash 和 IDE。如果仍未生效，请确认 `%USERPROFILE%\.git-ai\bin` 已加入 PATH，并排在原 Git 路径之前。

### 浏览器没有自动打开

使用以下命令打印授权地址，然后手动复制到浏览器：

```bash
git-ai login --server https://117.147.213.234:38080 --no-browser
```

### 登录后身份不正确

先退出 CLI，然后确认浏览器登录的是本人账号，再重新授权：

```bash
git-ai logout
git-ai login --server https://117.147.213.234:38080
```
