use crate::{
    authorship::{
        transcript::{AiTranscript, Message},
        working_log::{AgentId, CheckpointKind},
    },
    commands::checkpoint_agent::bash_tool::{
        self, Agent, BashCheckpointAction, HookEvent, ToolClass,
    },
    error::GitAiError,
    git::repository::find_repository_for_file,
    observability::log_error,
    utils::normalize_to_posix,
};
use chrono::{TimeZone, Utc};
use dirs;
use glob::glob;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::{Component, Path, PathBuf};

pub struct AgentCheckpointFlags {
    pub hook_input: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRunResult {
    pub agent_id: AgentId,
    pub agent_metadata: Option<HashMap<String, String>>,
    pub checkpoint_kind: CheckpointKind,
    pub transcript: Option<AiTranscript>,
    pub repo_working_dir: Option<String>,
    pub edited_filepaths: Option<Vec<String>>,
    pub will_edit_filepaths: Option<Vec<String>>,
    pub dirty_files: Option<HashMap<String, String>>,
    /// Pre-prepared captured checkpoint ID from bash tool (bypasses normal capture flow).
    pub captured_checkpoint_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BashPreHookStrategy {
    EmitHumanCheckpoint,
    SnapshotOnly,
}

pub(crate) enum BashPreHookResult {
    EmitHumanCheckpoint {
        captured_checkpoint_id: Option<String>,
    },
    SkipCheckpoint {
        captured_checkpoint_id: Option<String>,
    },
}

impl BashPreHookResult {
    pub(crate) fn captured_checkpoint_id(self) -> Option<String> {
        match self {
            Self::EmitHumanCheckpoint {
                captured_checkpoint_id,
            }
            | Self::SkipCheckpoint {
                captured_checkpoint_id,
            } => captured_checkpoint_id,
        }
    }
}

pub(crate) fn prepare_agent_bash_pre_hook(
    is_bash_tool: bool,
    repo_working_dir: Option<&str>,
    session_id: &str,
    tool_use_id: &str,
    agent_id: &AgentId,
    agent_metadata: Option<&HashMap<String, String>>,
    strategy: BashPreHookStrategy,
) -> Result<BashPreHookResult, GitAiError> {
    let captured_checkpoint_id = if is_bash_tool {
        if let Some(cwd) = repo_working_dir {
            match bash_tool::handle_bash_pre_tool_use_with_context(
                Path::new(cwd),
                session_id,
                tool_use_id,
                agent_id,
                agent_metadata,
            ) {
                Ok(result) => result.captured_checkpoint.map(|info| info.capture_id),
                Err(error) => {
                    tracing::debug!(
                        "Bash pre-hook snapshot failed for {} session {}: {}",
                        agent_id.tool,
                        session_id,
                        error
                    );
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok(match strategy {
        BashPreHookStrategy::EmitHumanCheckpoint => BashPreHookResult::EmitHumanCheckpoint {
            captured_checkpoint_id,
        },
        BashPreHookStrategy::SnapshotOnly => BashPreHookResult::SkipCheckpoint {
            captured_checkpoint_id,
        },
    })
}

pub trait AgentCheckpointPreset {
    fn run(&self, flags: AgentCheckpointFlags) -> Result<AgentRunResult, GitAiError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepare_agent_bash_pre_hook_swallows_snapshot_errors() {
        let temp = tempfile::tempdir().unwrap();
        let missing_repo = temp.path().join("missing-repo");
        let agent_id = AgentId {
            tool: "codex".to_string(),
            id: "session-1".to_string(),
            model: "gpt-5.4".to_string(),
        };

        let result = prepare_agent_bash_pre_hook(
            true,
            Some(missing_repo.to_string_lossy().as_ref()),
            "session-1",
            "tool-1",
            &agent_id,
            None,
            BashPreHookStrategy::EmitHumanCheckpoint,
        )
        .expect("pre-hook helper should treat snapshot failures as best-effort");

        match result {
            BashPreHookResult::EmitHumanCheckpoint {
                captured_checkpoint_id,
            } => {
                assert!(
                    captured_checkpoint_id.is_none(),
                    "failed pre-hook snapshot should not produce a captured checkpoint"
                );
            }
            BashPreHookResult::SkipCheckpoint { .. } => {
                panic!("expected EmitHumanCheckpoint result");
            }
        }
    }
}

mod ai_tab;
mod claude;
mod codebuddy;
mod codex;
mod continue_cli;
mod cursor;
mod droid;
mod firebender;
mod gemini;
mod github_copilot;
mod trae;
mod windsurf;

pub use ai_tab::AiTabPreset;
pub use claude::{ClaudePreset, extract_plan_from_tool_use, is_plan_file_path};
pub use codebuddy::CodeBuddyPreset;
pub use codex::CodexPreset;
pub use continue_cli::ContinueCliPreset;
pub use cursor::CursorPreset;
pub use droid::DroidPreset;
pub use firebender::FirebenderPreset;
pub use gemini::GeminiPreset;
pub use github_copilot::GithubCopilotPreset;
pub use trae::TraePreset;
pub use windsurf::WindsurfPreset;
