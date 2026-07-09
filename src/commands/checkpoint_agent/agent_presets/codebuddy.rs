// CodeBuddy 智能体（agent）的 checkpoint 预设实现。
//
// 该文件实现了 `AgentCheckpointPreset` trait，使 git-ai 能在 CodeBuddy 的
// PreToolUse / PostToolUse hook 中捕获“人类 / AI 代理”对代码库的改动，
// 并生成对应的 checkpoint（检查点），用于后续的行级 AI/人类归属统计。
//
// 核心流程：
//   1. 从 hook 传入的 JSON（stdin）中解析会话、工作目录、工具名等信息；
//   2. 解析 CodeBuddy 的 JSONL 转写文件以还原对话 transcript 与模型名；
//   3. 根据 hook 事件类型（Pre/Post）分别生成「人类检查点」或「AI 代理检查点」。
use super::*;

/// CodeBuddy 预设的空结构体。所有能力通过 `AgentCheckpointPreset` trait 提供，
/// 本身不持有状态，仅作为类型标识。
pub struct CodeBuddyPreset;

impl AgentCheckpointPreset for CodeBuddyPreset {
    /// 预设入口：根据 hook 输入执行一次 checkpoint 捕获。
    ///
    /// `flags.hook_input` 为 CodeBuddy 通过 stdin 传入的 JSON 字符串，
    /// 包含 session_id、cwd、tool_name 等上下文。
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError> {
        // 1) 必须提供 hook_input（来自 stdin 的 JSON），否则无法继续。
        let stdin_json = flags.hook_input.ok_or_else(|| {
            GitAiError::PresetError("hook_input is required for CodeBuddy preset".to_string())
        })?;

        // 2) 将 JSON 字符串解析为通用的 serde_json::Value，方便按字段取值。
        let hook_data: serde_json::Value = serde_json::from_str(&stdin_json)
            .map_err(|e| GitAiError::PresetError(format!("Invalid JSON in hook_input: {}", e)))?;

        // 3) 提取会话 ID：兼容 snake_case（session_id）与 camelCase（sessionId）两种命名。
        let session_id =
            Self::string_at(&hook_data, &["session_id", "sessionId"]).ok_or_else(|| {
                GitAiError::PresetError("session_id not found in hook_input".to_string())
            })?;

        // 4) 提取工作目录 cwd（checkpoint 以此为基准计算相对路径）。
        let cwd = Self::string_at(&hook_data, &["cwd"])
            .ok_or_else(|| GitAiError::PresetError("cwd not found in hook_input".to_string()))?;

        // 5) 以下字段为可选，且同样兼容 snake_case / camelCase：
        //    - transcript_path：CodeBuddy 对话转写（JSONL）文件路径；
        //    - hook_event_name：当前 hook 事件名（PreToolUse / PostToolUse）；
        //    - tool_name：被调用的工具名（如 Write、Bash 等）；
        //    - tool_use_id：本次工具调用的唯一 ID，缺失时回退为默认值。
        let transcript_path = Self::string_at(&hook_data, &["transcript_path", "transcriptPath"]);
        let hook_event_name = Self::string_at(&hook_data, &["hook_event_name", "hookEventName"]);
        let tool_name = Self::string_at(&hook_data, &["tool_name", "toolName"]);
        let tool_use_id =
            Self::string_at(&hook_data, &["tool_use_id", "toolUseId"]).unwrap_or("codebuddy-tool");

        // 6) 尝试从 JSONL 转写文件解析出对话 transcript 与模型名。
        //    文件不存在或解析失败时，降级为空 transcript，不影响主流程。
        let (transcript, transcript_model) = match transcript_path {
            Some(path) if Path::new(path).exists() => {
                match Self::transcript_and_model_from_codebuddy_jsonl(path) {
                    Ok((transcript, model)) => (transcript, model),
                    Err(e) => {
                        // 解析失败仅告警并打点，不中断（转写只是辅助信息）。
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

        // 7) 确定模型名：优先用转写文件中的，其次从 hook 数据取，最后回退 "unknown"。
        let model = transcript_model
            .or_else(|| Self::model_from_value(&hook_data))
            .unwrap_or_else(|| "unknown".to_string());

        // 8) 构造 AgentId：用工具名 + 会话 ID + 模型唯一标识一次智能体会话。
        let agent_id = AgentId {
            tool: "codebuddy".to_string(),
            id: session_id.to_string(),
            model,
        };

        // 9) 记录转写路径到 agent 元数据（便于后续排查 / 可视化）。
        let mut agent_metadata = HashMap::new();
        if let Some(path) = transcript_path {
            agent_metadata.insert("transcript_path".to_string(), path.to_string());
        }

        // 10) 计算本次将涉及的文件路径：
        //     - explicit_filepaths：hook 中显式给出的编辑路径；
        //     - dirty_files：hook 中给出的“脏文件内容”（文件->内容）；
        //     - target_filepaths：显式路径优先，否则用脏文件路径集合。
        let explicit_filepaths = Self::filepaths_from_hook_data(&hook_data);
        let dirty_files = Self::dirty_files_from_hook_data(&hook_data, explicit_filepaths.as_ref());
        let dirty_filepaths = dirty_files.as_ref().map(|files| {
            let mut paths = files.keys().cloned().collect::<Vec<_>>();
            paths.sort();
            paths
        });
        let target_filepaths = explicit_filepaths.clone().or(dirty_filepaths);

        // 11) 对工具分类：Bash 类走 bash 工具处理，编辑类走文件编辑逻辑，
        //     无法归类且无目标文件时标记为 Skip（跳过）。
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

        // 12) PreToolUse 阶段：在工具执行“之前”捕获一个人类检查点，
        //     用于记录改动前状态（人类意图 / 即将发生的编辑）。
        if Self::is_pre_tool_use(hook_event_name) {
            if tool_class == ToolClass::Skip {
                return Err(GitAiError::PresetError(
                    "Skipping CodeBuddy PreToolUse without mutating tool/path".to_string(),
                ));
            }

            // 对 bash 工具，预捕获会按需触发一次人类 checkpoint（记录执行前快照）。
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

        // 13) 既非 Pre 也非 Post 的事件（如其它自定义 hook）直接拒绝。
        if !Self::is_post_tool_use(hook_event_name) {
            return Err(GitAiError::PresetError(format!(
                "Skipping unsupported CodeBuddy hook event: {}",
                hook_event_name.unwrap_or("unknown")
            )));
        }

        // 14) PostToolUse 阶段：工具执行“之后”捕获。
        //     若是 bash 工具，调用统一处理逻辑提取实际改动的文件。
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

        // 15) 确定“已编辑的文件路径”：
        //     - bash 工具：根据处理结果（Checkpoint / NoChanges / 回退）决定；
        //     - 非 bash 工具：直接使用 target_filepaths。
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

        // 16) 既非 bash 工具，又没有编辑路径和脏文件内容时，无可用于归属的信息，跳过。
        if !is_bash_tool && edited_filepaths.is_none() && dirty_files.is_none() {
            return Err(GitAiError::PresetError(
                "Skipping CodeBuddy PostToolUse without edited path or dirty file content"
                    .to_string(),
            ));
        }

        // 17) 提取 bash 工具处理过程中产生的 captured checkpoint id（若有）。
        let bash_captured_checkpoint_id = bash_result
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|r| r.captured_checkpoint.as_ref())
            .map(|info| info.capture_id.clone());

        // 18) 返回 PostToolUse 的 AI 代理检查点结果。
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
    /// 从 CodeBuddy 的 JSONL 转写文件解析出对话 transcript 与模型名。
    ///
    /// JSONL 每行是一条独立的 JSON 记录；逐行解析并累计为 `AiTranscript`，
    /// 模型名取首个能解析到的（通常在首条记录中）。
    pub fn transcript_and_model_from_codebuddy_jsonl(
        transcript_path: &str,
    ) -> Result<(AiTranscript, Option<String>), GitAiError> {
        // 读取整个转写文件内容。
        let jsonl_content =
            std::fs::read_to_string(transcript_path).map_err(GitAiError::IoError)?;
        let mut transcript = AiTranscript::new();
        let mut model = None;

        // 逐行解析 JSONL（跳过空行）。
        for line in jsonl_content.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let raw_entry: serde_json::Value = serde_json::from_str(line)?;
            let timestamp = Self::timestamp_from_value(&raw_entry);

            // 仅取第一次出现的模型名，作为整段对话的模型标识。
            if model.is_none() {
                model = Self::model_from_value(&raw_entry);
            }

            // 将顶层记录作为一条消息加入 transcript。
            Self::add_transcript_message(&mut transcript, &raw_entry, timestamp.clone());

            // 部分格式把真实消息放在 `message` 字段中（且非字符串），再补一条。
            if let Some(message) = raw_entry.get("message")
                && !message.is_string()
            {
                Self::add_transcript_message(&mut transcript, message, timestamp);
            }
        }

        Ok((transcript, model))
    }

    /// 在 JSON value 中按候选 key 顺序查找第一个字符串字段，实现 snake/camelCase 兼容取值。
    fn string_at<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
        keys.iter().find_map(|key| value.get(*key)?.as_str())
    }

    /// 判断 hook 事件是否为 PreToolUse（不区分大小写）。
    fn is_pre_tool_use(event_name: Option<&str>) -> bool {
        event_name
            .map(|name| name.eq_ignore_ascii_case("PreToolUse"))
            .unwrap_or(false)
    }

    /// 判断 hook 事件是否为 PostToolUse（不区分大小写）。
    fn is_post_tool_use(event_name: Option<&str>) -> bool {
        event_name
            .map(|name| name.eq_ignore_ascii_case("PostToolUse"))
            .unwrap_or(false)
    }

    /// 从 hook 数据中收集显式的文件路径列表（去重、去空白）。
    ///
    /// 来源包括两类：
    /// - 顶层数组字段：`edited_filepaths` / `editedFilepaths` / `file_paths` / `filePaths`；
    /// - 嵌套对象中的单路径字段：`tool_input` / `tool_response` 里的 `file_path` / `path` 等。
    fn filepaths_from_hook_data(hook_data: &serde_json::Value) -> Option<Vec<String>> {
        let mut paths = Vec::new();

        // 顶层数组形式的文件路径。
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

        // 嵌套对象中可能的单文件路径字段。
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

    /// 从 hook 数据中提取“脏文件”内容（文件路径 -> 文件内容）。
    ///
    /// 两种途径：
    /// 1. 直接提供 `dirty_files` / `dirtyFiles` 对象（path -> content）；
    /// 2. 对 Write/Create 类工具且恰好编辑单个文件时，从 `tool_input.content` 推导。
    fn dirty_files_from_hook_data(
        hook_data: &serde_json::Value,
        explicit_filepaths: Option<&Vec<String>>,
    ) -> Option<HashMap<String, String>> {
        // 途径 1：显式提供的脏文件对象。
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

        // 途径 2：仅对 Write/Create 这类“创建/写入整文件”的工具生效。
        let tool_name = Self::string_at(hook_data, &["tool_name", "toolName"]);
        if !matches!(
            tool_name,
            Some("Write") | Some("write") | Some("Create") | Some("create")
        ) {
            return None;
        }

        // 取出工具输入中的 content 字段（即写入的文件内容）。
        let content = hook_data
            .get("tool_input")
            .or_else(|| hook_data.get("toolInput"))
            .and_then(|input| input.get("content"))
            .and_then(|content| content.as_str())?;

        // 必须能确定唯一的文件路径，否则无法建立 (path, content) 映射。
        let paths = explicit_filepaths?;
        if paths.len() != 1 {
            return None;
        }

        Some(HashMap::from([(paths[0].clone(), content.to_string())]))
    }

    /// 向路径列表追加路径，跳过空白与重复项。
    fn push_path(paths: &mut Vec<String>, path: &str) {
        if path.trim().is_empty() || paths.iter().any(|p| p == path) {
            return;
        }
        paths.push(path.to_string());
    }

    /// 从记录中解析时间戳字段（兼容 timestamp / created_at / createdAt）。
    fn timestamp_from_value(value: &serde_json::Value) -> Option<String> {
        Self::string_at(value, &["timestamp", "created_at", "createdAt"]).map(str::to_string)
    }

    /// 从记录中解析模型名：优先顶层 model 字段，其次 message 内嵌的 model。
    fn model_from_value(value: &serde_json::Value) -> Option<String> {
        Self::string_at(value, &["model", "model_name", "modelName"])
            .or_else(|| {
                value
                    .get("message")
                    .and_then(|message| Self::string_at(message, &["model", "model_name"]))
            })
            .map(str::to_string)
    }

    /// 将一条 JSON 记录转换为 transcript 消息（工具调用 / 用户 / 助手）。
    ///
    /// - 若记录是 `type == "tool_use"` 且含 `tool_name`，记为 ToolUse 消息；
    /// - 否则根据 role（user/assistant 等）记为 User / Assistant 文本消息；
    /// - 无法识别角色或文本为空时忽略。
    fn add_transcript_message(
        transcript: &mut AiTranscript,
        value: &serde_json::Value,
        timestamp: Option<String>,
    ) {
        if let Some(tool_name) = Self::string_at(value, &["tool_name", "toolName", "name"])
            && Self::string_at(value, &["type"]) == Some("tool_use")
        {
            // 工具调用消息：合并 input / tool_input / toolInput 作为入参。
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

        // 非工具调用：按角色归类文本消息。
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

    /// 从内容字段中递归提取纯文本。
    ///
    /// 支持三种形态：
    /// - 直接是字符串；
    /// - 是数组（如 `[{text: "..."}, ...]`），拼接各子项文本；
    /// - 是对象，递归取其 `content` / `text` 字段。
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
