use git_ai::mdm::agents::{
    render_codebuddy_hook_command_for_test, render_qoder_hook_command_for_test,
    render_trae_hook_command_for_test,
};
use git_ai::mdm::command_line_test_support::{TestHookShell, render_for_shell};
use serde::Deserialize;
use std::fs;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const HOOK_STDIN: &str = concat!(
    r#"{"hook_event_name":"PostToolUse","tool_name":"Write","cwd":"C:\\Users\\Test User\\repo"}"#,
    "\n"
);
const HOOK_ARGS: &[&str] = &[
    "checkpoint",
    "agent name",
    "A&B",
    "O'Neil",
    "100% Dev",
    "--hook-input",
    "stdin",
];

#[derive(Debug, Deserialize)]
struct HookCommandRecord {
    args: Vec<String>,
    cwd: String,
    stdin: String,
}

fn recorder_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_git-ai-hook-test-recorder"))
}

fn git_bash_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("GIT_AI_TEST_GIT_BASH") {
        let path = PathBuf::from(path);
        assert!(
            path.is_file(),
            "GIT_AI_TEST_GIT_BASH is not a file: {path:?}"
        );
        return path;
    }

    let mut candidates = Vec::new();
    for root in [
        std::env::var_os("ProgramFiles"),
        std::env::var_os("ProgramW6432"),
        std::env::var_os("ProgramFiles(x86)"),
    ]
    .into_iter()
    .flatten()
    {
        let root = PathBuf::from(root);
        candidates.push(root.join("Git").join("bin").join("bash.exe"));
        candidates.push(root.join("Git").join("usr").join("bin").join("bash.exe"));
    }

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .expect("Git Bash is required for Windows Hook command tests")
}

fn link_or_copy_recorder(target: &Path) {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    if fs::hard_link(recorder_binary(), target).is_err() {
        fs::copy(recorder_binary(), target).unwrap();
    }
}

fn shell_process(shell: TestHookShell, rendered: &str) -> Command {
    let mut command = match shell {
        TestHookShell::PowerShell => {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]);
            command
        }
        TestHookShell::Cmd => {
            let mut command = Command::new("cmd.exe");
            command.args(["/D", "/S", "/C"]);
            command.raw_arg(format!(r#""{rendered}""#));
            return command;
        }
        TestHookShell::CmdAndGitBash => {
            panic!("CmdAndGitBash selects a renderer, not a process")
        }
        TestHookShell::GitBash => {
            let mut command = Command::new(git_bash_binary());
            command.arg("-c");
            command
        }
    };
    command.arg(rendered);
    command
}

fn run_rendered_command(
    shell: TestHookShell,
    rendered: &str,
    cwd: &Path,
    record_path: &Path,
) -> Output {
    let mut child = shell_process(shell, rendered)
        .current_dir(cwd)
        .env("GIT_AI_HOOK_RECORDER_OUTPUT", record_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn {shell:?}: {error}"));
    child
        .stdin
        .take()
        .unwrap()
        .write_all(HOOK_STDIN.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn assert_shell_executes_rendered_commands(shell: TestHookShell) {
    let temp_dir = tempfile::tempdir().unwrap();
    let cwd = temp_dir.path().join("repo & workspace");
    fs::create_dir_all(&cwd).unwrap();

    for (index, special_dir) in ["Test User", "A&B", "100% Dev", "O'Neil", "Tools"]
        .iter()
        .enumerate()
    {
        let binary = temp_dir
            .path()
            .join(special_dir)
            .join(".git-ai")
            .join("bin")
            .join("git-ai.exe");
        link_or_copy_recorder(&binary);
        let record_path = temp_dir.path().join(format!("record-{index}.json"));
        let rendered = render_for_shell(&binary, HOOK_ARGS, shell);
        let output = run_rendered_command(shell, &rendered, &cwd, &record_path);

        assert!(
            output.status.success(),
            "{shell:?} command failed\ncommand: {rendered}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let record: HookCommandRecord =
            serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
        assert_eq!(record.args, HOOK_ARGS, "command: {rendered}");
        assert_eq!(
            fs::canonicalize(record.cwd).unwrap(),
            fs::canonicalize(&cwd).unwrap(),
            "command: {rendered}"
        );
        assert_eq!(record.stdin, HOOK_STDIN, "command: {rendered}");
    }
}

#[test]
fn powershell_executes_rendered_hook_commands() {
    assert_shell_executes_rendered_commands(TestHookShell::PowerShell);
}

#[test]
fn cmd_executes_rendered_hook_commands() {
    assert_shell_executes_rendered_commands(TestHookShell::Cmd);
}

#[test]
fn git_bash_executes_rendered_hook_commands() {
    assert_shell_executes_rendered_commands(TestHookShell::GitBash);
}

#[test]
fn native_windows_shells_reject_git_bash_executable_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cwd = temp_dir.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let binary = temp_dir
        .path()
        .join("Test User")
        .join(".git-ai")
        .join("bin")
        .join("git-ai.exe");
    link_or_copy_recorder(&binary);
    let rendered = render_for_shell(&binary, HOOK_ARGS, TestHookShell::GitBash);

    for shell in [TestHookShell::PowerShell, TestHookShell::Cmd] {
        let record_path = temp_dir.path().join(format!("unexpected-{shell:?}.json"));
        let output = run_rendered_command(shell, &rendered, &cwd, &record_path);
        assert!(
            !output.status.success(),
            "{shell:?} unexpectedly accepted Git Bash command: {rendered}"
        );
        assert!(
            !record_path.exists(),
            "recorder unexpectedly ran for {shell:?}: {rendered}"
        );
    }
}

#[test]
fn trae_hook_command_executes_in_powershell() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cwd = temp_dir.path().join("repo & workspace");
    fs::create_dir_all(&cwd).unwrap();
    let binary = temp_dir
        .path()
        .join("Test User")
        .join(".git-ai")
        .join("bin")
        .join("git-ai.exe");
    link_or_copy_recorder(&binary);
    let record_path = temp_dir.path().join("trae-record.json");
    let rendered = render_trae_hook_command_for_test(&binary);

    assert!(rendered.starts_with("& '"), "{rendered}");
    assert!(!rendered.contains("/c/"), "{rendered}");

    let output = run_rendered_command(TestHookShell::PowerShell, &rendered, &cwd, &record_path);
    assert!(
        output.status.success(),
        "Trae PowerShell command failed\ncommand: {rendered}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let record: HookCommandRecord =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(record.args, ["checkpoint", "trae", "--hook-input", "stdin"]);
    assert_eq!(
        fs::canonicalize(record.cwd).unwrap(),
        fs::canonicalize(cwd).unwrap()
    );
    assert_eq!(record.stdin, HOOK_STDIN);
}

#[test]
fn codebuddy_hook_command_executes_in_cmd_and_git_bash() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cwd = temp_dir.path().join("repo & workspace");
    fs::create_dir_all(&cwd).unwrap();

    for (index, special_dir) in ["Test User", "A&B", "100% Dev", "O'Neil", "Tools"]
        .iter()
        .enumerate()
    {
        let binary = temp_dir
            .path()
            .join(special_dir)
            .join(".git-ai")
            .join("bin")
            .join("git-ai.exe");
        link_or_copy_recorder(&binary);
        let rendered = render_codebuddy_hook_command_for_test(&binary);

        assert!(!rendered.contains("/c/"), "{rendered}");
        assert!(!rendered.contains('\\'), "{rendered}");

        for shell in [TestHookShell::Cmd, TestHookShell::GitBash] {
            let record_path = temp_dir
                .path()
                .join(format!("codebuddy-{index}-{shell:?}.json"));
            let output = run_rendered_command(shell, &rendered, &cwd, &record_path);
            assert!(
                output.status.success(),
                "CodeBuddy {shell:?} command failed\ncommand: {rendered}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            let record: HookCommandRecord =
                serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
            assert_eq!(
                record.args,
                ["checkpoint", "codebuddy", "--hook-input", "stdin"]
            );
            assert_eq!(
                fs::canonicalize(record.cwd).unwrap(),
                fs::canonicalize(&cwd).unwrap()
            );
            assert_eq!(record.stdin, HOOK_STDIN);
        }
    }
}

#[test]
fn qoder_hook_command_executes_in_cmd_and_git_bash() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cwd = temp_dir.path().join("repo & workspace");
    fs::create_dir_all(&cwd).unwrap();

    for (index, special_dir) in ["Test User", "A&B", "100% Dev", "O'Neil", "Tools"]
        .iter()
        .enumerate()
    {
        let binary = temp_dir
            .path()
            .join(special_dir)
            .join(".git-ai")
            .join("bin")
            .join("git-ai.exe");
        link_or_copy_recorder(&binary);
        let rendered = render_qoder_hook_command_for_test(&binary);

        assert!(!rendered.contains("/d/"), "{rendered}");
        assert!(!rendered.contains('\\'), "{rendered}");

        for shell in [TestHookShell::Cmd, TestHookShell::GitBash] {
            let record_path = temp_dir
                .path()
                .join(format!("qoder-{index}-{shell:?}.json"));
            let output = run_rendered_command(shell, &rendered, &cwd, &record_path);
            assert!(
                output.status.success(),
                "Qoder {shell:?} command failed\ncommand: {rendered}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            let record: HookCommandRecord =
                serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
            assert_eq!(
                record.args,
                ["checkpoint", "qoder", "--hook-input", "stdin"]
            );
            assert_eq!(
                fs::canonicalize(record.cwd).unwrap(),
                fs::canonicalize(&cwd).unwrap()
            );
            assert_eq!(record.stdin, HOOK_STDIN);
        }
    }
}
