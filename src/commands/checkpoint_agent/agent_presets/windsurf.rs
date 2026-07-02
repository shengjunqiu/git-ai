use super::*;

pub struct WindsurfPreset;
impl AgentCheckpointPreset for WindsurfPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        let stdin_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for Windsurf preset".to_string())
        })?;

        let hook_data: serde_json::Value = serde_json::from_str(&stdin_json)
            .map_err(|e| GitAiError::PresetError(format!("Invalid JSON in hook_input: {}", e)))?;

        let trajectory_id = hook_data
            .get("trajectory_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                GitAiError::PresetError("trajectory_id not found in hook_input".to_string())
            })?;

        let agent_action_name = hook_data
            .get("agent_action_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Extract cwd if present (Windsurf may or may not provide it)
        let cwd = hook_data
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Determine transcript path: either directly from tool_info or derived from trajectory_id
        let transcript_path = hook_data
            .get("tool_info")
            .and_then(|ti| ti.get("transcript_path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                let home = dirs::home_dir().unwrap_or_default();
                home.join(".windsurf")
                    .join("transcripts")
                    .join(format!("{}.jsonl", trajectory_id))
                    .to_string_lossy()
                    .to_string()
            });

        // Extract model_name from hook payload (Windsurf provides this on every hook event)
        let hook_model = hook_data
            .get("model_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && *s != "Unknown")
            .map(|s| s.to_string());

        // Parse transcript (best-effort)
        let (transcript, transcript_model) =
            match WindsurfPreset::transcript_and_model_from_windsurf_jsonl(&transcript_path) {
                Ok((transcript, model)) => (transcript, model),
                Err(e) => {
                    eprintln!("[Warning] Failed to parse Windsurf JSONL: {e}");
                    log_error(
                        &e,
                        Some(serde_json::json!({
                            "agent_tool": "windsurf",
                            "operation": "transcript_and_model_from_windsurf_jsonl"
                        })),
                    );
                    (crate::authorship::transcript::AiTranscript::new(), None)
                }
            };

        // Prefer hook-level model_name, fall back to transcript, then "unknown"
        let model = hook_model
            .or(transcript_model)
            .unwrap_or_else(|| "unknown".to_string());

        let agent_id = AgentId {
            tool: "windsurf".to_string(),
            id: trajectory_id.to_string(),
            model,
        };

        // Extract file_path from tool_info if present
        let file_path_as_vec = hook_data
            .get("tool_info")
            .and_then(|ti| ti.get("file_path"))
            .and_then(|v| v.as_str())
            .map(|path| vec![path.to_string()]);

        // Store transcript_path in metadata
        let agent_metadata =
            HashMap::from([("transcript_path".to_string(), transcript_path.to_string())]);

        // Windsurf's run_command is the bash-tool equivalent.  Mirror the Claude
        // pre/post stat-diff flow so file changes made by shell commands can be
        // attributed to the Windsurf agent.
        if matches!(agent_action_name, "pre_run_command" | "post_run_command") {
            // run_command payloads nest cwd under tool_info; fall back to the
            // top-level cwd for payload-shape resilience.
            let bash_cwd = hook_data
                .get("tool_info")
                .and_then(|ti| ti.get("cwd"))
                .and_then(|v| v.as_str())
                .or_else(|| hook_data.get("cwd").and_then(|v| v.as_str()))
                .map(|s| s.to_string());

            let session_id = trajectory_id;
            let tool_use_id = hook_data
                .get("execution_id")
                .and_then(|v| v.as_str())
                .unwrap_or("bash");

            if agent_action_name == "pre_run_command" {
                let pre_hook_captured_id = prepare_agent_bash_pre_hook(
                    true,
                    bash_cwd.as_deref(),
                    session_id,
                    tool_use_id,
                    &agent_id,
                    Some(&agent_metadata),
                    BashPreHookStrategy::EmitHumanCheckpoint,
                )?
                .captured_checkpoint_id();

                return Ok(AgentRunResult {
                    agent_id,
                    agent_metadata: None,
                    checkpoint_kind: CheckpointKind::Human,
                    transcript: None,
                    repo_working_dir: bash_cwd,
                    edited_filepaths: None,
                    will_edit_filepaths: None,
                    dirty_files: None,
                    captured_checkpoint_id: pre_hook_captured_id,
                });
            }

            // post_run_command: diff snapshots to recover the files the shell
            // command touched.
            let (edited_filepaths, bash_captured_checkpoint_id) = match bash_cwd.as_deref() {
                Some(cwd_str) => {
                    let repo_root = Path::new(cwd_str);
                    match bash_tool::handle_bash_tool(
                        HookEvent::PostToolUse,
                        repo_root,
                        session_id,
                        tool_use_id,
                    ) {
                        Ok(result) => {
                            let paths = match &result.action {
                                BashCheckpointAction::Checkpoint(paths) => Some(paths.clone()),
                                _ => None,
                            };
                            let capture_id = result
                                .captured_checkpoint
                                .as_ref()
                                .map(|info| info.capture_id.clone());
                            (paths, capture_id)
                        }
                        Err(e) => {
                            tracing::debug!("Windsurf bash post-hook error: {}", e);
                            (None, None)
                        }
                    }
                }
                None => (None, None),
            };

            return Ok(AgentRunResult {
                agent_id,
                agent_metadata: Some(agent_metadata),
                checkpoint_kind: CheckpointKind::AiAgent,
                transcript: Some(transcript),
                repo_working_dir: bash_cwd,
                edited_filepaths,
                will_edit_filepaths: None,
                dirty_files: None,
                captured_checkpoint_id: bash_captured_checkpoint_id,
            });
        }

        // pre_write_code is the human checkpoint (before AI edit)
        if agent_action_name == "pre_write_code" {
            return Ok(AgentRunResult {
                agent_id,
                agent_metadata: None,
                checkpoint_kind: CheckpointKind::Human,
                transcript: None,
                repo_working_dir: cwd.clone(),
                edited_filepaths: None,
                will_edit_filepaths: file_path_as_vec,
                dirty_files: None,
                captured_checkpoint_id: None,
            });
        }

        // post_write_code and post_cascade_response_with_transcript are AI checkpoints
        Ok(AgentRunResult {
            agent_id,
            agent_metadata: Some(agent_metadata),
            checkpoint_kind: CheckpointKind::AiAgent,
            transcript: Some(transcript),
            repo_working_dir: cwd,
            edited_filepaths: file_path_as_vec,
            will_edit_filepaths: None,
            dirty_files: None,
            captured_checkpoint_id: None,
        })
    }
}
impl WindsurfPreset {
    /// Parse a Windsurf JSONL transcript file into a transcript.
    /// Each line is a JSON object with a "type" field.
    /// Model info is not present in the JSONL format — always returns None.
    /// (Model is instead provided via `model_name` in the hook payload.)
    pub fn transcript_and_model_from_windsurf_jsonl(
        transcript_path: &str,
    ) -> Result<(AiTranscript, Option<String>), GitAiError> {
        let content = std::fs::read_to_string(transcript_path).map_err(GitAiError::IoError)?;

        let mut transcript = AiTranscript::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let entry: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // skip malformed lines
            };

            let entry_type = match entry.get("type").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => continue,
            };

            let timestamp = entry
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            // Windsurf nests data under a key matching the type name,
            // e.g. {"type": "user_input", "user_input": {"user_response": "..."}}
            let inner = entry.get(entry_type);

            match entry_type {
                "user_input" => {
                    if let Some(text) = inner
                        .and_then(|v| v.get("user_response"))
                        .and_then(|v| v.as_str())
                    {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            transcript.add_message(Message::User {
                                text: trimmed.to_string(),
                                timestamp,
                            });
                        }
                    }
                }
                "planner_response" => {
                    if let Some(text) = inner
                        .and_then(|v| v.get("response"))
                        .and_then(|v| v.as_str())
                    {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            transcript.add_message(Message::Assistant {
                                text: trimmed.to_string(),
                                timestamp,
                            });
                        }
                    }
                }
                "code_action" => {
                    if let Some(action) = inner {
                        let path = action
                            .get("path")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let new_content = action
                            .get("new_content")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);

                        transcript.add_message(Message::ToolUse {
                            name: "code_action".to_string(),
                            input: serde_json::json!({
                                "path": path,
                                "new_content": new_content,
                            }),
                            timestamp,
                        });
                    }
                }
                "view_file" | "run_command" | "find" | "grep_search" | "list_directory"
                | "list_resources" => {
                    // Map all tool-like actions to ToolUse
                    let input = inner.cloned().unwrap_or(serde_json::json!({}));
                    transcript.add_message(Message::ToolUse {
                        name: entry_type.to_string(),
                        input,
                        timestamp,
                    });
                }
                _ => {
                    // Skip truly unknown types silently
                    continue;
                }
            }
        }

        // Model info is not present in Windsurf JSONL format
        Ok((transcript, None))
    }
}
