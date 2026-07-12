use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiFileRecord {
    #[serde(default)]
    pub annotations: HashMap<String, Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundleData {
    pub prompts: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub files: HashMap<String, ApiFileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateBundleRequest {
    pub title: String,
    pub data: BundleData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateBundleResponse {
    pub success: bool,
    pub id: String,
    pub url: String,
}
