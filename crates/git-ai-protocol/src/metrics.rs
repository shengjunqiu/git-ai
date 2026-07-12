use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const METRICS_API_VERSION: u8 = 1;

pub type SparseArray = HashMap<String, Value>;

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricEventId {
    Committed = 1,
    AgentUsage = 2,
    InstallHooks = 3,
    Checkpoint = 4,
}

pub trait EventValues: Sized {
    fn event_id() -> MetricEventId;
    fn to_sparse(&self) -> SparseArray;
    fn from_sparse(arr: &SparseArray) -> Self;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricEvent {
    #[serde(rename = "t")]
    pub timestamp: i64,
    #[serde(rename = "e")]
    pub event_id: i32,
    #[serde(rename = "v")]
    pub values: SparseArray,
    #[serde(rename = "a", default)]
    pub attrs: SparseArray,
}

impl MetricEvent {
    pub fn new<V: EventValues>(values: &V, attrs: SparseArray) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            event_id: V::event_id() as i32,
            values: values.to_sparse(),
            attrs,
        }
    }

    pub fn with_timestamp<V: EventValues>(timestamp: u32, values: &V, attrs: SparseArray) -> Self {
        Self {
            timestamp: i64::from(timestamp),
            event_id: V::event_id() as i32,
            values: values.to_sparse(),
            attrs,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsBatch {
    #[serde(rename = "v")]
    pub version: u8,
    pub events: Vec<MetricEvent>,
}

impl MetricsBatch {
    pub fn new(events: Vec<MetricEvent>) -> Self {
        Self {
            version: METRICS_API_VERSION,
            events,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricUploadError {
    pub index: usize,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricsUploadResponse {
    pub errors: Vec<MetricUploadError>,
}

impl MetricsUploadResponse {
    pub fn successful_indices(&self, batch_size: usize) -> Vec<usize> {
        let error_indices: HashSet<_> = self.errors.iter().map(|error| error.index).collect();
        (0..batch_size)
            .filter(|index| !error_indices.contains(index))
            .collect()
    }
}
