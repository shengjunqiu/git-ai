use super::*;

pub struct CursorPreset;

impl AgentCheckpointPreset for CursorPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        // Parse hook_input JSON to extract workspace_roots and conversation_id
        let hook_input_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for Cursor preset".to_string())
        })?;

        let hook_data: serde_json::Value = serde_json::from_str(&hook_input_json)
            .map_err(|e| GitAiError::PresetError(format!("Invalid JSON in hook_input: {}", e)))?;

        // Extract conversation_id and workspace_roots from the JSON
        let conversation_id = hook_data
            .get("conversation_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                GitAiError::PresetError("conversation_id not found in hook_input".to_string())
            })?
            .to_string();

        let workspace_roots = hook_data
            .get("workspace_roots")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                GitAiError::PresetError("workspace_roots not found in hook_input".to_string())
            })?
            .iter()
            .filter_map(|v| v.as_str().map(Self::normalize_cursor_path))
            .collect::<Vec<String>>();

        let hook_event_name = hook_data
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                GitAiError::PresetError("hook_event_name not found in hook_input".to_string())
            })?
            .to_string();

        // Extract model from hook input (Cursor provides this directly)
        let model = hook_data
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Legacy hooks no longer installed; exit silently for existing users who haven't reinstalled.
        if hook_event_name == "beforeSubmitPrompt" || hook_event_name == "afterFileEdit" {
            std::process::exit(0);
        }

        // Validate hook_event_name
        if hook_event_name != "preToolUse" && hook_event_name != "postToolUse" {
            return Err(GitAiError::PresetError(format!(
                "Invalid hook_event_name: {}. Expected 'preToolUse' or 'postToolUse'",
                hook_event_name
            )));
        }

        // Only checkpoint on file-mutating tools (Write, Delete, StrReplace)
        let tool_name = hook_data
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !matches!(tool_name, "Write" | "Delete" | "StrReplace") {
            return Err(GitAiError::PresetError(format!(
                "Skipping Cursor hook for non-edit tool_name '{}'.",
                tool_name
            )));
        }

        let file_path = hook_data
            .get("tool_input")
            .and_then(|ti| ti.get("file_path"))
            .and_then(|v| v.as_str())
            .map(Self::normalize_cursor_path)
            .unwrap_or_default();

        let repo_working_dir = Self::resolve_repo_working_dir(&file_path, &workspace_roots)
            .ok_or_else(|| {
                GitAiError::PresetError("No workspace root found in hook_input".to_string())
            })?;

        if hook_event_name == "preToolUse" {
            let will_edit = if !file_path.is_empty() {
                Some(vec![file_path.clone()])
            } else {
                None
            };

            // early return, we're just adding a human checkpoint.
            return Ok(AgentRunResult {
                agent_id: AgentId {
                    tool: "cursor".to_string(),
                    id: conversation_id.clone(),
                    model: model.clone(),
                },
                agent_metadata: None,
                checkpoint_kind: CheckpointKind::Human,
                transcript: None,
                repo_working_dir: Some(repo_working_dir),
                edited_filepaths: None,
                will_edit_filepaths: will_edit,
                dirty_files: None,
                captured_checkpoint_id: None,
            });
        }

        // Read transcript from JSONL file if available
        let transcript_path = hook_data
            .get("transcript_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let transcript = if let Some(ref tp) = transcript_path {
            match Self::transcript_and_model_from_cursor_jsonl(tp) {
                Ok((transcript, _)) => transcript,
                Err(e) => {
                    eprintln!(
                        "[Warning] Failed to parse Cursor JSONL at {}: {}. Will retry at commit.",
                        tp, e
                    );
                    AiTranscript::new()
                }
            }
        } else {
            eprintln!("[Warning] No transcript_path in Cursor hook input. Will retry at commit.");
            AiTranscript::new()
        };

        let edited_filepaths = if !file_path.is_empty() {
            Some(vec![file_path.to_string()])
        } else {
            None
        };

        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: conversation_id,
            model,
        };

        // Store transcript_path in metadata for re-reading at commit time
        let agent_metadata =
            transcript_path.map(|tp| HashMap::from([("transcript_path".to_string(), tp)]));

        Ok(AgentRunResult {
            agent_id,
            agent_metadata,
            checkpoint_kind: CheckpointKind::AiAgent,
            transcript: Some(transcript),
            repo_working_dir: Some(repo_working_dir),
            edited_filepaths,
            will_edit_filepaths: None,
            dirty_files: None,
            captured_checkpoint_id: None,
        })
    }
}

impl CursorPreset {
    fn matching_workspace_root(file_path: &str, workspace_roots: &[String]) -> Option<String> {
        workspace_roots
            .iter()
            .find(|root| {
                let root_str = root.as_str();
                file_path.starts_with(root_str)
                    && (file_path.len() == root_str.len()
                        || file_path[root_str.len()..].starts_with('/')
                        || file_path[root_str.len()..].starts_with('\\')
                        || root_str.ends_with('/')
                        || root_str.ends_with('\\'))
            })
            .cloned()
    }

    fn resolve_repo_working_dir(file_path: &str, workspace_roots: &[String]) -> Option<String> {
        if file_path.is_empty() {
            return workspace_roots.first().cloned();
        }

        let matched_workspace = Self::matching_workspace_root(file_path, workspace_roots)
            .or_else(|| workspace_roots.first().cloned())?;

        find_repository_for_file(file_path, Some(&matched_workspace))
            .ok()
            .and_then(|repo| repo.workdir().ok())
            .map(|path| path.to_string_lossy().to_string())
            .or(Some(matched_workspace))
    }

    /// Normalize Windows paths that Cursor sends in Unix-style format.
    ///
    /// On Windows, Cursor sometimes sends paths like `/c:/Users/...` instead of `C:\Users\...`.
    /// This function converts those paths to proper Windows format.
    #[cfg(windows)]
    fn normalize_cursor_path(path: &str) -> String {
        // Check for pattern like /c:/ or /C:/ at the start
        // e.g. "/c:/Users/foo" -> "C:\Users\foo"
        let mut chars = path.chars();
        if chars.next() == Some('/')
            && let (Some(drive), Some(':')) = (chars.next(), chars.next())
            && drive.is_ascii_alphabetic()
        {
            let rest: String = chars.collect();
            // Convert forward slashes to backslashes for Windows
            let normalized_rest = rest.replace('/', "\\");
            return format!("{}:{}", drive.to_ascii_uppercase(), normalized_rest);
        }
        // No conversion needed
        path.to_string()
    }

    #[cfg(not(windows))]
    fn normalize_cursor_path(path: &str) -> String {
        // On non-Windows platforms, no conversion needed
        path.to_string()
    }

    /// Parse a Cursor JSONL transcript file into a transcript.
    ///
    /// Cursor JSONL uses `role` (not `type`) at the top level, has no timestamps
    /// or model fields in entries, and wraps user text in `<user_query>` tags.
    /// Tool inputs use `path`/`contents` instead of `file_path`/`content`.
    pub fn transcript_and_model_from_cursor_jsonl(
        transcript_path: &str,
    ) -> Result<(AiTranscript, Option<String>), GitAiError> {
        let jsonl_content =
            std::fs::read_to_string(transcript_path).map_err(GitAiError::IoError)?;
        let mut transcript = AiTranscript::new();
        let mut plan_states = std::collections::HashMap::new();

        for line in jsonl_content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Skip malformed lines (file may be partially written)
            let raw_entry: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match raw_entry["role"].as_str() {
                Some("user") => {
                    if let Some(content_array) = raw_entry["message"]["content"].as_array() {
                        for item in content_array {
                            if item["type"].as_str() == Some("tool_result") {
                                continue;
                            }
                            if item["type"].as_str() == Some("text")
                                && let Some(text) = item["text"].as_str()
                            {
                                let cleaned = Self::strip_user_query_tags(text);
                                if !cleaned.is_empty() {
                                    transcript.add_message(Message::user(cleaned, None));
                                }
                            }
                        }
                    }
                }
                Some("assistant") => {
                    if let Some(content_array) = raw_entry["message"]["content"].as_array() {
                        for item in content_array {
                            match item["type"].as_str() {
                                Some("text") => {
                                    if let Some(text) = item["text"].as_str()
                                        && !text.trim().is_empty()
                                    {
                                        transcript.add_message(Message::assistant(
                                            text.to_string(),
                                            None,
                                        ));
                                    }
                                }
                                Some("thinking") => {
                                    if let Some(thinking) = item["thinking"].as_str()
                                        && !thinking.trim().is_empty()
                                    {
                                        transcript.add_message(Message::assistant(
                                            thinking.to_string(),
                                            None,
                                        ));
                                    }
                                }
                                Some("tool_use") => {
                                    if let Some(name) = item["name"].as_str() {
                                        let input = &item["input"];
                                        // Normalize tool input: Cursor uses `path` where git-ai uses `file_path`
                                        let normalized_input =
                                            Self::normalize_cursor_tool_input(name, input);

                                        // Check for plan file writes
                                        if let Some(plan_text) = extract_plan_from_tool_use(
                                            name,
                                            &normalized_input,
                                            &mut plan_states,
                                        ) {
                                            transcript.add_message(Message::Plan {
                                                text: plan_text,
                                                timestamp: None,
                                            });
                                        } else {
                                            // Apply same tool filtering as SQLite path
                                            Self::add_cursor_tool_message(
                                                &mut transcript,
                                                name,
                                                &normalized_input,
                                            );
                                        }
                                    }
                                }
                                _ => continue,
                            }
                        }
                    }
                }
                _ => continue,
            }
        }

        // Model is not in Cursor JSONL — it comes from hook input
        Ok((transcript, None))
    }

    /// Strip `<user_query>...</user_query>` wrapper tags from Cursor user messages.
    fn strip_user_query_tags(text: &str) -> String {
        let trimmed = text.trim();
        if let Some(inner) = trimmed
            .strip_prefix("<user_query>")
            .and_then(|s| s.strip_suffix("</user_query>"))
        {
            inner.trim().to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Normalize Cursor tool input field names to git-ai conventions.
    /// Cursor uses `path`/`contents` where git-ai uses `file_path`/`content`.
    fn normalize_cursor_tool_input(
        tool_name: &str,
        input: &serde_json::Value,
    ) -> serde_json::Value {
        let mut normalized = input.clone();
        if let Some(obj) = normalized.as_object_mut() {
            // Rename `path` → `file_path`
            if let Some(path_val) = obj.remove("path")
                && !obj.contains_key("file_path")
            {
                obj.insert("file_path".to_string(), path_val);
            }
            // For Write tool: rename `contents` → `content`
            if tool_name == "Write"
                && let Some(contents_val) = obj.remove("contents")
                && !obj.contains_key("content")
            {
                obj.insert("content".to_string(), contents_val);
            }
        }
        normalized
    }

    /// Add a tool_use message to the transcript. Edit tools store only
    /// file_path (content is too large); everything else keeps full args.
    fn add_cursor_tool_message(
        transcript: &mut AiTranscript,
        tool_name: &str,
        normalized_input: &serde_json::Value,
    ) {
        match tool_name {
            // Edit tools: store only file_path (content is too large)
            "Write"
            | "Edit"
            | "StrReplace"
            | "Delete"
            | "MultiEdit"
            | "edit_file"
            | "apply_patch"
            | "edit_file_v2_apply_patch"
            | "search_replace"
            | "edit_file_v2_search_replace" => {
                let file_path = normalized_input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .or_else(|| normalized_input.get("target_file").and_then(|v| v.as_str()));
                transcript.add_message(Message::tool_use(
                    tool_name.to_string(),
                    serde_json::json!({ "file_path": file_path.unwrap_or("") }),
                ));
            }
            // Everything else: store full args
            _ => {
                transcript.add_message(Message::tool_use(
                    tool_name.to_string(),
                    normalized_input.clone(),
                ));
            }
        }
    }
}
