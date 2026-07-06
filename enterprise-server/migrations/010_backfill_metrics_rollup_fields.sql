-- Backfill metrics_events rollup fields that were decoded but not stored by
-- the metrics ingestion path.

WITH decoded AS (
    SELECT
        m.id,
        COALESCE(rollups.mixed_additions, 0) AS mixed_additions,
        COALESCE(rollups.ai_accepted, 0) AS ai_accepted,
        NULLIF(COALESCE(tool_values.ai_additions_by_tool, '{}'::jsonb), '{}'::jsonb) AS ai_additions_by_tool
    FROM metrics_events m
    LEFT JOIN LATERAL (
        SELECT
            MAX(CASE WHEN pairs.value = 'all' THEN mixed_values.value::integer END) AS mixed_additions,
            MAX(CASE WHEN pairs.value = 'all' THEN accepted_values.value::integer END) AS ai_accepted
        FROM jsonb_array_elements_text(
            CASE WHEN jsonb_typeof(m.tool_model_pairs) = 'array' THEN m.tool_model_pairs ELSE '[]'::jsonb END
        ) WITH ORDINALITY AS pairs(value, ord)
        LEFT JOIN jsonb_array_elements_text(
            CASE WHEN jsonb_typeof(m.raw_values -> '4') = 'array' THEN m.raw_values -> '4' ELSE '[]'::jsonb END
        ) WITH ORDINALITY AS mixed_values(value, ord)
          ON mixed_values.ord = pairs.ord AND mixed_values.value ~ '^-?[0-9]+$'
        LEFT JOIN jsonb_array_elements_text(
            CASE WHEN jsonb_typeof(m.raw_values -> '6') = 'array' THEN m.raw_values -> '6' ELSE '[]'::jsonb END
        ) WITH ORDINALITY AS accepted_values(value, ord)
          ON accepted_values.ord = pairs.ord AND accepted_values.value ~ '^-?[0-9]+$'
    ) rollups ON TRUE
    LEFT JOIN LATERAL (
        SELECT jsonb_object_agg(pairs.value, ai_values.value::integer)
            FILTER (WHERE pairs.value <> 'all' AND ai_values.value ~ '^-?[0-9]+$') AS ai_additions_by_tool
        FROM jsonb_array_elements_text(
            CASE WHEN jsonb_typeof(m.tool_model_pairs) = 'array' THEN m.tool_model_pairs ELSE '[]'::jsonb END
        ) WITH ORDINALITY AS pairs(value, ord)
        LEFT JOIN jsonb_array_elements_text(
            CASE WHEN jsonb_typeof(m.raw_values -> '5') = 'array' THEN m.raw_values -> '5' ELSE '[]'::jsonb END
        ) WITH ORDINALITY AS ai_values(value, ord)
          ON ai_values.ord = pairs.ord AND ai_values.value ~ '^-?[0-9]+$'
    ) tool_values ON TRUE
    WHERE m.event_type = 1
)
UPDATE metrics_events m
SET
    mixed_additions = decoded.mixed_additions,
    ai_accepted = decoded.ai_accepted,
    unknown_additions = GREATEST(
        COALESCE(m.git_diff_added_lines, 0)
        - COALESCE(m.ai_additions, 0)
        - COALESCE(m.human_additions, 0),
        0
    ),
    ai_additions_by_tool = decoded.ai_additions_by_tool
FROM decoded
WHERE m.id = decoded.id;
