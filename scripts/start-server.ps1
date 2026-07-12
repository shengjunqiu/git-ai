# ==============================================================
# git-ai report server — 启动脚本 (Windows PowerShell)
# ==============================================================
# 用法：
#   .\scripts\start-server.ps1                        # 直接运行（前台）
#   .\scripts\start-server.ps1 -Daemon                # 后台进程模式
#   .\scripts\start-server.ps1 -Docker                # 使用 Docker Compose 启动
#   .\scripts\start-server.ps1 -Docker -Build         # 重新构建镜像后启动
#   .\scripts\start-server.ps1 -Stop                  # 停止后台进程
#   .\scripts\start-server.ps1 -Status                # 查看运行状态
# ==============================================================

param(
    [string]$Addr   = $env:GIT_AI_REPORT_ADDR ?? "0.0.0.0:8787",
    [string]$Db     = $env:GIT_AI_REPORT_DB   ?? ".\data\report.sqlite",
    [string]$LogFile = ".\data\server.log",
    [string]$PidFile = ".\data\server.pid",
    [switch]$Daemon,
    [switch]$Docker,
    [switch]$Build,
    [switch]$Stop,
    [switch]$Status
)

$ErrorActionPreference = "Stop"

# ---------- 颜色输出 ----------
function Info    { param($msg) Write-Host "[git-ai] $msg" -ForegroundColor Cyan }
function Success { param($msg) Write-Host "[git-ai] $msg" -ForegroundColor Green }
function Warn    { param($msg) Write-Host "[git-ai] $msg" -ForegroundColor Yellow }
function Err     { param($msg) Write-Host "[git-ai] $msg" -ForegroundColor Red }

# ---------- 确保数据目录存在 ----------
$DataDir = Split-Path $Db -Parent
if (-not (Test-Path $DataDir)) {
    New-Item -ItemType Directory -Path $DataDir -Force | Out-Null
}

$Port = $Addr -replace ".*:", ""

# ============================================================
# Docker Compose 模式
# ============================================================
if ($Docker) {
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        Err "Docker 未安装，请先安装 Docker Desktop: https://docs.docker.com/desktop/windows/"
        exit 1
    }

    $RootDir = Split-Path $PSScriptRoot -Parent
    New-Item -ItemType Directory -Path "$RootDir\data" -Force | Out-Null

    Info "使用 Docker Compose 启动 git-ai report server..."
    Push-Location $RootDir
    try {
        if ($Build) {
            Info "重新构建 Docker 镜像..."
            docker compose build
        }
        docker compose up -d
    } finally {
        Pop-Location
    }

    Info "等待服务就绪..."
    $ready = $false
    for ($i = 1; $i -le 20; $i++) {
        try {
            Invoke-RestMethod "http://localhost:$Port/api/v1/aggregate/summary" -ErrorAction Stop | Out-Null
            $ready = $true; break
        } catch { Start-Sleep 1 }
    }

    if ($ready) {
        Success "服务器已就绪！"
        Success "仪表盘地址: http://localhost:$Port/"
    } else {
        Warn "服务未在预期时间内就绪，请检查日志：docker compose logs -f"
    }

    Info "查看日志: docker compose logs -f"
    Info "停止服务: docker compose down"
    exit 0
}

# ============================================================
# 停止后台进程
# ============================================================
if ($Stop) {
    if (Test-Path $PidFile) {
        $Pid_ = Get-Content $PidFile
        try {
            $proc = Get-Process -Id $Pid_ -ErrorAction Stop
            Stop-Process -Id $Pid_ -Force
            Remove-Item $PidFile -Force
            Success "已停止 git-ai report server (PID $Pid_)"
        } catch {
            Warn "进程 $Pid_ 已不存在，清理 PID 文件"
            Remove-Item $PidFile -Force -ErrorAction SilentlyContinue
        }
    } else {
        Warn "未找到 PID 文件，服务可能未在运行"
    }
    exit 0
}

# ============================================================
# 查看状态
# ============================================================
if ($Status) {
    if (Test-Path $PidFile) {
        $Pid_ = Get-Content $PidFile
        try {
            Get-Process -Id $Pid_ -ErrorAction Stop | Out-Null
            Success "运行中 (PID $Pid_)"
        } catch {
            Warn "PID 文件存在但进程已退出"
        }
    } else {
        Info "未找到 PID 文件（可能未以 -Daemon 模式启动）"
    }

    try {
        Invoke-RestMethod "http://localhost:$Port/api/v1/aggregate/summary" -ErrorAction Stop | Out-Null
        Success "HTTP 服务正常响应: http://localhost:$Port/"
    } catch {
        Warn "HTTP 服务无响应"
    }
    exit 0
}

# ============================================================
# 查找 git-ai 二进制
# ============================================================
$Binary = $null
if (Get-Command "git-ai" -ErrorAction SilentlyContinue) {
    $Binary = "git-ai"
} else {
    $RootDir = Split-Path $PSScriptRoot -Parent
    $Candidates = @(
        "$RootDir\target\release\git-ai.exe",
        "$RootDir\target\debug\git-ai.exe"
    )
    foreach ($c in $Candidates) {
        if (Test-Path $c) {
            $Binary = $c
            if ($c -match "debug") { Warn "使用 debug 构建，建议生产环境使用 release 构建" }
            break
        }
    }
}

if (-not $Binary) {
    Err "找不到 git-ai 二进制文件。请先安装或构建："
    Err "  powershell -NoProfile -ExecutionPolicy Bypass -Command `"irm https://usegitai.com/install.ps1 | iex`""
    Err "  或: cargo build --release"
    exit 1
}

Info "使用二进制: $Binary"
Info "监听地址:   $Addr"
Info "数据库路径: $Db"

# ============================================================
# 后台守护进程模式
# ============================================================
if ($Daemon) {
    if (Test-Path $PidFile) {
        $OldPid = Get-Content $PidFile
        try {
            Get-Process -Id $OldPid -ErrorAction Stop | Out-Null
            Warn "服务已在运行 (PID $OldPid)，如需重启请先运行 -Stop"
            exit 0
        } catch { }
    }

    $proc = Start-Process `
        -FilePath $Binary `
        -ArgumentList "report", "server", "--addr", $Addr, "--db", $Db `
        -RedirectStandardError $LogFile `
        -WindowStyle Hidden `
        -PassThru

    $proc.Id | Out-File $PidFile -Encoding ascii
    Success "git-ai report server 已在后台启动 (PID $($proc.Id))"
    Info "日志文件: $LogFile"

    # 等待服务就绪
    $ready = $false
    for ($i = 1; $i -le 15; $i++) {
        try {
            Invoke-RestMethod "http://localhost:$Port/api/v1/aggregate/summary" -ErrorAction Stop | Out-Null
            $ready = $true; break
        } catch { Start-Sleep 1 }
    }

    if ($ready) {
        Success "服务就绪！仪表盘: http://localhost:$Port/"
    } else {
        Warn "服务未在预期时间内就绪，请检查日志: Get-Content $LogFile -Tail 20"
    }
    exit 0
}

# ============================================================
# 前台模式（默认）
# ============================================================
Success "启动 git-ai report server (前台模式)"
Info "仪表盘地址: http://localhost:$Port/"
Info "按 Ctrl+C 停止"
Write-Host ""

& $Binary report server --addr $Addr --db $Db
