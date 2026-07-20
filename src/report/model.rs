use crate::authorship::stats::{CommitStats, ToolModelHeadlineStats};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// These are report-generation domain models. HTTP upload converts them into
// git_ai_protocol::report wire types in report::upload.

pub const REPORT_SCHEMA_VERSION: &str = "git-ai-report/1.0.0";
pub const DEVELOPER_SUMMARY_SCHEMA_VERSION: &str = "git-ai-summary/1.0.0";
pub type ReportToolModelStats = ToolModelHeadlineStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Json,
    Csv,
}

impl ReportFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "csv" => Some(Self::Csv),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportRangeMode {
    Head,
    Range,
    Branch,
    Date,
}

#[derive(Debug, Clone)]
pub struct ReportOptions {
    pub repo_path: Option<String>,
    pub range: Option<String>,
    pub branch: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub ignore_patterns: Vec<String>,
}

impl ReportOptions {
    pub fn new(repo_path: Option<String>) -> Self {
        Self {
            repo_path,
            range: None,
            branch: None,
            since: None,
            until: None,
            ignore_patterns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportDocument {
    pub schema_version: String,
    pub generated_at: String,
    pub tool_version: String,
    pub repo: ReportRepoInfo,
    pub range: ReportRangeInfo,
    pub summary: ReportSummary,
    pub ratios: ReportRatios,
    pub tool_model_breakdown: BTreeMap<String, ReportToolModelStats>,
    pub commits: Vec<ReportCommit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRepoInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Hash of the normalized remote URL, or of `local/<directory-name>` when no remote exists.
    pub remote_url_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRangeInfo {
    pub mode: ReportRangeMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    pub commit_count: usize,
    pub commits_with_authorship: usize,
    pub commits_without_authorship: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportSummary {
    pub git_diff_added_lines: u32,
    pub git_diff_deleted_lines: u32,
    pub ai_additions: u32,
    pub human_additions: u32,
    pub mixed_additions: u32,
    pub unknown_additions: u32,
    pub ai_accepted: u32,
    pub total_ai_additions: u32,
    pub total_ai_deletions: u32,
    pub time_waiting_for_ai: u64,
}

impl ReportSummary {
    pub fn add_commit_stats(&mut self, stats: &CommitStats) {
        self.git_diff_added_lines += stats.git_diff_added_lines;
        self.git_diff_deleted_lines += stats.git_diff_deleted_lines;
        self.ai_additions += stats.ai_additions;
        self.human_additions += stats.human_additions;
        self.mixed_additions += stats.mixed_additions;
        self.unknown_additions += stats.unknown_additions;
        self.ai_accepted += stats.ai_accepted;
        self.total_ai_additions += stats.total_ai_additions;
        self.total_ai_deletions += stats.total_ai_deletions;
        self.time_waiting_for_ai += stats.time_waiting_for_ai;
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ReportRatios {
    pub ai: f64,
    pub human: f64,
    pub mixed: f64,
    pub unknown: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportCommit {
    pub sha: String,
    pub author: String,
    pub author_time: String,
    pub subject: String,
    pub has_authorship_note: bool,
    pub stats: CommitStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResult {
    pub uploaded: bool,
    pub message: String,
    pub commit_count: usize,
}

// ---------------------------------------------------------------------------
// Simplified project summary report (git-ai report summary)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummaryReport {
    pub project_name: String,
    pub git_url: Option<String>,
    pub branch: Option<String>,
    pub total_commits: usize,
    pub developers: Vec<DeveloperSummary>,
    pub project_ratios: ProjectRatios,
    /// 上报元数据（用户上传时填写，全部可选，向后兼容旧版上传）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reporter_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reporter_email: Option<String>,
    /// 统计周期标注，如 "2026-Q2"，纯文本，便于人工区分多次上报
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_period: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeveloperSummary {
    pub name: String,
    pub email: String,
    pub commits: usize,
    pub added_lines: u32,
    pub ai_additions: u32,
    pub human_additions: u32,
    pub ai_ratio: f64,
    pub human_ratio: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectRatios {
    pub ai: f64,
    pub human: f64,
}

pub fn calculate_ratios(summary: &ReportSummary) -> ReportRatios {
    let total = summary.ai_additions + summary.human_additions + summary.unknown_additions;
    if total == 0 {
        return ReportRatios::default();
    }

    let total = total as f64;
    ReportRatios {
        ai: summary.ai_additions as f64 / total,
        human: summary.human_additions as f64 / total,
        mixed: summary.mixed_additions as f64 / total,
        unknown: summary.unknown_additions as f64 / total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratios_handle_empty_summary() {
        assert_eq!(
            calculate_ratios(&ReportSummary::default()),
            ReportRatios::default()
        );
    }

    #[test]
    fn ratios_use_added_line_categories() {
        let summary = ReportSummary {
            ai_additions: 40,
            human_additions: 30,
            mixed_additions: 10,
            unknown_additions: 30,
            ..Default::default()
        };

        let ratios = calculate_ratios(&summary);
        assert_eq!(ratios.ai, 0.4);
        assert_eq!(ratios.human, 0.3);
        assert_eq!(ratios.mixed, 0.1);
        assert_eq!(ratios.unknown, 0.3);
    }

    #[test]
    fn report_serializes_schema_version() {
        let report = ReportDocument {
            schema_version: REPORT_SCHEMA_VERSION.to_string(),
            generated_at: "2026-04-21T00:00:00Z".to_string(),
            tool_version: "test".to_string(),
            repo: ReportRepoInfo {
                workdir: None,
                remote_url_hash: None,
                branch: None,
                head_commit: None,
            },
            range: ReportRangeInfo {
                mode: ReportRangeMode::Head,
                from: None,
                to: None,
                since: None,
                until: None,
                commit_count: 0,
                commits_with_authorship: 0,
                commits_without_authorship: 0,
            },
            summary: ReportSummary::default(),
            ratios: ReportRatios::default(),
            tool_model_breakdown: BTreeMap::new(),
            commits: vec![],
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(REPORT_SCHEMA_VERSION));
    }
}
