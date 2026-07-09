use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

pub struct TraePreset;

impl AgentCheckpointPreset for TraePreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        let stdin_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for Trae preset".to_string())
        })?;

        let hook_data: serde_json::Value = serde_json::from_str(&stdin_json)
            .map_err(|e| GitAiError::PresetError(format!("Invalid JSON in hook_input: {}", e)))?;

        let session_id =
            Self::string_at(&hook_data, &["session_id", "sessionId"]).ok_or_else(|| {
                GitAiError::PresetError("session_id not found in hook_input".to_string())
            })?;
        let cwd = Self::string_at(&hook_data, &["cwd"])
            .ok_or_else(|| GitAiError::PresetError("cwd not found in hook_input".to_string()))?;
        let hook_event_name = Self::string_at(&hook_data, &["hook_event_name", "hookEventName"]);
        let tool_name = Self::string_at(&hook_data, &["tool_name", "toolName"]);
        let tool_use_id =
            Self::string_at(&hook_data, &["tool_use_id", "toolUseId"]).unwrap_or("trae-tool");

        let explicit_filepaths = Self::filepaths_from_hook_data(&hook_data);
        let dirty_files = Self::dirty_files_from_hook_data(&hook_data, explicit_filepaths.as_ref());
        let dirty_filepaths = dirty_files.as_ref().map(|files| {
            let mut paths = files.keys().cloned().collect::<Vec<_>>();
            paths.sort();
            paths
        });
        let target_filepaths = explicit_filepaths.clone().or(dirty_filepaths);

        let tool_class = tool_name
            .map(|name| bash_tool::classify_tool(Agent::Trae, name))
            .unwrap_or_else(|| {
                if target_filepaths.is_some() {
                    ToolClass::FileEdit
                } else {
                    ToolClass::Skip
                }
            });
        let is_bash_tool = tool_class == ToolClass::Bash;

        let model = Self::model_from_value(&hook_data)
            .or_else(
                || match Self::model_from_trae_storage(session_id, Some(cwd)) {
                    Ok(model) => model,
                    Err(e) => {
                        tracing::debug!(
                            "Failed to resolve Trae model for session {} from storage: {}",
                            session_id,
                            e
                        );
                        log_error(
                            &e,
                            Some(serde_json::json!({
                                "agent_tool": "trae",
                                "operation": "model_from_trae_storage"
                            })),
                        );
                        None
                    }
                },
            )
            .unwrap_or_else(|| "unknown".to_string());

        let agent_id = AgentId {
            tool: "trae".to_string(),
            id: session_id.to_string(),
            model,
        };

        if Self::is_pre_tool_use(hook_event_name) {
            if tool_class == ToolClass::Skip {
                return Err(GitAiError::PresetError(
                    "Skipping Trae PreToolUse without mutating tool/path".to_string(),
                ));
            }

            let pre_hook_captured_id = prepare_agent_bash_pre_hook(
                is_bash_tool,
                Some(cwd),
                session_id,
                tool_use_id,
                &agent_id,
                None,
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
                "Skipping unsupported Trae hook event: {}",
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
                    tracing::debug!("Trae bash tool post-hook error: {}", e);
                    target_filepaths
                }
            }
        } else {
            target_filepaths
        };

        if edited_filepaths.is_none() && dirty_files.is_none() {
            return Err(GitAiError::PresetError(
                "Skipping Trae PostToolUse without edited path or dirty file content".to_string(),
            ));
        }

        let bash_captured_checkpoint_id = bash_result
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|r| r.captured_checkpoint.as_ref())
            .map(|info| info.capture_id.clone());

        Ok(AgentRunResult {
            agent_id,
            agent_metadata: None,
            checkpoint_kind: CheckpointKind::AiAgent,
            transcript: Some(AiTranscript::new()),
            repo_working_dir: Some(cwd.to_string()),
            edited_filepaths,
            will_edit_filepaths: None,
            dirty_files,
            captured_checkpoint_id: bash_captured_checkpoint_id,
        })
    }
}

impl TraePreset {
    pub fn model_from_trae_storage(
        session_id: &str,
        cwd: Option<&str>,
    ) -> Result<Option<String>, GitAiError> {
        let cwd = cwd.map(Path::new);

        for user_dir in Self::trae_user_dirs() {
            if let Some(model) = Self::model_from_trae_user_dir(session_id, cwd, &user_dir)? {
                return Ok(Some(model));
            }
        }

        Ok(None)
    }

    pub fn model_from_trae_user_dir(
        session_id: &str,
        cwd: Option<&Path>,
        user_dir: &Path,
    ) -> Result<Option<String>, GitAiError> {
        if let Some(model) = Self::model_from_trae_workspace_storage(
            session_id,
            cwd,
            &user_dir.join("workspaceStorage"),
            user_dir,
        )? {
            return Ok(Some(model));
        }

        Self::model_from_trae_global_storage(user_dir)
    }

    fn string_at<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
        keys.iter().find_map(|key| value.get(*key)?.as_str())
    }

    fn trae_user_dirs() -> Vec<PathBuf> {
        if let Ok(path) = env::var("GIT_AI_TRAE_USER_DIR")
            && !path.trim().is_empty()
        {
            return vec![PathBuf::from(path)];
        }

        let Some(config_dir) = dirs::config_dir() else {
            return Vec::new();
        };

        vec![
            config_dir.join("Trae").join("User"),
            config_dir.join("Trae CN").join("User"),
        ]
    }

    fn model_from_value(value: &serde_json::Value) -> Option<String> {
        for key in [
            "model",
            "model_name",
            "modelName",
            "model_id",
            "modelId",
            "selected_model",
            "selectedModel",
        ] {
            if let Some(model) = value.get(key).and_then(Self::model_from_model_value) {
                return Some(model);
            }
        }

        for parent_key in [
            "model_config",
            "modelConfig",
            "tool_input",
            "toolInput",
            "message",
        ] {
            if let Some(parent) = value.get(parent_key)
                && let Some(model) = Self::model_from_value(parent)
            {
                return Some(model);
            }
        }

        None
    }

    fn model_from_model_value(value: &serde_json::Value) -> Option<String> {
        if let Some(model) = value.as_str() {
            return Self::normalize_model_id(model);
        }

        if let Some(array) = value.as_array() {
            return array.iter().find_map(Self::model_from_model_value);
        }

        for key in [
            "display_name",
            "displayName",
            "name",
            "model",
            "model_name",
            "modelName",
            "id",
        ] {
            if let Some(model) = value.get(key).and_then(|value| value.as_str())
                && let Some(model) = Self::normalize_model_id(model)
            {
                return Some(model);
            }
        }

        None
    }

    fn normalize_model_id(model: &str) -> Option<String> {
        let trimmed = model.trim().trim_matches('"');
        if trimmed.is_empty() {
            return None;
        }

        let normalized = match trimmed.rsplit_once("_-_") {
            Some((_, suffix)) if !suffix.trim().is_empty() => suffix.trim(),
            _ => trimmed,
        };

        if normalized.is_empty() {
            None
        } else {
            Some(normalized.to_string())
        }
    }

    fn model_from_trae_workspace_storage(
        session_id: &str,
        cwd: Option<&Path>,
        workspace_storage_dir: &Path,
        user_dir: &Path,
    ) -> Result<Option<String>, GitAiError> {
        if !workspace_storage_dir.exists() {
            return Ok(None);
        }

        let mut cwd_matched_dbs = Vec::new();
        let entries = std::fs::read_dir(workspace_storage_dir).map_err(GitAiError::IoError)?;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::debug!("Failed to read Trae workspace storage entry: {}", e);
                    continue;
                }
            };
            let db_path = entry.path().join("state.vscdb");
            if !db_path.exists() {
                continue;
            }

            if let Some(selected_model) =
                Self::selected_model_from_trae_workspace_db(&db_path, session_id)?
                && let Some(model) = Self::resolve_trae_model_name(&selected_model, user_dir)?
            {
                return Ok(Some(model));
            }

            if let Some(cwd) = cwd {
                match Self::workspace_db_matches_cwd(&db_path, cwd) {
                    Ok(true) => cwd_matched_dbs.push(db_path),
                    Ok(false) => {}
                    Err(e) => {
                        tracing::debug!("Failed to match Trae workspace db {:?}: {}", db_path, e);
                    }
                }
            }
        }

        for db_path in cwd_matched_dbs {
            if let Some(current_session_id) = Self::current_session_id_from_workspace_db(&db_path)?
                && let Some(selected_model) =
                    Self::selected_model_from_trae_workspace_db(&db_path, &current_session_id)?
                && let Some(model) = Self::resolve_trae_model_name(&selected_model, user_dir)?
            {
                return Ok(Some(model));
            }

            if let Some(selected_model) = Self::global_model_from_trae_db(&db_path)?
                && let Some(model) = Self::resolve_trae_model_name(&selected_model, user_dir)?
            {
                return Ok(Some(model));
            }
        }

        Ok(None)
    }

    fn model_from_trae_global_storage(user_dir: &Path) -> Result<Option<String>, GitAiError> {
        let global_db = user_dir.join("globalStorage").join("state.vscdb");
        if !global_db.exists() {
            return Ok(None);
        }

        if let Some(selected_model) = Self::global_model_from_trae_db(&global_db)?
            && let Some(model) = Self::resolve_trae_model_name(&selected_model, user_dir)?
        {
            return Ok(Some(model));
        }

        for pattern in ["%AI.agent.model.selected_model", "AI.agent.model.v1"] {
            for model_json in Self::read_trae_storage_values_like(&global_db, pattern)? {
                if let Ok(model_value) = serde_json::from_str::<serde_json::Value>(&model_json)
                    && let Some(model) = Self::model_from_model_value(&model_value)
                {
                    return Ok(Some(model));
                }

                if let Some(model) = Self::normalize_model_id(&model_json) {
                    return Ok(Some(model));
                }
            }
        }

        Ok(None)
    }

    fn selected_model_from_trae_workspace_db(
        db_path: &Path,
        session_id: &str,
    ) -> Result<Option<String>, GitAiError> {
        let preferred_role = Self::session_agent_from_workspace_db(db_path, session_id)?;

        for model_map_json in
            Self::read_trae_storage_values_like(db_path, "%ai-chat:sessionRelation:modelMap")?
        {
            let Ok(model_map) = serde_json::from_str::<serde_json::Value>(&model_map_json) else {
                continue;
            };
            let Some(session_model_map) = model_map.get(session_id) else {
                continue;
            };

            if let Some(model) =
                Self::model_from_role_map(session_model_map, preferred_role.as_deref())
            {
                return Ok(Some(model));
            }
        }

        Ok(None)
    }

    fn global_model_from_trae_db(db_path: &Path) -> Result<Option<String>, GitAiError> {
        for global_model_map_json in
            Self::read_trae_storage_values_like(db_path, "%ai-chat:sessionRelation:globalModelMap")?
        {
            let Ok(global_model_map) =
                serde_json::from_str::<serde_json::Value>(&global_model_map_json)
            else {
                continue;
            };
            if let Some(model) = Self::model_from_role_map(&global_model_map, None) {
                return Ok(Some(model));
            }
        }

        Ok(None)
    }

    fn model_from_role_map(
        role_map: &serde_json::Value,
        preferred_role: Option<&str>,
    ) -> Option<String> {
        let role_map = role_map.as_object()?;
        let mut roles = Vec::new();

        if let Some(role) = preferred_role {
            Self::push_role(&mut roles, role);
            match role {
                "solo_agent" => Self::push_role(&mut roles, "solo_coder"),
                "solo_coder" => Self::push_role(&mut roles, "solo_agent"),
                _ => {}
            }
        }

        for role in [
            "solo_coder",
            "solo_agent",
            "dev_builder",
            "solo_builder",
            "builder",
            "chat",
        ] {
            Self::push_role(&mut roles, role);
        }

        for role in roles {
            if let Some(model) = role_map.get(&role).and_then(Self::model_from_model_value) {
                return Some(model);
            }
        }

        role_map.values().find_map(Self::model_from_model_value)
    }

    fn push_role(roles: &mut Vec<String>, role: &str) {
        if !role.trim().is_empty() && !roles.iter().any(|existing| existing == role) {
            roles.push(role.to_string());
        }
    }

    fn session_agent_from_workspace_db(
        db_path: &Path,
        session_id: &str,
    ) -> Result<Option<String>, GitAiError> {
        let Some(agent_map_json) =
            Self::read_trae_storage_value(db_path, "icube_session_agent_map")?
        else {
            return Ok(None);
        };

        let agent_map: serde_json::Value =
            serde_json::from_str(&agent_map_json).map_err(GitAiError::JsonError)?;
        Ok(agent_map
            .get(session_id)
            .and_then(|value| value.as_str())
            .map(str::to_string))
    }

    fn current_session_id_from_workspace_db(db_path: &Path) -> Result<Option<String>, GitAiError> {
        let Some(session_storage_json) =
            Self::read_trae_storage_value(db_path, "memento/icube-ai-agent-storage")?
        else {
            return Ok(None);
        };

        let session_storage: serde_json::Value =
            serde_json::from_str(&session_storage_json).map_err(GitAiError::JsonError)?;
        if let Some(session_id) = Self::string_at(&session_storage, &["currentSessionId"]) {
            return Ok(Some(session_id.to_string()));
        }

        let current_session = session_storage
            .get("list")
            .and_then(|value| value.as_array())
            .and_then(|sessions| {
                sessions.iter().find(|session| {
                    session
                        .get("isCurrent")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                })
            })
            .and_then(|session| Self::string_at(session, &["sessionId", "session_id"]))
            .map(str::to_string);

        Ok(current_session)
    }

    fn workspace_db_matches_cwd(db_path: &Path, cwd: &Path) -> Result<bool, GitAiError> {
        let cwd = cwd.to_string_lossy();
        if cwd.trim().is_empty() {
            return Ok(false);
        }

        let conn = Self::open_sqlite_readonly(db_path)?;
        let mut stmt = conn
            .prepare("SELECT 1 FROM ItemTable WHERE value LIKE ?1 LIMIT 1")
            .map_err(GitAiError::SqliteError)?;
        let pattern = format!("%{}%", cwd);
        let result = stmt
            .query_row([pattern], |row| row.get::<_, i64>(0))
            .optional()
            .map_err(GitAiError::SqliteError)?;

        Ok(result.is_some())
    }

    fn resolve_trae_model_name(
        selected_model: &str,
        user_dir: &Path,
    ) -> Result<Option<String>, GitAiError> {
        let Some(selected_model) = Self::normalize_model_id(selected_model) else {
            return Ok(None);
        };

        let global_db = user_dir.join("globalStorage").join("state.vscdb");
        if !global_db.exists() {
            return Ok(Some(selected_model));
        }

        for pattern in [
            "%AI.agent.model.model_list_map",
            "%AI.agent.model.model_list",
            "%AI.agent.modelList%",
        ] {
            for models_json in Self::read_trae_storage_values_like(&global_db, pattern)? {
                if let Some(model) =
                    Self::model_name_from_trae_model_configs(&models_json, &selected_model)
                {
                    return Ok(Some(model));
                }
            }
        }

        Ok(Some(selected_model))
    }

    fn model_name_from_trae_model_configs(
        models_json: &str,
        selected_model: &str,
    ) -> Option<String> {
        let models: serde_json::Value = serde_json::from_str(models_json).ok()?;
        Self::model_name_from_trae_model_config_value(&models, selected_model)
    }

    fn model_name_from_trae_model_config_value(
        value: &serde_json::Value,
        selected_model: &str,
    ) -> Option<String> {
        match value {
            serde_json::Value::Array(items) => items.iter().find_map(|item| {
                Self::model_name_from_trae_model_config_value(item, selected_model)
            }),
            serde_json::Value::Object(map) => {
                let matched = ["name", "model", "model_name", "modelName", "id"]
                    .iter()
                    .filter_map(|key| map.get(*key).and_then(|value| value.as_str()))
                    .filter_map(Self::normalize_model_id)
                    .any(|model| model == selected_model);

                if matched
                    && let Some(display_name) =
                        Self::string_at(value, &["display_name", "displayName", "name", "model"])
                            .and_then(Self::normalize_model_id)
                {
                    return Some(display_name);
                }

                map.values().find_map(|child| {
                    Self::model_name_from_trae_model_config_value(child, selected_model)
                })
            }
            _ => None,
        }
    }

    fn open_sqlite_readonly(path: &Path) -> Result<Connection, GitAiError> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| GitAiError::Generic(format!("Failed to open {:?}: {}", path, e)))?;

        let _ = conn.execute_batch("PRAGMA cache_size = -2000;");

        Ok(conn)
    }

    fn read_trae_storage_value(db_path: &Path, key: &str) -> Result<Option<String>, GitAiError> {
        let conn = Self::open_sqlite_readonly(db_path)?;
        let mut stmt = conn
            .prepare("SELECT value FROM ItemTable WHERE key = ?1")
            .map_err(GitAiError::SqliteError)?;
        stmt.query_row([key], |row| row.get::<_, String>(0))
            .optional()
            .map_err(GitAiError::SqliteError)
    }

    fn read_trae_storage_values_like(
        db_path: &Path,
        key_like: &str,
    ) -> Result<Vec<String>, GitAiError> {
        let conn = Self::open_sqlite_readonly(db_path)?;
        let mut stmt = conn
            .prepare("SELECT value FROM ItemTable WHERE key LIKE ?1 ORDER BY key")
            .map_err(GitAiError::SqliteError)?;
        let values = stmt
            .query_map([key_like], |row| row.get::<_, String>(0))
            .map_err(GitAiError::SqliteError)?;

        let mut results = Vec::new();
        for value in values {
            results.push(value.map_err(GitAiError::SqliteError)?);
        }

        Ok(results)
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
        if !matches!(tool_name, Some("Write") | Some("write")) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    fn create_storage_db(path: &Path, rows: &[(&str, serde_json::Value)]) {
        let conn = Connection::open(path).expect("sqlite db should be created");
        conn.execute(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .expect("ItemTable should be created");

        for (key, value) in rows {
            conn.execute(
                "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                [*key, &value.to_string()],
            )
            .expect("storage row should be inserted");
        }
    }

    #[test]
    fn test_model_from_value_prefers_explicit_model() {
        let hook_data = json!({
            "session_id": "session-1",
            "cwd": "/repo",
            "model_name": "1_-_Dola-Seed-2.0-Code"
        });

        assert_eq!(
            TraePreset::model_from_value(&hook_data),
            Some("Dola-Seed-2.0-Code".to_string())
        );
    }

    #[test]
    fn test_model_from_trae_user_dir_reads_session_model_map() {
        let temp = tempfile::tempdir().unwrap();
        let user_dir = temp.path().join("User");
        let workspace_dir = user_dir.join("workspaceStorage").join("workspace-1");
        let global_dir = user_dir.join("globalStorage");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::create_dir_all(&global_dir).unwrap();

        create_storage_db(
            &workspace_dir.join("state.vscdb"),
            &[
                (
                    "icube_session_agent_map",
                    json!({ "session-1": "solo_agent" }),
                ),
                (
                    "7467860964935730184_ai-chat:sessionRelation:modelMap",
                    json!({
                        "session-1": {
                            "solo_coder": "1_-_Dola-Seed-2.0-Code",
                            "dev_builder": "_-_gpt-5"
                        }
                    }),
                ),
            ],
        );
        create_storage_db(
            &global_dir.join("state.vscdb"),
            &[(
                "7467860964935730184_AI.agent.model.model_list_map",
                json!({
                    "solo_agent": [
                        {
                            "name": "Dola-Seed-2.0-Code",
                            "display_name": "Dola-Seed-2.0-Code"
                        }
                    ]
                }),
            )],
        );

        let model = TraePreset::model_from_trae_user_dir("session-1", None, &user_dir)
            .expect("model lookup should not fail");

        assert_eq!(model, Some("Dola-Seed-2.0-Code".to_string()));
    }

    #[test]
    fn test_model_from_trae_user_dir_falls_back_to_current_workspace_session() {
        let temp = tempfile::tempdir().unwrap();
        let user_dir = temp.path().join("User");
        let workspace_dir = user_dir.join("workspaceStorage").join("workspace-1");
        let global_dir = user_dir.join("globalStorage");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::create_dir_all(&global_dir).unwrap();

        create_storage_db(
            &workspace_dir.join("state.vscdb"),
            &[
                (
                    "memento/icube-ai-agent-storage",
                    json!({
                        "list": [
                            {
                                "isCurrent": true,
                                "sessionId": "real-session",
                                "messages": []
                            }
                        ],
                        "currentSessionId": "real-session",
                        "workspace": "/repo/project"
                    }),
                ),
                (
                    "7467860964935730184_ai-chat:sessionRelation:modelMap",
                    json!({
                        "real-session": {
                            "solo_coder": "1_-_Dola-Seed-2.0-Code"
                        }
                    }),
                ),
            ],
        );
        create_storage_db(
            &global_dir.join("state.vscdb"),
            &[(
                "7467860964935730184_AI.agent.model.model_list_map",
                json!({
                    "solo_agent": [
                        {
                            "name": "Dola-Seed-2.0-Code",
                            "display_name": "Dola-Seed-2.0-Code"
                        }
                    ]
                }),
            )],
        );

        let model = TraePreset::model_from_trae_user_dir(
            "manual-session",
            Some(Path::new("/repo/project")),
            &user_dir,
        )
        .expect("model lookup should not fail");

        assert_eq!(model, Some("Dola-Seed-2.0-Code".to_string()));
    }

    #[test]
    fn test_model_from_trae_user_dir_falls_back_to_global_selected_model() {
        let temp = tempfile::tempdir().unwrap();
        let user_dir = temp.path().join("User");
        let global_dir = user_dir.join("globalStorage");
        std::fs::create_dir_all(&global_dir).unwrap();

        create_storage_db(
            &global_dir.join("state.vscdb"),
            &[(
                "7467860964935730184_AI.agent.model.selected_model",
                json!({
                    "name": "gpt-5",
                    "display_name": "GPT-5"
                }),
            )],
        );

        let model = TraePreset::model_from_trae_user_dir("missing-session", None, &user_dir)
            .expect("model lookup should not fail");

        assert_eq!(model, Some("GPT-5".to_string()));
    }
}
