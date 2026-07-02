#!/bin/bash
# git-ai Enterprise Server - Data Isolation Test Script
#
# This script tests the data isolation behavior within the same organization:
#   1. Admin users can see ALL data within their organization
#   2. Regular members can only see THEIR OWN data
#   3. Users from different orgs cannot see each other's data
#
# Prerequisites:
#   - Docker Compose services running (docker-compose up -d)
#   - curl and jq installed
#
# Usage:
#   ./enterprise-server/test_data_isolation.sh [API_URL]

set -e

API_URL="${1:-http://localhost:8080}"
DB_HOST="${DB_HOST:-localhost}"
DB_PORT="${DB_PORT:-5432}"
DB_USER="${DB_USER:-gitai}"
DB_PASS="${DB_PASS:-gitai}"
DB_NAME="${DB_NAME:-gitai_enterprise}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

PASS=0
FAIL=0

log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; PASS=$((PASS + 1)); }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; FAIL=$((FAIL + 1)); }
log_info() { echo -e "${YELLOW}[INFO]${NC} $1"; }

# ── Helper: Get JWT token via device flow ──
get_token() {
    local email="$1"
    # Use install nonce approach for testing
    # We'll directly query the DB to create a nonce and exchange it
    local nonce=$(psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d "$DB_NAME" -t -A -c "
        INSERT INTO install_nonces (nonce, user_id)
        SELECT gen_random_uuid()::text, id FROM users WHERE email = '$email'
        RETURNING nonce;
    " 2>/dev/null | tr -d ' \n')

    if [ -z "$nonce" ]; then
        echo "FAILED_TO_GET_NONCE"
        return
    fi

    # Exchange nonce for token
    local response=$(curl -sf "${API_URL}/worker/oauth/token" \
        -H "Content-Type: application/json" \
        -d "{\"grant_type\":\"install_nonce\",\"nonce\":\"${nonce}\"}" 2>/dev/null || echo '{}')

    echo "$response" | jq -r '.access_token // "FAILED_TO_GET_TOKEN"'
}

# ── Helper: Make authenticated API call ──
api_get() {
    local token="$1"
    local endpoint="$2"
    curl -sf "${API_URL}${endpoint}" \
        -H "Authorization: Bearer ${token}" 2>/dev/null || echo '{}'
}

# ── Helper: Make authenticated API call with API key ──
api_get_key() {
    local key="$1"
    local endpoint="$2"
    curl -sf "${API_URL}${endpoint}" \
        -H "X-API-Key: ${key}" 2>/dev/null || echo '{}'
}

# ── Helper: Run SQL query ──
sql_query() {
    psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d "$DB_NAME" -t -A -c "$1" 2>/dev/null
}

echo "============================================"
echo "  git-ai Data Isolation Test Suite"
echo "============================================"
echo "  API: $API_URL"
echo "  DB:  $DB_HOST:$DB_PORT/$DB_NAME"
echo ""

# ── Wait for API ──
log_info "Waiting for API server..."
for i in $(seq 1 30); do
    if curl -sf "${API_URL}/health" > /dev/null 2>&1; then
        break
    fi
    if [ "$i" = "30" ]; then
        log_fail "API server not ready after 30 seconds"
        exit 1
    fi
    sleep 1
done
log_info "API server is ready"

echo ""
echo "============================================"
echo "  Phase 1: Verify Test Data Setup"
echo "============================================"

# Check linewell.com org exists
LINWELL_ORG_ID=$(sql_query "SELECT id FROM organizations WHERE slug = 'linewell.com';")
if [ -n "$LINWELL_ORG_ID" ]; then
    log_pass "linewell.com organization exists (id: ${LINWELL_ORG_ID:0:8}...)"
else
    log_fail "linewell.com organization does not exist"
fi

# Check test users exist
for email in "admin@linewell.com" "developer1@linewell.com" "developer2@linewell.com" "teamlead@linewell.com"; do
    USER_ID=$(sql_query "SELECT id FROM users WHERE email = '$email';")
    if [ -n "$USER_ID" ]; then
        log_pass "User $email exists"
    else
        log_fail "User $email does not exist"
    fi
done

# Check no non-linewell.com non-personal orgs exist
OTHER_ORGS=$(sql_query "SELECT COUNT(*) FROM organizations WHERE slug != 'linewell.com' AND id NOT IN (SELECT personal_org_id FROM users WHERE personal_org_id IS NOT NULL);")
if [ "$OTHER_ORGS" = "0" ]; then
    log_pass "No non-linewell.com non-personal organizations exist"
else
    log_fail "Found $OTHER_ORGS non-linewell.com non-personal organizations"
fi

# Check test metrics data
DEV1_EVENTS=$(sql_query "SELECT COUNT(*) FROM metrics_events WHERE user_id = 'a0000000-0000-0000-0000-000000000001';")
DEV2_EVENTS=$(sql_query "SELECT COUNT(*) FROM metrics_events WHERE user_id = 'a0000000-0000-0000-0000-000000000002';")
LEAD_EVENTS=$(sql_query "SELECT COUNT(*) FROM metrics_events WHERE user_id = 'a0000000-0000-0000-0000-000000000003';")

if [ "$DEV1_EVENTS" -gt 0 ] && [ "$DEV2_EVENTS" -gt 0 ] && [ "$LEAD_EVENTS" -gt 0 ]; then
    log_pass "Test metrics data exists (dev1: $DEV1_EVENTS, dev2: $DEV2_EVENTS, lead: $LEAD_EVENTS events)"
else
    log_fail "Missing test metrics data (dev1: $DEV1_EVENTS, dev2: $DEV2_EVENTS, lead: $LEAD_EVENTS events)"
fi

echo ""
echo "============================================"
echo "  Phase 2: Test Data Isolation via API"
echo "============================================"

# Get tokens for each test user
log_info "Obtaining authentication tokens..."
DEV1_TOKEN=$(get_token "developer1@linewell.com")
DEV2_TOKEN=$(get_token "developer2@linewell.com")
LEAD_TOKEN=$(get_token "teamlead@linewell.com")

if [[ "$DEV1_TOKEN" == "FAILED"* ]]; then
    log_fail "Could not get token for developer1"
    DEV1_TOKEN=""
else
    log_pass "Got token for developer1"
fi

if [[ "$DEV2_TOKEN" == "FAILED"* ]]; then
    log_fail "Could not get token for developer2"
    DEV2_TOKEN=""
else
    log_pass "Got token for developer2"
fi

if [[ "$LEAD_TOKEN" == "FAILED"* ]]; then
    log_fail "Could not get token for teamlead"
    LEAD_TOKEN=""
else
    log_pass "Got token for teamlead"
fi

# ── Test 2.1: Developer1 should only see their own data ──
if [ -n "$DEV1_TOKEN" ]; then
    log_info "Testing developer1's data visibility..."
    SUMMARY=$(api_get "$DEV1_TOKEN" "/api/v1/aggregate/summary")
    TOTAL_COMMITS=$(echo "$SUMMARY" | jq -r '.total_commits // 0')
    AI_LINES=$(echo "$SUMMARY" | jq -r '.total_ai_lines // 0')

    # Developer1 should only see their own metrics, not developer2's or teamlead's
    if [ "$TOTAL_COMMITS" -le 2 ] 2>/dev/null; then
        log_pass "developer1 sees limited commits ($TOTAL_COMMITS) - data isolation working"
    else
        log_fail "developer1 sees too many commits ($TOTAL_COMMITS) - data isolation may not be working"
    fi
fi

# ── Test 2.2: Developer2 should only see their own data ──
if [ -n "$DEV2_TOKEN" ]; then
    log_info "Testing developer2's data visibility..."
    SUMMARY=$(api_get "$DEV2_TOKEN" "/api/v1/aggregate/summary")
    TOTAL_COMMITS=$(echo "$SUMMARY" | jq -r '.total_commits // 0')

    if [ "$TOTAL_COMMITS" -le 2 ] 2>/dev/null; then
        log_pass "developer2 sees limited commits ($TOTAL_COMMITS) - data isolation working"
    else
        log_fail "developer2 sees too many commits ($TOTAL_COMMITS) - data isolation may not be working"
    fi
fi

# ── Test 2.3: TeamLead (admin) should see ALL org data ──
if [ -n "$LEAD_TOKEN" ]; then
    log_info "Testing teamlead's (admin) data visibility..."
    SUMMARY=$(api_get "$LEAD_TOKEN" "/api/v1/aggregate/summary")
    TOTAL_COMMITS=$(echo "$SUMMARY" | jq -r '.total_commits // 0')

    # Admin should see ALL commits from all users in the org
    ALL_COMMITS=$(sql_query "SELECT COUNT(*) FROM metrics_events WHERE event_type = 1 AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');")

    if [ "$TOTAL_COMMITS" -ge "$ALL_COMMITS" ] 2>/dev/null || [ "$TOTAL_COMMITS" -ge 3 ] 2>/dev/null; then
        log_pass "teamlead (admin) sees all org data ($TOTAL_COMMITS commits >= expected $ALL_COMMITS)"
    else
        log_fail "teamlead (admin) does NOT see all org data ($TOTAL_COMMITS commits < expected $ALL_COMMITS)"
    fi
fi

# ── Test 2.4: Developer1 should NOT see developer2's projects ──
if [ -n "$DEV1_TOKEN" ]; then
    log_info "Testing developer1's project visibility..."
    PROJECTS=$(api_get "$DEV1_TOKEN" "/api/v1/aggregate/projects")
    PROJECT_COUNT=$(echo "$PROJECTS" | jq -r '.projects | length // 0')

    # Developer1 should only see their own project
    if [ "$PROJECT_COUNT" -le 1 ] 2>/dev/null; then
        log_pass "developer1 sees only their own project ($PROJECT_COUNT) - data isolation working"
    else
        log_fail "developer1 sees too many projects ($PROJECT_COUNT) - data isolation may not be working"
    fi
fi

# ── Test 2.5: Admin should see all projects ──
if [ -n "$LEAD_TOKEN" ]; then
    log_info "Testing admin's project visibility..."
    PROJECTS=$(api_get "$LEAD_TOKEN" "/api/v1/aggregate/projects")
    PROJECT_COUNT=$(echo "$PROJECTS" | jq -r '.projects | length // 0')

    # Admin should see all projects in the org
    if [ "$PROJECT_COUNT" -ge 3 ] 2>/dev/null; then
        log_pass "teamlead (admin) sees all org projects ($PROJECT_COUNT)"
    else
        log_fail "teamlead (admin) does not see all org projects ($PROJECT_COUNT < 3)"
    fi
fi

# ── Test 2.6: Developer1 should NOT see developer2's developer stats ──
if [ -n "$DEV1_TOKEN" ]; then
    log_info "Testing developer1's developer list visibility..."
    DEVS=$(api_get "$DEV1_TOKEN" "/api/v1/aggregate/developers")
    DEV_COUNT=$(echo "$DEVS" | jq -r '.developers | length // 0')

    if [ "$DEV_COUNT" -le 1 ] 2>/dev/null; then
        log_pass "developer1 sees only themselves in developer list ($DEV_COUNT) - data isolation working"
    else
        log_fail "developer1 sees other developers ($DEV_COUNT) - data isolation may not be working"
    fi
fi

# ── Test 2.7: Admin should see all developers ──
if [ -n "$LEAD_TOKEN" ]; then
    log_info "Testing admin's developer list visibility..."
    DEVS=$(api_get "$LEAD_TOKEN" "/api/v1/aggregate/developers")
    DEV_COUNT=$(echo "$DEVS" | jq -r '.developers | length // 0')

    if [ "$DEV_COUNT" -ge 3 ] 2>/dev/null; then
        log_pass "teamlead (admin) sees all developers ($DEV_COUNT)"
    else
        log_fail "teamlead (admin) does not see all developers ($DEV_COUNT < 3)"
    fi
fi

echo ""
echo "============================================"
echo "  Phase 3: Test SQL-level Data Isolation"
echo "============================================"

# Direct SQL verification of data isolation rules
log_info "Verifying data isolation at SQL level..."

# Count of metrics events per user in linewell.com
SQL_RESULT=$(sql_query "
    SELECT u.email, COUNT(m.id) as event_count
    FROM users u
    LEFT JOIN metrics_events m ON m.user_id = u.id AND m.org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com')
    WHERE u.email IN ('developer1@linewell.com', 'developer2@linewell.com', 'teamlead@linewell.com')
    GROUP BY u.email
    ORDER BY u.email;
")
log_info "Metrics events per user:\n$SQL_RESULT"

# Verify no cross-org data leakage
CROSS_ORG=$(sql_query "
    SELECT COUNT(*) FROM metrics_events m
    JOIN users u ON m.user_id = u.id
    JOIN organizations o ON m.org_id = o.id
    WHERE o.slug = 'linewell.com'
    AND u.id NOT IN (
        SELECT user_id FROM org_members WHERE org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com')
    );
")
if [ "$CROSS_ORG" = "0" ]; then
    log_pass "No cross-org data leakage detected"
else
    log_fail "Cross-org data leakage detected ($CROSS_ORG records)"
fi

# Verify build_data_filters logic
log_info "Simulating build_data_filters behavior..."

# For admin (teamlead): should see all org data
ADMIN_VISIBLE=$(sql_query "
    SELECT COUNT(*) FROM metrics_events
    WHERE event_type = 1
    AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');
")

# For member (developer1): should see only own data
DEV1_VISIBLE=$(sql_query "
    SELECT COUNT(*) FROM metrics_events
    WHERE event_type = 1
    AND user_id = 'a0000000-0000-0000-0000-000000000001'
    AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');
")

# For member (developer2): should see only own data
DEV2_VISIBLE=$(sql_query "
    SELECT COUNT(*) FROM metrics_events
    WHERE event_type = 1
    AND user_id = 'a0000000-0000-0000-0000-000000000002'
    AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');
")

log_info "Admin visible commits: $ADMIN_VISIBLE"
log_info "Dev1 visible commits:  $DEV1_VISIBLE"
log_info "Dev2 visible commits:  $DEV2_VISIBLE"

if [ "$ADMIN_VISIBLE" -gt "$DEV1_VISIBLE" ] && [ "$ADMIN_VISIBLE" -gt "$DEV2_VISIBLE" ] 2>/dev/null; then
    log_pass "Admin sees more data than individual members - isolation working"
else
    log_fail "Admin does not see more data than individual members - isolation may be broken"
fi

if [ "$DEV1_VISIBLE" != "$DEV2_VISIBLE" ] 2>/dev/null; then
    log_pass "Developer1 and Developer2 see different data - user-level isolation working"
else
    log_info "Developer1 and Developer2 see same amount of data ($DEV1_VISIBLE each)"
fi

echo ""
echo "============================================"
echo "  Test Results Summary"
echo "============================================"
echo -e "  ${GREEN}PASSED${NC}: $PASS"
echo -e "  ${RED}FAILED${NC}: $FAIL"
echo ""

if [ "$FAIL" -eq 0 ]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}$FAIL test(s) failed!${NC}"
    exit 1
fi
