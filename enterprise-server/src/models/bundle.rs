use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Create bundle request matching client's CreateBundleRequest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBundleRequest {
    pub title: String,
    pub data: BundleData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleData {
    pub prompts: HashMap<String, serde_json::Value>,  // hash -> PromptRecord JSON
    #[serde(default)]
    pub files: HashMap<String, ApiFileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiFileRecord {
    #[serde(default)]
    pub annotations: HashMap<String, Vec<serde_json::Value>>,  // prompt_hash -> line ranges
    #[serde(default)]
    pub diff: Option<String>,
    #[serde(default)]
    pub base_content: Option<String>,
}

/// Create bundle response matching client's CreateBundleResponse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBundleResponse {
    pub success: bool,
    pub id: String,
    pub url: String,
}
