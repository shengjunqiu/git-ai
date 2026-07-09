use git_ai::authorship::transcript::Message;
use git_ai::authorship::working_log::CheckpointKind;
use git_ai::commands::checkpoint_agent::agent_presets::{
    AgentCheckpointFlags, AgentCheckpointPreset, QoderPreset,
};
use rusqlite::Connection;
use serde_json::json;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

struct EnvVarGuard {
    key: &'static str,
    original: Option<OsString>,
}

impl EnvVarGuard {
    fn set_path(key: &'static str, value: &Path) -> Self {
        let original = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn flags_from_json(value: serde_json::Value) -> AgentCheckpointFlags {
    AgentCheckpointFlags {
        hook_input: Some(value.to_string()),
    }
}

fn create_qoder_storage(user_dir: &Path, session_id: &str, selected_model: &str) {
    let workspace_dir = user_dir.join("workspaceStorage").join("workspace-id");
    let global_dir = user_dir.join("globalStorage");
    fs::create_dir_all(&workspace_dir).unwrap();
    fs::create_dir_all(&global_dir).unwrap();

    let workspace_db = workspace_dir.join("state.vscdb");
    let conn = Connection::open(workspace_db).unwrap();
    conn.execute(
        "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
        (
            format!("chat.modelConfig.session.{session_id}"),
            selected_model,
        ),
    )
    .unwrap();

    let global_db = global_dir.join("state.vscdb");
    let conn = Connection::open(global_db).unwrap();
    conn.execute(
        "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
        (
            "aicoding.customModels",
            json!([{
                "id": "model_1783585678837_utox5zq",
                "provider": "deepseek",
                "model": "deepseek-v4-flash-pg",
                "displayName": "DeepSeek-V4-Flash"
            }])
            .to_string(),
        ),
    )
    .unwrap();
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
fn test_qoder_model_from_storage_resolves_custom_session_model() {
    let temp_dir = tempfile::tempdir().unwrap();
    let user_dir = temp_dir.path().join("Qoder").join("User");
    create_qoder_storage(
        &user_dir,
        "qoder-session-model",
        "custom:model_1783585678837_utox5zq",
    );

    let model = QoderPreset::model_from_qoder_user_dir("qoder-session-model", &user_dir)
        .expect("Qoder storage should parse");

    assert_eq!(model.as_deref(), Some("DeepSeek-V4-Flash"));
}

#[test]
#[serial_test::serial]
fn test_qoder_preset_uses_storage_model_when_hook_and_transcript_omit_model() {
    let temp_dir = tempfile::tempdir().unwrap();
    let user_dir = temp_dir.path().join("Qoder").join("User");
    create_qoder_storage(
        &user_dir,
        "qoder-session-storage-model",
        "custom:model_1783585678837_utox5zq",
    );
    let _qoder_user_dir = EnvVarGuard::set_path("GIT_AI_QODER_USER_DIR", &user_dir);

    let file_path = temp_dir.path().join("src/main.rs");
    let hook_input = json!({
        "session_id": "qoder-session-storage-model",
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

    assert_eq!(result.agent_id.model, "DeepSeek-V4-Flash");
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
