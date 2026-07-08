-- Store parsed commit author timestamps for indexed dashboard/report filtering.

ALTER TABLE commit_stats
ADD COLUMN IF NOT EXISTS author_time_at TIMESTAMPTZ;

UPDATE commit_stats
SET author_time_at = BTRIM(author_time)::timestamptz
WHERE author_time_at IS NULL
  AND author_time IS NOT NULL
  AND BTRIM(author_time) != ''
  AND BTRIM(author_time) ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}[T ][0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?(Z|[+-][0-9]{2}:?[0-9]{2})$';

CREATE INDEX IF NOT EXISTS idx_commit_stats_project_author_time_at
    ON commit_stats(project_id, author_time_at);
