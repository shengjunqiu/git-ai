use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// CAS upload request matching client's CasUploadRequest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasUploadRequest {
    pub objects: Vec<CasObject>,
}

/// Single CAS object for upload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasObject {
    pub content: serde_json::Value,      // PromptRecord JSON (format varies by agent)
    pub hash: String,                     // SHA256 hash string
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// CAS upload result for a single object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasUploadResult {
    pub hash: String,
    pub status: String,                   // "ok" or "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// CAS upload response matching client expectations
/// Client uses usize for success_count/failure_count — i64 is compatible via JSON number coercion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasUploadResponse {
    pub results: Vec<CasUploadResult>,
    pub success_count: i64,
    pub failure_count: i64,
}

/// CAS read response matching client's CAPromptStoreReadResponse
/// Client uses usize for success_count/failure_count — i64 is compatible via JSON number coercion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasReadResponse {
    pub results: Vec<CasReadResult>,
    pub success_count: i64,
    pub failure_count: i64,
}

/// Single CAS read result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasReadResult {
    pub hash: String,
    pub status: String,                   // "ok" or "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Agent ID structure from PromptRecord
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentId {
    pub tool: String,
    pub id: String,
    pub model: String,
}

/// PromptRecord structure (CAS content)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRecord {
    pub agent_id: AgentId,
    pub human_author: Option<String>,
    pub messages: Vec<AiTranscriptMessage>,
    pub total_additions: u32,
    pub total_deletions: u32,
}

/// AI transcript message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiTranscriptMessage {
    pub role: String,
    pub content: Option<serde_json::Value>,
}
