#!/bin/bash
# ============================================================
# git-ai 开发者安装包打包脚本 (Linux / macOS)
# 用法:
#   ./scripts/package.sh                  # 打包当前平台
#   ./scripts/package.sh --all            # 交叉编译所有平台 (需要安装 cross)
#   ./scripts/package.sh --target x86_64-unknown-linux-gnu
# ============================================================

set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()    { echo -e "${CYAN}[package]${NC} $*"; }
success() { echo -e "${GREEN}[package]${NC} $*"; }
warn()    { echo -e "${YELLOW}[package]${NC} $*"; }
error()   { echo -e "${RED}[package] ERROR:${NC} $*" >&2; exit 1; }

# ── 读取 Cargo.toml 中的版本 ────────────────────────────────
get_version() {
    grep '^version' "$ROOT_DIR/Cargo.toml" | head -n1 | sed 's/.*= *"\(.*\)"/\1/'
}

VERSION="$(get_version)"
info "git-ai version: $VERSION"

# ── 解析参数 ────────────────────────────────────────────────
BUILD_ALL=false
TARGETS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --all)
            BUILD_ALL=true
            shift
            ;;
        --target)
            TARGETS+=("$2")
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--all] [--target <triple>]"
            echo ""
            echo "Examples:"
            echo "  $0                              # 当前平台 release 编译"
            echo "  $0 --all                        # 所有平台 (需要 cross)"
            echo "  $0 --target aarch64-apple-darwin"
            exit 0
            ;;
        *)
            error "Unknown argument: $1"
            ;;
    esac
done

# ── 确定目标平台列表 ─────────────────────────────────────────
if $BUILD_ALL; then
    TARGETS=(
        "x86_64-unknown-linux-gnu"
        "aarch64-unknown-linux-gnu"
        "x86_64-apple-darwin"
        "aarch64-apple-darwin"
        "x86_64-pc-windows-gnu"
    )
elif [[ ${#TARGETS[@]} -eq 0 ]]; then
    # 默认：当前平台
    NATIVE_TARGET="$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')"
    TARGETS=("$NATIVE_TARGET")
fi

info "Build targets: ${TARGETS[*]}"

mkdir -p "$DIST_DIR"

# ── 平台名称映射 ─────────────────────────────────────────────
target_to_friendly() {
    local t="$1"
    case "$t" in
        x86_64-unknown-linux-gnu)   echo "linux-x64" ;;
        aarch64-unknown-linux-gnu)  echo "linux-arm64" ;;
        x86_64-apple-darwin)        echo "macos-x64" ;;
        aarch64-apple-darwin)       echo "macos-arm64" ;;
        x86_64-pc-windows-gnu)      echo "windows-x64" ;;
        x86_64-pc-windows-msvc)     echo "windows-x64" ;;
        *)                          echo "$t" ;;
    esac
}

is_windows_target() {
    [[ "$1" == *windows* ]]
}

# ── 逐目标编译 ───────────────────────────────────────────────
BUILT=()
FAILED=()

for TARGET in "${TARGETS[@]}"; do
    FRIENDLY="$(target_to_friendly "$TARGET")"
    info "Building for $TARGET ($FRIENDLY)..."

    cd "$ROOT_DIR"

    # 判断是否使用 cross（交叉编译）
    USE_CROSS=false
    NATIVE="$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')"
    if [[ "$TARGET" != "$NATIVE" ]]; then
        if command -v cross &>/dev/null; then
            USE_CROSS=true
        else
            warn "cross not installed, skipping non-native target $TARGET"
            warn "Install cross: cargo install cross --git https://github.com/cross-rs/cross"
            FAILED+=("$TARGET (cross not installed)")
            continue
        fi
    fi

    BUILD_CMD="cargo"
    if $USE_CROSS; then
        BUILD_CMD="cross"
    fi

    if ! $BUILD_CMD build --release --locked --bin git-ai --target "$TARGET" 2>&1; then
        warn "Failed to build for $TARGET"
        FAILED+=("$TARGET")
        continue
    fi

    # ── 定位编译产物 ─────────────────────────────────────────
    if is_windows_target "$TARGET"; then
        BIN_SRC="$ROOT_DIR/target/$TARGET/release/git-ai.exe"
        ARCHIVE_NAME="git-ai-${VERSION}-${FRIENDLY}.zip"
        BINARY_NAME="git-ai.exe"
    else
        BIN_SRC="$ROOT_DIR/target/$TARGET/release/git-ai"
        ARCHIVE_NAME="git-ai-${VERSION}-${FRIENDLY}.tar.gz"
        BINARY_NAME="git-ai"
    fi

    if [[ ! -f "$BIN_SRC" ]]; then
        warn "Binary not found at $BIN_SRC, skipping"
        FAILED+=("$TARGET (binary not found)")
        continue
    fi

    # ── 打包 ─────────────────────────────────────────────────
    TMP_PKG_DIR="$(mktemp -d)"
    trap 'rm -rf "$TMP_PKG_DIR"' EXIT

    cp "$BIN_SRC"                     "$TMP_PKG_DIR/$BINARY_NAME"

    # Compute SHA256 early so we can embed into install scripts
    BINARY_HASH=""
    if command -v sha256sum &>/dev/null; then
        BINARY_HASH=$(sha256sum "$TMP_PKG_DIR/$BINARY_NAME" | awk '{print $1}')
    elif command -v shasum &>/dev/null; then
        BINARY_HASH=$(shasum -a 256 "$TMP_PKG_DIR/$BINARY_NAME" | awk '{print $1}')
    fi

    # ── Replace install.sh placeholders ──────────────────────
    if [ -f "$ROOT_DIR/install.sh" ]; then
        sed -e "s/__VERSION_PLACEHOLDER__/v${VERSION}/g" \
            -e "s/__CHECKSUMS_PLACEHOLDER__/${BINARY_HASH}  ${BINARY_NAME}/g" \
            "$ROOT_DIR/install.sh" > "$TMP_PKG_DIR/install.sh"
        chmod +x "$TMP_PKG_DIR/install.sh"
    fi

    # ── Replace install.ps1 placeholders ─────────────────────
    if [ -f "$ROOT_DIR/install.ps1" ]; then
        sed -e "s/__VERSION_PLACEHOLDER__/v${VERSION}/g" \
            -e "s/__CHECKSUMS_PLACEHOLDER__/${BINARY_HASH}  ${BINARY_NAME}.exe/g" \
            "$ROOT_DIR/install.ps1" > "$TMP_PKG_DIR/install.ps1"
    fi

    cp "$ROOT_DIR/README.md"          "$TMP_PKG_DIR/README.md"    2>/dev/null || true

    # 生成 SHA256 校验文件
    cd "$TMP_PKG_DIR"
    if [ -n "$BINARY_HASH" ]; then
        echo "$BINARY_HASH  $BINARY_NAME" > "SHA256SUMS"
    elif command -v sha256sum &>/dev/null; then
        sha256sum "$BINARY_NAME" > "SHA256SUMS"
    elif command -v shasum &>/dev/null; then
        shasum -a 256 "$BINARY_NAME" > "SHA256SUMS"
    fi

    ARCHIVE_PATH="$DIST_DIR/$ARCHIVE_NAME"

    if is_windows_target "$TARGET"; then
        if command -v zip &>/dev/null; then
            zip -q "$ARCHIVE_PATH" ./*
        else
            warn "zip not available, producing raw binary only"
            cp "$BINARY_NAME" "$DIST_DIR/git-ai-${VERSION}-${FRIENDLY}.exe"
        fi
    else
        tar -czf "$ARCHIVE_PATH" ./*
    fi

    # 同时输出裸二进制（供 install.sh 的 GIT_AI_LOCAL_BINARY 使用）
    cp "$BIN_SRC" "$DIST_DIR/git-ai-${FRIENDLY}"
    if is_windows_target "$TARGET"; then
        cp "$BIN_SRC" "$DIST_DIR/git-ai-${FRIENDLY}.exe"
    fi

    # 顶层 SHA256
    cd "$DIST_DIR"
    if command -v sha256sum &>/dev/null; then
        sha256sum "$ARCHIVE_NAME" >> "SHA256SUMS.txt" 2>/dev/null || true
    elif command -v shasum &>/dev/null; then
        shasum -a 256 "$ARCHIVE_NAME" >> "SHA256SUMS.txt" 2>/dev/null || true
    fi

    rm -rf "$TMP_PKG_DIR"
    trap - EXIT

    success "  -> $ARCHIVE_PATH"
    BUILT+=("$ARCHIVE_NAME")
done

# ── 汇总 ────────────────────────────────────────────────────
echo ""
if [[ ${#BUILT[@]} -gt 0 ]]; then
    success "=== Build complete: ${#BUILT[@]} package(s) ==="
    for pkg in "${BUILT[@]}"; do
        echo "   dist/$pkg"
    done
fi

if [[ ${#FAILED[@]} -gt 0 ]]; then
    warn "=== Failed: ${#FAILED[@]} target(s) ==="
    for f in "${FAILED[@]}"; do
        echo "   $f"
    done
fi

echo ""
info "Packages saved to: $DIST_DIR"
info "Quick install (current machine):"
echo "   GIT_AI_LOCAL_BINARY=$DIST_DIR/git-ai-\$(uname -s | tr A-Z a-z)-\$(uname -m | sed 's/x86_64/x64/;s/aarch64/arm64/') bash install.sh"
