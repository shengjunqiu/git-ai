# ============================================================
# git-ai 服务端打包脚本 (Windows PowerShell)
# 使用 Docker 多阶段构建，产出：
#   dist\git-ai-server-<版本>-linux-x64.tar.gz
#   dist\git-ai-linux-x64              (裸 Linux 二进制)
#   Docker 镜像: git-ai-server:<版本>
#
# 前提: Docker Desktop 已安装并启动 (Linux 容器模式)
# 用法:
#   .\scripts\package-server.ps1
#   .\scripts\package-server.ps1 -Push        # 同时推送镜像
#   .\scripts\package-server.ps1 -Registry "registry.example.com/myorg"
# ============================================================

[CmdletBinding()]
param(
    [switch]$Push,
    [string]$Registry = "",
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RootDir   = Split-Path -Parent $ScriptDir
$DistDir   = Join-Path $RootDir "dist"

function Write-Info    { param($msg) Write-Host "[package-server] $msg" -ForegroundColor Cyan }
function Write-Success { param($msg) Write-Host "[package-server] $msg" -ForegroundColor Green }
function Write-Warn    { param($msg) Write-Host "[package-server] $msg" -ForegroundColor Yellow }
function Write-Err     { param($msg) Write-Host "[package-server] ERROR: $msg" -ForegroundColor Red; exit 1 }

if ($Help) {
    Write-Host @"
Usage: .\scripts\package-server.ps1 [options]

Options:
  -Push                Push image to registry after build
  -Registry <url>      Registry prefix (e.g. registry.example.com/myorg)
  -Help                Show this help

Examples:
  .\scripts\package-server.ps1
  .\scripts\package-server.ps1 -Push -Registry "registry.example.com/myorg"
"@
    exit 0
}

# ── 检查 Docker ───────────────────────────────────────────────
if ($null -eq (Get-Command "docker" -ErrorAction SilentlyContinue)) {
    Write-Err "Docker not found. Please install Docker Desktop: https://docs.docker.com/desktop/windows/"
}

$dockerInfo = docker info 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Err "Docker is not running. Please start Docker Desktop (Linux container mode)."
}

# ── 读取版本 ─────────────────────────────────────────────────
$CargoToml = Get-Content (Join-Path $RootDir "Cargo.toml") -Raw
if ($CargoToml -match 'version\s*=\s*"([^"]+)"') {
    $Version = $Matches[1]
} else {
    Write-Err "Cannot read version from Cargo.toml"
}
Write-Info "git-ai version: $Version"

$ImageName   = "git-ai-server"
$BuilderTag  = "git-ai-builder-tmp-$([System.IO.Path]::GetRandomFileName().Replace('.',''))"

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null

# ── Step 1: 构建 builder 阶段 ────────────────────────────────
Write-Info "Building Docker image (builder stage)..."
docker build --target builder --tag $BuilderTag $RootDir
if ($LASTEXITCODE -ne 0) { Write-Err "Docker build failed" }

# ── Step 2: 提取 Linux 二进制 ────────────────────────────────
Write-Info "Extracting Linux binary from Docker image..."
$ContainerId = (docker create $BuilderTag /bin/true).Trim()

try {
    $LinuxBin = Join-Path $DistDir "git-ai-linux-x64"
    docker cp "${ContainerId}:/build/target/release/git-ai" $LinuxBin
    if ($LASTEXITCODE -ne 0) { Write-Err "Failed to extract binary" }
    Write-Success "Extracted: $LinuxBin"
} finally {
    docker rm -f $ContainerId 2>$null | Out-Null
    docker rmi -f $BuilderTag 2>$null | Out-Null
}

# ── Step 3: 打包服务端部署包 ─────────────────────────────────
Write-Info "Creating server deployment archive..."

$TmpDir = Join-Path $env:TEMP "git-ai-server-pkg-$(Get-Random)"
New-Item -ItemType Directory -Path $TmpDir | Out-Null

try {
    Copy-Item $LinuxBin          (Join-Path $TmpDir "git-ai")
    $DockerCompose = Join-Path $RootDir "docker-compose.yml"
    $InstallSh     = Join-Path $RootDir "install.sh"
    $DeployDoc     = Join-Path $RootDir "docs\server-deployment.md"
    if (Test-Path $DockerCompose) { Copy-Item $DockerCompose (Join-Path $TmpDir "docker-compose.yml") }
    if (Test-Path $InstallSh)     { Copy-Item $InstallSh     (Join-Path $TmpDir "install.sh") }
    if (Test-Path $DeployDoc)     { Copy-Item $DeployDoc     (Join-Path $TmpDir "SERVER-DEPLOY.md") }

    # SHA256
    $BinHash = (Get-FileHash (Join-Path $TmpDir "git-ai") -Algorithm SHA256).Hash.ToLower()
    [System.IO.File]::WriteAllText(
        (Join-Path $TmpDir "SHA256SUMS"),
        "$BinHash  git-ai`n",
        [System.Text.Encoding]::UTF8
    )

    $ServerArchive = "git-ai-server-${Version}-linux-x64.tar.gz"
    $ArchivePath   = Join-Path $DistDir $ServerArchive

    $prevPwd = $PWD
    Set-Location $TmpDir
    tar -czf $ArchivePath *
    Set-Location $prevPwd

    if ($LASTEXITCODE -ne 0) { Write-Err "tar packaging failed" }
    Write-Success "Archive: $ArchivePath"

    # 追加到全局 SHA256
    $AHash = (Get-FileHash $ArchivePath -Algorithm SHA256).Hash.ToLower()
    $sha256File = Join-Path $DistDir "SHA256SUMS.txt"
    [System.IO.File]::AppendAllText(
        $sha256File,
        "$AHash  $ServerArchive`n",
        [System.Text.Encoding]::UTF8
    )
} finally {
    Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}

# ── Step 4: 构建完整运行时镜像 ──────────────────────────────
$FullImageName = if ($Registry -ne "") { "$Registry/$ImageName" } else { $ImageName }

Write-Info "Building runtime Docker image: ${FullImageName}:${Version} ..."
docker build `
    --tag "${FullImageName}:${Version}" `
    --tag "${FullImageName}:latest" `
    $RootDir
if ($LASTEXITCODE -ne 0) { Write-Err "Docker runtime image build failed" }
Write-Success "Docker image built: ${FullImageName}:${Version}"

if ($Push) {
    Write-Info "Pushing image to registry..."
    docker push "${FullImageName}:${Version}"
    docker push "${FullImageName}:latest"
    Write-Success "Pushed: ${FullImageName}:${Version}"
}

# ── 汇总 ────────────────────────────────────────────────────
Write-Host ""
Write-Success "=== Server packaging complete ==="
Write-Host "   Binary : dist\git-ai-linux-x64"
Write-Host "   Archive: dist\$ServerArchive"
Write-Host "   Image  : ${FullImageName}:${Version}"
Write-Host ""
Write-Info "Deploy on Linux server:"
Write-Host "   # 方式一：直接运行二进制"
Write-Host "   scp dist\git-ai-linux-x64 user@server:/usr/local/bin/git-ai"
Write-Host ""
Write-Host "   # 方式二：Docker Compose"
Write-Host "   scp dist\$ServerArchive user@server:~/"
Write-Host "   ssh user@server 'tar -xzf $ServerArchive && docker compose up -d'"
