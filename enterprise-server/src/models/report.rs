use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
pub use git_ai_protocol::report::{
    DeveloperStats, ProjectRatios, ProjectSummaryReport, ReportCommit, ReportCommitStats,
    ReportDocument, ReportRange, ReportRatios, ReportRepo, ReportSummary, ToolModelBreakdown,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResponse {
    pub project_id: i64,
    pub upload_id: i64,
    pub inserted_commits: i64,
    pub duplicate_commits: i64,
}
