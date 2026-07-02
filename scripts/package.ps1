# ============================================================
# git-ai 开发者安装包打包脚本 (Windows PowerShell)
# 用法:
#   .\scripts\package.ps1                   # 打包当前平台 (Windows x64)
#   .\scripts\package.ps1 -Target x86_64-pc-windows-msvc
#   .\scripts\package.ps1 -All              # 所有平台 (需要 cross/WSL)
# ============================================================

[CmdletBinding()]
param(
    [switch]$All,
    [string]$Target = "",
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RootDir   = Split-Path -Parent $ScriptDir
$DistDir   = Join-Path $RootDir "dist"

function Write-Info    { param($msg) Write-Host "[package] $msg" -ForegroundColor Cyan }
function Write-Success { param($msg) Write-Host "[package] $msg" -ForegroundColor Green }
function Write-Warn    { param($msg) Write-Host "[package] $msg" -ForegroundColor Yellow }
function Write-Err     { param($msg) Write-Host "[package] ERROR: $msg" -ForegroundColor Red; exit 1 }

if ($Help) {
    Write-Host @"
Usage: .\scripts\package.ps1 [options]

Options:
  -All              Build all platforms (requires cross / WSL)
  -Target <triple>  Build specific Rust target triple
  -Help             Show this help

Examples:
  .\scripts\package.ps1
  .\scripts\package.ps1 -Target x86_64-pc-windows-msvc
  .\scripts\package.ps1 -All
"@
    exit 0
}

# ── 读取版本 ─────────────────────────────────────────────────
$CargoToml = Get-Content (Join-Path $RootDir "Cargo.toml") -Raw
if ($CargoToml -match 'version\s*=\s*"([^"]+)"') {
    $Version = $Matches[1]
} else {
    Write-Err "Cannot read version from Cargo.toml"
}
Write-Info "git-ai version: $Version"

# ── 目标平台 ─────────────────────────────────────────────────
if ($All) {
    $Targets = @(
        "x86_64-pc-windows-msvc",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin"
    )
} elseif ($Target -ne "") {
    $Targets = @($Target)
} else {
    # 默认当前平台
    $NativeTarget = (rustc -vV 2>$null | Select-String "^host:").ToString().Split(" ")[1].Trim()
    $Targets = @($NativeTarget)
}

Write-Info "Build targets: $($Targets -join ', ')"

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null

function Get-FriendlyName([string]$t) {
    switch ($t) {
        "x86_64-pc-windows-msvc"    { return "windows-x64" }
        "x86_64-pc-windows-gnu"     { return "windows-x64" }
        "x86_64-unknown-linux-gnu"  { return "linux-x64" }
        "aarch64-unknown-linux-gnu" { return "linux-arm64" }
        "x86_64-apple-darwin"       { return "macos-x64" }
        "aarch64-apple-darwin"      { return "macos-arm64" }
        default                     { return $t }
    }
}

function Is-WindowsTarget([string]$t) {
    return $t -match "windows"
}

$Built  = [System.Collections.Generic.List[string]]::new()
$Failed = [System.Collections.Generic.List[string]]::new()

foreach ($BuildTarget in $Targets) {
    $Friendly = Get-FriendlyName $BuildTarget
    Write-Info "Building for $BuildTarget ($Friendly)..."

    Set-Location $RootDir

    # 判断是否是原生目标
    $NativeTarget = (rustc -vV 2>$null | Select-String "^host:").ToString().Split(" ")[1].Trim()
    $IsNative = ($BuildTarget -eq $NativeTarget)

    if (-not $IsNative) {
        # 非原生目标：在 Windows 上仅能通过 cross (需要 Docker) 或 WSL 编译
        $crossAvailable = $null -ne (Get-Command "cross" -ErrorAction SilentlyContinue)
        if (-not $crossAvailable) {
            Write-Warn "cross not installed, skipping $BuildTarget"
            Write-Warn "For cross-compilation on Windows, install Docker + cross:"
            Write-Warn "  cargo install cross --git https://github.com/cross-rs/cross"
            $Failed.Add("$BuildTarget (cross not installed)")
            continue
        }
        $BuildCmd = "cross"
    } else {
        $BuildCmd = "cargo"
    }

    try {
        & $BuildCmd build --release --locked --bin git-ai --target $BuildTarget
        if ($LASTEXITCODE -ne 0) { throw "Build failed" }
    } catch {
        Write-Warn "Failed to build for $BuildTarget"
        $Failed.Add($BuildTarget)
        continue
    }

    # ── 定位产物 ──────────────────────────────────────────────
    $IsWin = Is-WindowsTarget $BuildTarget
    if ($IsWin) {
        $BinSrc     = Join-Path $RootDir "target\$BuildTarget\release\git-ai.exe"
        $BinaryName = "git-ai.exe"
        $ArchiveName = "git-ai-$Version-$Friendly.zip"
    } else {
        $BinSrc     = Join-Path $RootDir "target\$BuildTarget\release\git-ai"
        $BinaryName = "git-ai"
        $ArchiveName = "git-ai-$Version-$Friendly.tar.gz"
    }

    if (-not (Test-Path $BinSrc)) {
        Write-Warn "Binary not found at $BinSrc"
        $Failed.Add("$BuildTarget (binary not found)")
        continue
    }

    # ── 打包 ──────────────────────────────────────────────────
    $TmpDir = Join-Path $env:TEMP "git-ai-pkg-$(Get-Random)"
    New-Item -ItemType Directory -Path $TmpDir | Out-Null

    Copy-Item $BinSrc (Join-Path $TmpDir $BinaryName)

    # SHA256 校验 (compute early so we can embed into install scripts)
    $Hash = (Get-FileHash (Join-Path $TmpDir $BinaryName) -Algorithm SHA256).Hash.ToLower()

    $ReadmeSrc  = Join-Path $RootDir "README.md"
    $InstallPs1Src = Join-Path $RootDir "install.ps1"
    $InstallShSrc  = Join-Path $RootDir "install.sh"
    if (Test-Path $ReadmeSrc)  { Copy-Item $ReadmeSrc  (Join-Path $TmpDir "README.md") }

    # ── 替换 install.ps1 占位符 ──────────────────────────────
    if (Test-Path $InstallPs1Src) {
        $installContent = Get-Content $InstallPs1Src -Raw
        # Replace version placeholder
        $installContent = $installContent -replace "__VERSION_PLACEHOLDER__", "v$Version"
        # Replace checksums placeholder with actual checksum
        $installContent = $installContent -replace "__CHECKSUMS_PLACEHOLDER__", "$Hash  $BinaryName"
        Set-Content -Path (Join-Path $TmpDir "install.ps1") -Value $installContent -NoNewline
    }

    # ── 替换 install.sh 占位符 ──────────────────────────────
    if (Test-Path $InstallShSrc) {
        $installShContent = Get-Content $InstallShSrc -Raw
        # Replace version placeholder
        $installShContent = $installShContent -replace "__VERSION_PLACEHOLDER__", "v$Version"
        # Replace checksums placeholder with actual checksum
        $installShContent = $installShContent -replace "__CHECKSUMS_PLACEHOLDER__", "$Hash  $BinaryName"
        # Write with LF line endings for Unix compatibility
        $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText((Join-Path $TmpDir "install.sh"), $installShContent, $utf8NoBom)
    }

    # SHA256 校验文件
    "$Hash  $BinaryName" | Set-Content (Join-Path $TmpDir "SHA256SUMS")

    $ArchivePath = Join-Path $DistDir $ArchiveName

    if ($IsWin) {
        Compress-Archive -Path "$TmpDir\*" -DestinationPath $ArchivePath -Force
    } else {
        # 在 Windows 上通过 tar 打包（Windows 10 1803+ 自带）
        $prevPwd = $PWD
        Set-Location $TmpDir
        try {
            tar -czf $ArchivePath *
        } catch {
            Write-Warn "tar not available, copying raw binary"
            Copy-Item $BinSrc (Join-Path $DistDir "git-ai-$Friendly")
        }
        Set-Location $prevPwd
    }

    # 裸二进制（供 GIT_AI_LOCAL_BINARY 使用）
    if ($IsWin) {
        Copy-Item $BinSrc (Join-Path $DistDir "git-ai-$Friendly.exe")
    } else {
        Copy-Item $BinSrc (Join-Path $DistDir "git-ai-$Friendly")
    }

    # 追加到全局 SHA256
    $ArchiveHash = (Get-FileHash $ArchivePath -Algorithm SHA256).Hash.ToLower()
    "$ArchiveHash  $ArchiveName" | Add-Content (Join-Path $DistDir "SHA256SUMS.txt")

    Remove-Item -Recurse -Force $TmpDir

    Write-Success "  -> $ArchivePath"
    $Built.Add($ArchiveName)
}

# ── 汇总 ────────────────────────────────────────────────────
Write-Host ""
if ($Built.Count -gt 0) {
    Write-Success "=== Build complete: $($Built.Count) package(s) ==="
    foreach ($pkg in $Built) {
        Write-Host "   dist\$pkg"
    }
}

if ($Failed.Count -gt 0) {
    Write-Warn "=== Failed: $($Failed.Count) target(s) ==="
    foreach ($f in $Failed) {
        Write-Host "   $f"
    }
}

Write-Host ""
Write-Info "Packages saved to: $DistDir"
Write-Info "Quick install on Windows:"
Write-Host "  .\install.ps1  (after extracting the zip)"
