-- Phase 6: Enterprise enhancement tables
-- PR-level aggregation, CI/CD events, alerts, data retention, CAS access audit

-- =====================================================
-- PR-level aggregation: track pull requests and their AI attribution
-- =====================================================
CREATE TABLE pull_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    repo_url TEXT NOT NULL,
    pr_id TEXT NOT NULL,                     -- External PR number/ID (e.g., "123")
    pr_url TEXT,                             -- Full URL to the PR
    title TEXT,
    author_email TEXT,
    merged_at TIMESTAMPTZ,
    total_lines INTEGER NOT NULL DEFAULT 0,
    ai_lines INTEGER NOT NULL DEFAULT 0,
    human_lines INTEGER NOT NULL DEFAULT 0,
    pct_ai REAL NOT NULL DEFAULT 0.0,
    tools_used TEXT[],                       -- Array of tool::model strings
    files_changed INTEGER NOT NULL DEFAULT 0,
    ai_files INTEGER NOT NULL DEFAULT 0,
    commit_shas TEXT[],                      -- Associated commit SHAs
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(repo_url, pr_id)
);

CREATE INDEX idx_pull_requests_org_id ON pull_requests(org_id);
CREATE INDEX idx_pull_requests_merged_at ON pull_requests(merged_at);
CREATE INDEX idx_pull_requests_repo_url ON pull_requests(repo_url);

-- =====================================================
-- CI/CD lifecycle events
-- =====================================================
CREATE TABLE ci_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,                -- "ci_run", "deployment", "pr_review"
    timestamp TIMESTAMPTZ NOT NULL DEFAULT now(),
    repo_url TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    deployment_env TEXT,                     -- "production", "staging", "development"
    status TEXT,                             -- "success", "failure", "running"
    deployer TEXT,                           -- Who/what triggered the deployment
    ci_platform TEXT,                        -- "github_actions", "gitlab_ci", "jenkins", etc.
    metadata JSONB,                          -- Additional event-specific data
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_ci_events_org_id ON ci_events(org_id);
CREATE INDEX idx_ci_events_commit_sha ON ci_events(commit_sha);
CREATE INDEX idx_ci_events_event_type ON ci_events(event_type);
CREATE INDEX idx_ci_events_timestamp ON ci_events(timestamp);

-- =====================================================
-- Alert events (production incidents correlated with AI code)
-- =====================================================
CREATE TABLE alert_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    alert_source TEXT NOT NULL,              -- "pagerduty", "datadog", "grafana", "custom"
    event_type TEXT NOT NULL DEFAULT 'alert',
    timestamp TIMESTAMPTZ NOT NULL DEFAULT now(),
    repo_url TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'info' CHECK (severity IN ('info', 'warning', 'critical')),
    description TEXT,
    resolved_at TIMESTAMPTZ,
    metadata JSONB,                          -- Additional alert-specific data
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_alert_events_org_id ON alert_events(org_id);
CREATE INDEX idx_alert_events_commit_sha ON alert_events(commit_sha);
CREATE INDEX idx_alert_events_severity ON alert_events(severity);
CREATE INDEX idx_alert_events_timestamp ON alert_events(timestamp);

-- =====================================================
-- Data retention policies per organization
-- =====================================================
CREATE TABLE data_retention_policies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL UNIQUE REFERENCES organizations(id) ON DELETE CASCADE,
    metrics_retention_days INTEGER NOT NULL DEFAULT 365,     -- 1 year default
    cas_retention_days INTEGER NOT NULL DEFAULT 365,         -- 1 year default
    audit_retention_days INTEGER NOT NULL DEFAULT 730,       -- 2 years default
    ci_events_retention_days INTEGER NOT NULL DEFAULT 365,
    alerts_retention_days INTEGER NOT NULL DEFAULT 365,
    auto_purge BOOLEAN NOT NULL DEFAULT false,               -- Auto-delete expired data
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================
-- CAS access audit log (tracks who reads prompts)
-- =====================================================
CREATE TABLE cas_access_log (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    api_key_id UUID REFERENCES api_keys(id) ON DELETE SET NULL,
    cas_hash TEXT NOT NULL,
    access_method TEXT NOT NULL DEFAULT 'api' CHECK (access_method IN ('api', 'dashboard', 'ide_plugin')),
    purpose TEXT,                            -- Optional reason for access
    ip_address TEXT,
    user_agent TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_cas_access_log_user_id ON cas_access_log(user_id);
CREATE INDEX idx_cas_access_log_cas_hash ON cas_access_log(cas_hash);
CREATE INDEX idx_cas_access_log_created_at ON cas_access_log(created_at);
CREATE INDEX idx_cas_access_log_org_id ON cas_access_log(org_id);

-- =====================================================
-- AI code persistence snapshots (track survival of AI-generated code over time)
-- =====================================================
CREATE TABLE ai_code_persistence_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    repo_url TEXT NOT NULL,
    snapshot_date DATE NOT NULL,
    total_ai_lines_introduced INTEGER NOT NULL DEFAULT 0,
    lines_still_present INTEGER NOT NULL DEFAULT 0,
    lines_modified INTEGER NOT NULL DEFAULT 0,
    lines_deleted INTEGER NOT NULL DEFAULT 0,
    survival_rate REAL NOT NULL DEFAULT 0.0,
    by_tool JSONB,                           -- Per-tool breakdown {"tool::model": {introduced, survival_rate}}
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(repo_url, snapshot_date)
);

CREATE INDEX idx_persistence_snapshots_org_id ON ai_code_persistence_snapshots(org_id);
CREATE INDEX idx_persistence_snapshots_date ON ai_code_persistence_snapshots(snapshot_date);

-- =====================================================
-- Agent readiness scores (periodic evaluation)
-- =====================================================
CREATE TABLE agent_readiness_scores (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    tool TEXT NOT NULL,                      -- e.g., "claude-code"
    model TEXT NOT NULL,                     -- e.g., "claude-3.5-sonnet"
    overall_score INTEGER NOT NULL DEFAULT 0 CHECK (overall_score >= 0 AND overall_score <= 100),
    trend TEXT NOT NULL DEFAULT 'stable' CHECK (trend IN ('improving', 'stable', 'declining')),
    config_changes JSONB,                    -- Array of {type, changed_at, before, after}
    eval_period_start DATE NOT NULL,
    eval_period_end DATE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_agent_readiness_org_id ON agent_readiness_scores(org_id);
CREATE INDEX idx_agent_readiness_tool ON agent_readiness_scores(tool);
