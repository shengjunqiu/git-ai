use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Report document matching client's ReportDocument
/// Client sends required fields; server uses Option<> to be lenient during deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportDocument {
    pub schema_version: String,
    pub generated_at: String,
    pub tool_version: String,
    #[serde(default)]
    pub repo: Option<ReportRepo>,
    #[serde(default)]
    pub range: Option<ReportRange>,
    #[serde(default)]
    pub summary: Option<ReportSummary>,
    #[serde(default)]
    pub ratios: Option<ReportRatios>,
    #[serde(default)]
    pub tool_model_breakdown: Option<HashMap<String, ToolModelBreakdown>>,
    #[serde(default)]
    pub commits: Vec<ReportCommit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRepo {
    pub workdir: Option<String>,
    pub remote_url_hash: Option<String>,
    pub branch: Option<String>,
    pub head_commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRange {
    pub mode: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub commit_count: Option<i64>,               // Client sends usize — i64 is compatible
    pub commits_with_authorship: Option<i64>,     // Client sends usize — i64 is compatible
    pub commits_without_authorship: Option<i64>,  // Client sends usize — i64 is compatible
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    pub git_diff_added_lines: Option<i64>,        // Client sends u32 — i64 is compatible
    pub git_diff_deleted_lines: Option<i64>,      // Client sends u32
    pub ai_additions: Option<i64>,                // Client sends u32
    pub human_additions: Option<i64>,             // Client sends u32
    pub mixed_additions: Option<i64>,             // Client sends u32
    pub unknown_additions: Option<i64>,           // Client sends u32
    pub ai_accepted: Option<i64>,                 // Client sends u32
    pub total_ai_additions: Option<i64>,          // Client sends u32
    pub total_ai_deletions: Option<i64>,          // Client sends u32
    pub time_waiting_for_ai: Option<i64>,         // Client sends u64 — i64 is compatible for reasonable values
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRatios {
    pub ai: Option<f64>,
    pub human: Option<f64>,
    pub mixed: Option<f64>,
    pub unknown: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolModelBreakdown {
    pub ai_additions: Option<i64>,
    pub human_additions: Option<i64>,
    pub mixed_additions: Option<i64>,
    pub total_ai_additions: Option<i64>,
    pub total_ai_deletions: Option<i64>,
    pub ai_accepted: Option<i64>,
    pub time_waiting_for_ai: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReportCommit {
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub author_time: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub has_authorship_note: bool,
    #[serde(default)]
    pub stats: ReportCommitStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReportCommitStats {
    pub git_diff_added_lines: Option<i64>,
    pub git_diff_deleted_lines: Option<i64>,
    pub ai_additions: Option<i64>,
    pub human_additions: Option<i64>,
    pub mixed_additions: Option<i64>,
    pub unknown_additions: Option<i64>,
    pub ai_accepted: Option<i64>,
    pub total_ai_additions: Option<i64>,
    pub total_ai_deletions: Option<i64>,
    pub time_waiting_for_ai: Option<i64>,
}

/// Ingest response matching client's IngestResponse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResponse {
    pub project_id: i64,
    pub upload_id: i64,
    pub inserted_commits: i64,
    pub duplicate_commits: i64,
}

/// Project summary report matching client's ProjectSummaryReport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummaryReport {
    pub project_name: String,
    pub git_url: Option<String>,
    pub branch: Option<String>,
    pub total_commits: i64,                       // Client sends usize — i64 is compatible
    pub developers: Vec<DeveloperStats>,
    pub project_ratios: ProjectRatios,
    pub organization: Option<String>,
    pub department: Option<String>,
    pub reporter_name: Option<String>,
    pub reporter_email: Option<String>,
    pub report_period: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeveloperStats {
    pub name: String,
    pub email: String,
    pub commits: i64,                             // Client sends usize — i64 is compatible
    pub added_lines: i64,                         // Client sends u32
    pub ai_additions: i64,                        // Client sends u32
    pub human_additions: i64,                     // Client sends u32
    pub ai_ratio: f64,
    pub human_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRatios {
    pub ai: f64,
    pub human: f64,
}
