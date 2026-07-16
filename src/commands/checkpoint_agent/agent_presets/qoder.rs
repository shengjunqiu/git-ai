use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

pub struct QoderPreset;

impl AgentCheckpointPreset for QoderPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        let stdin_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for Qoder preset".to_string())
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
            Self::string_at(&hook_data, &["tool_use_id", "toolUseId"]).unwrap_or("qoder-tool");

        let (transcript, transcript_model) = match transcript_path {
            Some(path) if Path::new(path).exists() => {
                match Self::transcript_and_model_from_qoder_path(path) {
                    Ok((transcript, model)) => (transcript, model),
                    Err(e) => {
                        eprintln!("[Warning] Failed to parse Qoder transcript: {e}");
                        log_error(
                            &e,
                            Some(serde_json::json!({
                                "agent_tool": "qoder",
                                "operation": "transcript_and_model_from_qoder_path"
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
            .or_else(|| match Self::model_from_qoder_storage(session_id) {
                Ok(model) => model,
                Err(e) => {
                    tracing::debug!(
                        "Failed to resolve Qoder model for session {} from storage: {}",
                        session_id,
                        e
                    );
                    log_error(
                        &e,
                        Some(serde_json::json!({
                            "agent_tool": "qoder",
                            "operation": "model_from_qoder_storage"
                        })),
                    );
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        let agent_id = AgentId {
            tool: "qoder".to_string(),
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
            .map(|name| bash_tool::classify_tool(Agent::Qoder, name))
            .unwrap_or_else(|| {
                if target_filepaths.is_some() {
                    ToolClass::FileEdit
                } else {
                    ToolClass::Skip
                }
            });
        let is_bash_tool = tool_class == ToolClass::Bash;
        let tool_use_id = if is_bash_tool && tool_use_id == "qoder-tool" {
            // Native Qoder payloads do not always include a unique tool_use_id.
            // The bash handler correlates this fallback through its per-session sidecar.
            "bash"
        } else {
            tool_use_id
        };

        if Self::is_pre_tool_use(hook_event_name) {
            if tool_class == ToolClass::Skip {
                return Err(GitAiError::PresetError(
                    "Skipping Qoder PreToolUse without mutating tool/path".to_string(),
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
                "Skipping unsupported Qoder hook event: {}",
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
                    tracing::debug!("Qoder bash tool post-hook error: {}", e);
                    target_filepaths
                }
            }
        } else {
            target_filepaths
        };

        if !is_bash_tool && edited_filepaths.is_none() && dirty_files.is_none() {
            return Err(GitAiError::PresetError(
                "Skipping Qoder PostToolUse without edited path or dirty file content".to_string(),
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

impl QoderPreset {
    pub fn model_from_qoder_storage(session_id: &str) -> Result<Option<String>, GitAiError> {
        let mut first_error = None;
        for user_dir in Self::qoder_user_dirs() {
            match Self::model_from_qoder_user_dir(session_id, &user_dir) {
                Ok(Some(model)) => return Ok(Some(model)),
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(
                        "Failed to resolve Qoder model from user directory {:?}: {}",
                        user_dir,
                        error
                    );
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    pub fn model_from_qoder_user_dir(
        session_id: &str,
        user_dir: &Path,
    ) -> Result<Option<String>, GitAiError> {
        let Some(selected_model) = Self::selected_model_from_qoder_workspace_storage(
            session_id,
            &user_dir.join("workspaceStorage"),
        )?
        else {
            return Ok(None);
        };

        Self::resolve_qoder_model_name(&selected_model, &user_dir.join("globalStorage"))
    }

    pub fn transcript_and_model_from_qoder_path(
        transcript_path: &str,
    ) -> Result<(AiTranscript, Option<String>), GitAiError> {
        let content = std::fs::read_to_string(transcript_path).map_err(GitAiError::IoError)?;
        let mut transcript = AiTranscript::new();
        let mut model = None;

        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
            Self::walk_transcript_value(&value, &mut transcript, &mut model);
            return Ok((transcript, model));
        }

        for line in content.lines().filter(|line| !line.trim().is_empty()) {
            let value: serde_json::Value = serde_json::from_str(line)?;
            Self::walk_transcript_value(&value, &mut transcript, &mut model);
        }

        Ok((transcript, model))
    }

    fn string_at<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
        keys.iter().find_map(|key| value.get(*key)?.as_str())
    }

    fn qoder_user_dirs() -> Vec<PathBuf> {
        if let Ok(path) = std::env::var("GIT_AI_QODER_USER_DIR")
            && !path.trim().is_empty()
        {
            return vec![PathBuf::from(path)];
        }

        dirs::config_dir()
            .map(|config| Self::qoder_user_dirs_from_config(&config))
            .unwrap_or_default()
    }

    fn qoder_user_dirs_from_config(config_dir: &Path) -> Vec<PathBuf> {
        ["Qoder", "QoderCN"]
            .iter()
            .map(|product| config_dir.join(product).join("User"))
            .collect()
    }

    fn open_sqlite_readonly(path: &Path) -> Result<Connection, GitAiError> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| GitAiError::Generic(format!("Failed to open {:?}: {}", path, e)))?;

        let _ = conn.execute_batch("PRAGMA cache_size = -2000;");

        Ok(conn)
    }

    fn selected_model_from_qoder_workspace_storage(
        session_id: &str,
        workspace_storage_dir: &Path,
    ) -> Result<Option<String>, GitAiError> {
        if !workspace_storage_dir.exists() {
            return Ok(None);
        }

        let keys = [
            format!("chat.modelMapSession.{session_id}"),
            format!("chat.modelConfig.session.{session_id}"),
        ];
        let entries = std::fs::read_dir(workspace_storage_dir).map_err(GitAiError::IoError)?;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::debug!("Failed to read Qoder workspace storage entry: {}", e);
                    continue;
                }
            };
            let db_path = entry.path().join("state.vscdb");
            if !db_path.exists() {
                continue;
            }

            for key in &keys {
                match Self::read_qoder_storage_value(&db_path, key) {
                    Ok(Some(model)) if !model.trim().is_empty() => return Ok(Some(model)),
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!(
                            "Failed to read Qoder workspace model from {:?}: {}",
                            db_path,
                            e
                        );
                    }
                }
            }
        }

        Ok(None)
    }

    fn resolve_qoder_model_name(
        selected_model: &str,
        global_storage_dir: &Path,
    ) -> Result<Option<String>, GitAiError> {
        let selected_model = selected_model.trim();
        if selected_model.is_empty() {
            return Ok(None);
        }

        let global_db = global_storage_dir.join("state.vscdb");
        if !global_db.exists() {
            return Ok(Some(selected_model.to_string()));
        }

        if let Some(custom_model_id) = selected_model.strip_prefix("custom:") {
            if let Some(models_json) =
                Self::read_qoder_storage_value(&global_db, "aicoding.customModels")?
                && let Some(model) =
                    Self::model_name_from_qoder_custom_models(&models_json, custom_model_id)
            {
                return Ok(Some(model));
            }

            return Ok(Some(custom_model_id.to_string()));
        }

        for key in [
            "aicoding.modelConfigs.cache.assistant",
            "aicoding.modelConfigs.cache.quest",
            "aicoding.modelConfigs.cache.experts",
        ] {
            if let Some(models_json) = Self::read_qoder_storage_value(&global_db, key)?
                && let Some(model) =
                    Self::model_name_from_qoder_model_configs(&models_json, selected_model)
            {
                return Ok(Some(model));
            }
        }

        Ok(Some(selected_model.to_string()))
    }

    fn read_qoder_storage_value(db_path: &Path, key: &str) -> Result<Option<String>, GitAiError> {
        let conn = Self::open_sqlite_readonly(db_path)?;
        let mut stmt = conn
            .prepare("SELECT value FROM ItemTable WHERE key = ?1")
            .map_err(GitAiError::SqliteError)?;
        stmt.query_row([key], |row| row.get::<_, String>(0))
            .optional()
            .map_err(GitAiError::SqliteError)
    }

    fn model_name_from_qoder_custom_models(models_json: &str, model_id: &str) -> Option<String> {
        let models: serde_json::Value = serde_json::from_str(models_json).ok()?;
        models.as_array()?.iter().find_map(|model| {
            let id = Self::string_at(model, &["id"])?;
            if id != model_id {
                return None;
            }

            Self::string_at(model, &["displayName", "model", "name"])
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
        })
    }

    fn model_name_from_qoder_model_configs(
        models_json: &str,
        selected_model: &str,
    ) -> Option<String> {
        let models: serde_json::Value = serde_json::from_str(models_json).ok()?;
        models.as_array()?.iter().find_map(|model| {
            let name = Self::string_at(model, &["name"])?;
            if name != selected_model {
                return None;
            }

            Self::string_at(model, &["displayName", "name"])
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
        })
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
            if let Some(values) = hook_data.get(key).and_then(|value| value.as_array()) {
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
                    if let Some(path) = parent.get(key).and_then(|value| value.as_str()) {
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
            Some("Write") | Some("write") | Some("create_file")
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
        if path.trim().is_empty() || paths.iter().any(|existing| existing == path) {
            return;
        }
        paths.push(path.to_string());
    }

    fn walk_transcript_value(
        value: &serde_json::Value,
        transcript: &mut AiTranscript,
        model: &mut Option<String>,
    ) {
        if model.is_none() {
            *model = Self::model_from_value(value);
        }

        Self::add_message_from_value(value, transcript);

        match value {
            serde_json::Value::Array(items) => {
                for item in items {
                    Self::walk_transcript_value(item, transcript, model);
                }
            }
            serde_json::Value::Object(map) => {
                for key in ["messages", "conversation", "entries"] {
                    if let Some(child) = map.get(key) {
                        Self::walk_transcript_value(child, transcript, model);
                    }
                }
                if let Some(message) = map.get("message")
                    && !message.is_string()
                {
                    Self::walk_transcript_value(message, transcript, model);
                }
            }
            _ => {}
        }
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

    fn add_message_from_value(value: &serde_json::Value, transcript: &mut AiTranscript) {
        let timestamp =
            Self::string_at(value, &["timestamp", "created_at", "createdAt"]).map(str::to_string);

        let role = Self::string_at(value, &["role", "type", "sender"]);
        let text = value
            .get("content")
            .or_else(|| value.get("text"))
            .or_else(|| value.get("message"))
            .and_then(Self::text_from_content);

        if let Some(text) = text
            && !text.trim().is_empty()
        {
            match role {
                Some("user") | Some("human") => transcript.add_message(Message::User {
                    text,
                    timestamp: timestamp.clone(),
                }),
                Some("assistant") | Some("ai") | Some("qoder") => {
                    transcript.add_message(Message::Assistant {
                        text,
                        timestamp: timestamp.clone(),
                    })
                }
                _ => {}
            }
        }

        if let Some(tool_name) = Self::string_at(value, &["tool_name", "toolName", "name"])
            && matches!(role, Some("tool_use") | Some("tool"))
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
                        .and_then(Self::text_from_content)
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

#[cfg(test)]
mod tests {
    use super::QoderPreset;
    use std::path::{Path, PathBuf};

    #[test]
    fn qoder_user_dirs_cover_international_and_cn_products() {
        assert_eq!(
            QoderPreset::qoder_user_dirs_from_config(Path::new("/config")),
            vec![
                PathBuf::from("/config/Qoder/User"),
                PathBuf::from("/config/QoderCN/User")
            ]
        );
    }
}
