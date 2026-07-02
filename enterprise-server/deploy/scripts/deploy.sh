#!/bin/bash
# ============================================================
# Git AI Enterprise Server - Deployment Script
# ============================================================
# This script deploys the enterprise server on a target machine.
#
# Prerequisites:
#   - Docker and Docker Compose installed
#   - At least 2GB RAM, 10GB disk
#   - Ports 8080, 5433, 6379, 9000, 9001 available
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[INFO]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

# Step 1: Load Docker images
info "Loading Docker images..."
if [ -d "$SCRIPT_DIR/images" ]; then
    for img in "$SCRIPT_DIR"/images/*.tar; do
        if [ -f "$img" ]; then
            info "Loading: $(basename $img)"
            docker load -i "$img"
        fi
    done
    info "Images loaded successfully"
else
    warn "No images directory found. Make sure images are already loaded."
fi

# Step 2: Check .env
if [ ! -f "$SCRIPT_DIR/.env" ]; then
    warn ".env not found. Creating from .env.example..."
    cp "$SCRIPT_DIR/.env.example" "$SCRIPT_DIR/.env"
    error "Please edit .env with your configuration, then re-run this script."
fi

# Step 3: Start services
info "Starting services..."
docker compose -f "$SCRIPT_DIR/docker-compose.yml" up -d

# Step 4: Wait for health checks
info "Waiting for services to be healthy..."
sleep 15

# Check API health
API_HEALTH=$(curl -sf http://localhost:${API_PORT:-8080}/health 2>/dev/null || echo "FAILED")
if echo "$API_HEALTH" | grep -q '"status":"ok"'; then
    info "API is healthy: $API_HEALTH"
else
    warn "API health check returned: $API_HEALTH"
    warn "Check logs: docker compose logs api"
fi

# Step 5: Run migrations
info "Running database migrations..."
bash "$SCRIPT_DIR/scripts/migrate.sh" --init || true

info "======================================"
info "Deployment complete!"
info "API:      http://localhost:${API_PORT:-8080}"
info "MinIO:    http://localhost:${MINIO_CONSOLE_PORT:-9001}"
info "Postgres: localhost:${POSTGRES_PORT:-5433}"
info "======================================"
