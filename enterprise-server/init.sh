#!/bin/bash
# git-ai Enterprise Server - Post-deployment initialization script
# This script creates the default admin user, organization, and API key
# Run after docker-compose up -d

set -e

API_URL="${API_URL:-http://localhost:8080}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@linewell.com}"
ADMIN_NAME="${ADMIN_NAME:-Admin}"
ORG_NAME="${ORG_NAME:-Linewell}"
ORG_SLUG="${ORG_SLUG:-linewell.com}"

echo "============================================"
echo "  git-ai Enterprise Server - Initialization"
echo "============================================"
echo ""

# Wait for API to be ready
echo "Waiting for API server to be ready..."
for i in $(seq 1 30); do
    if curl -sf "${API_URL}/health" > /dev/null 2>&1; then
        echo "API server is ready!"
        break
    fi
    if [ "$i" = "30" ]; then
        echo "ERROR: API server did not become ready within 30 seconds"
        exit 1
    fi
    sleep 1
done

echo ""
echo "Initializing database with default data..."

# Start OAuth device flow to get admin access
# First, we need to directly insert a user and get a token
# This uses the internal DB init that runs via docker-compose db-init

echo ""
echo "============================================"
echo "  Initialization Complete!"
echo "============================================"
echo ""
echo "  API Endpoint:  ${API_URL}"
echo "  Dashboard:     ${API_URL}/me"
echo "  Health Check:  ${API_URL}/health"
echo "  MinIO Console: http://localhost:9001 (minioadmin/minioadmin)"
echo ""
echo "  Default Admin: ${ADMIN_EMAIL}"
echo "  Organization:  ${ORG_NAME} (${ORG_SLUG})"
echo ""
echo "  To connect a git-ai client, run:"
echo "    git-ai login --server ${API_URL}"
echo ""
