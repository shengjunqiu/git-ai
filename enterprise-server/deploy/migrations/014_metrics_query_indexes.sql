-- Dashboard and aggregate query indexes for high-volume metrics/report tables.

CREATE INDEX IF NOT EXISTS idx_metrics_event_org_time
    ON metrics_events(event_type, org_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_event_user_time
    ON metrics_events(event_type, user_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_event_commit
    ON metrics_events(event_type, commit_sha);

CREATE INDEX IF NOT EXISTS idx_commit_stats_project_author_time
    ON commit_stats(project_id, author_time);

CREATE INDEX IF NOT EXISTS idx_projects_org_user
    ON projects(org_id, user_id);
