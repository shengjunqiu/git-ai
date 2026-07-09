-- Migration 025: Cover dashboard rollup project aggregate lookups

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_project_total
    ON metrics_daily_rollups (org_id, repo_url, day, user_id)
    INCLUDE (commits, total_lines, ai_lines)
    WHERE tool_model = '' AND repo_url != '';
