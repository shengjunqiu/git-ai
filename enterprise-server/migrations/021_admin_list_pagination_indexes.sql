-- Migration 021: Composite indexes for cursor-paginated admin list APIs

CREATE INDEX IF NOT EXISTS idx_users_created_id_desc
    ON users (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_api_keys_active_created_id_desc
    ON api_keys (created_at DESC, id DESC)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_api_keys_user_active_created_id_desc
    ON api_keys (user_id, created_at DESC, id DESC)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_organizations_name_id
    ON organizations (name ASC, id ASC);

CREATE INDEX IF NOT EXISTS idx_departments_org_name_id
    ON departments (org_id, name ASC, id ASC);

CREATE INDEX IF NOT EXISTS idx_departments_name_id
    ON departments (name ASC, id ASC);

CREATE INDEX IF NOT EXISTS idx_org_members_org_department_user
    ON org_members (org_id, department_id, user_id);
