-- Structured per-tool metrics rows for dashboard queries.

CREATE TABLE IF NOT EXISTS metrics_tool_model_events (
    metric_event_id BIGINT NOT NULL REFERENCES metrics_events(id) ON DELETE CASCADE,
    org_id UUID,
    user_id UUID,
    timestamp BIGINT NOT NULL,
    tool_model TEXT NOT NULL,
    ai_additions BIGINT NOT NULL DEFAULT 0,
    mixed_additions BIGINT NOT NULL DEFAULT 0,
    ai_accepted BIGINT NOT NULL DEFAULT 0,
    total_ai_additions BIGINT NOT NULL DEFAULT 0,
    total_ai_deletions BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (metric_event_id, tool_model)
);

CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_org_time
    ON metrics_tool_model_events(org_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_user_time
    ON metrics_tool_model_events(user_id, timestamp);

CREATE INDEX IF NOT EXISTS idx_metrics_tool_model_tool_time
    ON metrics_tool_model_events(tool_model, timestamp);

INSERT INTO metrics_tool_model_events (
    metric_event_id, org_id, user_id, timestamp, tool_model,
    ai_additions, mixed_additions, ai_accepted, total_ai_additions, total_ai_deletions
)
SELECT
    m.id,
    m.org_id,
    m.user_id,
    m.timestamp,
    pair.tool_model,
    COALESCE(CASE WHEN jsonb_typeof(ai.value) = 'number' THEN (ai.value #>> '{}')::bigint ELSE 0 END, 0),
    COALESCE(CASE WHEN jsonb_typeof(mixed.value) = 'number' THEN (mixed.value #>> '{}')::bigint ELSE 0 END, 0),
    COALESCE(CASE WHEN jsonb_typeof(accepted.value) = 'number' THEN (accepted.value #>> '{}')::bigint ELSE 0 END, 0),
    COALESCE(CASE WHEN jsonb_typeof(total_add.value) = 'number' THEN (total_add.value #>> '{}')::bigint ELSE 0 END, 0),
    COALESCE(CASE WHEN jsonb_typeof(total_del.value) = 'number' THEN (total_del.value #>> '{}')::bigint ELSE 0 END, 0)
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
LEFT JOIN LATERAL jsonb_array_elements(
    CASE WHEN jsonb_typeof(m.raw_values->'7') = 'array' THEN m.raw_values->'7' ELSE '[]'::jsonb END
) WITH ORDINALITY AS total_add(value, ord) ON total_add.ord = pair.ord
LEFT JOIN LATERAL jsonb_array_elements(
    CASE WHEN jsonb_typeof(m.raw_values->'8') = 'array' THEN m.raw_values->'8' ELSE '[]'::jsonb END
) WITH ORDINALITY AS total_del(value, ord) ON total_del.ord = pair.ord
WHERE m.event_type = 1
  AND pair.tool_model != 'all'
  AND pair.tool_model != ''
ON CONFLICT (metric_event_id, tool_model) DO UPDATE SET
    org_id = EXCLUDED.org_id,
    user_id = EXCLUDED.user_id,
    timestamp = EXCLUDED.timestamp,
    ai_additions = EXCLUDED.ai_additions,
    mixed_additions = EXCLUDED.mixed_additions,
    ai_accepted = EXCLUDED.ai_accepted,
    total_ai_additions = EXCLUDED.total_ai_additions,
    total_ai_deletions = EXCLUDED.total_ai_deletions;
