use crate::authorship::authorship_log::LineRange;
use crate::commands::diff::FileDiffJson;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use git_ai_protocol::api::ApiErrorResponse;
pub use git_ai_protocol::bundle::{
    ApiFileRecord, BundleData, CreateBundleRequest, CreateBundleResponse,
};
pub use git_ai_protocol::cas::{
    CasObject, CasReadResponse as CAPromptStoreReadResponse,
    CasReadResult as CAPromptStoreReadResult, CasUploadRequest, CasUploadResponse, CasUploadResult,
};

/// Convert the CLI's diff model into its transport representation.
pub fn api_file_record_from_diff(file_diff: &FileDiffJson) -> ApiFileRecord {
    let annotations: HashMap<String, Vec<serde_json::Value>> = file_diff
        .annotations
        .iter()
        .map(|(key, ranges)| {
            let json_ranges = ranges
                .iter()
                .map(|range| match range {
                    LineRange::Single(line) => serde_json::Value::Number((*line as u64).into()),
                    LineRange::Range(start, end) => serde_json::Value::Array(vec![
                        serde_json::Value::Number((*start as u64).into()),
                        serde_json::Value::Number((*end as u64).into()),
                    ]),
                })
                .collect();
            (key.clone(), json_ranges)
        })
        .collect();

    ApiFileRecord {
        annotations,
        diff: Some(file_diff.diff.clone()),
        base_content: Some(file_diff.base_content.clone()),
    }
}

/// Client-only typed view of messages stored inside a generic CAS object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CasMessagesObject {
    pub messages: Vec<crate::authorship::transcript::Message>,
}
