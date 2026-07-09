use git_ai::authorship::transcript::Message;
use git_ai::authorship::working_log::CheckpointKind;
use git_ai::commands::checkpoint_agent::agent_presets::{
    AgentCheckpointFlags, AgentCheckpointPreset, QoderPreset,
};
use serde_json::json;
use std::fs;

fn flags_from_json(value: serde_json::Value) -> AgentCheckpointFlags {
    AgentCheckpointFlags {
        hook_input: Some(value.to_string()),
    }
}

#[test]
fn test_qoder_preset_pre_write_returns_human_scope() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("src/main.rs");

    let hook_input = json!({
        "session_id": "qoder-session-1",
        "cwd": temp_dir.path(),
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": {
            "file_path": file_path,
            "content": "fn main() {}\n"
        }
    });

    let result = QoderPreset
        .run(flags_from_json(hook_input))
        .expect("Qoder pre hook should parse");

    assert_eq!(result.agent_id.tool, "qoder");
    assert_eq!(result.agent_id.id, "qoder-session-1");
    assert!(matches!(result.checkpoint_kind, CheckpointKind::Human));
    assert_eq!(
        result.will_edit_filepaths,
        Some(vec![file_path.to_string_lossy().to_string()])
    );
    assert!(result.edited_filepaths.is_none());
}

#[test]
fn test_qoder_preset_post_create_file_extracts_path_dirty_file_and_transcript() {
    let temp_dir = tempfile::tempdir().unwrap();
    let transcript_path = temp_dir.path().join("session.json");
    fs::write(
        &transcript_path,
        json!({
            "messages": [
                {
                    "role": "user",
                    "content": "add a main function",
                    "timestamp": "2026-07-09T01:00:00Z"
                },
                {
                    "role": "assistant",
                    "content": [{"text": "done"}],
                    "model": "qwen-code",
                    "timestamp": "2026-07-09T01:00:01Z"
                }
            ]
        })
        .to_string(),
    )
    .unwrap();

    let file_path = temp_dir.path().join("src/main.rs");
    let hook_input = json!({
        "session_id": "qoder-session-2",
        "cwd": temp_dir.path(),
        "hook_event_name": "PostToolUse",
        "transcript_path": transcript_path,
        "tool_name": "create_file",
        "tool_input": {
            "file_path": file_path,
            "content": "fn main() {}\n"
        },
        "tool_response": "File written successfully"
    });

    let result = QoderPreset
        .run(flags_from_json(hook_input))
        .expect("Qoder post hook should parse");

    assert_eq!(result.agent_id.tool, "qoder");
    assert_eq!(result.agent_id.id, "qoder-session-2");
    assert_eq!(result.agent_id.model, "qwen-code");
    assert!(matches!(result.checkpoint_kind, CheckpointKind::AiAgent));

    let expected_path = file_path.to_string_lossy().to_string();
    assert_eq!(result.edited_filepaths, Some(vec![expected_path.clone()]));
    assert_eq!(
        result
            .dirty_files
            .unwrap()
            .get(&expected_path)
            .map(String::as_str),
        Some("fn main() {}\n")
    );

    let transcript = result.transcript.expect("transcript should be present");
    assert_eq!(transcript.messages().len(), 2);
    assert!(matches!(transcript.messages()[0], Message::User { .. }));
    assert!(matches!(
        transcript.messages()[1],
        Message::Assistant { .. }
    ));
}

#[test]
fn test_qoder_preset_post_search_replace_extracts_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("src/main.rs");

    let hook_input = json!({
        "session_id": "qoder-session-4",
        "cwd": temp_dir.path(),
        "hook_event_name": "PostToolUse",
        "tool_name": "SearchReplace",
        "tool_input": {
            "file_path": file_path,
            "replacements": [{
                "original_text": "old",
                "new_text": "new"
            }]
        },
        "tool_response": "edit file by SearchReplace success"
    });

    let result = QoderPreset
        .run(flags_from_json(hook_input))
        .expect("Qoder SearchReplace hook should parse");

    assert_eq!(result.agent_id.tool, "qoder");
    assert_eq!(
        result.edited_filepaths,
        Some(vec![file_path.to_string_lossy().to_string()])
    );
}

#[test]
fn test_qoder_preset_skips_read_without_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let hook_input = json!({
        "session_id": "qoder-session-3",
        "cwd": temp_dir.path(),
        "hook_event_name": "PreToolUse",
        "tool_name": "Read"
    });

    let error = QoderPreset
        .run(flags_from_json(hook_input))
        .expect_err("Read without path should be skipped");

    assert!(
        error
            .to_string()
            .contains("Skipping Qoder PreToolUse without mutating tool/path")
    );
}
