//! PosEncoded decoder for git-ai Metrics events.
//!
//! The git-ai client uses positional encoding for Metrics events: field names are
//! numeric index strings rather than semantic names. This decoder maps those indices
//! back to meaningful field names based on the encoding scheme in
//! `src/metrics/events.rs` and `src/metrics/attrs.rs`.
//!
//! Event types (matching client MetricEventId enum):
//!   1 = Committed
//!   2 = AgentUsage
//!   3 = InstallHooks
//!   4 = Checkpoint
//!
//! NOTE: The event type IDs are NOT sequential — they match the client enum values,
//! NOT zero-based indices.

use crate::error::AppError;
use crate::models::metrics::{DecodedMetricEvent, MetricEvent, MetricEventType};
use std::collections::HashMap;

// =====================================================================
// PosEncoded value field indices by event type
// Based on src/metrics/events.rs (client)
// =====================================================================

/// Committed event (type 1) value field mapping
/// Based on src/metrics/events.rs committed_pos module
const COMMITTED_VALUE_FIELDS: &[(&str, &str)] = &[
    ("0", "human_additions"),         // u32
    ("1", "git_diff_deleted_lines"),  // u32
    ("2", "git_diff_added_lines"),    // u32
    ("3", "tool_model_pairs"),        // Vec<String>
    ("4", "mixed_additions"),         // Vec<u32>
    ("5", "ai_additions"),            // Vec<u32>
    ("6", "ai_accepted"),             // Vec<u32>
    ("7", "total_ai_additions"),      // Vec<u32>
    ("8", "total_ai_deletions"),      // Vec<u32>
    ("9", "time_waiting_for_ai"),     // Vec<u64>
    ("10", "first_checkpoint_ts"),    // u64
    ("11", "commit_subject"),         // String
    ("12", "commit_body"),            // String
];

/// AgentUsage event (type 2) value field mapping
/// AgentUsage has no values in the sparse array (all data is in attrs)
const AGENT_USAGE_VALUE_FIELDS: &[(&str, &str)] = &[];

/// InstallHooks event (type 3) value field mapping
/// Based on src/metrics/events.rs install_hooks_pos module
const INSTALL_HOOKS_VALUE_FIELDS: &[(&str, &str)] = &[
    ("0", "tool_id"),    // String - tool id (e.g., "cursor", "fork")
    ("1", "status"),     // String - "not_found", "installed", "already_installed", "failed"
    ("2", "message"),    // Option<String> - error message or warnings
];

/// Checkpoint event (type 4) value field mapping
/// Based on src/metrics/events.rs checkpoint_pos module
const CHECKPOINT_VALUE_FIELDS: &[(&str, &str)] = &[
    ("0", "checkpoint_ts"),       // u64
    ("1", "kind"),                // String ("human", "ai_agent", "ai_tab")
    ("2", "file_path"),           // String
    ("3", "lines_added"),         // u32
    ("4", "lines_deleted"),       // u32
    ("5", "lines_added_sloc"),    // u32
    ("6", "lines_deleted_sloc"),  // u32
];

// =====================================================================
// Attribute field indices (shared across all event types)
// Based on src/metrics/attrs.rs attr_pos module
// =====================================================================

const ATTR_FIELDS: &[(&str, &str)] = &[
    ("0", "git_ai_version"),       // String (required)
    ("1", "repo_url"),             // String (nullable)
    ("2", "author"),               // String (nullable)
    ("3", "commit_sha"),           // String (nullable)
    ("4", "base_commit_sha"),      // String (nullable)
    ("5", "branch"),               // String (nullable)
    // Positions 6-19 reserved for future use
    ("20", "tool"),                // String (nullable)
    ("21", "model"),               // String (nullable)
    ("22", "prompt_id"),           // String (nullable)
    ("23", "external_prompt_id"),  // String (nullable)
    // Positions 24-29 reserved for future use
    ("30", "custom_attributes"),   // String (JSON, nullable)
];

/// Known attribute indices for filtering custom attributes
const KNOWN_ATTR_INDICES: &[&str] = &[
    "0", "1", "2", "3", "4", "5",
    "20", "21", "22", "23", "30",
];

/// Decode a PosEncoded metric event into a structured DecodedMetricEvent
pub fn decode_event(event: &MetricEvent) -> Result<DecodedMetricEvent, AppError> {
    let event_type = MetricEventType::try_from(event.event_id)
        .map_err(|e| AppError::BadRequest(e))?;

    let value_fields = match event_type {
        MetricEventType::InstallHooks => INSTALL_HOOKS_VALUE_FIELDS,
        MetricEventType::Committed => COMMITTED_VALUE_FIELDS,
        MetricEventType::AgentUsage => AGENT_USAGE_VALUE_FIELDS,
        MetricEventType::Checkpoint => CHECKPOINT_VALUE_FIELDS,
    };

    // Decode attributes
    let mut git_ai_version = None;
    let mut repo_url = None;
    let mut author = None;
    let mut commit_sha_attr = None;
    let mut base_commit_sha = None;
    let mut branch = None;
    let mut tool = None;
    let mut model = None;
    let mut prompt_id = None;
    let mut external_prompt_id = None;
    let mut custom_attributes_json = None;

    for (idx, field_name) in ATTR_FIELDS {
        if let Some(val) = event.attrs.get(*idx) {
            match *field_name {
                "git_ai_version" => git_ai_version = val.as_str().map(|s| s.to_string()),
                "repo_url" => repo_url = val.as_str().map(|s| s.to_string()),
                "author" => author = val.as_str().map(|s| s.to_string()),
                "commit_sha" => commit_sha_attr = val.as_str().map(|s| s.to_string()),
                "base_commit_sha" => base_commit_sha = val.as_str().map(|s| s.to_string()),
                "branch" => branch = val.as_str().map(|s| s.to_string()),
                "tool" => tool = val.as_str().map(|s| s.to_string()),
                "model" => model = val.as_str().map(|s| s.to_string()),
                "prompt_id" => prompt_id = val.as_str().map(|s| s.to_string()),
                "external_prompt_id" => external_prompt_id = val.as_str().map(|s| s.to_string()),
                "custom_attributes" => custom_attributes_json = val.as_str().map(|s| s.to_string()),
                _ => {}
            }
        }
    }

    // Decode values
    let mut human_additions = None;
    let mut git_diff_deleted_lines = None;
    let mut git_diff_added_lines = None;
    let mut tool_model_pairs: Option<Vec<String>> = None;
    let mut ai_additions: Option<Vec<i32>> = None;
    let mut mixed_additions: Option<Vec<i32>> = None;
    let mut ai_accepted: Option<Vec<i32>> = None;
    let mut total_ai_additions: Option<Vec<i32>> = None;
    let mut total_ai_deletions: Option<Vec<i32>> = None;
    let mut time_waiting_for_ai: Option<Vec<i64>> = None;
    let mut first_checkpoint_ts: Option<i64> = None;
    let mut commit_subject = None;
    let mut commit_body = None;
    let mut tool_id = None;
    let mut status = None;
    let mut message = None;
    let mut checkpoint_ts: Option<i64> = None;
    let mut kind = None;
    let mut file_path = None;
    let mut lines_added = None;
    let mut lines_deleted = None;
    let mut lines_added_sloc = None;
    let mut lines_deleted_sloc = None;

    for (idx, field_name) in value_fields {
        if let Some(val) = event.values.get(*idx) {
            match *field_name {
                // Committed fields
                "human_additions" => human_additions = val.as_i64().map(|v| v as i32),
                "git_diff_deleted_lines" => git_diff_deleted_lines = val.as_i64().map(|v| v as i32),
                "git_diff_added_lines" => git_diff_added_lines = val.as_i64().map(|v| v as i32),
                "tool_model_pairs" => {
                    if let Some(arr) = val.as_array() {
                        tool_model_pairs = Some(arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());
                    }
                }
                "mixed_additions" => {
                    if let Some(arr) = val.as_array() {
                        mixed_additions = Some(arr.iter().filter_map(|v| v.as_i64().map(|n| n as i32)).collect());
                    }
                }
                "ai_additions" => {
                    // Can be a single number or an array
                    if let Some(arr) = val.as_array() {
                        ai_additions = Some(arr.iter().filter_map(|v| v.as_i64().map(|n| n as i32)).collect());
                    } else {
                        ai_additions = val.as_i64().map(|v| vec![v as i32]);
                    }
                }
                "ai_accepted" => {
                    if let Some(arr) = val.as_array() {
                        ai_accepted = Some(arr.iter().filter_map(|v| v.as_i64().map(|n| n as i32)).collect());
                    }
                }
                "total_ai_additions" => {
                    if let Some(arr) = val.as_array() {
                        total_ai_additions = Some(arr.iter().filter_map(|v| v.as_i64().map(|n| n as i32)).collect());
                    }
                }
                "total_ai_deletions" => {
                    if let Some(arr) = val.as_array() {
                        total_ai_deletions = Some(arr.iter().filter_map(|v| v.as_i64().map(|n| n as i32)).collect());
                    }
                }
                "time_waiting_for_ai" => {
                    if let Some(arr) = val.as_array() {
                        time_waiting_for_ai = Some(arr.iter().filter_map(|v| v.as_i64()).collect());
                    }
                }
                "first_checkpoint_ts" => first_checkpoint_ts = val.as_i64(),
                "commit_subject" => commit_subject = val.as_str().map(|s| s.to_string()),
                "commit_body" => commit_body = val.as_str().map(|s| s.to_string()),
                // InstallHooks fields
                "tool_id" => tool_id = val.as_str().map(|s| s.to_string()),
                "status" => status = val.as_str().map(|s| s.to_string()),
                "message" => message = val.as_str().map(|s| s.to_string()),
                // Checkpoint fields
                "checkpoint_ts" => checkpoint_ts = val.as_i64(),
                "kind" => kind = val.as_str().map(|s| s.to_string()),
                "file_path" => file_path = val.as_str().map(|s| s.to_string()),
                "lines_added" => lines_added = val.as_i64().map(|v| v as i32),
                "lines_deleted" => lines_deleted = val.as_i64().map(|v| v as i32),
                "lines_added_sloc" => lines_added_sloc = val.as_i64().map(|v| v as i32),
                "lines_deleted_sloc" => lines_deleted_sloc = val.as_i64().map(|v| v as i32),
                _ => {}
            }
        }
    }

    // Extract custom_attributes from remaining attribute keys
    let custom_attributes: HashMap<String, String> = event
        .attrs
        .iter()
        .filter(|(k, _)| !KNOWN_ATTR_INDICES.contains(&k.as_str()))
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect();

    let custom_attributes = if custom_attributes.is_empty() && custom_attributes_json.is_none() {
        None
    } else if !custom_attributes.is_empty() {
        Some(custom_attributes)
    } else if let Some(json_str) = &custom_attributes_json {
        // Parse the JSON custom_attributes from position 30
        serde_json::from_str::<HashMap<String, String>>(json_str).ok()
    } else {
        None
    };

    // Calculate aggregate ai_additions for the metrics_events table
    let _ai_additions_total: Option<i32> = ai_additions
        .as_ref()
        .map(|v| v.iter().sum());

    // Use commit_sha from attrs (position 3) for Committed events,
    // or from the commit_sha field if available in attrs
    let commit_sha = commit_sha_attr;

    Ok(DecodedMetricEvent {
        event_type,
        timestamp: event.timestamp,
        distinct_id: None, // Not a standard attr position — could be derived from headers
        version: git_ai_version,
        repo_url,
        author,
        tool,
        commit_sha,
        human_additions,
        mixed_additions,
        ai_additions,
        ai_accepted,
        git_diff_added_lines,
        git_diff_deleted_lines,
        tool_model_pairs,
        model,
        prompt_id,
        session_id: None, // AgentUsage doesn't have session_id in values; it's in attrs
        file_path,
        custom_attributes,
        raw_values: event.values.clone(),
        raw_attrs: event.attrs.clone(),
    })
}

/// Validate that a hash string only contains hexadecimal characters
/// (mirrors client-side CAS hash validation)
pub fn validate_hex_hash(hash: &str) -> Result<(), AppError> {
    if hash.is_empty() {
        return Err(AppError::BadRequest("Hash cannot be empty".into()));
    }
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest(
            format!("Hash contains non-hexadecimal characters: {}", &hash[..hash.len().min(16)]),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_committed_event() {
        let mut v = HashMap::new();
        v.insert("0".into(), serde_json::json!(50));          // human_additions
        v.insert("1".into(), serde_json::json!(20));          // git_diff_deleted_lines
        v.insert("2".into(), serde_json::json!(150));         // git_diff_added_lines
        v.insert("3".into(), serde_json::json!(["all", "cursor:gpt-4"])); // tool_model_pairs
        v.insert("5".into(), serde_json::json!([100, 70]));   // ai_additions

        let mut a = HashMap::new();
        a.insert("0".into(), serde_json::json!("1.3.2"));     // git_ai_version
        a.insert("2".into(), serde_json::json!("dev@example.com")); // author
        a.insert("3".into(), serde_json::json!("abc123"));    // commit_sha

        let event = MetricEvent {
            timestamp: 1700000000,
            event_id: 1,
            values: v,
            attrs: a,
        };
        let decoded = decode_event(&event).unwrap();

        assert_eq!(decoded.event_type, MetricEventType::Committed);
        assert_eq!(decoded.human_additions, Some(50));
        assert_eq!(decoded.git_diff_added_lines, Some(150));
        assert_eq!(decoded.git_diff_deleted_lines, Some(20));
        assert_eq!(decoded.ai_additions, Some(vec![100, 70]));
        assert_eq!(decoded.author.as_deref(), Some("dev@example.com"));
        assert_eq!(decoded.version.as_deref(), Some("1.3.2"));
        assert_eq!(decoded.commit_sha.as_deref(), Some("abc123"));
        assert_eq!(
            decoded.tool_model_pairs,
            Some(vec!["all".to_string(), "cursor:gpt-4".to_string()])
        );
    }

    #[test]
    fn test_decode_install_hooks_event() {
        let mut v = HashMap::new();
        v.insert("0".into(), serde_json::json!("cursor"));        // tool_id
        v.insert("1".into(), serde_json::json!("installed"));      // status
        v.insert("2".into(), serde_json::json!("Successfully installed")); // message

        let mut a = HashMap::new();
        a.insert("0".into(), serde_json::json!("1.3.2"));         // git_ai_version

        let event = MetricEvent {
            timestamp: 1700000000,
            event_id: 3,
            values: v,
            attrs: a,
        };
        let decoded = decode_event(&event).unwrap();

        assert_eq!(decoded.event_type, MetricEventType::InstallHooks);
    }

    #[test]
    fn test_decode_checkpoint_event() {
        let mut v = HashMap::new();
        v.insert("0".into(), serde_json::json!(1704067200));      // checkpoint_ts
        v.insert("1".into(), serde_json::json!("ai_agent"));      // kind
        v.insert("2".into(), serde_json::json!("src/main.rs"));   // file_path
        v.insert("3".into(), serde_json::json!(50));              // lines_added
        v.insert("4".into(), serde_json::json!(10));              // lines_deleted

        let mut a = HashMap::new();
        a.insert("0".into(), serde_json::json!("1.3.2"));         // git_ai_version
        a.insert("20".into(), serde_json::json!("claude-code"));  // tool
        a.insert("21".into(), serde_json::json!("claude-3"));     // model

        let event = MetricEvent {
            timestamp: 1700000000,
            event_id: 4,
            values: v,
            attrs: a,
        };
        let decoded = decode_event(&event).unwrap();

        assert_eq!(decoded.event_type, MetricEventType::Checkpoint);
        assert_eq!(decoded.tool.as_deref(), Some("claude-code"));
        assert_eq!(decoded.model.as_deref(), Some("claude-3"));
    }

    #[test]
    fn test_decode_agent_usage_event() {
        let mut a = HashMap::new();
        a.insert("0".into(), serde_json::json!("1.3.2"));         // git_ai_version
        a.insert("20".into(), serde_json::json!("copilot"));      // tool
        a.insert("21".into(), serde_json::json!("gpt-4"));        // model

        let event = MetricEvent {
            timestamp: 1700000000,
            event_id: 2,
            values: HashMap::new(),
            attrs: a,
        };
        let decoded = decode_event(&event).unwrap();

        assert_eq!(decoded.event_type, MetricEventType::AgentUsage);
        assert_eq!(decoded.tool.as_deref(), Some("copilot"));
        assert_eq!(decoded.model.as_deref(), Some("gpt-4"));
    }

    #[test]
    fn test_validate_hex_hash() {
        assert!(validate_hex_hash("a1b2c3d4e5f6").is_ok());
        assert!(validate_hex_hash("").is_err());
        assert!(validate_hex_hash("g12345").is_err());
    }

    #[test]
    fn test_committed_event_with_new_fields() {
        let mut v = HashMap::new();
        v.insert("0".into(), serde_json::json!(10));              // human_additions
        v.insert("5".into(), serde_json::json!([100]));           // ai_additions
        v.insert("10".into(), serde_json::json!(1704067200));     // first_checkpoint_ts
        v.insert("11".into(), serde_json::json!("Initial commit")); // commit_subject

        let mut a = HashMap::new();
        a.insert("0".into(), serde_json::json!("1.3.2"));         // git_ai_version

        let event = MetricEvent {
            timestamp: 1700000000,
            event_id: 1,
            values: v,
            attrs: a,
        };
        let decoded = decode_event(&event).unwrap();

        assert_eq!(decoded.human_additions, Some(10));
        assert_eq!(decoded.ai_additions, Some(vec![100]));
    }
}
