#!/usr/bin/env bash
# ==============================================================
# git-ai report server — 启动脚本 (Linux / macOS)
# ==============================================================
# 用法：
#   ./scripts/start-server.sh                        # 直接运行二进制（前台）
#   ./scripts/start-server.sh --daemon               # 后台守护进程模式
#   ./scripts/start-server.sh --docker               # 使用 Docker Compose 启动
#   ./scripts/start-server.sh --docker --build       # 重新构建镜像后启动
#   ./scripts/start-server.sh --stop                 # 停止后台守护进程
#   ./scripts/start-server.sh --status               # 查看运行状态
# ==============================================================

set -euo pipefail

# ---------- 默认配置（可通过环境变量覆盖） ----------
ADDR="${GIT_AI_REPORT_ADDR:-0.0.0.0:8787}"
DB="${GIT_AI_REPORT_DB:-./data/report.sqlite}"
LOG_FILE="${GIT_AI_REPORT_LOG:-./data/server.log}"
PID_FILE="${GIT_AI_REPORT_PID:-./data/server.pid}"
BINARY="${GIT_AI_BINARY:-git-ai}"

# ---------- 颜色输出 ----------
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()    { echo -e "${CYAN}[git-ai]${NC} $*"; }
success() { echo -e "${GREEN}[git-ai]${NC} $*"; }
warn()    { echo -e "${YELLOW}[git-ai]${NC} $*"; }
error()   { echo -e "${RED}[git-ai]${NC} $*" >&2; }

# ---------- 解析参数 ----------
MODE="foreground"
DOCKER_BUILD=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --daemon)   MODE="daemon";   shift ;;
        --docker)   MODE="docker";   shift ;;
        --build)    DOCKER_BUILD=true; shift ;;
        --stop)     MODE="stop";     shift ;;
        --status)   MODE="status";   shift ;;
        --addr)     ADDR="$2";       shift 2 ;;
        --db)       DB="$2";         shift 2 ;;
        --help|-h)
            echo "Usage: $0 [--daemon|--docker [--build]|--stop|--status] [--addr HOST:PORT] [--db PATH]"
            exit 0
            ;;
        *)
            error "Unknown argument: $1"; exit 1 ;;
    esac
done

# ---------- 确保数据目录存在 ----------
DATA_DIR="$(dirname "$DB")"
mkdir -p "$DATA_DIR"

# ============================================================
# Docker Compose 模式
# ============================================================
if [[ "$MODE" == "docker" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    ROOT_DIR="$(dirname "$SCRIPT_DIR")"

    if ! command -v docker &>/dev/null; then
        error "Docker 未安装，请先安装 Docker: https://docs.docker.com/get-docker/"
        exit 1
    fi

    mkdir -p "$ROOT_DIR/data"

    info "使用 Docker Compose 启动 git-ai report server..."
    cd "$ROOT_DIR"

    if [[ "$DOCKER_BUILD" == true ]]; then
        info "重新构建 Docker 镜像..."
        docker compose build
    fi

    docker compose up -d

    info "等待服务就绪..."
    for i in $(seq 1 20); do
        if curl -sf "http://localhost:${ADDR##*:}/api/v1/aggregate/summary" &>/dev/null; then
            success "服务器已就绪！"
            break
        fi
        sleep 1
        [[ $i -eq 20 ]] && warn "服务未在预期时间内就绪，请检查日志：docker compose logs -f"
    done

    PORT="${ADDR##*:}"
    success "仪表盘地址: http://localhost:${PORT}/"
    info "查看日志:    docker compose logs -f"
    info "停止服务:    docker compose down"
    exit 0
fi

# ============================================================
# 停止守护进程
# ============================================================
if [[ "$MODE" == "stop" ]]; then
    if [[ -f "$PID_FILE" ]]; then
        PID=$(cat "$PID_FILE")
        if kill -0 "$PID" 2>/dev/null; then
            kill "$PID"
            rm -f "$PID_FILE"
            success "已停止 git-ai report server (PID $PID)"
        else
            warn "进程 $PID 已不存在，清理 PID 文件"
            rm -f "$PID_FILE"
        fi
    else
        warn "未找到 PID 文件，服务可能未在运行"
    fi
    exit 0
fi

# ============================================================
# 查看状态
# ============================================================
if [[ "$MODE" == "status" ]]; then
    PORT="${ADDR##*:}"
    if [[ -f "$PID_FILE" ]]; then
        PID=$(cat "$PID_FILE")
        if kill -0 "$PID" 2>/dev/null; then
            success "运行中 (PID $PID)"
        else
            warn "PID 文件存在但进程已退出"
        fi
    else
        info "未找到 PID 文件（可能未以 --daemon 模式启动）"
    fi

    # 检查 API 响应
    if curl -sf "http://localhost:${PORT}/api/v1/aggregate/summary" &>/dev/null; then
        success "HTTP 服务正常响应: http://localhost:${PORT}/"
    else
        warn "HTTP 服务无响应"
    fi
    exit 0
fi

# ============================================================
# 检查二进制是否可用
# ============================================================
if ! command -v "$BINARY" &>/dev/null; then
    # 尝试从项目 target 目录查找
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    ROOT_DIR="$(dirname "$SCRIPT_DIR")"
    if [[ -f "$ROOT_DIR/target/release/git-ai" ]]; then
        BINARY="$ROOT_DIR/target/release/git-ai"
    elif [[ -f "$ROOT_DIR/target/debug/git-ai" ]]; then
        BINARY="$ROOT_DIR/target/debug/git-ai"
        warn "使用 debug 构建，建议生产环境使用 release 构建"
    else
        error "找不到 git-ai 二进制文件。请先安装或构建："
        error "  curl -sSL https://usegitai.com/install.sh | bash"
        error "  或: cargo build --release"
        exit 1
    fi
fi

info "使用二进制: $BINARY"
info "监听地址:   $ADDR"
info "数据库路径: $DB"

# ============================================================
# 后台守护进程模式
# ============================================================
if [[ "$MODE" == "daemon" ]]; then
    if [[ -f "$PID_FILE" ]]; then
        OLD_PID=$(cat "$PID_FILE")
        if kill -0 "$OLD_PID" 2>/dev/null; then
            warn "服务已在运行 (PID $OLD_PID)，如需重启请先运行 --stop"
            exit 0
        fi
    fi

    nohup "$BINARY" report server --addr "$ADDR" --db "$DB" >> "$LOG_FILE" 2>&1 &
    echo $! > "$PID_FILE"
    PID=$(cat "$PID_FILE")
    success "git-ai report server 已在后台启动 (PID $PID)"
    info "日志文件: $LOG_FILE"

    # 等待服务就绪
    PORT="${ADDR##*:}"
    for i in $(seq 1 15); do
        if curl -sf "http://localhost:${PORT}/api/v1/aggregate/summary" &>/dev/null; then
            success "服务就绪！仪表盘: http://localhost:${PORT}/"
            break
        fi
        sleep 1
        [[ $i -eq 15 ]] && warn "服务未在预期时间内就绪，请检查日志: tail -f $LOG_FILE"
    done
    exit 0
fi

# ============================================================
# 前台模式（默认）
# ============================================================
PORT="${ADDR##*:}"
success "启动 git-ai report server (前台模式)"
info "仪表盘地址: http://localhost:${PORT}/"
info "按 Ctrl+C 停止"
echo ""

exec "$BINARY" report server --addr "$ADDR" --db "$DB"
