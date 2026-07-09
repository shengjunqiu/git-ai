-- Migration 020: Composite indexes for cursor-paginated log queries

CREATE INDEX IF NOT EXISTS idx_audit_log_created_id_desc
    ON audit_log (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_audit_log_user_created_id_desc
    ON audit_log (user_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_audit_log_org_created_id_desc
    ON audit_log (org_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_audit_log_action_created_id_desc
    ON audit_log (action, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_cas_access_log_created_id_desc
    ON cas_access_log (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_cas_access_log_hash_created_id_desc
    ON cas_access_log (cas_hash, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_cas_access_log_user_created_id_desc
    ON cas_access_log (user_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_cas_access_log_org_created_id_desc
    ON cas_access_log (org_id, created_at DESC, id DESC);
