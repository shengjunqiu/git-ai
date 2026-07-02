#!/bin/bash
# ============================================================
# Git AI Enterprise Server - Database Migration Script
# ============================================================
# Usage: ./migrate.sh [--init|--upgrade]
#   --init    First-time initialization (runs all migrations)
#   --upgrade Run pending migrations only
#   (default) Same as --upgrade
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/../docker-compose.yml"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[INFO]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

# Check if .env exists
if [ ! -f "$SCRIPT_DIR/../.env" ]; then
    error ".env file not found. Copy .env.example to .env and configure it first."
fi

# Source .env
set -a; source "$SCRIPT_DIR/../.env"; set +a

DB_USER="${POSTGRES_USER:-gitai}"
DB_NAME="${POSTGRES_DB:-gitai_enterprise}"
DB_PASSWORD="${POSTGRES_PASSWORD:?POSTGRES_PASSWORD is required}"

MODE="${1:---upgrade}"

# Find the postgres container
PG_CONTAINER=$(docker compose -f "$COMPOSE_FILE" ps -q postgres 2>/dev/null | head -1)
if [ -z "$PG_CONTAINER" ]; then
    error "PostgreSQL container is not running. Start services first: docker compose up -d"
fi

MIGRATION_DIR="$SCRIPT_DIR/../migrations"

run_migration() {
    local sql_file="$1"
    local basename=$(basename "$sql_file")
    
    echo -e "${YELLOW}[MIGRATE]${NC} Running: $basename"
    
    docker exec -i "$(docker compose -f "$COMPOSE_FILE" ps -q postgres | head -1)" \
        psql -U "$DB_USER" -d "$DB_NAME" \
        -v ON_ERROR_STOP=1 \
        --single-transaction \
        -f /docker-entrypoint-initdb.d/"$basename" 2>&1
    
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}[OK]${NC} $basename applied successfully"
    else
        echo -e "${RED}[FAIL]${NC} $basename failed"
        return 1
    fi
}

case "$MODE" in
    --init)
        info "Running full initialization..."
        for f in $(ls "$MIGRATION_DIR"/*.sql 2>/dev/null | sort); do
            run_migration "$f"
        done
        info "Initialization complete!"
        ;;
    --upgrade)
        info "Running pending migrations..."
        # Check _migrations table
        PG_CONTAINER_ID=$(docker compose -f "$COMPOSE_FILE" ps -q postgres | head -1)
        APPLIED=$(docker exec "$PG_CONTAINER_ID" psql -U "$DB_USER" -d "$DB_NAME" -t -A -c \
            "SELECT version FROM _migrations ORDER BY version;" 2>/dev/null || echo "")
        
        for f in $(ls "$MIGRATION_DIR"/*.sql 2>/dev/null | sort); do
            basename=$(basename "$f" .sql)
            version=$(echo "$basename" | grep -oP '^\d+')
            if echo "$APPLIED" | grep -q "^${version}$"; then
                warn "Skipping $basename (already applied)"
            else
                run_migration "$f"
            fi
        done
        info "Upgrade complete!"
        ;;
    *)
        error "Unknown option: $MODE. Use --init or --upgrade"
        ;;
esac
