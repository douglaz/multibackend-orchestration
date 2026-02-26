//! Integration-style tests for the daemon rebase agent conflict recovery loop.
//!
//! These tests create synthetic git repos with merge conflicts and use mock
//! `claude` executables to exercise the resolve/continue cycle.
//!
//! All tests call the public string-based entrypoint
//! `resolve_rebase_conflicts(path, target, backend_str, deadline)` which
//! handles backend parsing internally.
//!
//! All tests acquire a shared mutex before mutating the test-only claude-bin
//! override environment variable, ensuring deterministic execution even under
//! parallel test runners.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ralph::daemon::rebase_agent::{is_rebase_in_progress, resolve_rebase_conflicts};
use tempfile::TempDir;

/// Global mutex to serialize tests that mutate the process env override.
static CLAUDE_BIN_MUTEX: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn init_repo(path: &Path) {
    run_git(path, &["init"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test User"]);
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git {:?} failed in {}.\nstdout:\n{}\nstderr:\n{}",
        args,
        repo.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Create a repo where rebasing `feature` onto `master` produces a conflict
/// in `conflict.txt`. Returns the TempDir (with HEAD on `feature` mid-rebase).
fn create_conflict_repo() -> TempDir {
    let tmp = TempDir::new().expect("create tempdir");
    let repo = tmp.path();

    init_repo(repo);

    // Base commit
    fs::write(repo.join("conflict.txt"), "base\n").expect("write base");
    run_git(repo, &["add", "conflict.txt"]);
    run_git(repo, &["commit", "-m", "base"]);

    // Master diverges
    fs::write(repo.join("conflict.txt"), "master-change\n").expect("write master");
    run_git(repo, &["add", "conflict.txt"]);
    run_git(repo, &["commit", "-m", "master diverges"]);

    // Feature branch diverges from base
    run_git(repo, &["checkout", "-b", "feature", "HEAD~1"]);
    fs::write(repo.join("conflict.txt"), "feature-change\n").expect("write feature");
    run_git(repo, &["add", "conflict.txt"]);
    run_git(repo, &["commit", "-m", "feature diverges"]);

    // Start rebase (will conflict)
    let output = Command::new("git")
        .args(["rebase", "master"])
        .current_dir(repo)
        .output()
        .expect("run git rebase");
    assert!(
        !output.status.success(),
        "expected rebase to fail with conflict"
    );

    tmp
}

/// Create a repo where rebasing `feature` onto `master` produces conflicts
/// across two commits. Returns the TempDir mid-rebase.
fn create_multi_commit_conflict_repo() -> TempDir {
    let tmp = TempDir::new().expect("create tempdir");
    let repo = tmp.path();

    init_repo(repo);

    // Base commit
    fs::write(repo.join("file_a.txt"), "base-a\n").expect("write base-a");
    fs::write(repo.join("file_b.txt"), "base-b\n").expect("write base-b");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "base"]);

    // Master diverges on both files
    fs::write(repo.join("file_a.txt"), "master-a\n").expect("write master-a");
    fs::write(repo.join("file_b.txt"), "master-b\n").expect("write master-b");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "master diverges"]);

    // Feature branch: two commits, each touching a different conflicting file
    run_git(repo, &["checkout", "-b", "feature", "HEAD~1"]);
    fs::write(repo.join("file_a.txt"), "feature-a\n").expect("write feature-a");
    run_git(repo, &["add", "file_a.txt"]);
    run_git(repo, &["commit", "-m", "feature commit 1"]);

    fs::write(repo.join("file_b.txt"), "feature-b\n").expect("write feature-b");
    run_git(repo, &["add", "file_b.txt"]);
    run_git(repo, &["commit", "-m", "feature commit 2"]);

    // Start rebase (will conflict on first commit)
    let output = Command::new("git")
        .args(["rebase", "master"])
        .current_dir(repo)
        .output()
        .expect("run git rebase");
    assert!(
        !output.status.success(),
        "expected rebase to fail with conflict"
    );

    tmp
}

/// Write a mock `claude` script to a per-test bin directory inside the tempdir.
/// Returns the executable path.
fn write_mock_claude(tmp: &TempDir, script_content: &str) -> PathBuf {
    let bin_dir = tmp.path().join("mock-bin");
    fs::create_dir_all(&bin_dir).expect("create mock-bin dir");
    let claude_path = bin_dir.join("claude");
    fs::write(&claude_path, script_content).expect("write mock claude");
    let mut perms = fs::metadata(&claude_path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&claude_path, perms).expect("set permissions");

    claude_path
}

/// Run a closure with the mock claude binary path injected via env override.
/// Acquires the global mutex for safety and restores env on exit.
fn with_mock_claude_bin<F, R>(mock_claude_bin: &Path, f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = CLAUDE_BIN_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let original = std::env::var("RALPH_REBASE_AGENT_CLAUDE_BIN").ok();
    unsafe {
        std::env::set_var(
            "RALPH_REBASE_AGENT_CLAUDE_BIN",
            mock_claude_bin.to_string_lossy().as_ref(),
        )
    };
    let result = f();
    if let Some(value) = original {
        unsafe { std::env::set_var("RALPH_REBASE_AGENT_CLAUDE_BIN", value) };
    } else {
        unsafe { std::env::remove_var("RALPH_REBASE_AGENT_CLAUDE_BIN") };
    }
    result
}

// ---------------------------------------------------------------------------
// Tests — using string-based public entrypoint
// ---------------------------------------------------------------------------

#[test]
fn successful_conflict_recovery() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    // Mock claude that resolves the conflict and stages
    let script = format!(
        "#!/bin/sh\necho 'resolved-content' > {}/conflict.txt\ngit -C {} add conflict.txt\n",
        repo.display(),
        repo.display(),
    );
    let mock_claude = write_mock_claude(&tmp, &script);

    let deadline = Instant::now() + Duration::from_secs(30);

    let result = with_mock_claude_bin(&mock_claude, || {
        resolve_rebase_conflicts(repo, "master", "claude(opus)", deadline)
    });

    assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    assert!(
        !is_rebase_in_progress(repo),
        "rebase should be fully complete after successful recovery"
    );
}

#[test]
fn multi_commit_conflict_recovery() {
    let tmp = create_multi_commit_conflict_repo();
    let repo = tmp.path();

    // Mock claude that resolves whatever conflict exists by writing resolved content
    // and staging all conflicting files
    let script = format!(
        r#"#!/bin/sh
# Resolve all conflicting files
for f in $(git -C {repo} diff --name-only --diff-filter=U); do
    echo "resolved" > {repo}/$f
    git -C {repo} add "$f"
done
"#,
        repo = repo.display(),
    );
    let mock_claude = write_mock_claude(&tmp, &script);

    let deadline = Instant::now() + Duration::from_secs(30);

    let result = with_mock_claude_bin(&mock_claude, || {
        resolve_rebase_conflicts(repo, "master", "claude(opus)", deadline)
    });

    assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    assert!(
        !is_rebase_in_progress(repo),
        "rebase should be fully complete after multi-commit recovery"
    );
}

#[test]
fn agent_non_zero_exit_aborts_rebase() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    // Mock claude that exits with code 1
    let script = "#!/bin/sh\nexit 1\n";
    let mock_claude = write_mock_claude(&tmp, script);

    let deadline = Instant::now() + Duration::from_secs(30);

    let result = with_mock_claude_bin(&mock_claude, || {
        resolve_rebase_conflicts(repo, "master", "claude(opus)", deadline)
    });

    assert!(result.is_err(), "expected error on non-zero agent exit");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("non-zero"),
        "error should mention non-zero exit: {err}"
    );
    assert!(
        !is_rebase_in_progress(repo),
        "rebase should be aborted after agent failure"
    );
}

#[test]
fn agent_success_without_resolution_fails() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    // Mock claude that exits 0 but doesn't actually resolve anything
    let script = "#!/bin/sh\nexit 0\n";
    let mock_claude = write_mock_claude(&tmp, script);

    let deadline = Instant::now() + Duration::from_secs(30);

    let result = with_mock_claude_bin(&mock_claude, || {
        resolve_rebase_conflicts(repo, "master", "claude(opus)", deadline)
    });

    assert!(result.is_err(), "expected error when conflicts remain");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("conflicts remain unresolved"),
        "error should mention unresolved conflicts: {err}"
    );
    assert!(
        !is_rebase_in_progress(repo),
        "rebase should be aborted after unresolved conflicts"
    );
}

#[test]
fn agent_timeout_fails() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    // Mock claude that sleeps longer than the deadline
    let script = "#!/bin/sh\nsleep 60\n";
    let mock_claude = write_mock_claude(&tmp, script);

    // Give only 2 seconds deadline (enough for git status but not for sleep 60)
    let deadline = Instant::now() + Duration::from_secs(2);

    let result = with_mock_claude_bin(&mock_claude, || {
        resolve_rebase_conflicts(repo, "master", "claude(opus)", deadline)
    });

    assert!(result.is_err(), "expected timeout error");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("timed out") || err.contains("timeout"),
        "error should mention timeout: {err}"
    );
    assert!(
        !is_rebase_in_progress(repo),
        "rebase should be aborted after timeout"
    );
}

#[test]
fn none_backend_string_returns_error() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    let deadline = Instant::now() + Duration::from_secs(30);
    let result = resolve_rebase_conflicts(repo, "master", "none", deadline);

    assert!(result.is_err(), "none backend should return error");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("none") && err.contains("skipped/disabled"),
        "error should indicate agent was skipped/disabled: {err}"
    );
    // After none-backend, rebase should be aborted
    assert!(
        !is_rebase_in_progress(repo),
        "rebase should be aborted in none-backend path"
    );
}

#[test]
fn invalid_backend_string_returns_error() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    let deadline = Instant::now() + Duration::from_secs(30);
    let result = resolve_rebase_conflicts(repo, "master", "codex(gpt-5)", deadline);

    assert!(result.is_err(), "unsupported backend should return error");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("unsupported"),
        "error should mention unsupported backend: {err}"
    );
}

#[test]
fn claude_shorthand_backend_string_works() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    // Mock claude that resolves the conflict
    let script = format!(
        "#!/bin/sh\necho 'resolved-content' > {}/conflict.txt\ngit -C {} add conflict.txt\n",
        repo.display(),
        repo.display(),
    );
    let mock_claude = write_mock_claude(&tmp, &script);

    let deadline = Instant::now() + Duration::from_secs(30);

    // Use shorthand "claude" (no model parenthetical — should default to opus)
    let result = with_mock_claude_bin(&mock_claude, || {
        resolve_rebase_conflicts(repo, "master", "claude", deadline)
    });

    assert!(
        result.is_ok(),
        "shorthand 'claude' backend should work, got: {:?}",
        result.err()
    );
    assert!(!is_rebase_in_progress(repo), "rebase should be complete");
}

#[test]
fn normalized_trimmed_none_backend_returns_skipped_error() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    let deadline = Instant::now() + Duration::from_secs(30);
    // Use " none " with extra whitespace — should be trimmed and treated as disabled
    let result = resolve_rebase_conflicts(repo, "master", " none ", deadline);

    assert!(result.is_err(), "trimmed none backend should return error");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("skipped/disabled"),
        "error should indicate agent was skipped/disabled: {err}"
    );
    assert!(
        !is_rebase_in_progress(repo),
        "rebase should be aborted in trimmed none-backend path"
    );
}

#[test]
fn agent_attempted_error_includes_attempted_wording() {
    let tmp = create_conflict_repo();
    let repo = tmp.path();

    // Mock claude that exits non-zero
    let script = "#!/bin/sh\nexit 1\n";
    let mock_claude = write_mock_claude(&tmp, script);

    let deadline = Instant::now() + Duration::from_secs(30);

    let result = with_mock_claude_bin(&mock_claude, || {
        resolve_rebase_conflicts(repo, "master", "claude(opus)", deadline)
    });

    assert!(result.is_err(), "expected error on non-zero agent exit");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("agent resolution was attempted"),
        "agent-attempted error should contain 'agent resolution was attempted': {err}"
    );
}
