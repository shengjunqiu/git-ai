-- Migration 024: Composite indexes for cursor-paginated dashboard aggregates

CREATE INDEX IF NOT EXISTS idx_organizations_name_slug
    ON organizations (name ASC, slug ASC);

CREATE INDEX IF NOT EXISTS idx_metrics_events_project_aggregate
    ON metrics_events (event_type, org_id, timestamp, repo_url)
    WHERE repo_url IS NOT NULL AND repo_url != '';

CREATE INDEX IF NOT EXISTS idx_metrics_events_developer_aggregate
    ON metrics_events (event_type, org_id, user_id, timestamp)
    WHERE user_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_events_org_model
    ON metrics_tool_model_events (org_id, tool_model);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_org_tool_model
    ON metrics_daily_rollups (org_id, tool_model)
    INCLUDE (ai_lines, mixed_lines, ai_accepted, commits);

CREATE INDEX IF NOT EXISTS idx_projects_org_remote_hash
    ON projects (org_id, remote_url_hash);

CREATE INDEX IF NOT EXISTS idx_tool_model_stats_project_model
    ON tool_model_stats (project_id, tool_model);
