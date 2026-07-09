use crate::error::GitAiError;
use crate::mdm::hook_installer::{HookCheckResult, HookInstaller, HookInstallerParams};
use crate::mdm::utils::{
    binary_exists, generate_diff, home_dir, is_git_ai_checkpoint_command, to_git_bash_path,
    write_atomic,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const CODEBUDDY_PRE_TOOL_CMD: &str = "checkpoint codebuddy --hook-input stdin";
const CODEBUDDY_POST_TOOL_CMD: &str = "checkpoint codebuddy --hook-input stdin";
const CODEBUDDY_CATCH_ALL_MATCHER: &str = "*";

pub struct CodeBuddyInstaller;

impl CodeBuddyInstaller {
    fn settings_path() -> PathBuf {
        home_dir().join(".codebuddy").join("settings.json")
    }

    fn config_dir() -> PathBuf {
        home_dir().join(".codebuddy")
    }

    fn hook_status(settings: &Value) -> (bool, bool) {
        let hooks_installed = ["PreToolUse", "PostToolUse"]
            .iter()
            .any(|event| Self::event_has_git_ai(settings, event, false));
        let hooks_up_to_date = ["PreToolUse", "PostToolUse"]
            .iter()
            .all(|event| Self::event_has_git_ai(settings, event, true));

        (hooks_installed, hooks_up_to_date)
    }

    fn event_has_git_ai(settings: &Value, event: &str, catch_all_only: bool) -> bool {
        let Some(blocks) = settings
            .get("hooks")
            .and_then(|hooks| hooks.get(event))
            .and_then(|value| value.as_array())
        else {
            return false;
        };

        blocks.iter().any(|block| {
            let is_catch_all = block
                .get("matcher")
                .and_then(|matcher| matcher.as_str())
                .map(|matcher| matcher == CODEBUDDY_CATCH_ALL_MATCHER)
                .unwrap_or(false);

            if catch_all_only && !is_catch_all {
                return false;
            }

            block
                .get("hooks")
                .and_then(|hooks| hooks.as_array())
                .map(|hooks| {
                    hooks.iter().any(|hook| {
                        hook.get("command")
                            .and_then(|command| command.as_str())
                            .map(is_git_ai_checkpoint_command)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        })
    }

    fn install_hooks_at(
        settings_path: &Path,
        params: &HookInstallerParams,
        dry_run: bool,
    ) -> Result<Option<String>, GitAiError> {
        if let Some(dir) = settings_path.parent() {
            fs::create_dir_all(dir)?;
        }

        let existing_content = if settings_path.exists() {
            fs::read_to_string(settings_path)?
        } else {
            String::new()
        };
        let existing: Value = if existing_content.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&existing_content)?
        };

        let binary_path_str = to_git_bash_path(&params.binary_path);
        let pre_tool_cmd = format!("{} {}", binary_path_str, CODEBUDDY_PRE_TOOL_CMD);
        let post_tool_cmd = format!("{} {}", binary_path_str, CODEBUDDY_POST_TOOL_CMD);

        let mut merged = existing.clone();
        let mut hooks_obj = merged.get("hooks").cloned().unwrap_or_else(|| json!({}));
        if !hooks_obj.is_object() {
            hooks_obj = json!({});
        }

        for (event, desired_cmd) in [
            ("PreToolUse", pre_tool_cmd.as_str()),
            ("PostToolUse", post_tool_cmd.as_str()),
        ] {
            let hook_type_array = hooks_obj
                .get(event)
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let hook_type_array = Self::merge_event_hooks(hook_type_array, desired_cmd);

            if let Some(obj) = hooks_obj.as_object_mut() {
                obj.insert(event.to_string(), Value::Array(hook_type_array));
            }
        }

        if let Some(root) = merged.as_object_mut() {
            root.insert("hooks".to_string(), hooks_obj);
        }

        if existing == merged {
            return Ok(None);
        }

        let new_content = serde_json::to_string_pretty(&merged)?;
        let diff_output = generate_diff(settings_path, &existing_content, &new_content);

        if !dry_run {
            write_atomic(settings_path, new_content.as_bytes())?;
        }

        Ok(Some(diff_output))
    }

    fn merge_event_hooks(mut blocks: Vec<Value>, desired_cmd: &str) -> Vec<Value> {
        let mut emptied_by_migration = vec![false; blocks.len()];

        for (index, block) in blocks.iter_mut().enumerate() {
            if let Some(hooks) = block
                .get_mut("hooks")
                .and_then(|hooks| hooks.as_array_mut())
            {
                let before = hooks.len();
                hooks.retain(|hook| {
                    hook.get("command")
                        .and_then(|command| command.as_str())
                        .map(|command| !is_git_ai_checkpoint_command(command))
                        .unwrap_or(true)
                });
                if before > 0 && hooks.is_empty() {
                    emptied_by_migration[index] = true;
                }
            }
        }

        let mut index = 0;
        blocks.retain(|_| {
            let remove = emptied_by_migration[index];
            index += 1;
            !remove
        });

        let catch_all_idx = blocks
            .iter()
            .position(|block| {
                block
                    .get("matcher")
                    .and_then(|matcher| matcher.as_str())
                    .map(|matcher| matcher == CODEBUDDY_CATCH_ALL_MATCHER)
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| {
                blocks.push(json!({
                    "matcher": CODEBUDDY_CATCH_ALL_MATCHER,
                    "hooks": []
                }));
                blocks.len() - 1
            });

        let mut hooks_array = blocks[catch_all_idx]
            .get("hooks")
            .and_then(|hooks| hooks.as_array())
            .cloned()
            .unwrap_or_default();

        hooks_array.push(json!({
            "type": "command",
            "command": desired_cmd
        }));

        if let Some(block) = blocks[catch_all_idx].as_object_mut() {
            block.insert("hooks".to_string(), Value::Array(hooks_array));
        }

        blocks
    }

    fn uninstall_hooks_at(
        settings_path: &Path,
        dry_run: bool,
    ) -> Result<Option<String>, GitAiError> {
        if !settings_path.exists() {
            return Ok(None);
        }

        let existing_content = fs::read_to_string(settings_path)?;
        let existing: Value = serde_json::from_str(&existing_content)?;
        let mut merged = existing.clone();
        let Some(hooks_obj) = merged.get_mut("hooks") else {
            return Ok(None);
        };

        let mut changed = false;
        for event in ["PreToolUse", "PostToolUse"] {
            if let Some(blocks) = hooks_obj
                .get_mut(event)
                .and_then(|value| value.as_array_mut())
            {
                for block in blocks {
                    if let Some(hooks) = block
                        .get_mut("hooks")
                        .and_then(|value| value.as_array_mut())
                    {
                        let before = hooks.len();
                        hooks.retain(|hook| {
                            hook.get("command")
                                .and_then(|command| command.as_str())
                                .map(|command| !is_git_ai_checkpoint_command(command))
                                .unwrap_or(true)
                        });
                        changed |= hooks.len() != before;
                    }
                }
            }
        }

        if !changed {
            return Ok(None);
        }

        let new_content = serde_json::to_string_pretty(&merged)?;
        let diff_output = generate_diff(settings_path, &existing_content, &new_content);

        if !dry_run {
            write_atomic(settings_path, new_content.as_bytes())?;
        }

        Ok(Some(diff_output))
    }
}

impl HookInstaller for CodeBuddyInstaller {
    fn name(&self) -> &str {
        "CodeBuddy"
    }

    fn id(&self) -> &str {
        "codebuddy"
    }

    fn check_hooks(&self, _params: &HookInstallerParams) -> Result<HookCheckResult, GitAiError> {
        let has_binary = binary_exists("codebuddy");
        let has_dotfiles = Self::config_dir().exists();

        if !has_binary && !has_dotfiles {
            return Ok(HookCheckResult {
                tool_installed: false,
                hooks_installed: false,
                hooks_up_to_date: false,
            });
        }

        let settings_path = Self::settings_path();
        if !settings_path.exists() {
            return Ok(HookCheckResult {
                tool_installed: true,
                hooks_installed: false,
                hooks_up_to_date: false,
            });
        }

        let content = fs::read_to_string(&settings_path)?;
        let existing: Value = serde_json::from_str(&content).unwrap_or_else(|_| json!({}));
        let (hooks_installed, hooks_up_to_date) = Self::hook_status(&existing);

        Ok(HookCheckResult {
            tool_installed: true,
            hooks_installed,
            hooks_up_to_date,
        })
    }

    fn process_names(&self) -> Vec<&str> {
        vec!["codebuddy", "CodeBuddy"]
    }

    fn install_hooks(
        &self,
        params: &HookInstallerParams,
        dry_run: bool,
    ) -> Result<Option<String>, GitAiError> {
        Self::install_hooks_at(&Self::settings_path(), params, dry_run)
    }

    fn uninstall_hooks(
        &self,
        _params: &HookInstallerParams,
        dry_run: bool,
    ) -> Result<Option<String>, GitAiError> {
        Self::uninstall_hooks_at(&Self::settings_path(), dry_run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_env() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let settings_path = temp_dir.path().join(".codebuddy").join("settings.json");
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        (temp_dir, settings_path)
    }

    fn params() -> HookInstallerParams {
        HookInstallerParams {
            binary_path: PathBuf::from("/usr/local/bin/git-ai"),
        }
    }

    fn expected_cmd() -> String {
        format!(
            "{} {}",
            to_git_bash_path(&params().binary_path),
            CODEBUDDY_PRE_TOOL_CMD
        )
    }

    fn read_settings(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn catch_all_hooks<'a>(settings: &'a Value, event: &str) -> Vec<&'a Value> {
        settings
            .get("hooks")
            .and_then(|hooks| hooks.get(event))
            .and_then(|value| value.as_array())
            .and_then(|blocks| {
                blocks.iter().find(|block| {
                    block
                        .get("matcher")
                        .and_then(|matcher| matcher.as_str())
                        .map(|matcher| matcher == CODEBUDDY_CATCH_ALL_MATCHER)
                        .unwrap_or(false)
                })
            })
            .and_then(|block| block.get("hooks").and_then(|hooks| hooks.as_array()))
            .map(|hooks| hooks.iter().collect())
            .unwrap_or_default()
    }

    #[test]
    fn fresh_install_creates_pre_and_post_catch_all_hooks() {
        let (_temp_dir, settings_path) = setup_test_env();
        fs::remove_file(&settings_path).ok();

        let diff = CodeBuddyInstaller::install_hooks_at(&settings_path, &params(), false).unwrap();
        assert!(diff.is_some());

        let settings = read_settings(&settings_path);
        for event in ["PreToolUse", "PostToolUse"] {
            let hooks = catch_all_hooks(&settings, event);
            assert_eq!(hooks.len(), 1, "{event}: expected one catch-all hook");
            assert_eq!(
                hooks[0].get("command").and_then(|command| command.as_str()),
                Some(expected_cmd().as_str())
            );
        }
    }

    #[test]
    fn hook_status_requires_pre_and_post_catch_all() {
        let settings = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "*",
                    "hooks": [{"type": "command", "command": "/usr/local/bin/git-ai checkpoint codebuddy --hook-input stdin"}]
                }]
            }
        });

        assert_eq!(CodeBuddyInstaller::hook_status(&settings), (true, false));
    }

    #[test]
    fn install_preserves_user_hooks_and_moves_git_ai_to_catch_all() {
        let (_temp_dir, settings_path) = setup_test_env();
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "PostToolUse": [{
                        "matcher": "Edit|Write",
                        "hooks": [
                            {"type": "command", "command": "echo user-hook"},
                            {"type": "command", "command": "/old/git-ai checkpoint codebuddy --hook-input stdin"}
                        ]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        CodeBuddyInstaller::install_hooks_at(&settings_path, &params(), false).unwrap();
        let settings = read_settings(&settings_path);

        let post_blocks = settings["hooks"]["PostToolUse"].as_array().unwrap();
        let old_block = post_blocks
            .iter()
            .find(|block| {
                block.get("matcher").and_then(|matcher| matcher.as_str()) == Some("Edit|Write")
            })
            .unwrap();
        let old_hooks = old_block["hooks"].as_array().unwrap();
        assert_eq!(old_hooks.len(), 1);
        assert_eq!(
            old_hooks[0]
                .get("command")
                .and_then(|command| command.as_str()),
            Some("echo user-hook")
        );

        let catch_all = catch_all_hooks(&settings, "PostToolUse");
        assert_eq!(catch_all.len(), 1);
        assert_eq!(
            catch_all[0]
                .get("command")
                .and_then(|command| command.as_str()),
            Some(expected_cmd().as_str())
        );
    }
}
