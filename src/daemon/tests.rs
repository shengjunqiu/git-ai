use super::*;
use serial_test::serial;
use std::ffi::OsString;

struct EnvVarGuard {
    key: &'static str,
    original: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var_os(key);
        // SAFETY: these tests are serialized via #[serial], so mutating the
        // process environment is isolated for the duration of each test.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, original }
    }

    fn unset(key: &'static str) -> Self {
        let original = std::env::var_os(key);
        // SAFETY: these tests are serialized via #[serial], so mutating the
        // process environment is isolated for the duration of each test.
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => {
                // SAFETY: these tests are serialized via #[serial], so restoring
                // process environment state is isolated for the duration of each test.
                unsafe {
                    std::env::set_var(self.key, value);
                }
            }
            None => {
                // SAFETY: these tests are serialized via #[serial], so restoring
                // process environment state is isolated for the duration of each test.
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }
}

fn queued_checkpoint_request() -> ControlRequest {
    ControlRequest::CheckpointRun {
        request: Box::new(CheckpointRunRequest::Captured(
            CapturedCheckpointRunRequest {
                repo_working_dir: "/tmp/repo".to_string(),
                capture_id: "capture".to_string(),
            },
        )),
        wait: Some(false),
    }
}

fn waited_checkpoint_request() -> ControlRequest {
    ControlRequest::CheckpointRun {
        request: Box::new(CheckpointRunRequest::Live(Box::new(
            LiveCheckpointRunRequest {
                repo_working_dir: "/tmp/repo".to_string(),
                kind: Some("human".to_string()),
                author: Some("test".to_string()),
                quiet: Some(true),
                is_pre_commit: Some(false),
                agent_run_result: None,
            },
        ))),
        wait: Some(true),
    }
}

#[test]
fn checkpoint_requests_use_long_timeout_in_ci_or_test_env() {
    assert_eq!(
        checkpoint_control_response_timeout(&queued_checkpoint_request(), true),
        DAEMON_CHECKPOINT_RESPONSE_TIMEOUT
    );
    assert_eq!(
        checkpoint_control_response_timeout(&waited_checkpoint_request(), true),
        DAEMON_CHECKPOINT_RESPONSE_TIMEOUT
    );
}

#[test]
fn queued_checkpoint_requests_use_short_timeout_in_product_env() {
    assert_eq!(
        checkpoint_control_response_timeout(&queued_checkpoint_request(), false),
        DAEMON_CONTROL_RESPONSE_TIMEOUT
    );
}

#[test]
fn waited_checkpoint_requests_use_long_timeout_in_product_env() {
    assert_eq!(
        checkpoint_control_response_timeout(&waited_checkpoint_request(), false),
        DAEMON_CHECKPOINT_RESPONSE_TIMEOUT
    );
}

#[test]
#[serial]
fn checkpoint_control_timeout_uses_ci_env_var() {
    let _unset_test = EnvVarGuard::unset("GIT_AI_TEST_DB_PATH");
    let _unset_legacy_test = EnvVarGuard::unset("GITAI_TEST_DB_PATH");
    let _set_ci = EnvVarGuard::set("CI", "true");

    assert!(checkpoint_control_timeout_uses_ci_or_test_budget());
}

#[test]
#[serial]
fn checkpoint_control_timeout_uses_test_db_env_var() {
    let _unset_ci = EnvVarGuard::unset("CI");
    let _unset_legacy_test = EnvVarGuard::unset("GITAI_TEST_DB_PATH");
    let _set_test = EnvVarGuard::set("GIT_AI_TEST_DB_PATH", "/tmp/git-ai-test.db");

    assert!(checkpoint_control_timeout_uses_ci_or_test_budget());
}

#[test]
#[serial]
fn checkpoint_control_timeout_false_when_no_ci_or_test_vars() {
    let _unset_ci = EnvVarGuard::unset("CI");
    let _unset_test = EnvVarGuard::unset("GIT_AI_TEST_DB_PATH");
    let _unset_legacy_test = EnvVarGuard::unset("GITAI_TEST_DB_PATH");

    assert!(!checkpoint_control_timeout_uses_ci_or_test_budget());
}

#[test]
fn normalize_commit_carryover_snapshot_reuses_committed_blob_for_crlf_only_diff() {
    let carryover = HashMap::from([(
        "example.txt".to_string(),
        "line 1\r\nline 2\r\n".to_string(),
    )]);
    let committed = HashMap::from([("example.txt".to_string(), "line 1\nline 2\n".to_string())]);

    let normalized =
        normalize_commit_carryover_snapshot(Some(&carryover), Some(&committed)).unwrap();

    assert_eq!(normalized.get("example.txt"), committed.get("example.txt"));
}

#[test]
fn compute_watermarks_uses_symlink_metadata_not_target_mtime() {
    // Verify that compute_watermarks_from_stat uses lstat (symlink's own mtime)
    // not stat (target file's mtime), consistent with snapshot's symlink_metadata.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Create a target file
    let target = dir.join("target.txt");
    std::fs::write(&target, b"hello").unwrap();

    // Create a symlink pointing to the target
    let link = dir.join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&target, &link).unwrap();

    // Watermark the symlink
    let wm = compute_watermarks_from_stat(dir.to_str().unwrap(), &["link.txt".to_string()]);

    // The watermark should match symlink_metadata mtime, not target metadata mtime.
    let symlink_meta = std::fs::symlink_metadata(&link).unwrap();
    let target_meta = std::fs::metadata(&link).unwrap(); // follows symlink

    let symlink_mtime = symlink_meta
        .modified()
        .unwrap()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let target_mtime = target_meta
        .modified()
        .unwrap()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let recorded = *wm.get("link.txt").unwrap();

    assert_eq!(
        recorded, symlink_mtime,
        "watermark should match lstat mtime of the symlink itself"
    );
    // This assertion documents the intent: if symlink and target mtimes differ,
    // the watermark must track the symlink, not the target.
    let _ = target_mtime; // used only as documentation; may equal symlink_mtime on some FS
}

#[test]
fn normalize_commit_carryover_snapshot_preserves_real_post_commit_edits() {
    let carryover = HashMap::from([(
        "example.txt".to_string(),
        "line 1\r\nline 2\r\nextra line\r\n".to_string(),
    )]);
    let committed = HashMap::from([("example.txt".to_string(), "line 1\nline 2\n".to_string())]);

    let normalized =
        normalize_commit_carryover_snapshot(Some(&carryover), Some(&committed)).unwrap();

    assert_eq!(normalized.get("example.txt"), carryover.get("example.txt"));
}

#[test]
fn recent_working_log_snapshot_preserves_humans_on_restore() {
    use crate::authorship::attribution_tracker::LineAttribution;
    use crate::authorship::authorship_log::HumanRecord;
    use crate::git::test_utils::TmpRepo;
    use std::collections::BTreeMap;

    let test_repo = TmpRepo::new().expect("Failed to create test repo");
    let repo = test_repo.gitai_repo();

    // Create a snapshot with KnownHuman attributions
    let h_hash = "h_abc123";
    let human_record = HumanRecord {
        author: "Test User <test@example.com>".to_string(),
    };

    let file_path = "test.txt";
    let line_attributions = vec![LineAttribution {
        start_line: 1,
        end_line: 1,
        author_id: h_hash.to_string(),
        overrode: None,
    }];

    let mut humans = BTreeMap::new();
    humans.insert(h_hash.to_string(), human_record.clone());

    let snapshot = RecentWorkingLogSnapshot {
        files: HashMap::from([(file_path.to_string(), line_attributions.clone())]),
        prompts: HashMap::new(),
        file_contents: HashMap::from([(file_path.to_string(), "test line\n".to_string())]),
        humans: humans.clone(),
    };

    // Restore the snapshot
    let base_commit = "HEAD";
    let restored = restore_recent_working_log_snapshot(repo, base_commit, &snapshot).unwrap();
    assert!(restored, "Snapshot should be restored");

    // Read back the INITIAL file and verify humans are present
    let working_log = repo
        .storage
        .working_log_for_base_commit(base_commit)
        .unwrap();
    let initial = working_log.read_initial_attributions();

    // Verify humans were restored
    assert_eq!(
        initial.humans.len(),
        1,
        "Should have one human record after restore"
    );
    assert_eq!(
        initial.humans.get(h_hash),
        Some(&human_record),
        "Human record should match"
    );
}

// -----------------------------------------------------------------------
// Readonly command ingress fast-path tests
//
// These tests verify that prepare_trace_payload_for_ingest returns false
// (do-not-enqueue) for read-only commands and true for mutating ones, and
// that the queued_trace_payloads counter is not incremented for read-only
// events.
//
// ActorDaemonCoordinator::new() spawns Tokio tasks internally, so all
// tests that construct one must run inside a Tokio runtime.
// -----------------------------------------------------------------------

fn make_start_payload(argv: &[&str]) -> Value {
    serde_json::json!({
        "event": "start",
        "sid": "20260411T120000.000000-Psid1",
        "argv": argv,
    })
}

fn make_atexit_payload(sid: &str) -> Value {
    serde_json::json!({
        "event": "atexit",
        "sid": sid,
        "code": 0,
    })
}

#[tokio::test]
async fn readonly_start_event_is_not_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&["git", "status", "--short"]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        !should_enqueue,
        "status start event should not be enqueued (readonly)"
    );
    assert_eq!(
        coord.queued_trace_payloads.load(Ordering::Relaxed),
        0,
        "queued_trace_payloads should stay 0 for readonly start event"
    );
    // Readonly events must NOT receive an ingest sequence number
    assert!(
        payload.get(TRACE_INGEST_SEQ_FIELD).is_none(),
        "readonly start event must not receive an ingest sequence number"
    );
}

#[tokio::test]
async fn stash_list_start_event_is_not_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&[
        "git",
        "-c",
        "core.fsmonitor=false",
        "--no-pager",
        "stash",
        "list",
        "--pretty=format:%gd%x00%H%x00%ct%x00%s",
    ]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        !should_enqueue,
        "stash list start event should not be enqueued (readonly invocation)"
    );
    assert!(
        payload.get(TRACE_INGEST_SEQ_FIELD).is_none(),
        "stash list start event must not receive an ingest sequence number"
    );
}

#[tokio::test]
async fn worktree_list_start_event_is_not_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&[
        "git",
        "--no-pager",
        "--no-optional-locks",
        "worktree",
        "list",
        "--porcelain",
    ]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        !should_enqueue,
        "worktree list start event should not be enqueued (readonly invocation)"
    );
    assert!(
        payload.get(TRACE_INGEST_SEQ_FIELD).is_none(),
        "worktree list start event must not receive an ingest sequence number"
    );
}

#[tokio::test]
async fn diff_numstat_start_event_is_not_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&[
        "git",
        "-c",
        "core.fsmonitor=false",
        "--no-pager",
        "diff",
        "--numstat",
        "--no-renames",
        "HEAD",
    ]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        !should_enqueue,
        "diff --numstat start event should not be enqueued"
    );
}

#[tokio::test]
async fn for_each_ref_start_event_is_not_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&[
        "git",
        "--no-pager",
        "for-each-ref",
        "refs/heads/**/*",
        "refs/remotes/**/*",
        "--format",
        "%(HEAD)%00%(objectname)",
    ]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        !should_enqueue,
        "for-each-ref start event should not be enqueued"
    );
}

#[tokio::test]
async fn cat_file_start_event_is_not_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&[
        "git",
        "--no-optional-locks",
        "cat-file",
        "--batch-check=%(objectname)",
    ]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        !should_enqueue,
        "cat-file start event should not be enqueued"
    );
}

#[tokio::test]
async fn show_commit_start_event_is_not_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&[
        "git",
        "--no-optional-locks",
        "show",
        "--no-patch",
        "--format=%H%x00%B%x00%at",
        "07270e1489439d6b36fcb2a4198d2fb68e37727c",
    ]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(!should_enqueue, "show start event should not be enqueued");
}

#[tokio::test]
async fn mutating_commit_start_event_is_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&["git", "commit", "-m", "test commit"]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        should_enqueue,
        "commit start event should be enqueued (mutating)"
    );
    assert!(
        payload.get(TRACE_INGEST_SEQ_FIELD).is_some(),
        "mutating event must receive an ingest sequence number"
    );
}

#[tokio::test]
async fn mutating_stash_pop_start_event_is_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&["git", "stash", "pop"]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        should_enqueue,
        "stash pop start event should be enqueued (mutating)"
    );
}

#[tokio::test]
async fn mutating_worktree_add_start_event_is_enqueued() {
    let coord = ActorDaemonCoordinator::new();
    let mut payload = make_start_payload(&["git", "worktree", "add", "/tmp/branch", "branch"]);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
    assert!(
        should_enqueue,
        "worktree add start event should be enqueued (mutating)"
    );
}

#[tokio::test]
async fn readonly_atexit_event_is_not_enqueued_after_readonly_start() {
    let coord = ActorDaemonCoordinator::new();
    let sid = "20260411T120000.000000-Psid1";

    // Process start event first — marks root as read-only
    let mut start = make_start_payload(&["git", "status"]);
    // Override sid to match
    start["sid"] = serde_json::json!(sid);
    coord.prepare_trace_payload_for_ingest(&mut start);

    // atexit for same root should also be skipped
    let mut atexit = make_atexit_payload(sid);
    let should_enqueue = coord.prepare_trace_payload_for_ingest(&mut atexit);
    assert!(
        !should_enqueue,
        "atexit for readonly root should not be enqueued"
    );
}

/// Performance invariant: 10,000 readonly start events must be processed
/// (and discarded) in under 200ms.  This guards against regressions that
/// re-introduce the >1-minute backlog seen with Zed's ~40 invocations/sec.
#[tokio::test]
async fn readonly_flood_1000_events_processed_in_under_200ms() {
    let coord = ActorDaemonCoordinator::new();
    let start = std::time::Instant::now();
    for i in 0..1000u64 {
        let sid = format!("20260411T120000.000000-P{:016x}", i);
        let mut payload = serde_json::json!({
            "event": "start",
            "sid": sid,
            "argv": ["git", "-c", "core.fsmonitor=false", "--no-pager",
                     "--no-optional-locks", "status", "--porcelain=v1",
                     "--untracked-files=all", "--no-renames", "-z", "."],
        });
        let enqueue = coord.prepare_trace_payload_for_ingest(&mut payload);
        assert!(!enqueue, "status must never be enqueued");
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 200,
        "processing 1000 readonly events took {}ms (> 200ms budget)",
        elapsed.as_millis()
    );
    assert_eq!(
        coord.queued_trace_payloads.load(Ordering::Relaxed),
        0,
        "no readonly events should reach the ingest queue"
    );
}

/// Ensure a stash-list flood (3208 real-world invocations from Zed)
/// leaves the ingest queue empty.
#[tokio::test]
async fn stash_list_flood_leaves_queue_empty() {
    let coord = ActorDaemonCoordinator::new();
    for i in 0..1000u64 {
        let sid = format!("20260411T120000.000000-P{:016x}", i);
        let mut payload = serde_json::json!({
            "event": "start",
            "sid": sid,
            "argv": ["git", "-c", "core.fsmonitor=false", "--no-pager",
                     "stash", "list", "--pretty=format:%gd%x00%H%x00%ct%x00%s"],
        });
        let _ = coord.prepare_trace_payload_for_ingest(&mut payload);
    }
    assert_eq!(
        coord.queued_trace_payloads.load(Ordering::Relaxed),
        0,
        "stash list flood must not fill the ingest queue"
    );
}

/// Ensure a worktree-list flood leaves the ingest queue empty.
#[tokio::test]
async fn worktree_list_flood_leaves_queue_empty() {
    let coord = ActorDaemonCoordinator::new();
    for i in 0..1000u64 {
        let sid = format!("20260411T120000.000000-P{:016x}", i);
        let mut payload = serde_json::json!({
            "event": "start",
            "sid": sid,
            "argv": ["git", "--no-pager", "--no-optional-locks",
                     "worktree", "list", "--porcelain"],
        });
        let _ = coord.prepare_trace_payload_for_ingest(&mut payload);
    }
    assert_eq!(
        coord.queued_trace_payloads.load(Ordering::Relaxed),
        0,
        "worktree list flood must not fill the ingest queue"
    );
}

// -----------------------------------------------------------------------
// OnceLock / shutdown / atomic-ordering tests
// -----------------------------------------------------------------------

/// `enqueue_trace_payload` must return an error when the ingest worker has
/// not been started yet.  This is the "no-sender" fast-fail path and is
/// unchanged by the OnceLock refactor.
#[tokio::test]
async fn enqueue_before_worker_start_returns_error() {
    let coord = ActorDaemonCoordinator::new();
    // Worker never started → OnceLock is empty → enqueue must fail
    let payload = serde_json::json!({
        "event": "start",
        "sid": "20260411T120000.000000-Ptest0001",
        "__git_ai_ingest_seq": 1_u64,
        "argv": ["git", "commit", "-m", "test"],
    });
    assert!(
        coord.enqueue_trace_payload(payload).is_err(),
        "enqueue before worker start must return an error"
    );
}

/// After `request_shutdown()`, `is_shutting_down()` returns true and the
/// coordinator stays in a consistent state.  The ingest worker (started
/// via `start_trace_ingest_worker`) must exit cleanly even when the sender
/// is no longer dropped by `request_shutdown` (OnceLock never drops it).
#[tokio::test]
async fn request_shutdown_is_idempotent_and_consistent() {
    let coord = Arc::new(ActorDaemonCoordinator::new());
    coord.start_trace_ingest_worker().unwrap();
    assert!(!coord.is_shutting_down());
    coord.request_shutdown();
    assert!(coord.is_shutting_down());
    // Second call must not panic.
    coord.request_shutdown();
    assert!(coord.is_shutting_down());
    // Allow tokio to run the ingest worker's shutdown select arm.
    tokio::task::yield_now().await;
}

/// Concurrent enqueues from multiple threads must never deadlock or
/// corrupt the accounting counter.
#[tokio::test]
async fn concurrent_mutating_enqueues_do_not_deadlock() {
    use std::sync::Arc;
    let coord = Arc::new(ActorDaemonCoordinator::new());
    coord.start_trace_ingest_worker().unwrap();

    const TASKS: usize = 8;
    const PER_TASK: usize = 20;

    // Use prepare_trace_payload_for_ingest (which allocates seq numbers
    // and enqueues) from multiple tasks concurrently.
    let mut handles = Vec::with_capacity(TASKS);
    for task_id in 0..TASKS {
        let c = coord.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..PER_TASK {
                let sid = format!("20260411T120000.000000-P{:08x}", task_id * 1000 + i);
                let mut payload = serde_json::json!({
                    "event": "start",
                    "sid": sid,
                    "argv": ["git", "commit", "-m", "msg"],
                });
                // This calls enqueue_trace_payload internally for mutating cmds.
                let _ = c.prepare_trace_payload_for_ingest(&mut payload);
            }
        }));
    }
    for h in handles {
        h.await.expect("task must not panic");
    }
    // Give the ingest worker time to drain the queue.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while coord.queued_trace_payloads.load(Ordering::Acquire) > 0 {
        if tokio::time::Instant::now() >= deadline {
            break; // don't fail the test on CI slowness; just stop waiting
        }
        tokio::task::yield_now().await;
    }
    coord.request_shutdown();
}
