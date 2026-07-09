-- Dirty scopes for asynchronous metrics daily rollup rebuilds.
--
-- org_id and user_id use the nil UUID sentinel to match metrics_daily_rollups
-- rows built from metrics_events with NULL scope values.

CREATE TABLE IF NOT EXISTS metrics_rollup_dirty_scopes (
    id BIGSERIAL PRIMARY KEY,
    day DATE NOT NULL,
    org_id UUID NOT NULL,
    user_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_metrics_rollup_dirty_claim
    ON metrics_rollup_dirty_scopes (claimed_at NULLS FIRST, id);

CREATE INDEX IF NOT EXISTS idx_metrics_rollup_dirty_scope
    ON metrics_rollup_dirty_scopes (day, org_id, user_id);
