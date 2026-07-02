use super::{
    ActorDaemonCoordinator, DAEMON_SOCKET_PROBE_TIMEOUT, DaemonConfig, GitAiError,
    local_socket_connects_with_timeout,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const PID_META_FILE: &str = "daemon.pid.json";
#[cfg(windows)]
const WINDOWS_STDOUT_HANDLE: u32 = (-11i32) as u32;
#[cfg(windows)]
const WINDOWS_STDERR_HANDLE: u32 = (-12i32) as u32;

#[cfg(windows)]
unsafe extern "system" {
    fn SetStdHandle(nstdhandle: u32, hhandle: *mut std::ffi::c_void) -> i32;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonPidMeta {
    pid: u32,
    started_at_ns: u128,
}

pub(super) fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub(super) fn remove_socket_if_exists(path: &Path) -> Result<(), GitAiError> {
    #[cfg(unix)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn set_socket_owner_only(path: &Path) -> Result<(), GitAiError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn pid_metadata_path(config: &DaemonConfig) -> PathBuf {
    config
        .lock_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(PID_META_FILE)
}

/// Returns the log file path for the currently running daemon, if any.
/// Reads the PID from daemon.pid.json and constructs the log path.
pub fn daemon_log_file_path(config: &DaemonConfig) -> Result<PathBuf, GitAiError> {
    let meta_path = pid_metadata_path(config);
    let contents = fs::read_to_string(&meta_path).map_err(|e| {
        GitAiError::Generic(format!(
            "failed to read daemon pid metadata at {}: {}",
            meta_path.display(),
            e
        ))
    })?;
    let meta: DaemonPidMeta = serde_json::from_str(&contents)?;
    let log_dir = config.internal_dir.join("daemon").join("logs");
    Ok(log_dir.join(format!("{}.log", meta.pid)))
}

pub(super) fn write_pid_metadata(config: &DaemonConfig) -> Result<(), GitAiError> {
    let meta = DaemonPidMeta {
        pid: std::process::id(),
        started_at_ns: now_unix_nanos(),
    };
    let path = pid_metadata_path(config);
    fs::write(path, serde_json::to_string_pretty(&meta)?)?;
    Ok(())
}

/// Read the PID of the currently running daemon from the pid metadata file.
pub fn read_daemon_pid(config: &DaemonConfig) -> Result<u32, GitAiError> {
    let meta_path = pid_metadata_path(config);
    let contents = fs::read_to_string(&meta_path).map_err(|e| {
        GitAiError::Generic(format!(
            "failed to read daemon pid metadata at {}: {}",
            meta_path.display(),
            e
        ))
    })?;
    let meta: DaemonPidMeta = serde_json::from_str(&contents)?;
    Ok(meta.pid)
}

pub(super) fn remove_pid_metadata(config: &DaemonConfig) -> Result<(), GitAiError> {
    let path = pid_metadata_path(config);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn daemon_is_test_mode() -> bool {
    std::env::var_os("GIT_AI_TEST_DB_PATH").is_some()
        || std::env::var_os("GITAI_TEST_DB_PATH").is_some()
}

fn daemon_log_dir(config: &DaemonConfig) -> PathBuf {
    config.internal_dir.join("daemon").join("logs")
}

/// Redirect stdout and stderr to a per-PID log file inside the daemon logs
/// directory. Skipped in test mode to keep test output on the console.
/// Returns a guard that keeps the log file open for the lifetime of the daemon.
#[cfg(unix)]
pub(super) fn maybe_setup_daemon_log_file(config: &DaemonConfig) -> Option<DaemonLogGuard> {
    if daemon_is_test_mode() {
        return None;
    }
    match setup_daemon_log_file(config) {
        Ok(guard) => Some(guard),
        Err(e) => {
            tracing::error!(%e, "log file setup failed");
            None
        }
    }
}

#[cfg(windows)]
pub(super) fn maybe_setup_daemon_log_file(config: &DaemonConfig) -> Option<DaemonLogGuard> {
    match setup_daemon_log_file(config) {
        Ok(guard) => Some(guard),
        Err(e) => {
            tracing::error!(%e, "log file setup failed");
            None
        }
    }
}

pub(super) struct DaemonLogGuard {
    _file: File,
}

#[cfg(unix)]
fn setup_daemon_log_file(config: &DaemonConfig) -> Result<DaemonLogGuard, GitAiError> {
    use std::os::unix::io::AsRawFd;

    let log_dir = daemon_log_dir(config);
    fs::create_dir_all(&log_dir)?;

    let prune_dir = log_dir.clone();
    std::thread::spawn(move || prune_stale_daemon_logs(&prune_dir));

    let log_path = log_dir.join(format!("{}.log", std::process::id()));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let fd = file.as_raw_fd();
    // SAFETY: dup2 is a standard POSIX call; we redirect stdout/stderr to our
    // open log file descriptor. The file is kept alive by the returned guard.
    unsafe {
        if libc::dup2(fd, libc::STDOUT_FILENO) == -1 {
            return Err(GitAiError::Generic("dup2 stdout failed".to_string()));
        }
        if libc::dup2(fd, libc::STDERR_FILENO) == -1 {
            return Err(GitAiError::Generic("dup2 stderr failed".to_string()));
        }
    }

    Ok(DaemonLogGuard { _file: file })
}

#[cfg(windows)]
fn setup_daemon_log_file(config: &DaemonConfig) -> Result<DaemonLogGuard, GitAiError> {
    let log_dir = daemon_log_dir(config);
    fs::create_dir_all(&log_dir)?;

    let prune_dir = log_dir.clone();
    std::thread::spawn(move || prune_stale_daemon_logs(&prune_dir));

    let log_path = log_dir.join(format!("{}.log", std::process::id()));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    redirect_windows_stdio_to_log_file(&file)?;
    eprintln!("[git-ai] daemon log initialized at {}", log_path.display());

    Ok(DaemonLogGuard { _file: file })
}

#[cfg(windows)]
fn redirect_windows_stdio_to_log_file(file: &File) -> Result<(), GitAiError> {
    redirect_windows_stdio_stream(file, 1, WINDOWS_STDOUT_HANDLE)?;
    redirect_windows_stdio_stream(file, 2, WINDOWS_STDERR_HANDLE)?;
    Ok(())
}

#[cfg(windows)]
fn redirect_windows_stdio_stream(
    file: &File,
    std_fd: libc::c_int,
    std_handle: u32,
) -> Result<(), GitAiError> {
    let clone = file.try_clone()?;
    let raw_handle = clone.into_raw_handle();
    let fd = unsafe {
        libc::open_osfhandle(
            raw_handle as libc::intptr_t,
            libc::O_APPEND | libc::O_BINARY,
        )
    };
    if fd == -1 {
        unsafe {
            drop(File::from_raw_handle(raw_handle));
        }
        return Err(GitAiError::Generic(format!(
            "open_osfhandle failed for daemon log stream {}: {}",
            std_fd,
            std::io::Error::last_os_error()
        )));
    }

    let dup_result = unsafe { libc::dup2(fd, std_fd) };
    if dup_result == -1 {
        let err = std::io::Error::last_os_error();
        let _ = unsafe { libc::close(fd) };
        return Err(GitAiError::Generic(format!(
            "dup2 failed for daemon log stream {}: {}",
            std_fd, err
        )));
    }
    if unsafe { libc::close(fd) } == -1 {
        tracing::debug!(
            std_fd,
            error = %std::io::Error::last_os_error(),
            "close failed for log stream after successful redirect"
        );
    }

    let set_handle_result = unsafe { SetStdHandle(std_handle, file.as_raw_handle()) };
    if set_handle_result == 0 {
        return Err(GitAiError::Generic(format!(
            "SetStdHandle failed for daemon log stream {}: {}",
            std_fd,
            std::io::Error::last_os_error()
        )));
    }

    Ok(())
}

/// Remove log files from previous daemon runs that are older than one week and
/// whose PID is no longer alive, to avoid unbounded growth while keeping recent
/// logs available for debugging.
fn prune_stale_daemon_logs(log_dir: &Path) {
    let one_week = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    let entries = match fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        let _pid: u32 = match stem.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let dominated = path
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > one_week);
        if !dominated {
            continue;
        }
        #[cfg(unix)]
        {
            if process_alive(_pid) {
                continue;
            }
        }
        let _ = fs::remove_file(&path);
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // kill(pid, 0) checks existence without sending a signal.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// Git environment variables that must not leak into the daemon process.
///
/// The daemon is a long-lived, repository-agnostic process that serves requests
/// for many different repositories. Environment variables like `GIT_DIR` and
/// `GIT_WORK_TREE` pin git operations to a single repository and override the
/// `-C <path>` flag that the daemon uses to target each repository individually.
///
/// When a daemon is spawned by a git wrapper invocation (e.g. `git add`), the
/// parent process may have these variables set by git itself (hook context) or
/// by test harnesses. Clearing them at daemon startup prevents incorrect
/// repository resolution that manifests as `fatal: not a git repository: '/dev/null'`.
///
/// This list is used in two places:
/// - `spawn_daemon_run_detached` strips them from the child process via `env_remove`.
/// - `sanitize_git_env_for_daemon` clears them from the current process at daemon startup
///   as a belt-and-suspenders defence (the daemon may be launched by another mechanism).
pub const GIT_ENV_VARS_TO_SANITIZE: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_CEILING_DIRECTORIES",
    "GIT_QUARANTINE_PATH",
    "GIT_NAMESPACE",
];

pub(super) fn sanitize_git_env_for_daemon() {
    for var in GIT_ENV_VARS_TO_SANITIZE {
        // SAFETY: daemon startup is single-threaded at this point -- the tokio
        // runtime is not yet running and no other threads exist.
        unsafe {
            std::env::remove_var(var);
        }
    }
}

pub(super) fn disable_trace2_for_daemon_process() {
    // The daemon executes internal git commands while processing events and control requests.
    // If trace2.eventTarget points at this daemon socket globally, those internal git
    // commands can recursively feed trace2 events back into the daemon and starve progress.
    // Force-disable trace2 emission for the daemon process and all of its child git commands.
    unsafe {
        std::env::set_var("GIT_TRACE2_EVENT", "0");
    }
}

/// How often the daemon wakes up to evaluate whether an update check is due.
const DAEMON_UPDATE_CHECK_INTERVAL_SECS: u64 = 3600;

/// Maximum daemon uptime before a proactive restart (24.5 hours).
/// Deliberately offset from the 24h update-check cadence so the uptime restart
/// never races with an update-triggered shutdown.
const DAEMON_MAX_UPTIME_SECS: u64 = 24 * 3600 + 30 * 60;

/// Returns the update check interval, respecting an env var override for testing.
fn daemon_update_check_interval() -> u64 {
    std::env::var("GIT_AI_DAEMON_UPDATE_CHECK_INTERVAL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DAEMON_UPDATE_CHECK_INTERVAL_SECS)
}

/// Returns the maximum uptime in nanoseconds, respecting an env var override for testing.
fn daemon_max_uptime_ns() -> u128 {
    let secs = std::env::var("GIT_AI_DAEMON_MAX_UPTIME_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DAEMON_MAX_UPTIME_SECS);
    secs as u128 * 1_000_000_000
}

const DAEMON_SOCKET_HEALTH_CHECK_SECS: u64 = 30;

fn daemon_socket_health_check_interval() -> u64 {
    std::env::var("GIT_AI_DAEMON_SOCKET_HEALTH_CHECK_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DAEMON_SOCKET_HEALTH_CHECK_SECS)
}

/// Spawn a detached `git-ai bg restart --hard` process that will reap the
/// current (zombie) daemon and start a fresh one.  The child inherits the
/// daemon env vars (GIT_AI_DAEMON_HOME, etc.) so it targets the same
/// instance.  Returns Ok if the process was spawned; the caller should
/// still request_shutdown so the current daemon exits promptly.
fn spawn_self_restart() -> Result<(), String> {
    let exe = crate::utils::current_git_ai_exe().map_err(|e| e.to_string())?;
    tracing::info!(?exe, "spawning detached restart process");

    let mut cmd = std::process::Command::new(&exe);
    cmd.args(["bg", "restart", "--hard"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    for var in GIT_ENV_VARS_TO_SANITIZE {
        cmd.env_remove(var);
    }
    cmd.env_remove("GIT_AI");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }

    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to spawn restart process: {}", e))
}

const DAEMON_MIN_UPTIME_FOR_SELF_RESTART_SECS: u64 = 60;

fn daemon_min_uptime_for_self_restart() -> u64 {
    std::env::var("GIT_AI_DAEMON_MIN_UPTIME_FOR_RESTART_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DAEMON_MIN_UPTIME_FOR_SELF_RESTART_SECS)
}

/// Background loop that verifies the daemon's sockets are reachable by
/// actually connecting to them.  A successful connect proves the socket file
/// exists, points to this daemon's listener, and that the listener thread is
/// alive and calling accept().  If either probe fails (deleted file, stale
/// socket, hung listener), the daemon spawns a detached restart process and
/// shuts down.
///
/// To prevent restart loops when the underlying issue is systemic (e.g.
/// filesystem permissions, broken paths), the daemon only self-restarts if
/// it has been up for at least 60 seconds.  If sockets fail before that,
/// it shuts down without restart — the next wrapper invocation will attempt
/// to start a fresh daemon.
pub(super) fn daemon_socket_health_check_loop(
    coordinator: Arc<ActorDaemonCoordinator>,
    control_socket_path: PathBuf,
    trace_socket_path: PathBuf,
) {
    let started = std::time::Instant::now();
    let interval = daemon_socket_health_check_interval().max(1);
    tracing::info!(
        interval,
        control = %control_socket_path.display(),
        trace = %trace_socket_path.display(),
        "socket health check started"
    );

    loop {
        {
            let guard = coordinator
                .shutdown_condvar_mutex
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if coordinator.is_shutting_down() {
                return;
            }
            let _ = coordinator
                .shutdown_condvar
                .wait_timeout(guard, std::time::Duration::from_secs(interval));
        }

        if coordinator.is_shutting_down() {
            return;
        }

        let control_ok =
            local_socket_connects_with_timeout(&control_socket_path, DAEMON_SOCKET_PROBE_TIMEOUT);
        let trace_ok =
            local_socket_connects_with_timeout(&trace_socket_path, DAEMON_SOCKET_PROBE_TIMEOUT);

        if control_ok.is_err() || trace_ok.is_err() {
            let uptime = started.elapsed();
            let min_uptime = std::time::Duration::from_secs(daemon_min_uptime_for_self_restart());

            if uptime >= min_uptime {
                tracing::warn!(
                    control = %control_ok.err().map(|e| e.to_string()).unwrap_or_else(|| "ok".into()),
                    trace = %trace_ok.err().map(|e| e.to_string()).unwrap_or_else(|| "ok".into()),
                    "socket health check failed, spawning restart and shutting down"
                );
                if let Err(e) = spawn_self_restart() {
                    tracing::error!("failed to spawn self-restart: {}", e);
                }
            } else {
                tracing::warn!(
                    control = %control_ok.err().map(|e| e.to_string()).unwrap_or_else(|| "ok".into()),
                    trace = %trace_ok.err().map(|e| e.to_string()).unwrap_or_else(|| "ok".into()),
                    uptime_secs = uptime.as_secs(),
                    "socket health check failed within minimum uptime, shutting down without restart"
                );
            }
            coordinator.request_shutdown();
            return;
        }
    }
}

/// Background loop that periodically checks for available updates.
///
/// Sleeps in short increments so it can exit promptly when the coordinator
/// signals shutdown.  When an update is detected, it requests a graceful
/// shutdown so the daemon can self-update after draining in-flight work.
pub(super) fn daemon_update_check_loop(
    coordinator: Arc<ActorDaemonCoordinator>,
    started_at_ns: u128,
) {
    use crate::commands::upgrade::{DaemonUpdateCheckResult, check_for_update_available};

    let interval = daemon_update_check_interval().max(1);

    loop {
        {
            let guard = coordinator
                .shutdown_condvar_mutex
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if coordinator.is_shutting_down() {
                return;
            }
            let _ = coordinator
                .shutdown_condvar
                .wait_timeout(guard, std::time::Duration::from_secs(interval));
        }

        if coordinator.is_shutting_down() {
            return;
        }

        coordinator.gc_stale_family_state();

        match check_for_update_available() {
            Ok(DaemonUpdateCheckResult::UpdateReady) => {
                tracing::info!("update check: newer version available, requesting shutdown");
                coordinator.request_shutdown();
                return;
            }
            Ok(DaemonUpdateCheckResult::NoUpdate) => {
                tracing::info!("update check: no update needed");
            }
            Err(err) => {
                tracing::warn!(%err, "update check failed");
            }
        }

        let uptime_ns = now_unix_nanos().saturating_sub(started_at_ns);
        if uptime_ns >= daemon_max_uptime_ns() {
            tracing::info!("uptime exceeded max, requesting restart");
            coordinator.request_shutdown();
            return;
        }
    }
}

/// After the daemon has fully shut down, attempt to install any pending update.
///
/// On Unix the installer atomically replaces the binary via `mv`; on Windows
/// the installer is spawned as a detached process that polls until the exe is
/// unlocked.
pub(crate) fn daemon_run_pending_self_update() {
    use crate::commands::upgrade::{
        DaemonUpdateCheckResult, check_and_install_update_if_available,
    };

    match check_and_install_update_if_available() {
        Ok(DaemonUpdateCheckResult::UpdateReady) => {
            tracing::info!("self-update: installation completed successfully");
        }
        Ok(DaemonUpdateCheckResult::NoUpdate) => {
            tracing::info!("self-update: no update to install");
        }
        Err(err) => {
            tracing::warn!(%err, "self-update: installation failed");
        }
    }
}
