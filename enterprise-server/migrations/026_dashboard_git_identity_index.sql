-- Migration 026: Cover developer dashboard Git identity lookups

CREATE INDEX IF NOT EXISTS idx_metrics_events_git_identity
    ON metrics_events (event_type, org_id, user_id, author_email, timestamp)
    WHERE user_id IS NOT NULL
      AND author_email IS NOT NULL
      AND author_email != '';
