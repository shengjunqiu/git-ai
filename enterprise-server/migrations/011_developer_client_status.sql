CREATE TABLE IF NOT EXISTS developer_client_status (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    distinct_id TEXT,
    status TEXT NOT NULL DEFAULT 'logged_out' CHECK (status IN ('logged_in', 'logged_out')),
    last_status_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ,
    cli_version TEXT,
    os TEXT,
    arch TEXT,
    hostname TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_developer_client_status_org
    ON developer_client_status(org_id);

CREATE INDEX IF NOT EXISTS idx_developer_client_status_last_seen
    ON developer_client_status(last_seen_at);
