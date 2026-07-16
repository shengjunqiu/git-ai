use git_ai::authorship::working_log::CheckpointKind;
use git_ai::commands::checkpoint_agent::agent_presets::{
    AgentCheckpointFlags, AgentCheckpointPreset, TraePreset,
};
use git_ai::commands::checkpoint_agent::bash_tool::{self, Agent, ToolClass};
use serde_json::json;
use std::{fs, path::Path};

use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;

struct ScopedTraeUserDir {
    previous: Option<std::ffi::OsString>,
}

impl ScopedTraeUserDir {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os("GIT_AI_TRAE_USER_DIR");
        unsafe {
            // SAFETY: tests using this helper are serialized with serial_test.
            std::env::set_var("GIT_AI_TRAE_USER_DIR", path);
        }
        Self { previous }
    }
}

impl Drop for ScopedTraeUserDir {
    fn drop(&mut self) {
        unsafe {
            // SAFETY: tests using this helper are serialized with serial_test.
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("GIT_AI_TRAE_USER_DIR", previous);
            } else {
                std::env::remove_var("GIT_AI_TRAE_USER_DIR");
            }
        }
    }
}

fn flags_from_json(value: serde_json::Value) -> AgentCheckpointFlags {
    AgentCheckpointFlags {
        hook_input: Some(value.to_string()),
    }
}

fn trae_fixture_case(name: &str) -> serde_json::Value {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/agent-hooks/trae.json")).unwrap();
    fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("missing Trae fixture case: {name}"))["input"]
        .clone()
}

#[test]
#[serial_test::serial]
fn test_trae_windows_hook_fixtures_cover_current_tool_classification() {
    let expected = [
        ("pre-write", ToolClass::FileEdit),
        ("post-write", ToolClass::FileEdit),
        ("post-edit", ToolClass::FileEdit),
        ("post-run-command", ToolClass::Bash),
    ];

    for (case, expected_class) in expected {
        let input = trae_fixture_case(case);
        let tool_name = input["tool_name"].as_str().unwrap();
        assert_eq!(
            bash_tool::classify_tool(Agent::Trae, tool_name),
            expected_class
        );
    }

    let temp_trae_user_dir = tempfile::tempdir().unwrap();
    let _env = ScopedTraeUserDir::set(temp_trae_user_dir.path());
    let pre = TraePreset
        .run(flags_from_json(trae_fixture_case("pre-write")))
        .expect("Trae fixture PreToolUse should parse");
    let post = TraePreset
        .run(flags_from_json(trae_fixture_case("post-edit")))
        .expect("Trae fixture PostToolUse should parse");

    assert!(matches!(pre.checkpoint_kind, CheckpointKind::Human));
    assert!(matches!(post.checkpoint_kind, CheckpointKind::AiAgent));
}

#[test]
#[serial_test::serial]
fn test_trae_preset_pre_write_returns_human_scope() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_trae_user_dir = tempfile::tempdir().unwrap();
    let _env = ScopedTraeUserDir::set(temp_trae_user_dir.path());
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
#[serial_test::serial]
fn test_trae_preset_post_write_extracts_path_and_dirty_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_trae_user_dir = tempfile::tempdir().unwrap();
    let _env = ScopedTraeUserDir::set(temp_trae_user_dir.path());
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
#[serial_test::serial]
fn test_trae_preset_skips_read_without_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_trae_user_dir = tempfile::tempdir().unwrap();
    let _env = ScopedTraeUserDir::set(temp_trae_user_dir.path());
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

#[test]
#[serial_test::serial]
fn test_trae_checkpoint_commit_preserves_human_line_attribution() {
    let repo = TestRepo::new();
    let repo_root = repo.path().clone();
    let file_path = repo_root.join("src").join("main.rs");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(&file_path, "// human baseline\n").unwrap();
    repo.stage_all_and_commit("Add human baseline").unwrap();

    let temp_trae_user_dir = tempfile::tempdir().unwrap();
    let _env = ScopedTraeUserDir::set(temp_trae_user_dir.path());
    let repo_root_string = repo_root.to_string_lossy().to_string();
    let file_path_string = file_path.to_string_lossy().to_string();
    let pre_hook_input = json!({
        "session_id": "trae-e2e-session",
        "cwd": repo_root_string.clone(),
        "hook_event_name": "PreToolUse",
        "tool_use_id": "trae-e2e-write",
        "tool_name": "Write",
        "llm_tool_name": "Write",
        "tool_input": {
            "file_path": file_path_string.clone(),
            "content": "// human baseline\n// written by Trae\n"
        }
    })
    .to_string();
    let pre_output = repo
        .git_ai_with_stdin(
            &["checkpoint", "trae", "--hook-input", "stdin"],
            pre_hook_input.as_bytes(),
        )
        .expect("Trae pre-hook checkpoint should succeed");
    let active_edits_path = repo
        .current_working_logs()
        .dir
        .join("active_agent_edits.json");
    let active_edits = fs::read_to_string(&active_edits_path)
        .expect("Trae pre-hook should mark its target file as an active agent edit");
    assert!(active_edits.contains("src/main.rs"), "{active_edits}");
    assert!(active_edits.contains("trae"), "{active_edits}");

    fs::write(&file_path, "// human baseline\n// written by Trae\n").unwrap();

    let post_hook_input = json!({
        "session_id": "trae-e2e-session",
        "cwd": repo_root_string,
        "hook_event_name": "PostToolUse",
        "tool_use_id": "trae-e2e-write",
        "tool_name": "Write",
        "llm_tool_name": "Write",
        "tool_input": {
            "file_path": file_path_string.clone(),
            "content": "// human baseline\n// written by Trae\n"
        },
        "tool_response": {
            "filePath": file_path_string
        }
    })
    .to_string();
    let post_output = repo
        .git_ai_with_stdin(
            &["checkpoint", "trae", "--hook-input", "stdin"],
            post_hook_input.as_bytes(),
        )
        .expect("Trae post-hook checkpoint should succeed");

    let checkpoints = repo
        .current_working_logs()
        .read_all_checkpoints()
        .expect("Trae working log should be readable");
    assert!(
        checkpoints.iter().any(|checkpoint| {
            matches!(checkpoint.kind, CheckpointKind::AiAgent)
                && checkpoint
                    .entries
                    .iter()
                    .any(|entry| entry.file == "src/main.rs")
        }),
        "Trae post-hook should create an AI checkpoint for src/main.rs; pre output: {pre_output:?}; post output: {post_output:?}; checkpoints: {checkpoints:#?}"
    );
    assert!(
        !active_edits_path.exists(),
        "Trae post-hook should clear the active edit marker"
    );

    repo.stage_all_and_commit("Add Trae-authored line")
        .expect("commit should succeed");

    let mut tracked = repo.filename("src/main.rs");
    tracked.assert_lines_and_blame(crate::lines![
        "// human baseline".human(),
        "// written by Trae".ai(),
    ]);
}
