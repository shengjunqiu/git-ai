-- Daily rollups for metrics dashboard aggregate queries.
--
-- org_id and user_id use the nil UUID sentinel when the source metric row has
-- NULL scope values so the primary key remains usable for idempotent upserts.

CREATE TABLE IF NOT EXISTS metrics_daily_rollups (
    day DATE NOT NULL,
    org_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
    user_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
    repo_url TEXT NOT NULL DEFAULT '',
    tool_model TEXT NOT NULL DEFAULT '',
    commits BIGINT NOT NULL DEFAULT 0,
    total_lines BIGINT NOT NULL DEFAULT 0,
    ai_lines BIGINT NOT NULL DEFAULT 0,
    human_lines BIGINT NOT NULL DEFAULT 0,
    mixed_lines BIGINT NOT NULL DEFAULT 0,
    ai_accepted BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (day, org_id, user_id, repo_url, tool_model)
);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_org_day
    ON metrics_daily_rollups(org_id, day);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_user_day
    ON metrics_daily_rollups(user_id, day);

CREATE INDEX IF NOT EXISTS idx_metrics_daily_rollups_tool_day
    ON metrics_daily_rollups(tool_model, day);

INSERT INTO metrics_daily_rollups (
    day, org_id, user_id, repo_url, tool_model,
    commits, total_lines, ai_lines, human_lines, mixed_lines, ai_accepted
)
SELECT
    (to_timestamp(timestamp) AT TIME ZONE 'UTC')::date AS day,
    COALESCE(org_id, '00000000-0000-0000-0000-000000000000'::uuid) AS org_id,
    COALESCE(user_id, '00000000-0000-0000-0000-000000000000'::uuid) AS user_id,
    COALESCE(repo_url, '') AS repo_url,
    '' AS tool_model,
    COUNT(*)::bigint AS commits,
    COALESCE(SUM(git_diff_added_lines), 0)::bigint AS total_lines,
    COALESCE(SUM(ai_additions), 0)::bigint AS ai_lines,
    COALESCE(SUM(GREATEST(COALESCE(git_diff_added_lines, 0) - COALESCE(ai_additions, 0), 0)), 0)::bigint AS human_lines,
    COALESCE(SUM(mixed_additions), 0)::bigint AS mixed_lines,
    COALESCE(SUM(ai_accepted), 0)::bigint AS ai_accepted
FROM metrics_events
WHERE event_type = 1
GROUP BY 1, 2, 3, 4
ON CONFLICT (day, org_id, user_id, repo_url, tool_model) DO UPDATE SET
    commits = EXCLUDED.commits,
    total_lines = EXCLUDED.total_lines,
    ai_lines = EXCLUDED.ai_lines,
    human_lines = EXCLUDED.human_lines,
    mixed_lines = EXCLUDED.mixed_lines,
    ai_accepted = EXCLUDED.ai_accepted,
    updated_at = now();

INSERT INTO metrics_daily_rollups (
    day, org_id, user_id, repo_url, tool_model,
    commits, total_lines, ai_lines, human_lines, mixed_lines, ai_accepted
)
SELECT
    (to_timestamp(m.timestamp) AT TIME ZONE 'UTC')::date AS day,
    COALESCE(m.org_id, '00000000-0000-0000-0000-000000000000'::uuid) AS org_id,
    COALESCE(m.user_id, '00000000-0000-0000-0000-000000000000'::uuid) AS user_id,
    COALESCE(m.repo_url, '') AS repo_url,
    pair.tool_model,
    COUNT(DISTINCT m.id)::bigint AS commits,
    0::bigint AS total_lines,
    COALESCE(SUM(CASE WHEN jsonb_typeof(ai.value) = 'number' THEN (ai.value #>> '{}')::bigint ELSE 0 END), 0)::bigint AS ai_lines,
    0::bigint AS human_lines,
    COALESCE(SUM(CASE WHEN jsonb_typeof(mixed.value) = 'number' THEN (mixed.value #>> '{}')::bigint ELSE 0 END), 0)::bigint AS mixed_lines,
    COALESCE(SUM(CASE WHEN jsonb_typeof(accepted.value) = 'number' THEN (accepted.value #>> '{}')::bigint ELSE 0 END), 0)::bigint AS ai_accepted
FROM metrics_events m
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN jsonb_typeof(m.tool_model_pairs) = 'array' THEN m.tool_model_pairs ELSE '[]'::jsonb END
) WITH ORDINALITY AS pair(tool_model, ord)
LEFT JOIN LATERAL jsonb_array_elements(
    CASE WHEN jsonb_typeof(m.raw_values->'5') = 'array' THEN m.raw_values->'5' ELSE '[]'::jsonb END
) WITH ORDINALITY AS ai(value, ord) ON ai.ord = pair.ord
LEFT JOIN LATERAL jsonb_array_elements(
    CASE WHEN jsonb_typeof(m.raw_values->'4') = 'array' THEN m.raw_values->'4' ELSE '[]'::jsonb END
) WITH ORDINALITY AS mixed(value, ord) ON mixed.ord = pair.ord
LEFT JOIN LATERAL jsonb_array_elements(
    CASE WHEN jsonb_typeof(m.raw_values->'6') = 'array' THEN m.raw_values->'6' ELSE '[]'::jsonb END
) WITH ORDINALITY AS accepted(value, ord) ON accepted.ord = pair.ord
WHERE m.event_type = 1
  AND pair.tool_model != 'all'
  AND pair.tool_model != ''
GROUP BY 1, 2, 3, 4, 5
ON CONFLICT (day, org_id, user_id, repo_url, tool_model) DO UPDATE SET
    commits = EXCLUDED.commits,
    total_lines = EXCLUDED.total_lines,
    ai_lines = EXCLUDED.ai_lines,
    human_lines = EXCLUDED.human_lines,
    mixed_lines = EXCLUDED.mixed_lines,
    ai_accepted = EXCLUDED.ai_accepted,
    updated_at = now();
