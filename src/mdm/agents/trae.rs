use crate::error::GitAiError;
use crate::mdm::hook_installer::{HookCheckResult, HookInstaller, HookInstallerParams};
use crate::mdm::utils::{
    binary_exists, generate_diff, home_dir, is_git_ai_checkpoint_command, to_git_bash_path,
    write_atomic,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const TRAE_PRE_TOOL_CMD: &str = "checkpoint trae --hook-input stdin";
const TRAE_POST_TOOL_CMD: &str = "checkpoint trae --hook-input stdin";
const TRAE_CATCH_ALL_MATCHER: &str = "*";

pub struct TraeInstaller;

impl TraeInstaller {
    fn hooks_path() -> PathBuf {
        Self::config_dir().join("hooks.json")
    }

    fn config_dir() -> PathBuf {
        home_dir().join(".trae-cn")
    }

    fn app_exists() -> bool {
        #[cfg(target_os = "macos")]
        {
            let home = home_dir();
            [
                PathBuf::from("/Applications/Trae.app"),
                PathBuf::from("/Applications/TRAE.app"),
                PathBuf::from("/Applications/Trae CN.app"),
                home.join("Applications").join("Trae.app"),
                home.join("Applications").join("TRAE.app"),
                home.join("Applications").join("Trae CN.app"),
            ]
            .iter()
            .any(|path| path.exists())
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
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
                .map(|matcher| matcher == TRAE_CATCH_ALL_MATCHER)
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
        hooks_path: &Path,
        params: &HookInstallerParams,
        dry_run: bool,
    ) -> Result<Option<String>, GitAiError> {
        if let Some(dir) = hooks_path.parent() {
            fs::create_dir_all(dir)?;
        }

        let existing_content = if hooks_path.exists() {
            fs::read_to_string(hooks_path)?
        } else {
            String::new()
        };
        let existing: Value = if existing_content.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&existing_content)?
        };

        let binary_path_str = to_git_bash_path(&params.binary_path);
        let pre_tool_cmd = format!("{} {}", binary_path_str, TRAE_PRE_TOOL_CMD);
        let post_tool_cmd = format!("{} {}", binary_path_str, TRAE_POST_TOOL_CMD);

        let mut merged = existing.clone();
        if !merged.is_object() {
            merged = json!({});
        }

        if let Some(root) = merged.as_object_mut() {
            root.entry("version").or_insert(json!(1));
        }

        let mut hooks_obj = merged.get("hooks").cloned().unwrap_or_else(|| json!({}));
        if !hooks_obj.is_object() {
            hooks_obj = json!({});
        }

        for (event, desired_cmd) in [
            ("PreToolUse", pre_tool_cmd.as_str()),
            ("PostToolUse", post_tool_cmd.as_str()),
        ] {
            let event_blocks = hooks_obj
                .get(event)
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let event_blocks = Self::merge_event_hooks(event_blocks, desired_cmd);

            if let Some(obj) = hooks_obj.as_object_mut() {
                obj.insert(event.to_string(), Value::Array(event_blocks));
            }
        }

        if let Some(root) = merged.as_object_mut() {
            root.insert("hooks".to_string(), hooks_obj);
        }

        if existing == merged {
            return Ok(None);
        }

        let new_content = serde_json::to_string_pretty(&merged)?;
        let diff_output = generate_diff(hooks_path, &existing_content, &new_content);

        if !dry_run {
            write_atomic(hooks_path, new_content.as_bytes())?;
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
                    .map(|matcher| matcher == TRAE_CATCH_ALL_MATCHER)
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| {
                blocks.push(json!({
                    "matcher": TRAE_CATCH_ALL_MATCHER,
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

    fn uninstall_hooks_at(hooks_path: &Path, dry_run: bool) -> Result<Option<String>, GitAiError> {
        if !hooks_path.exists() {
            return Ok(None);
        }

        let existing_content = fs::read_to_string(hooks_path)?;
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
        let diff_output = generate_diff(hooks_path, &existing_content, &new_content);

        if !dry_run {
            write_atomic(hooks_path, new_content.as_bytes())?;
        }

        Ok(Some(diff_output))
    }
}

impl HookInstaller for TraeInstaller {
    fn name(&self) -> &str {
        "Trae"
    }

    fn id(&self) -> &str {
        "trae"
    }

    fn check_hooks(&self, _params: &HookInstallerParams) -> Result<HookCheckResult, GitAiError> {
        let has_binary = binary_exists("trae");
        let has_dotfiles = Self::config_dir().exists();
        let has_app = Self::app_exists();

        if !has_binary && !has_dotfiles && !has_app {
            return Ok(HookCheckResult {
                tool_installed: false,
                hooks_installed: false,
                hooks_up_to_date: false,
            });
        }

        let hooks_path = Self::hooks_path();
        if !hooks_path.exists() {
            return Ok(HookCheckResult {
                tool_installed: true,
                hooks_installed: false,
                hooks_up_to_date: false,
            });
        }

        let content = fs::read_to_string(&hooks_path)?;
        let existing: Value = serde_json::from_str(&content).unwrap_or_else(|_| json!({}));
        let (hooks_installed, hooks_up_to_date) = Self::hook_status(&existing);

        Ok(HookCheckResult {
            tool_installed: true,
            hooks_installed,
            hooks_up_to_date,
        })
    }

    fn process_names(&self) -> Vec<&str> {
        vec!["trae", "Trae", "TRAE"]
    }

    fn install_hooks(
        &self,
        params: &HookInstallerParams,
        dry_run: bool,
    ) -> Result<Option<String>, GitAiError> {
        Self::install_hooks_at(&Self::hooks_path(), params, dry_run)
    }

    fn uninstall_hooks(
        &self,
        _params: &HookInstallerParams,
        dry_run: bool,
    ) -> Result<Option<String>, GitAiError> {
        Self::uninstall_hooks_at(&Self::hooks_path(), dry_run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_env() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let hooks_path = temp_dir.path().join(".trae-cn").join("hooks.json");
        fs::create_dir_all(hooks_path.parent().unwrap()).unwrap();
        (temp_dir, hooks_path)
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
            TRAE_PRE_TOOL_CMD
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
                        .map(|matcher| matcher == TRAE_CATCH_ALL_MATCHER)
                        .unwrap_or(false)
                })
            })
            .and_then(|block| block.get("hooks").and_then(|hooks| hooks.as_array()))
            .map(|hooks| hooks.iter().collect())
            .unwrap_or_default()
    }

    #[test]
    fn fresh_install_creates_version_and_catch_all_hooks() {
        let (_temp_dir, hooks_path) = setup_test_env();
        fs::remove_file(&hooks_path).ok();

        let diff = TraeInstaller::install_hooks_at(&hooks_path, &params(), false).unwrap();
        assert!(diff.is_some());

        let settings = read_settings(&hooks_path);
        assert_eq!(
            settings.get("version").and_then(|value| value.as_i64()),
            Some(1)
        );
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
            "version": 1,
            "hooks": {
                "PreToolUse": [{
                    "matcher": "*",
                    "hooks": [{"type": "command", "command": "/usr/local/bin/git-ai checkpoint trae --hook-input stdin"}]
                }]
            }
        });

        assert_eq!(TraeInstaller::hook_status(&settings), (true, false));
    }
}
