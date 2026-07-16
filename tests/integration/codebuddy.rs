use git_ai::authorship::transcript::Message;
use git_ai::authorship::working_log::CheckpointKind;
use git_ai::commands::checkpoint_agent::agent_presets::{
    AgentCheckpointFlags, AgentCheckpointPreset, CodeBuddyPreset,
};
use git_ai::commands::checkpoint_agent::bash_tool::{self, Agent, ToolClass};
use serde_json::json;
use std::fs;

use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;

fn flags_from_json(value: serde_json::Value) -> AgentCheckpointFlags {
    AgentCheckpointFlags {
        hook_input: Some(value.to_string()),
    }
}

fn codebuddy_fixture_case(name: &str) -> serde_json::Value {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/agent-hooks/codebuddy.json")).unwrap();
    fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("missing CodeBuddy fixture case: {name}"))["input"]
        .clone()
}

#[test]
fn test_codebuddy_windows_hook_fixtures_cover_cli_and_ide_classification() {
    let expected = [
        ("cli-pre-write", ToolClass::FileEdit),
        ("cli-post-edit", ToolClass::FileEdit),
        ("cli-post-bash", ToolClass::Bash),
        ("ide-pre-write-to-file", ToolClass::FileEdit),
        ("ide-post-replace-in-file", ToolClass::FileEdit),
        ("ide-post-execute-command", ToolClass::Bash),
    ];
    for (case, expected_class) in expected {
        let input = codebuddy_fixture_case(case);
        let tool_name = input["tool_name"].as_str().unwrap();
        assert_eq!(
            bash_tool::classify_tool(Agent::CodeBuddy, tool_name),
            expected_class
        );
    }

    let cli_pre = CodeBuddyPreset
        .run(flags_from_json(codebuddy_fixture_case("cli-pre-write")))
        .expect("CodeBuddy CLI fixture should parse");
    assert!(matches!(cli_pre.checkpoint_kind, CheckpointKind::Human));

    let ide_pre = CodeBuddyPreset
        .run(flags_from_json(codebuddy_fixture_case(
            "ide-pre-write-to-file",
        )))
        .expect("CodeBuddy IDE fixture should parse");
    let ide_post = CodeBuddyPreset
        .run(flags_from_json(codebuddy_fixture_case(
            "ide-post-replace-in-file",
        )))
        .expect("CodeBuddy IDE PostToolUse fixture should parse");
    assert!(matches!(ide_pre.checkpoint_kind, CheckpointKind::Human));
    assert!(matches!(ide_post.checkpoint_kind, CheckpointKind::AiAgent));
}

#[test]
fn test_codebuddy_preset_pre_write_returns_human_scope() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("src/main.rs");

    let hook_input = json!({
        "session_id": "codebuddy-session-1",
        "transcript_path": temp_dir.path().join("session.jsonl"),
        "cwd": temp_dir.path(),
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": {
            "file_path": file_path,
            "content": "fn main() {}\n"
        }
    });

    let result = CodeBuddyPreset
        .run(flags_from_json(hook_input))
        .expect("CodeBuddy pre hook should parse");

    assert_eq!(result.agent_id.tool, "codebuddy");
    assert_eq!(result.agent_id.id, "codebuddy-session-1");
    assert!(matches!(result.checkpoint_kind, CheckpointKind::Human));
    assert_eq!(
        result.will_edit_filepaths,
        Some(vec![file_path.to_string_lossy().to_string()])
    );
    assert!(result.edited_filepaths.is_none());
}

#[test]
fn test_codebuddy_preset_post_write_extracts_path_dirty_file_and_transcript() {
    let temp_dir = tempfile::tempdir().unwrap();
    let transcript_path = temp_dir.path().join("session.jsonl");
    fs::write(
        &transcript_path,
        [
            r#"{"type":"user","message":{"content":"add a main function"},"timestamp":"2026-07-09T01:00:00Z"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"done"}],"model":"hunyuan-code"},"timestamp":"2026-07-09T01:00:01Z"}"#,
        ]
        .join("\n"),
    )
    .unwrap();

    let file_path = temp_dir.path().join("src/main.rs");
    let hook_input = json!({
        "session_id": "codebuddy-session-2",
        "transcript_path": transcript_path,
        "cwd": temp_dir.path(),
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_input": {
            "file_path": file_path,
            "content": "fn main() {}\n"
        },
        "tool_response": {
            "filePath": file_path,
            "success": true
        }
    });

    let result = CodeBuddyPreset
        .run(flags_from_json(hook_input))
        .expect("CodeBuddy post hook should parse");

    assert_eq!(result.agent_id.tool, "codebuddy");
    assert_eq!(result.agent_id.id, "codebuddy-session-2");
    assert_eq!(result.agent_id.model, "hunyuan-code");
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
fn test_codebuddy_preset_skips_read_without_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let hook_input = json!({
        "session_id": "codebuddy-session-3",
        "cwd": temp_dir.path(),
        "hook_event_name": "PreToolUse",
        "tool_name": "Read"
    });

    let error = CodeBuddyPreset
        .run(flags_from_json(hook_input))
        .expect_err("Read without path should be skipped");

    assert!(
        error
            .to_string()
            .contains("Skipping CodeBuddy PreToolUse without mutating tool/path")
    );
}

fn run_codebuddy_file_edit_attribution_flow(create_tool: &str, edit_tool: &str, label: &str) {
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
    let session_id = format!("codebuddy-{label}-file-session");

    let edit_pre = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PreToolUse",
        "tool_name": edit_tool,
        "tool_input": {
            "file_path": main_path_string,
            "old_string": "// human baseline\n",
            "new_string": "// human baseline\n// edited by CodeBuddy\n"
        }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "codebuddy", "--hook-input", "stdin"],
        edit_pre.as_bytes(),
    )
    .expect("CodeBuddy edit pre-hook should succeed");
    fs::write(&main_path, "// human baseline\n// edited by CodeBuddy\n").unwrap();
    let edit_post = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PostToolUse",
        "tool_name": edit_tool,
        "tool_input": {
            "file_path": main_path_string,
            "old_string": "// human baseline\n",
            "new_string": "// human baseline\n// edited by CodeBuddy\n"
        },
        "tool_response": { "filePath": main_path_string, "success": true }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "codebuddy", "--hook-input", "stdin"],
        edit_post.as_bytes(),
    )
    .expect("CodeBuddy edit post-hook should succeed");

    let create_pre = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PreToolUse",
        "tool_name": create_tool,
        "tool_input": {
            "file_path": generated_path_string,
            "content": "// created by CodeBuddy\n"
        }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "codebuddy", "--hook-input", "stdin"],
        create_pre.as_bytes(),
    )
    .expect("CodeBuddy create pre-hook should succeed");
    fs::write(&generated_path, "// created by CodeBuddy\n").unwrap();
    let create_post = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PostToolUse",
        "tool_name": create_tool,
        "tool_input": {
            "file_path": generated_path_string,
            "content": "// created by CodeBuddy\n"
        },
        "tool_response": { "filePath": generated_path_string, "success": true }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "codebuddy", "--hook-input", "stdin"],
        create_post.as_bytes(),
    )
    .expect("CodeBuddy create post-hook should succeed");

    let checkpoints = repo
        .current_working_logs()
        .read_all_checkpoints()
        .expect("CodeBuddy working log should be readable");
    for expected_path in ["src/main.rs", "src/generated.rs"] {
        assert!(
            checkpoints.iter().any(|checkpoint| {
                matches!(checkpoint.kind, CheckpointKind::AiAgent)
                    && checkpoint
                        .entries
                        .iter()
                        .any(|entry| entry.file == expected_path)
            }),
            "missing CodeBuddy AI checkpoint for {expected_path}: {checkpoints:#?}"
        );
    }

    repo.stage_all_and_commit("Add CodeBuddy changes").unwrap();
    let mut main_file = repo.filename("src/main.rs");
    main_file.assert_lines_and_blame(crate::lines![
        "// human baseline".human(),
        "// edited by CodeBuddy".ai(),
    ]);
    let mut generated_file = repo.filename("src/generated.rs");
    generated_file.assert_lines_and_blame(crate::lines!["// created by CodeBuddy".ai(),]);
}

#[test]
fn test_codebuddy_cli_create_and_edit_attribution_flow() {
    run_codebuddy_file_edit_attribution_flow("Write", "Edit", "cli");
}

#[test]
fn test_codebuddy_ide_create_and_edit_attribution_flow() {
    run_codebuddy_file_edit_attribution_flow("write_to_file", "replace_in_file", "ide");
}

fn run_codebuddy_terminal_attribution_flow(tool_name: &str, label: &str) {
    let repo = TestRepo::new();
    let repo_root = repo.path().clone();
    let generated_path = repo_root.join("generated.txt");
    fs::write(repo_root.join("README.md"), "human baseline\n").unwrap();
    repo.stage_all_and_commit("Initialize repository").unwrap();

    let repo_root_string = repo_root.to_string_lossy().to_string();
    let session_id = format!("codebuddy-{label}-terminal-session");
    let pre = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PreToolUse",
        "tool_name": tool_name,
        "tool_input": { "command": "write generated.txt" }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "codebuddy", "--hook-input", "stdin"],
        pre.as_bytes(),
    )
    .expect("CodeBuddy terminal pre-hook should succeed");

    fs::write(&generated_path, "generated by CodeBuddy\n").unwrap();
    let post = json!({
        "session_id": session_id,
        "cwd": repo_root_string,
        "hook_event_name": "PostToolUse",
        "tool_name": tool_name,
        "tool_input": { "command": "write generated.txt" },
        "tool_response": { "exit_code": 0, "stdout": "" }
    })
    .to_string();
    repo.git_ai_with_stdin(
        &["checkpoint", "codebuddy", "--hook-input", "stdin"],
        post.as_bytes(),
    )
    .expect("CodeBuddy terminal post-hook should succeed");

    let checkpoints = repo
        .current_working_logs()
        .read_all_checkpoints()
        .expect("CodeBuddy terminal working log should be readable");
    assert!(
        checkpoints.iter().any(|checkpoint| {
            matches!(checkpoint.kind, CheckpointKind::AiAgent)
                && checkpoint
                    .entries
                    .iter()
                    .any(|entry| entry.file == "generated.txt")
        }),
        "terminal post-hook did not create an AI checkpoint: {checkpoints:#?}"
    );

    repo.stage_all_and_commit("Add terminal-generated file")
        .unwrap();
    let mut generated = repo.filename("generated.txt");
    generated.assert_lines_and_blame(crate::lines!["generated by CodeBuddy".ai(),]);
}

#[test]
fn test_codebuddy_cli_terminal_sidecar_attribution_flow() {
    run_codebuddy_terminal_attribution_flow("Bash", "cli");
}

#[test]
fn test_codebuddy_ide_terminal_sidecar_attribution_flow() {
    run_codebuddy_terminal_attribution_flow("execute_command", "ide");
}
