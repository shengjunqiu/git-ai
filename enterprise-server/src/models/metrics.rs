use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Metrics batch upload request matching client's MetricsBatch
///
/// Client sends `{"v":1,"events":[...]}` — the version is a numeric `v` field.
/// The `api_version` field is kept for backward compatibility but is optional.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsBatch {
    /// Client sends version as `v: u8` (e.g. 1). Deserialized from JSON number.
    #[serde(rename = "v")]
    pub version: u8,
    pub events: Vec<MetricEvent>,
}

/// Single metric event in PosEncoded format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricEvent {
    #[serde(rename = "t")]
    pub t: i64,                        // Unix timestamp
    #[serde(rename = "e")]
    pub e: i32,                        // Event type ID (1-4)
    #[serde(rename = "v")]
    pub v: HashMap<String, serde_json::Value>,  // PosEncoded values
    #[serde(rename = "a", default)]
    pub a: HashMap<String, serde_json::Value>,  // PosEncoded attributes (SparseArray — values can be String, Number, Null, Array)
}

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

/// Metrics upload response matching client's MetricsUploadResponse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsUploadResponse {
    pub errors: Vec<MetricUploadError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricUploadError {
    pub index: usize,
    pub error: String,
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
    pub ai_additions: Option<Vec<i32>>,
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
