-- Repository access list (whitelist / blacklist)
-- Allows organizations to control which repositories can upload data

CREATE TABLE repo_access_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    rule_type TEXT NOT NULL CHECK (rule_type IN ('whitelist', 'blacklist')),
    pattern TEXT NOT NULL,               -- Glob pattern or exact URL match (e.g., "github.com/myorg/*")
    description TEXT,                     -- Human-readable description
    created_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    enabled BOOLEAN NOT NULL DEFAULT true
);

CREATE INDEX idx_repo_access_rules_org_id ON repo_access_rules(org_id);
CREATE INDEX idx_repo_access_rules_type ON repo_access_rules(rule_type);

-- Feature flags for remote configuration
CREATE TABLE feature_flags (
    key TEXT PRIMARY KEY,                 -- e.g., "rewrite_stash", "inter_commit_move"
    value JSONB NOT NULL DEFAULT 'true', -- true/false or object with debug/release defaults
    description TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Data export job tracking
CREATE TABLE export_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    export_type TEXT NOT NULL,            -- "csv" or "json"
    query_type TEXT NOT NULL,             -- "summary", "developers", "projects", "organizations", "tools"
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'processing', 'completed', 'failed')),
    file_path TEXT,                       -- Path to the generated export file
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_export_jobs_user_id ON export_jobs(user_id);
CREATE INDEX idx_export_jobs_status ON export_jobs(status);
