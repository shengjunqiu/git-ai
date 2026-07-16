use git_ai::authorship::transcript::Message;
use git_ai::authorship::working_log::CheckpointKind;
use git_ai::commands::checkpoint_agent::agent_presets::{
    AgentCheckpointFlags, AgentCheckpointPreset, QoderPreset,
};
use git_ai::commands::checkpoint_agent::bash_tool::{self, Agent, ToolClass};
use rusqlite::Connection;
use serde_json::json;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;

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

fn qoder_fixture_case(name: &str) -> serde_json::Value {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/agent-hooks/qoder.json")).unwrap();
    fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("missing Qoder fixture case: {name}"))["input"]
        .clone()
}

#[test]
fn test_qoder_windows_hook_fixtures_cover_native_tool_classification() {
    let expected = [
        ("pre-create-file", ToolClass::FileEdit),
        ("post-create-file", ToolClass::FileEdit),
        ("post-search-replace", ToolClass::FileEdit),
        ("post-run-in-terminal", ToolClass::Bash),
    ];

    for (case, expected_class) in expected {
        let input = qoder_fixture_case(case);
        let tool_name = input["tool_name"].as_str().unwrap();
        assert_eq!(
            bash_tool::classify_tool(Agent::Qoder, tool_name),
            expected_class
        );
    }

    let pre = QoderPreset
        .run(flags_from_json(qoder_fixture_case("pre-create-file")))
        .expect("Qoder fixture PreToolUse should parse");
    let post = QoderPreset
        .run(flags_from_json(qoder_fixture_case("post-search-replace")))
        .expect("Qoder fixture PostToolUse should parse");

    assert!(matches!(pre.checkpoint_kind, CheckpointKind::Human));
    assert!(matches!(post.checkpoint_kind, CheckpointKind::AiAgent));
}

fn create_qoder_storage_with_session_key(
    user_dir: &Path,
    session_key_prefix: &str,
    session_id: &str,
    selected_model: &str,
) {
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
        (format!("{session_key_prefix}.{session_id}"), selected_model),
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
    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
        (
            "aicoding.modelConfigs.cache.assistant",
            json!([{
                "name": "dfmodel",
                "displayName": "DeepSeek-V4-Flash"
            }])
            .to_string(),
        ),
    )
    .unwrap();
}

fn create_qoder_storage(user_dir: &Path, session_id: &str, selected_model: &str) {
    create_qoder_storage_with_session_key(
        user_dir,
        "chat.modelMapSession",
        session_id,
        selected_model,
    );
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
fn test_qoder_model_from_storage_resolves_current_model_map_key() {
    let temp_dir = tempfile::tempdir().unwrap();
    let user_dir = temp_dir.path().join("QoderCN").join("User");
    create_qoder_storage(&user_dir, "qoder-cn-session-model", "dfmodel");

    let model = QoderPreset::model_from_qoder_user_dir("qoder-cn-session-model", &user_dir)
        .expect("Qoder CN storage should parse");

    assert_eq!(model.as_deref(), Some("DeepSeek-V4-Flash"));
}

#[test]
fn test_qoder_model_from_storage_keeps_legacy_session_key_compatible() {
    let temp_dir = tempfile::tempdir().unwrap();
    let user_dir = temp_dir.path().join("Qoder").join("User");
    create_qoder_storage_with_session_key(
        &user_dir,
        "chat.modelConfig.session",
        "qoder-legacy-session-model",
        "dfmodel",
    );

    let model = QoderPreset::model_from_qoder_user_dir("qoder-legacy-session-model", &user_dir)
        .expect("legacy Qoder storage should parse");

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

#[test]
fn test_qoder_create_and_edit_attribution_flow() {
    let repo = TestRepo::new();
    let repo_root = repo.path().clone();
    let main_path = repo_root.join("src").join("main.rs");
    let generated_path = repo_root.join("src").join("generated.rs");
    fs::create_dir_all(main_path.parent().unwrap()).unwrap();
    fs::write(&main_path, "// human baseline\n").unwrap();
    repo.stage_all_and_commit("Add human baseline").unwrap();

    let repo_root_string = repo_root.to_string_lossy().to_string();
    let main_path_string = main_path.to_string_lossy().to_string();
    let generated_path_string = generated_path.to_string_lossy().to_string();
    let session_id = "qoder-file-attribution-session";

    let edit_pre = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PreToolUse",
        "tool_name": "search_replace",
        "tool_input": {
            "file_path": main_path_string,
            "old_string": "// human baseline\n",
            "new_string": "// human baseline\n// edited by Qoder\n"
        }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "qoder", "--hook-input", "stdin"],
        edit_pre.as_bytes(),
    )
    .expect("Qoder edit pre-hook should succeed");
    fs::write(&main_path, "// human baseline\n// edited by Qoder\n").unwrap();
    let edit_post = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PostToolUse",
        "tool_name": "search_replace",
        "tool_input": {
            "file_path": main_path_string,
            "old_string": "// human baseline\n",
            "new_string": "// human baseline\n// edited by Qoder\n"
        },
        "tool_response": "File edited successfully"
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "qoder", "--hook-input", "stdin"],
        edit_post.as_bytes(),
    )
    .expect("Qoder edit post-hook should succeed");

    let create_pre = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PreToolUse",
        "tool_name": "create_file",
        "tool_input": {
            "file_path": generated_path_string,
            "content": "// created by Qoder\n"
        }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "qoder", "--hook-input", "stdin"],
        create_pre.as_bytes(),
    )
    .expect("Qoder create pre-hook should succeed");
    fs::write(&generated_path, "// created by Qoder\n").unwrap();
    let create_post = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PostToolUse",
        "tool_name": "create_file",
        "tool_input": {
            "file_path": generated_path_string,
            "content": "// created by Qoder\n"
        },
        "tool_response": "File written successfully"
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "qoder", "--hook-input", "stdin"],
        create_post.as_bytes(),
    )
    .expect("Qoder create post-hook should succeed");

    let checkpoints = repo
        .current_working_logs()
        .read_all_checkpoints()
        .expect("Qoder working log should be readable");
    for expected_path in ["src/main.rs", "src/generated.rs"] {
        assert!(
            checkpoints.iter().any(|checkpoint| {
                matches!(checkpoint.kind, CheckpointKind::AiAgent)
                    && checkpoint
                        .entries
                        .iter()
                        .any(|entry| entry.file == expected_path)
            }),
            "missing Qoder AI checkpoint for {expected_path}: {checkpoints:#?}"
        );
    }

    repo.stage_all_and_commit("Add Qoder changes").unwrap();
    let mut main_file = repo.filename("src/main.rs");
    main_file.assert_lines_and_blame(crate::lines![
        "// human baseline".human(),
        "// edited by Qoder".ai(),
    ]);
    let mut generated_file = repo.filename("src/generated.rs");
    generated_file.assert_lines_and_blame(crate::lines!["// created by Qoder".ai(),]);
}

#[test]
fn test_qoder_terminal_sidecar_attribution_flow() {
    let repo = TestRepo::new();
    let repo_root = repo.path().clone();
    let generated_path = repo_root.join("generated.txt");
    fs::write(repo_root.join("README.md"), "human baseline\n").unwrap();
    repo.stage_all_and_commit("Initialize repository").unwrap();

    let repo_root_string = repo_root.to_string_lossy().to_string();
    let session_id = "qoder-terminal-attribution-session";
    let pre = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PreToolUse",
        "tool_name": "run_in_terminal",
        "tool_input": { "command": "write generated.txt" }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "qoder", "--hook-input", "stdin"],
        pre.as_bytes(),
    )
    .expect("Qoder terminal pre-hook should succeed");

    fs::write(&generated_path, "generated by Qoder\n").unwrap();
    let post = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PostToolUse",
        "tool_name": "run_in_terminal",
        "tool_input": { "command": "write generated.txt" },
        "tool_response": { "exit_code": 0, "stdout": "" }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "qoder", "--hook-input", "stdin"],
        post.as_bytes(),
    )
    .expect("Qoder terminal post-hook should succeed");

    let checkpoints = repo
        .current_working_logs()
        .read_all_checkpoints()
        .expect("Qoder terminal working log should be readable");
    assert!(
        checkpoints.iter().any(|checkpoint| {
            matches!(checkpoint.kind, CheckpointKind::AiAgent)
                && checkpoint
                    .entries
                    .iter()
                    .any(|entry| entry.file == "generated.txt")
        }),
        "terminal post-hook did not create a Qoder AI checkpoint: {checkpoints:#?}"
    );

    repo.stage_all_and_commit("Add terminal-generated file")
        .unwrap();
    let mut generated = repo.filename("generated.txt");
    generated.assert_lines_and_blame(crate::lines!["generated by Qoder".ai(),]);
}
