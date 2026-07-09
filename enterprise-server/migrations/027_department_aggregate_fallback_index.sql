-- Migration 027: Cover department aggregate report fallback totals

CREATE INDEX IF NOT EXISTS idx_commit_stats_project_totals
    ON commit_stats (project_id)
    INCLUDE (sha, git_diff_added_lines, ai_additions);
