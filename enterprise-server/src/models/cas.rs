use serde::{Deserialize, Serialize};

pub use git_ai_protocol::cas::{CasObject, CasUploadRequest};

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
