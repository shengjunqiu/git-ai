-- Migration 022: Composite indexes for cursor-paginated pull request aggregation

CREATE INDEX IF NOT EXISTS idx_pull_requests_merged_id_desc
    ON pull_requests (merged_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_pull_requests_org_merged_id_desc
    ON pull_requests (org_id, merged_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_pull_requests_repo_merged_id_desc
    ON pull_requests (repo_url, merged_at DESC, id DESC);
