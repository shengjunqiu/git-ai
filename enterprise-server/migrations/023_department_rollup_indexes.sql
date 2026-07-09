-- Migration 023: Cover dashboard department AI ratio rollup lookups

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_total_org_user
    ON metrics_daily_rollups (org_id, user_id)
    INCLUDE (total_lines, ai_lines)
    WHERE tool_model = '';
