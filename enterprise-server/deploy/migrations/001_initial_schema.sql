-- git-ai Enterprise Server: Initial Schema
-- Phase 1: Users, Organizations, Auth

-- Users
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    personal_org_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Organizations
CREATE TABLE organizations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Departments
CREATE TABLE departments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Organization membership
CREATE TABLE org_members (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    department_id UUID REFERENCES departments(id) ON DELETE SET NULL,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
    PRIMARY KEY (user_id, org_id)
);

-- API Keys (hash stored, not plaintext)
CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    key_prefix TEXT NOT NULL,           -- First 8 chars for identification
    key_hash TEXT UNIQUE NOT NULL,       -- SHA256 hash of the key
    name TEXT,                           -- Key description
    scopes TEXT[] DEFAULT ARRAY['metrics:write', 'cas:write', 'cas:read', 'reports:write'],
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_api_keys_hash ON api_keys(key_hash);
CREATE INDEX idx_api_keys_prefix ON api_keys(key_prefix);

-- OAuth device codes
CREATE TABLE oauth_devices (
    device_code TEXT PRIMARY KEY,
    user_code TEXT UNIQUE NOT NULL,
    verification_uri TEXT NOT NULL,
    client_id TEXT NOT NULL DEFAULT 'git-ai-cli',
    expires_at TIMESTAMPTZ NOT NULL,
    interval_seconds INTEGER NOT NULL DEFAULT 5,
    user_id UUID REFERENCES users(id),
    authorized_at TIMESTAMPTZ
);

CREATE INDEX idx_oauth_devices_user_code ON oauth_devices(user_code);

-- Refresh tokens
CREATE TABLE refresh_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT UNIQUE NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_refresh_tokens_hash ON refresh_tokens(token_hash);

-- Install nonces (one-time login)
CREATE TABLE install_nonces (
    nonce TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    used BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    used_at TIMESTAMPTZ
);

-- =====================================================
-- Phase 1: Metrics
-- =====================================================

CREATE TABLE metrics_events (
    id BIGSERIAL PRIMARY KEY,
    event_type SMALLINT NOT NULL,         -- 1=Committed, 2=AgentUsage, 3=InstallHooks, 4=Checkpoint (matches client MetricEventId)
    timestamp BIGINT NOT NULL,            -- Unix timestamp (seconds)
    user_id UUID REFERENCES users(id),
    distinct_id TEXT,                      -- X-Distinct-ID header
    org_id UUID REFERENCES organizations(id),
    repo_url TEXT,
    author_email TEXT,
    tool TEXT,
    model TEXT,
    commit_sha TEXT,
    human_additions INTEGER DEFAULT 0,
    ai_additions INTEGER DEFAULT 0,
    mixed_additions INTEGER DEFAULT 0,
    unknown_additions INTEGER DEFAULT 0,
    ai_accepted INTEGER DEFAULT 0,
    git_diff_added_lines INTEGER DEFAULT 0,
    git_diff_deleted_lines INTEGER DEFAULT 0,
    tool_model_pairs JSONB,               -- tool::model pairs array
    ai_additions_by_tool JSONB,           -- per-tool AI lines
    prompt_id TEXT,                        -- AgentUsage/Checkpoint event prompt ID
    session_id TEXT,                       -- AI session ID
    file_path TEXT,                        -- Checkpoint event file path
    custom_attributes JSONB,              -- custom attributes
    raw_values JSONB,                      -- original PosEncoded values
    raw_attrs JSONB,                       -- original PosEncoded attributes
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_metrics_user_id ON metrics_events(user_id);
CREATE INDEX idx_metrics_org_id ON metrics_events(org_id);
CREATE INDEX idx_metrics_repo_url ON metrics_events(repo_url);
CREATE INDEX idx_metrics_author ON metrics_events(author_email);
CREATE INDEX idx_metrics_tool ON metrics_events(tool);
CREATE INDEX idx_metrics_timestamp ON metrics_events(timestamp);
CREATE INDEX idx_metrics_commit_sha ON metrics_events(commit_sha);
CREATE INDEX idx_metrics_event_type ON metrics_events(event_type);
CREATE INDEX idx_metrics_distinct_id ON metrics_events(distinct_id);

-- =====================================================
-- Phase 2: CAS
-- =====================================================

CREATE TABLE cas_objects (
    hash TEXT PRIMARY KEY,                -- SHA256 hash
    content JSONB NOT NULL,               -- PromptRecord JSON
    metadata JSONB,                       -- optional metadata key-value pairs
    author_identity TEXT,                 -- X-Author-Identity
    user_id UUID REFERENCES users(id),
    org_id UUID REFERENCES organizations(id),
    size_bytes INTEGER,                   -- content size in bytes
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE cas_ownership (
    hash TEXT NOT NULL REFERENCES cas_objects(hash) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (hash, user_id)
);

CREATE INDEX idx_cas_objects_user_id ON cas_objects(user_id);
CREATE INDEX idx_cas_objects_org_id ON cas_objects(org_id);

-- =====================================================
-- Phase 2: Reports & Summaries
-- =====================================================

CREATE TABLE projects (
    id BIGSERIAL PRIMARY KEY,
    remote_url_hash TEXT UNIQUE NOT NULL,
    branch TEXT,
    head_commit TEXT,
    organization TEXT,
    department TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE report_uploads (
    id BIGSERIAL PRIMARY KEY,             -- upload_id
    project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    schema_version TEXT NOT NULL,
    generated_at TEXT NOT NULL,
    commit_count INTEGER NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE commit_stats (
    project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    sha TEXT NOT NULL,
    author TEXT NOT NULL,
    author_time TEXT NOT NULL,
    subject TEXT NOT NULL,
    has_authorship_note BOOLEAN NOT NULL,
    git_diff_added_lines INTEGER NOT NULL,
    git_diff_deleted_lines INTEGER NOT NULL,
    ai_additions INTEGER NOT NULL,
    human_additions INTEGER NOT NULL,
    mixed_additions INTEGER NOT NULL,
    unknown_additions INTEGER NOT NULL,
    ai_accepted INTEGER NOT NULL,
    total_ai_additions INTEGER NOT NULL,
    total_ai_deletions INTEGER NOT NULL,
    time_waiting_for_ai INTEGER NOT NULL,
    PRIMARY KEY (project_id, sha)
);

CREATE TABLE tool_model_stats (
    project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    tool_model TEXT NOT NULL,
    ai_additions INTEGER NOT NULL,
    mixed_additions INTEGER NOT NULL,
    ai_accepted INTEGER NOT NULL,
    total_ai_additions INTEGER NOT NULL,
    total_ai_deletions INTEGER NOT NULL,
    time_waiting_for_ai INTEGER NOT NULL,
    PRIMARY KEY (project_id, tool_model)
);

CREATE TABLE summary_uploads (
    id BIGSERIAL PRIMARY KEY,
    project_name TEXT NOT NULL,
    git_url TEXT,
    branch TEXT,
    total_commits INTEGER NOT NULL,
    organization TEXT,
    department TEXT,
    reporter_name TEXT,
    reporter_email TEXT,
    report_period TEXT,
    project_ratios JSONB NOT NULL,
    developers JSONB NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_projects_org ON projects(organization);
CREATE INDEX idx_projects_dept ON projects(department);
CREATE INDEX idx_commit_stats_author ON commit_stats(author);
CREATE INDEX idx_summary_uploads_org ON summary_uploads(organization);

-- =====================================================
-- Phase 2: Bundles
-- =====================================================

CREATE TABLE bundles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    data JSONB NOT NULL,                  -- BundleData: { prompts, files }
    share_url TEXT UNIQUE NOT NULL,
    view_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ
);

CREATE TABLE bundle_prompts (
    bundle_id UUID NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    hash TEXT NOT NULL,                    -- Prompt hash (key in prompts map)
    content JSONB NOT NULL,               -- PromptRecord JSON
    PRIMARY KEY (bundle_id, hash)
);

CREATE TABLE bundle_files (
    bundle_id UUID NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,              -- File path (key in files map)
    annotations JSONB,                    -- prompt_hash -> line number mapping
    diff TEXT,                            -- Git diff
    base_content TEXT,                    -- Original file content
    PRIMARY KEY (bundle_id, file_path)
);

CREATE INDEX idx_bundles_user_id ON bundles(user_id);
CREATE INDEX idx_bundles_share_url ON bundles(share_url);

-- =====================================================
-- Phase 3: Releases
-- =====================================================

CREATE TABLE release_channels (
    channel TEXT PRIMARY KEY,             -- latest, next, enterprise-latest, enterprise-next
    version TEXT NOT NULL,
    checksum TEXT NOT NULL,               -- SHA256 of SHA256SUMS file
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE release_assets (
    id BIGSERIAL PRIMARY KEY,
    channel TEXT NOT NULL REFERENCES release_channels(channel) ON DELETE CASCADE,
    filename TEXT NOT NULL,               -- e.g., git-ai-x86_64-unknown-linux-gnu
    sha256 TEXT NOT NULL,                 -- SHA256 of the file
    size_bytes BIGINT,
    storage_path TEXT,                    -- Path in object storage
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(channel, filename)
);

-- =====================================================
-- Phase 5: Audit Log
-- =====================================================

CREATE TABLE audit_log (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID REFERENCES users(id),
    org_id UUID REFERENCES organizations(id),
    action TEXT NOT NULL,                 -- e.g., "api_key.create", "user.login", "cas.read"
    resource_type TEXT,                   -- e.g., "api_key", "user", "cas_object"
    resource_id TEXT,
    details JSONB,                        -- Additional context
    ip_address TEXT,
    user_agent TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_audit_log_user_id ON audit_log(user_id);
CREATE INDEX idx_audit_log_org_id ON audit_log(org_id);
CREATE INDEX idx_audit_log_action ON audit_log(action);
CREATE INDEX idx_audit_log_created_at ON audit_log(created_at);
