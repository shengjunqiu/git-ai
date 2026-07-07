use crate::repos::test_repo::TestRepo;
use git_ai::git::{find_repository_in_path, sync_authorship::push_authorship_notes};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

const NOTES_PUSH_STRESS_CLIENTS: usize = 6;

fn unique_temp_path(prefix: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{seq}", std::process::id()))
}

fn clone_from_upstream(upstream: &TestRepo, prefix: &str) -> TestRepo {
    let clone_path = unique_temp_path(prefix);
    let upstream_path = upstream.path().to_string_lossy().to_string();
    let clone_path_string = clone_path.to_string_lossy().to_string();

    upstream
        .git_og(&["clone", upstream_path.as_str(), clone_path_string.as_str()])
        .expect("additional clone should succeed");

    TestRepo::new_at_path(clone_path.as_path())
}

fn commit_file(repo: &TestRepo, file_name: &str, contents: &str, message: &str) -> String {
    fs::write(repo.path().join(file_name), contents).expect("should write test file");
    repo.git_og(&["add", file_name])
        .expect("git add should succeed");
    repo.git_og(&["commit", "-m", message])
        .expect("git commit should succeed");
    repo.git_og(&["rev-parse", "HEAD"])
        .expect("rev-parse HEAD should succeed")
        .trim()
        .to_string()
}

fn add_ai_note(repo: &TestRepo, commit: &str, note: &str) {
    repo.git_og(&["notes", "--ref=ai", "add", "-f", "-m", note, commit])
        .expect("AI note add should succeed");
}

fn push_branch_head(repo: &TestRepo) {
    repo.git_og(&["push", "origin", "HEAD"])
        .expect("branch push should succeed");
}

fn show_remote_note(upstream: &TestRepo, commit: &str) -> String {
    upstream
        .git_og(&["notes", "--ref=ai", "show", commit])
        .expect("remote should contain note")
}

fn assert_remote_note_contains(upstream: &TestRepo, commit: &str, expected: &str) {
    let note = show_remote_note(upstream, commit);
    assert!(
        note.contains(expected),
        "remote note should contain {expected:?}, got: {note}"
    );
}

fn push_notes_concurrently(repos: Vec<&TestRepo>) {
    let resolved_repos = repos
        .iter()
        .map(|repo| {
            find_repository_in_path(repo.path().to_string_lossy().as_ref())
                .expect("repository should resolve")
        })
        .collect::<Vec<_>>();

    let push_barrier = Arc::new(Barrier::new(resolved_repos.len()));
    let handles = resolved_repos
        .into_iter()
        .enumerate()
        .map(|(index, repo)| {
            let push_barrier = Arc::clone(&push_barrier);
            std::thread::spawn(move || {
                push_barrier.wait();
                push_authorship_notes(&repo, "origin")
                    .map_err(|error| format!("client {index} notes push failed: {error}"))
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle
            .join()
            .expect("notes push thread should not panic")
            .expect("notes push should succeed");
    }
}

/// Extract the JSON object from combined stdout+stderr output.
/// The JSON is written to stdout, but test infra combines stdout and stderr.
fn extract_json(output: &str) -> serde_json::Value {
    for line in output.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
        {
            return val;
        }
    }
    panic!("no valid JSON object found in output:\n{}", output);
}

#[test]
fn test_fetch_notes_no_remote_notes() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    fs::write(mirror.path().join("hello.txt"), "hello\n").expect("should write file");
    mirror
        .stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    let output = mirror
        .git_ai(&["fetch-notes"])
        .expect("fetch-notes should succeed");
    assert!(
        output.contains("no notes found on remote"),
        "expected 'no notes found' message, got: {}",
        output
    );
}

#[test]
fn test_fetch_notes_with_explicit_remote() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    fs::write(mirror.path().join("hello.txt"), "hello\n").expect("should write file");
    mirror
        .stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    let output = mirror
        .git_ai(&["fetch-notes", "origin"])
        .expect("fetch-notes with remote should succeed");
    assert!(
        output.contains("no notes found on remote"),
        "expected 'no notes found' message, got: {}",
        output
    );
}

#[test]
fn test_fetch_notes_with_remote_flag() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    fs::write(mirror.path().join("hello.txt"), "hello\n").expect("should write file");
    mirror
        .stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    let output = mirror
        .git_ai(&["fetch-notes", "--remote", "origin"])
        .expect("fetch-notes --remote should succeed");
    assert!(
        output.contains("no notes found on remote"),
        "expected 'no notes found' message, got: {}",
        output
    );
}

#[test]
fn test_fetch_notes_json_output_no_notes() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    fs::write(mirror.path().join("hello.txt"), "hello\n").expect("should write file");
    mirror
        .stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    let output = mirror
        .git_ai(&["fetch-notes", "--json"])
        .expect("fetch-notes --json should succeed");

    let parsed = extract_json(&output);
    assert_eq!(parsed["status"], "not_found");
    assert_eq!(parsed["remote"], "origin");
}

#[test]
fn test_fetch_notes_json_output_with_notes() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    fs::write(mirror.path().join("hello.txt"), "hello\n").expect("should write file");
    mirror
        .stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    // Create a note on the commit and push it to the bare upstream.
    // Use -f in case git-ai hooks already created a note on this commit.
    mirror
        .git_og(&[
            "notes",
            "--ref=ai",
            "add",
            "-f",
            "-m",
            "test authorship note",
            "HEAD",
        ])
        .expect("should add note");
    mirror
        .git_og(&["push", "origin", "refs/notes/ai"])
        .expect("should push notes to upstream");

    // Remove the local note so fetch actually has something to pull
    mirror
        .git_og(&["update-ref", "-d", "refs/notes/ai"])
        .expect("should delete local note ref");

    let output = mirror
        .git_ai(&["fetch-notes", "--json"])
        .expect("fetch-notes --json should succeed");

    let parsed = extract_json(&output);
    assert_eq!(parsed["status"], "found");
    assert_eq!(parsed["remote"], "origin");
}

#[test]
fn test_notes_push_merges_independent_clone_notes() {
    let (clone_a, upstream) = TestRepo::new_with_remote();

    fs::write(clone_a.path().join("seed.txt"), "seed\n").expect("should write seed file");
    clone_a
        .git_og(&["add", "seed.txt"])
        .expect("seed add should succeed");
    clone_a
        .git_og(&["commit", "-m", "seed"])
        .expect("seed commit should succeed");
    clone_a
        .git_og(&["push", "-u", "origin", "HEAD"])
        .expect("seed push should succeed");

    let clone_b_path = unique_temp_path("notes-merge-clone-b");
    let upstream_path = upstream.path().to_string_lossy().to_string();
    let clone_b_path_string = clone_b_path.to_string_lossy().to_string();
    clone_a
        .git_og(&[
            "clone",
            upstream_path.as_str(),
            clone_b_path_string.as_str(),
        ])
        .expect("second clone should succeed");
    let clone_b = TestRepo::new_at_path(clone_b_path.as_path());

    fs::write(clone_a.path().join("a.txt"), "from clone a\n").expect("should write a file");
    clone_a
        .git_og(&["add", "a.txt"])
        .expect("clone a add should succeed");
    clone_a
        .git_og(&["commit", "-m", "clone a"])
        .expect("clone a commit should succeed");
    let commit_a = clone_a
        .git_og(&["rev-parse", "HEAD"])
        .expect("clone a rev-parse should succeed")
        .trim()
        .to_string();
    clone_a
        .git_og(&["push", "origin", "HEAD"])
        .expect("clone a branch push should succeed");

    clone_b
        .git_og(&["pull", "--ff-only", "origin", "main"])
        .expect("clone b pull should succeed");
    fs::write(clone_b.path().join("b.txt"), "from clone b\n").expect("should write b file");
    clone_b
        .git_og(&["add", "b.txt"])
        .expect("clone b add should succeed");
    clone_b
        .git_og(&["commit", "-m", "clone b"])
        .expect("clone b commit should succeed");
    let commit_b = clone_b
        .git_og(&["rev-parse", "HEAD"])
        .expect("clone b rev-parse should succeed")
        .trim()
        .to_string();
    clone_b
        .git_og(&["push", "origin", "HEAD"])
        .expect("clone b branch push should succeed");

    clone_a
        .git_og(&[
            "notes",
            "--ref=ai",
            "add",
            "-f",
            "-m",
            "note from clone a",
            commit_a.as_str(),
        ])
        .expect("clone a note add should succeed");
    clone_b
        .git_og(&[
            "notes",
            "--ref=ai",
            "add",
            "-f",
            "-m",
            "note from clone b",
            commit_b.as_str(),
        ])
        .expect("clone b note add should succeed");

    let repo_a = find_repository_in_path(clone_a.path().to_string_lossy().as_ref())
        .expect("clone a repository should resolve");
    let repo_b = find_repository_in_path(clone_b.path().to_string_lossy().as_ref())
        .expect("clone b repository should resolve");

    let push_barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let push_a = {
        let push_barrier = std::sync::Arc::clone(&push_barrier);
        std::thread::spawn(move || {
            push_barrier.wait();
            push_authorship_notes(&repo_a, "origin")
        })
    };
    let push_b = {
        let push_barrier = std::sync::Arc::clone(&push_barrier);
        std::thread::spawn(move || {
            push_barrier.wait();
            push_authorship_notes(&repo_b, "origin")
        })
    };

    push_a
        .join()
        .expect("clone a notes push thread should not panic")
        .expect("clone a notes push should succeed");
    push_b
        .join()
        .expect("clone b notes push thread should not panic")
        .expect("clone b notes push should merge and succeed");

    let note_a = upstream
        .git_og(&["notes", "--ref=ai", "show", commit_a.as_str()])
        .expect("remote should contain clone a note");
    let note_b = upstream
        .git_og(&["notes", "--ref=ai", "show", commit_b.as_str()])
        .expect("remote should contain clone b note");

    assert!(
        note_a.contains("note from clone a"),
        "remote note for clone a should be preserved, got: {note_a}"
    );
    assert!(
        note_b.contains("note from clone b"),
        "remote note for clone b should be preserved, got: {note_b}"
    );
}

#[test]
fn test_notes_push_merges_independent_clone_notes_when_remote_ref_exists() {
    let (clone_a, upstream) = TestRepo::new_with_remote();

    let seed_commit = commit_file(&clone_a, "seed.txt", "seed\n", "seed");
    clone_a
        .git_og(&["push", "-u", "origin", "HEAD"])
        .expect("seed push should succeed");
    add_ai_note(
        &clone_a,
        seed_commit.as_str(),
        "seed note already on remote",
    );
    clone_a
        .git_og(&["push", "origin", "refs/notes/ai"])
        .expect("seed notes push should succeed");

    let clone_b = clone_from_upstream(&upstream, "notes-existing-ref-clone-b");

    let commit_a = commit_file(&clone_a, "a.txt", "from clone a\n", "clone a");
    push_branch_head(&clone_a);

    clone_b
        .git_og(&["pull", "--ff-only", "origin", "main"])
        .expect("clone b pull should succeed");
    let commit_b = commit_file(&clone_b, "b.txt", "from clone b\n", "clone b");
    push_branch_head(&clone_b);

    add_ai_note(
        &clone_a,
        commit_a.as_str(),
        "note from clone a after remote ref exists",
    );
    add_ai_note(
        &clone_b,
        commit_b.as_str(),
        "note from clone b after remote ref exists",
    );

    push_notes_concurrently(vec![&clone_a, &clone_b]);

    assert_remote_note_contains(
        &upstream,
        seed_commit.as_str(),
        "seed note already on remote",
    );
    assert_remote_note_contains(
        &upstream,
        commit_a.as_str(),
        "note from clone a after remote ref exists",
    );
    assert_remote_note_contains(
        &upstream,
        commit_b.as_str(),
        "note from clone b after remote ref exists",
    );
}

#[test]
fn test_notes_push_conflicting_same_commit_note_local_wins() {
    let (clone_a, upstream) = TestRepo::new_with_remote();

    let commit = commit_file(&clone_a, "shared.txt", "shared\n", "shared");
    clone_a
        .git_og(&["push", "-u", "origin", "HEAD"])
        .expect("shared commit push should succeed");

    let clone_b = clone_from_upstream(&upstream, "notes-conflict-clone-b");

    add_ai_note(&clone_a, commit.as_str(), "remote version from clone a");
    clone_a
        .git_og(&["push", "origin", "refs/notes/ai"])
        .expect("remote note push should succeed");

    add_ai_note(&clone_b, commit.as_str(), "local version from clone b");
    let repo_b = find_repository_in_path(clone_b.path().to_string_lossy().as_ref())
        .expect("clone b repository should resolve");
    push_authorship_notes(&repo_b, "origin").expect("conflicting local note push should succeed");

    let final_note = show_remote_note(&upstream, commit.as_str());
    assert!(
        final_note.contains("local version from clone b"),
        "local pusher's note should win same-commit conflicts, got: {final_note}"
    );
    assert!(
        !final_note.contains("remote version from clone a"),
        "remote version should be replaced by the local pusher's note, got: {final_note}"
    );
}

#[test]
#[ignore = "stress test for concurrent notes pushes; run before releases"]
fn test_notes_push_many_clients_stress() {
    let (clone_a, upstream) = TestRepo::new_with_remote();

    let seed_commit = commit_file(&clone_a, "seed.txt", "seed\n", "seed");
    clone_a
        .git_og(&["push", "-u", "origin", "HEAD"])
        .expect("seed push should succeed");
    add_ai_note(&clone_a, seed_commit.as_str(), "seed note for stress test");
    clone_a
        .git_og(&["push", "origin", "refs/notes/ai"])
        .expect("seed notes push should succeed");

    let mut clones = vec![clone_a];
    for index in 1..NOTES_PUSH_STRESS_CLIENTS {
        clones.push(clone_from_upstream(
            &upstream,
            &format!("notes-stress-clone-{index}"),
        ));
    }

    let mut commits = Vec::new();
    for (index, repo) in clones.iter().enumerate() {
        repo.git_og(&["pull", "--ff-only", "origin", "main"])
            .expect("client pull should succeed");
        let commit = commit_file(
            repo,
            &format!("client-{index}.txt"),
            &format!("from client {index}\n"),
            &format!("client {index}"),
        );
        push_branch_head(repo);
        add_ai_note(
            repo,
            commit.as_str(),
            &format!("stress note from client {index}"),
        );
        commits.push(commit);
    }

    push_notes_concurrently(clones.iter().collect());

    assert_remote_note_contains(&upstream, seed_commit.as_str(), "seed note for stress test");
    for (index, commit) in commits.iter().enumerate() {
        assert_remote_note_contains(
            &upstream,
            commit.as_str(),
            &format!("stress note from client {index}"),
        );
    }
}

#[test]
fn test_fetch_notes_help_flag() {
    let repo = TestRepo::new();

    let output = repo
        .git_ai(&["fetch-notes", "--help"])
        .expect("fetch-notes --help should succeed");
    assert!(
        output.contains("Synchronously fetch AI authorship notes"),
        "help output should contain description, got: {}",
        output
    );
}

#[test]
fn test_fetch_notes_unknown_option_fails() {
    let repo = TestRepo::new();

    let err = repo
        .git_ai(&["fetch-notes", "--invalid-flag"])
        .expect_err("unknown option should fail");
    assert!(
        err.contains("unknown option"),
        "error should mention unknown option, got: {}",
        err
    );
}

#[test]
fn test_fetch_notes_human_output_with_notes() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    fs::write(mirror.path().join("hello.txt"), "hello\n").expect("should write file");
    mirror
        .stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    // Create a note and push it. Use -f in case hooks already created one.
    mirror
        .git_og(&["notes", "--ref=ai", "add", "-f", "-m", "test note", "HEAD"])
        .expect("should add note");
    mirror
        .git_og(&["push", "origin", "refs/notes/ai"])
        .expect("should push notes");
    mirror
        .git_og(&["update-ref", "-d", "refs/notes/ai"])
        .expect("should delete local note ref");

    let output = mirror
        .git_ai(&["fetch-notes"])
        .expect("fetch-notes should succeed");
    assert!(
        output.contains("done"),
        "expected 'done' message, got: {}",
        output
    );
}

#[test]
fn test_fetch_notes_remote_missing_value_fails() {
    let repo = TestRepo::new();

    let err = repo
        .git_ai(&["fetch-notes", "--remote"])
        .expect_err("--remote without value should fail");
    assert!(
        err.contains("--remote requires a value"),
        "error should mention missing value, got: {}",
        err
    );
}

#[test]
fn test_fetch_notes_duplicate_remote_fails() {
    let repo = TestRepo::new();

    let err = repo
        .git_ai(&["fetch-notes", "origin", "--remote", "upstream"])
        .expect_err("duplicate remote should fail");
    assert!(
        err.contains("remote specified more than once"),
        "error should mention duplicate remote, got: {}",
        err
    );
}

#[test]
fn test_fetch_notes_json_error_includes_remote() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    fs::write(mirror.path().join("hello.txt"), "hello\n").expect("should write file");
    mirror
        .stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    // Remove the origin remote so the fetch fails
    mirror
        .git_og(&["remote", "remove", "origin"])
        .expect("should remove origin");

    let err = mirror
        .git_ai(&["fetch-notes", "--json", "--remote", "nonexistent"])
        .expect_err("fetch from nonexistent remote should fail");

    // Error JSON should include the remote name we passed
    let parsed = extract_json(&err);
    assert_eq!(parsed["status"], "fetch_failed");
    assert_eq!(parsed["remote"], "nonexistent");
    assert!(parsed["error"].as_str().is_some());
}
