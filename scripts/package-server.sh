#!/bin/bash
# ============================================================
# git-ai 服务端打包脚本 (Linux / macOS / WSL)
# 使用 Docker 多阶段构建，产出：
#   dist/git-ai-server-<版本>-linux-x64.tar.gz
#   dist/git-ai-linux-x64              (裸二进制，可直接运行)
#
# 前提: Docker 已安装并运行
# 用法:
#   ./scripts/package-server.sh
#   ./scripts/package-server.sh --push   # 同时推送镜像到 registry
# ============================================================

set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
IMAGE_NAME="git-ai-server"
BUILDER_TAG="git-ai-builder-tmp-$$"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()    { echo -e "${CYAN}[package-server]${NC} $*"; }
success() { echo -e "${GREEN}[package-server]${NC} $*"; }
warn()    { echo -e "${YELLOW}[package-server]${NC} $*"; }
error()   { echo -e "${RED}[package-server] ERROR:${NC} $*" >&2; exit 1; }

PUSH_IMAGE=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --push) PUSH_IMAGE=true; shift ;;
        --help|-h)
            echo "Usage: $0 [--push]"
            exit 0 ;;
        *) error "Unknown argument: $1" ;;
    esac
done

# ── 检查 Docker ───────────────────────────────────────────────
if ! command -v docker &>/dev/null; then
    error "Docker not found. Please install Docker: https://docs.docker.com/get-docker/"
fi
if ! docker info &>/dev/null; then
    error "Docker daemon is not running. Please start Docker."
fi

# ── 读取版本 ─────────────────────────────────────────────────
VERSION="$(grep '^version' "$ROOT_DIR/Cargo.toml" | head -n1 | sed 's/.*= *"\(.*\)"/\1/')"
info "git-ai version: $VERSION"

mkdir -p "$DIST_DIR"

# ── Step 1: 构建 builder 阶段镜像（提取二进制用）────────────
info "Building Docker image (builder stage)..."
docker build \
    --target builder \
    --tag "$BUILDER_TAG" \
    "$ROOT_DIR"

# ── Step 2: 从镜像中提取 Linux x64 二进制 ───────────────────
info "Extracting Linux binary from Docker image..."
TMP_CONTAINER="$(docker create "$BUILDER_TAG" /bin/true)"
trap 'docker rm -f "$TMP_CONTAINER" 2>/dev/null || true; docker rmi -f "$BUILDER_TAG" 2>/dev/null || true' EXIT

LINUX_BIN="$DIST_DIR/git-ai-linux-x64"
docker cp "$TMP_CONTAINER:/build/target/release/git-ai" "$LINUX_BIN"
chmod +x "$LINUX_BIN"
success "Extracted: $LINUX_BIN"

# ── Step 3: 打包服务端部署包 ─────────────────────────────────
info "Creating server deployment archive..."

TMP_PKG_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_PKG_DIR"; docker rm -f "$TMP_CONTAINER" 2>/dev/null || true; docker rmi -f "$BUILDER_TAG" 2>/dev/null || true' EXIT

cp "$LINUX_BIN"                              "$TMP_PKG_DIR/git-ai"
cp "$ROOT_DIR/install.sh"                    "$TMP_PKG_DIR/install.sh"    2>/dev/null || true
cp "$ROOT_DIR/docker-compose.yml"            "$TMP_PKG_DIR/docker-compose.yml" 2>/dev/null || true
cp "$ROOT_DIR/docs/server-deployment.md"     "$TMP_PKG_DIR/SERVER-DEPLOY.md"   2>/dev/null || true

# SHA256
cd "$TMP_PKG_DIR"
if command -v sha256sum &>/dev/null; then
    sha256sum git-ai > SHA256SUMS
elif command -v shasum &>/dev/null; then
    shasum -a 256 git-ai > SHA256SUMS
fi

SERVER_ARCHIVE="git-ai-server-${VERSION}-linux-x64.tar.gz"
ARCHIVE_PATH="$DIST_DIR/$SERVER_ARCHIVE"
tar -czf "$ARCHIVE_PATH" ./*
success "Archive: $ARCHIVE_PATH"

# 追加到全局校验文件
cd "$DIST_DIR"
if command -v sha256sum &>/dev/null; then
    sha256sum "$SERVER_ARCHIVE" >> SHA256SUMS.txt
elif command -v shasum &>/dev/null; then
    shasum -a 256 "$SERVER_ARCHIVE" >> SHA256SUMS.txt
fi

# ── Step 4: 构建完整运行时镜像（可选推送）───────────────────
info "Building runtime Docker image: $IMAGE_NAME:$VERSION ..."
docker build \
    --tag "$IMAGE_NAME:$VERSION" \
    --tag "$IMAGE_NAME:latest" \
    "$ROOT_DIR"

success "Docker image built: $IMAGE_NAME:$VERSION"

if $PUSH_IMAGE; then
    info "Pushing image to registry..."
    docker push "$IMAGE_NAME:$VERSION"
    docker push "$IMAGE_NAME:latest"
    success "Pushed: $IMAGE_NAME:$VERSION"
fi

# ── 汇总 ────────────────────────────────────────────────────
echo ""
success "=== Server packaging complete ==="
echo "   Binary : dist/git-ai-linux-x64"
echo "   Archive: dist/$SERVER_ARCHIVE"
echo "   Image  : $IMAGE_NAME:$VERSION"
echo ""
info "Deploy on Linux server:"
echo "   # 方式一：直接运行二进制"
echo "   scp dist/git-ai-linux-x64 user@server:/usr/local/bin/git-ai"
echo "   ssh user@server 'chmod +x /usr/local/bin/git-ai && git-ai report server --addr 0.0.0.0:8787 --db /data/report.sqlite'"
echo ""
echo "   # 方式二：Docker Compose"
echo "   scp dist/$SERVER_ARCHIVE user@server:~/"
echo "   ssh user@server 'tar -xzf $SERVER_ARCHIVE && docker compose up -d'"
