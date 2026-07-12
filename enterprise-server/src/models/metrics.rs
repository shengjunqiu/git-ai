use std::collections::HashMap;

pub use git_ai_protocol::metrics::{
    MetricEvent, MetricUploadError, MetricsBatch, MetricsUploadResponse,
};

/// Metric event types matching client MetricEventId enum
/// Client uses: Committed=1, AgentUsage=2, InstallHooks=3, Checkpoint=4
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricEventType {
    Committed = 1,
    AgentUsage = 2,
    InstallHooks = 3,
    Checkpoint = 4,
}

impl TryFrom<i32> for MetricEventType {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Committed),
            2 => Ok(Self::AgentUsage),
            3 => Ok(Self::InstallHooks),
            4 => Ok(Self::Checkpoint),
            _ => Err(format!("Unknown metric event type: {}", value)),
        }
    }
}

/// Decoded metric event (after PosEncoded decoding)
#[derive(Debug, Clone)]
pub struct DecodedMetricEvent {
    pub event_type: MetricEventType,
    pub timestamp: i64,
    pub distinct_id: Option<String>,
    pub version: Option<String>,
    pub repo_url: Option<String>,
    pub author: Option<String>,
    pub tool: Option<String>,
    pub commit_sha: Option<String>,
    pub human_additions: Option<i32>,
    pub mixed_additions: Option<Vec<i32>>,
    pub ai_additions: Option<Vec<i32>>,
    pub ai_accepted: Option<Vec<i32>>,
    pub git_diff_added_lines: Option<i32>,
    pub git_diff_deleted_lines: Option<i32>,
    pub tool_model_pairs: Option<Vec<String>>,
    pub model: Option<String>,
    pub prompt_id: Option<String>,
    pub session_id: Option<String>,
    pub file_path: Option<String>,
    pub custom_attributes: Option<HashMap<String, String>>,
    pub raw_values: HashMap<String, serde_json::Value>,
    pub raw_attrs: HashMap<String, serde_json::Value>,
}
