use crate::mdm::utils::to_git_bash_path;
use std::path::Path;

/// Shells used by agent hook configuration files.
///
/// Keep this mapping explicit: a hook command is parsed by the agent's shell,
/// not by Rust's `std::process::Command` argument handling.
///
/// Agent runtime map:
/// - Git Bash: Claude Code, CodeBuddy, Qoder.
/// - POSIX shell on macOS/Linux and PowerShell on Windows: Trae.
/// - POSIX shell on macOS/Linux and `cmd.exe` on Windows: Cursor, Droid,
///   Firebender, Gemini.
/// - Explicit POSIX and PowerShell fields: Codex, GitHub Copilot, Windsurf.
/// - Amp, OpenCode, and PI generate JavaScript/TypeScript plugins and keep
///   executable/argv separation instead of using this shell-string renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookShell {
    /// `bash -c` or another POSIX-compatible shell.
    Posix,
    /// Git for Windows' Bash. Windows drive paths must be converted first.
    GitBash,
    /// `cmd.exe /C` on native Windows.
    Cmd,
    /// PowerShell's `-Command` mode.
    PowerShell,
}

/// Select the POSIX shell on macOS/Linux and the supplied native shell on Windows.
pub(crate) fn platform_hook_shell(windows_shell: HookShell) -> HookShell {
    if cfg!(windows) {
        windows_shell
    } else {
        HookShell::Posix
    }
}

/// Render an executable path and argv as one shell command for an agent hook.
///
/// Each token is quoted independently. Callers must pass structured arguments
/// rather than building a partially escaped command string themselves.
pub(crate) fn render_hook_command(binary_path: &Path, args: &[&str], shell: HookShell) -> String {
    let executable = match shell {
        HookShell::GitBash => to_git_bash_path(binary_path),
        HookShell::Posix | HookShell::Cmd | HookShell::PowerShell => {
            binary_path.to_string_lossy().into_owned()
        }
    };

    match shell {
        HookShell::Posix | HookShell::GitBash => join_tokens(&executable, args, quote_posix_token),
        HookShell::Cmd => join_tokens(&executable, args, quote_cmd_token),
        HookShell::PowerShell => {
            let command = join_tokens(&executable, args, quote_powershell_token);
            if powershell_token_needs_quotes(&executable) {
                format!("& {command}")
            } else {
                command
            }
        }
    }
}

fn join_tokens(executable: &str, args: &[&str], quote: fn(&str) -> String) -> String {
    let mut command = quote(executable);
    for arg in args {
        command.push(' ');
        command.push_str(&quote(arg));
    }
    command
}

fn is_posix_safe(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
                )
        })
}

fn quote_posix_token(value: &str) -> String {
    if is_posix_safe(value) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn is_cmd_safe(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'\\' | b'-'
                )
        })
}

fn quote_cmd_token(value: &str) -> String {
    if is_cmd_safe(value) {
        value.to_string()
    } else {
        // Windows paths cannot contain a double quote. Doubling quotes keeps
        // arbitrary non-path arguments inside the surrounding cmd quotes.
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

fn powershell_token_needs_quotes(value: &str) -> bool {
    value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'\\' | b'-'
                )
        })
}

fn quote_powershell_token(value: &str) -> String {
    if powershell_token_needs_quotes(value) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        value.to_string()
    }
}

#[cfg(feature = "test-support")]
pub mod test_support {
    use super::{HookShell, render_hook_command};
    use std::path::Path;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum TestHookShell {
        GitBash,
        Cmd,
        PowerShell,
    }

    pub fn render_for_shell(binary_path: &Path, args: &[&str], shell: TestHookShell) -> String {
        let shell = match shell {
            TestHookShell::GitBash => HookShell::GitBash,
            TestHookShell::Cmd => HookShell::Cmd,
            TestHookShell::PowerShell => HookShell::PowerShell,
        };
        render_hook_command(binary_path, args, shell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const ARGS: &[&str] = &["checkpoint", "test-agent", "--hook-input", "stdin"];

    fn special_paths() -> [PathBuf; 5] {
        [
            PathBuf::from(r"C:\Users\Test User\.git-ai\bin\git-ai.exe"),
            PathBuf::from(r"C:\Users\A&B\.git-ai\bin\git-ai.exe"),
            PathBuf::from(r"C:\Users\100% Dev\.git-ai\bin\git-ai.exe"),
            PathBuf::from(r"C:\Users\O'Neil\.git-ai\bin\git-ai.exe"),
            PathBuf::from(r"D:\Tools\git ai\git-ai.exe"),
        ]
    }

    #[test]
    fn git_bash_quotes_special_windows_paths() {
        let rendered: Vec<String> = special_paths()
            .iter()
            .map(|path| render_hook_command(path, ARGS, HookShell::GitBash))
            .collect();

        assert_eq!(
            rendered,
            vec![
                r#"'/c/Users/Test User/.git-ai/bin/git-ai.exe' checkpoint test-agent --hook-input stdin"#,
                r#"'/c/Users/A&B/.git-ai/bin/git-ai.exe' checkpoint test-agent --hook-input stdin"#,
                r#"'/c/Users/100% Dev/.git-ai/bin/git-ai.exe' checkpoint test-agent --hook-input stdin"#,
                r#"'/c/Users/O'"'"'Neil/.git-ai/bin/git-ai.exe' checkpoint test-agent --hook-input stdin"#,
                r#"'/d/Tools/git ai/git-ai.exe' checkpoint test-agent --hook-input stdin"#,
            ]
        );
    }

    #[test]
    fn cmd_quotes_special_windows_paths() {
        for path in special_paths() {
            let command = render_hook_command(&path, ARGS, HookShell::Cmd);
            assert!(command.starts_with('"'), "{command}");
            assert!(command.contains("\" checkpoint test-agent"), "{command}");
        }
    }

    #[test]
    fn powershell_quotes_special_windows_paths_and_uses_call_operator() {
        for path in special_paths() {
            let command = render_hook_command(&path, ARGS, HookShell::PowerShell);
            assert!(command.starts_with("& '"), "{command}");
            assert!(command.contains("' checkpoint test-agent"), "{command}");
        }

        let apostrophe = render_hook_command(
            Path::new(r"C:\Users\O'Neil\.git-ai\bin\git-ai.exe"),
            ARGS,
            HookShell::PowerShell,
        );
        assert!(apostrophe.contains("O''Neil"), "{apostrophe}");
    }

    #[test]
    fn ordinary_paths_remain_unquoted_for_existing_shells() {
        assert_eq!(
            render_hook_command(Path::new("/usr/local/bin/git-ai"), ARGS, HookShell::Posix),
            "/usr/local/bin/git-ai checkpoint test-agent --hook-input stdin"
        );
        assert_eq!(
            render_hook_command(
                Path::new(r"C:\Users\Test\.git-ai\bin\git-ai.exe"),
                ARGS,
                HookShell::GitBash
            ),
            "/c/Users/Test/.git-ai/bin/git-ai.exe checkpoint test-agent --hook-input stdin"
        );
        assert_eq!(
            render_hook_command(
                Path::new(r"C:\Users\Test\.git-ai\bin\git-ai.exe"),
                ARGS,
                HookShell::Cmd
            ),
            r"C:\Users\Test\.git-ai\bin\git-ai.exe checkpoint test-agent --hook-input stdin"
        );
    }

    #[test]
    fn quotes_every_argument_for_the_target_shell() {
        let args = ["checkpoint", "agent name", "A&B", "O'Neil", "100% Dev"];
        let binary = Path::new(r"C:\Users\Test User\.git-ai\bin\git-ai.exe");

        assert_eq!(
            render_hook_command(binary, &args, HookShell::GitBash),
            r#"'/c/Users/Test User/.git-ai/bin/git-ai.exe' checkpoint 'agent name' 'A&B' 'O'"'"'Neil' '100% Dev'"#
        );
        assert_eq!(
            render_hook_command(binary, &args, HookShell::PowerShell),
            r#"& 'C:\Users\Test User\.git-ai\bin\git-ai.exe' checkpoint 'agent name' 'A&B' 'O''Neil' '100% Dev'"#
        );
    }
}
