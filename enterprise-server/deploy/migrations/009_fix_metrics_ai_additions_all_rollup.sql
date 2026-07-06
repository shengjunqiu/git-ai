-- Fix metrics_events.ai_additions rows that double-counted the "all" rollup
-- together with per-tool entries from PosEncoded value 5.

WITH all_rollups AS (
    SELECT
        m.id,
        all_values.ai_value::integer AS ai_additions
    FROM metrics_events m
    CROSS JOIN LATERAL (
        SELECT ai_values.value AS ai_value
        FROM jsonb_array_elements_text(m.tool_model_pairs) WITH ORDINALITY AS pairs(value, ord)
        JOIN jsonb_array_elements_text(m.raw_values -> '5') WITH ORDINALITY AS ai_values(value, ord)
          ON ai_values.ord = pairs.ord
        WHERE pairs.value = 'all'
          AND ai_values.value ~ '^-?[0-9]+$'
        LIMIT 1
    ) all_values
    WHERE m.event_type = 1
      AND jsonb_typeof(m.tool_model_pairs) = 'array'
      AND jsonb_typeof(m.raw_values -> '5') = 'array'
)
UPDATE metrics_events m
SET ai_additions = all_rollups.ai_additions
FROM all_rollups
WHERE m.id = all_rollups.id
  AND m.ai_additions IS DISTINCT FROM all_rollups.ai_additions;
