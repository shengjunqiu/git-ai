use git_ai::authorship::working_log::CheckpointKind;
use git_ai::commands::checkpoint_agent::agent_presets::{
    AgentCheckpointFlags, AgentCheckpointPreset, TraePreset,
};
use serde_json::json;

fn flags_from_json(value: serde_json::Value) -> AgentCheckpointFlags {
    AgentCheckpointFlags {
        hook_input: Some(value.to_string()),
    }
}

#[test]
fn test_trae_preset_pre_write_returns_human_scope() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("src/main.rs");

    let hook_input = json!({
        "session_id": "trae-session-1",
        "cwd": temp_dir.path(),
        "hook_event_name": "PreToolUse",
        "tool_use_id": "tool-1",
        "tool_name": "Write",
        "llm_tool_name": "Write",
        "tool_input": {
            "file_path": file_path,
            "content": "fn main() {}\n"
        }
    });

    let result = TraePreset
        .run(flags_from_json(hook_input))
        .expect("Trae pre hook should parse");

    assert_eq!(result.agent_id.tool, "trae");
    assert_eq!(result.agent_id.id, "trae-session-1");
    assert!(matches!(result.checkpoint_kind, CheckpointKind::Human));
    assert_eq!(
        result.will_edit_filepaths,
        Some(vec![file_path.to_string_lossy().to_string()])
    );
    assert!(result.edited_filepaths.is_none());
}

#[test]
fn test_trae_preset_post_write_extracts_path_and_dirty_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("src/main.rs");

    let hook_input = json!({
        "session_id": "trae-session-2",
        "cwd": temp_dir.path(),
        "hook_event_name": "PostToolUse",
        "tool_use_id": "tool-2",
        "tool_name": "Write",
        "llm_tool_name": "Write",
        "tool_input": {
            "file_path": file_path,
            "content": "fn main() {}\n"
        },
        "tool_response": {
            "filePath": file_path
        }
    });

    let result = TraePreset
        .run(flags_from_json(hook_input))
        .expect("Trae post hook should parse");

    assert_eq!(result.agent_id.tool, "trae");
    assert_eq!(result.agent_id.id, "trae-session-2");
    assert_eq!(result.agent_id.model, "unknown");
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
}

#[test]
fn test_trae_preset_skips_read_without_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let hook_input = json!({
        "session_id": "trae-session-3",
        "cwd": temp_dir.path(),
        "hook_event_name": "PreToolUse",
        "tool_use_id": "tool-3",
        "tool_name": "Read"
    });

    let error = TraePreset
        .run(flags_from_json(hook_input))
        .expect_err("Read without path should be skipped");

    assert!(
        error
            .to_string()
            .contains("Skipping Trae PreToolUse without mutating tool/path")
    );
}
