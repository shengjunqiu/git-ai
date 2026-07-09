use super::*;

pub struct CodeBuddyPreset;

impl AgentCheckpointPreset for CodeBuddyPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        let stdin_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for CodeBuddy preset".to_string())
        })?;

        let hook_data: serde_json::Value = serde_json::from_str(&stdin_json)
            .map_err(|e| GitAiError::PresetError(format!("Invalid JSON in hook_input: {}", e)))?;

        let session_id =
            Self::string_at(&hook_data, &["session_id", "sessionId"]).ok_or_else(|| {
                GitAiError::PresetError("session_id not found in hook_input".to_string())
            })?;

        let cwd = Self::string_at(&hook_data, &["cwd"])
            .ok_or_else(|| GitAiError::PresetError("cwd not found in hook_input".to_string()))?;

        let transcript_path = Self::string_at(&hook_data, &["transcript_path", "transcriptPath"]);
        let hook_event_name = Self::string_at(&hook_data, &["hook_event_name", "hookEventName"]);
        let tool_name = Self::string_at(&hook_data, &["tool_name", "toolName"]);
        let tool_use_id =
            Self::string_at(&hook_data, &["tool_use_id", "toolUseId"]).unwrap_or("codebuddy-tool");

        let (transcript, transcript_model) = match transcript_path {
            Some(path) if Path::new(path).exists() => {
                match Self::transcript_and_model_from_codebuddy_jsonl(path) {
                    Ok((transcript, model)) => (transcript, model),
                    Err(e) => {
                        eprintln!("[Warning] Failed to parse CodeBuddy JSONL: {e}");
                        log_error(
                            &e,
                            Some(serde_json::json!({
                                "agent_tool": "codebuddy",
                                "operation": "transcript_and_model_from_codebuddy_jsonl"
                            })),
                        );
                        (AiTranscript::new(), None)
                    }
                }
            }
            _ => (AiTranscript::new(), None),
        };

        let model = transcript_model
            .or_else(|| Self::model_from_value(&hook_data))
            .unwrap_or_else(|| "unknown".to_string());

        let agent_id = AgentId {
            tool: "codebuddy".to_string(),
            id: session_id.to_string(),
            model,
        };

        let mut agent_metadata = HashMap::new();
        if let Some(path) = transcript_path {
            agent_metadata.insert("transcript_path".to_string(), path.to_string());
        }

        let explicit_filepaths = Self::filepaths_from_hook_data(&hook_data);
        let dirty_files = Self::dirty_files_from_hook_data(&hook_data, explicit_filepaths.as_ref());
        let dirty_filepaths = dirty_files.as_ref().map(|files| {
            let mut paths = files.keys().cloned().collect::<Vec<_>>();
            paths.sort();
            paths
        });
        let target_filepaths = explicit_filepaths.clone().or(dirty_filepaths);

        let tool_class = tool_name
            .map(|name| bash_tool::classify_tool(Agent::CodeBuddy, name))
            .unwrap_or_else(|| {
                if target_filepaths.is_some() {
                    ToolClass::FileEdit
                } else {
                    ToolClass::Skip
                }
            });
        let is_bash_tool = tool_class == ToolClass::Bash;

        if Self::is_pre_tool_use(hook_event_name) {
            if tool_class == ToolClass::Skip {
                return Err(GitAiError::PresetError(
                    "Skipping CodeBuddy PreToolUse without mutating tool/path".to_string(),
                ));
            }

            let pre_hook_captured_id = prepare_agent_bash_pre_hook(
                is_bash_tool,
                Some(cwd),
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
                repo_working_dir: Some(cwd.to_string()),
                edited_filepaths: None,
                will_edit_filepaths: target_filepaths,
                dirty_files: None,
                captured_checkpoint_id: pre_hook_captured_id,
            });
        }

        if !Self::is_post_tool_use(hook_event_name) {
            return Err(GitAiError::PresetError(format!(
                "Skipping unsupported CodeBuddy hook event: {}",
                hook_event_name.unwrap_or("unknown")
            )));
        }

        let bash_result = if is_bash_tool {
            Some(bash_tool::handle_bash_tool(
                HookEvent::PostToolUse,
                Path::new(cwd),
                session_id,
                tool_use_id,
            ))
        } else {
            None
        };

        let edited_filepaths = if is_bash_tool {
            match bash_result.as_ref().unwrap().as_ref().map(|r| &r.action) {
                Ok(BashCheckpointAction::Checkpoint(paths)) => Some(paths.clone()),
                Ok(BashCheckpointAction::NoChanges) => None,
                Ok(BashCheckpointAction::Fallback) | Ok(BashCheckpointAction::TakePreSnapshot) => {
                    target_filepaths
                }
                Err(e) => {
                    tracing::debug!("CodeBuddy bash tool post-hook error: {}", e);
                    target_filepaths
                }
            }
        } else {
            target_filepaths
        };

        if !is_bash_tool && edited_filepaths.is_none() && dirty_files.is_none() {
            return Err(GitAiError::PresetError(
                "Skipping CodeBuddy PostToolUse without edited path or dirty file content"
                    .to_string(),
            ));
        }

        let bash_captured_checkpoint_id = bash_result
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|r| r.captured_checkpoint.as_ref())
            .map(|info| info.capture_id.clone());

        Ok(AgentRunResult {
            agent_id,
            agent_metadata: if agent_metadata.is_empty() {
                None
            } else {
                Some(agent_metadata)
            },
            checkpoint_kind: CheckpointKind::AiAgent,
            transcript: Some(transcript),
            repo_working_dir: Some(cwd.to_string()),
            edited_filepaths,
            will_edit_filepaths: None,
            dirty_files,
            captured_checkpoint_id: bash_captured_checkpoint_id,
        })
    }
}

impl CodeBuddyPreset {
    pub fn transcript_and_model_from_codebuddy_jsonl(
        transcript_path: &str,
    ) -> Result<(AiTranscript, Option<String>), GitAiError> {
        let jsonl_content =
            std::fs::read_to_string(transcript_path).map_err(GitAiError::IoError)?;
        let mut transcript = AiTranscript::new();
        let mut model = None;

        for line in jsonl_content.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let raw_entry: serde_json::Value = serde_json::from_str(line)?;
            let timestamp = Self::timestamp_from_value(&raw_entry);

            if model.is_none() {
                model = Self::model_from_value(&raw_entry);
            }

            Self::add_transcript_message(&mut transcript, &raw_entry, timestamp.clone());

            if let Some(message) = raw_entry.get("message")
                && !message.is_string()
            {
                Self::add_transcript_message(&mut transcript, message, timestamp);
            }
        }

        Ok((transcript, model))
    }

    fn string_at<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
        keys.iter().find_map(|key| value.get(*key)?.as_str())
    }

    fn is_pre_tool_use(event_name: Option<&str>) -> bool {
        event_name
            .map(|name| name.eq_ignore_ascii_case("PreToolUse"))
            .unwrap_or(false)
    }

    fn is_post_tool_use(event_name: Option<&str>) -> bool {
        event_name
            .map(|name| name.eq_ignore_ascii_case("PostToolUse"))
            .unwrap_or(false)
    }

    fn filepaths_from_hook_data(hook_data: &serde_json::Value) -> Option<Vec<String>> {
        let mut paths = Vec::new();

        for key in [
            "edited_filepaths",
            "editedFilepaths",
            "file_paths",
            "filePaths",
        ] {
            if let Some(values) = hook_data.get(key).and_then(|v| v.as_array()) {
                for value in values {
                    if let Some(path) = value.as_str() {
                        Self::push_path(&mut paths, path);
                    }
                }
            }
        }

        for parent_key in ["tool_input", "toolInput", "tool_response", "toolResponse"] {
            if let Some(parent) = hook_data.get(parent_key) {
                for key in ["file_path", "filePath", "path"] {
                    if let Some(path) = parent.get(key).and_then(|v| v.as_str()) {
                        Self::push_path(&mut paths, path);
                    }
                }
            }
        }

        if paths.is_empty() { None } else { Some(paths) }
    }

    fn dirty_files_from_hook_data(
        hook_data: &serde_json::Value,
        explicit_filepaths: Option<&Vec<String>>,
    ) -> Option<HashMap<String, String>> {
        if let Some(files) = hook_data
            .get("dirty_files")
            .or_else(|| hook_data.get("dirtyFiles"))
            && let Some(obj) = files.as_object()
        {
            let mut dirty_files = HashMap::new();
            for (path, content) in obj {
                if let Some(content) = content.as_str() {
                    dirty_files.insert(path.clone(), content.to_string());
                }
            }
            if !dirty_files.is_empty() {
                return Some(dirty_files);
            }
        }

        let tool_name = Self::string_at(hook_data, &["tool_name", "toolName"]);
        if !matches!(
            tool_name,
            Some("Write") | Some("write") | Some("Create") | Some("create")
        ) {
            return None;
        }

        let content = hook_data
            .get("tool_input")
            .or_else(|| hook_data.get("toolInput"))
            .and_then(|input| input.get("content"))
            .and_then(|content| content.as_str())?;

        let paths = explicit_filepaths?;
        if paths.len() != 1 {
            return None;
        }

        Some(HashMap::from([(paths[0].clone(), content.to_string())]))
    }

    fn push_path(paths: &mut Vec<String>, path: &str) {
        if path.trim().is_empty() || paths.iter().any(|p| p == path) {
            return;
        }
        paths.push(path.to_string());
    }

    fn timestamp_from_value(value: &serde_json::Value) -> Option<String> {
        Self::string_at(value, &["timestamp", "created_at", "createdAt"]).map(str::to_string)
    }

    fn model_from_value(value: &serde_json::Value) -> Option<String> {
        Self::string_at(value, &["model", "model_name", "modelName"])
            .or_else(|| {
                value
                    .get("message")
                    .and_then(|message| Self::string_at(message, &["model", "model_name"]))
            })
            .map(str::to_string)
    }

    fn add_transcript_message(
        transcript: &mut AiTranscript,
        value: &serde_json::Value,
        timestamp: Option<String>,
    ) {
        if let Some(tool_name) = Self::string_at(value, &["tool_name", "toolName", "name"])
            && Self::string_at(value, &["type"]) == Some("tool_use")
        {
            let input = value
                .get("input")
                .or_else(|| value.get("tool_input"))
                .or_else(|| value.get("toolInput"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            transcript.add_message(Message::ToolUse {
                name: tool_name.to_string(),
                input,
                timestamp,
            });
            return;
        }

        let role = Self::string_at(value, &["role", "type", "sender"]);
        let text = value
            .get("content")
            .or_else(|| value.get("text"))
            .or_else(|| value.get("message"))
            .and_then(Self::text_from_content);

        let Some(text) = text else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }

        match role {
            Some("user") | Some("human") => {
                transcript.add_message(Message::User { text, timestamp })
            }
            Some("assistant") | Some("ai") | Some("codebuddy") => {
                transcript.add_message(Message::Assistant { text, timestamp })
            }
            _ => {}
        }
    }

    fn text_from_content(value: &serde_json::Value) -> Option<String> {
        if let Some(text) = value.as_str() {
            return Some(text.to_string());
        }

        if let Some(array) = value.as_array() {
            let parts = array
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .or_else(|| item.get("content"))
                        .and_then(|value| value.as_str())
                })
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                return None;
            }
            return Some(parts.join("\n"));
        }

        value
            .get("content")
            .or_else(|| value.get("text"))
            .and_then(Self::text_from_content)
    }
}
