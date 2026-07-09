# git-ai 开发者安装指南

本文档面向 **普通开发者**，说明如何获取并安装 git-ai 客户端工具。

---

## 安装包说明

| 文件名 | 平台 | 用途 |
|---|---|---|
| `git-ai-<版本>-windows-x64.zip` | Windows 64位 | 开发者安装包 |
| `git-ai-<版本>-linux-x64.tar.gz` | Linux 64位 | 开发者安装包 |
| `git-ai-<版本>-linux-arm64.tar.gz` | Linux ARM64 | 服务器/树莓派 |
| `git-ai-<版本>-macos-x64.tar.gz` | macOS Intel | 开发者安装包 |
| `git-ai-<版本>-macos-arm64.tar.gz` | macOS Apple Silicon | 开发者安装包 |
| `SHA256SUMS.txt` | — | 校验文件 |

> 从管理员处获取对应平台的安装包，或访问项目 Release 页面下载。

---

## Windows 安装

### 方式一：解压后运行安装脚本（推荐）

1. 解压 `git-ai-<版本>-windows-x64.zip` 到任意目录
2. 以 **普通权限** 打开 PowerShell，进入解压目录
3. 执行安装脚本：

```powershell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
.\install.ps1
```

安装后会自动：
- 将 `git-ai.exe` 安装到 `%USERPROFILE%\.git-ai\bin\`
- 创建 `git.exe` 符号链接（代理所有 git 命令）
- 将安装目录添加到当前用户的 `PATH`

4. **重新打开** PowerShell / 终端，验证安装：

```powershell
git-ai --version
git --version   # 应显示 git-ai 代理版本
```

### 方式二：手动安装

```powershell
# 解压后手动复制
$InstallDir = "$env:USERPROFILE\.git-ai\bin"
New-Item -ItemType Directory -Force -Path $InstallDir
Copy-Item .\git-ai.exe $InstallDir\git-ai.exe
# 创建 git 代理符号链接
New-Item -ItemType SymbolicLink -Path "$InstallDir\git.exe" -Target "$InstallDir\git-ai.exe" -Force
# 添加到 PATH（永久）
$userPath = [Environment]::GetEnvironmentVariable("PATH","User")
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("PATH","$InstallDir;$userPath","User")
}
```

---

## Linux / macOS 安装

### 方式一：安装脚本

```bash
# 解压
tar -xzf git-ai-<版本>-linux-x64.tar.gz   # Linux
tar -xzf git-ai-<版本>-macos-arm64.tar.gz  # macOS Apple Silicon

# 运行安装脚本
chmod +x install.sh
./install.sh
```

安装后会自动配置 `~/.git-ai/bin` 并更新 shell 配置文件（`.bashrc` / `.zshrc` / `config.fish`）。

**重新加载 shell：**
```bash
source ~/.bashrc   # bash
source ~/.zshrc    # zsh
```

### 方式二：使用本地二进制直接安装

```bash
# 指定本地二进制跳过下载，直接安装
GIT_AI_LOCAL_BINARY=./git-ai-linux-x64 bash install.sh
```

### 验证安装

```bash
git-ai --version
git --version     # 显示 git-ai 代理版本
git-ai status     # 查看当前仓库 AI 归因状态
```

---

## 校验安装包完整性

```bash
# Linux / macOS
sha256sum -c SHA256SUMS.txt

# Windows PowerShell
Get-Content SHA256SUMS.txt | ForEach-Object {
    $hash, $file = $_ -split '\s+', 2
    $actual = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
    if ($actual -eq $hash) { Write-Host "OK: $file" -ForegroundColor Green }
    else { Write-Host "FAIL: $file" -ForegroundColor Red }
}
```

---

## 卸载

```bash
# Linux / macOS
rm -rf ~/.git-ai
# 从 shell 配置文件中删除 PATH 中的 ~/.git-ai/bin 行
```

```powershell
# Windows
Remove-Item -Recurse -Force "$env:USERPROFILE\.git-ai"
# 从用户 PATH 中删除对应条目
```

---

## 常见问题

### `git: command not found` 或 git 命令没有经过 git-ai 代理

确认 `~/.git-ai/bin`（Linux/macOS）或 `%USERPROFILE%\.git-ai\bin`（Windows）在 `PATH` **最前面**，且该目录下有 `git` 符号链接指向 `git-ai`。

```bash
which git        # 应显示 ~/.git-ai/bin/git
git --version    # 应包含 "git-ai" 字样
```

### Windows 提示"无法加载脚本"

```powershell
Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned
```

### macOS 提示"无法验证开发者"

```bash
xattr -d com.apple.quarantine ~/.git-ai/bin/git-ai
```

---

## 打包新版本（维护者）

维护者在 CI 或本地打包时，使用以下脚本：

```bash
# Linux / macOS - 打包当前平台
./scripts/package.sh

# Linux / macOS - 打包所有平台（需要 cross）
./scripts/package.sh --all

# Windows - 打包当前平台
.\scripts\package.ps1

# Windows - 打包指定目标
.\scripts\package.ps1 -Target x86_64-pc-windows-msvc
```

产物保存到 `dist/` 目录：
```
dist/
  git-ai-1.3.2-windows-x64.zip
  git-ai-1.3.2-linux-x64.tar.gz
  git-ai-1.3.2-macos-arm64.tar.gz
  git-ai-windows-x64.exe          ← 裸二进制（供 GIT_AI_LOCAL_BINARY 使用）
  git-ai-linux-x64                 ← 裸二进制
  SHA256SUMS.txt
```
