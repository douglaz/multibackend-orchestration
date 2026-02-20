use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ralph::validate::harness::RalphHarness;
use ralph::validate::mock_scripts::standard_mock_script;

fn ralph_bin_absolute() -> PathBuf {
    if let Ok(path) = env::var("CARGO_BIN_EXE_ralph") {
        return PathBuf::from(path);
    }

    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set");
    let candidate = PathBuf::from(manifest).join("target").join("debug").join("ralph");
    assert!(
        candidate.exists(),
        "ralph binary not found at {}",
        candidate.display()
    );
    candidate
}

fn assert_exit_code(output: &Output, expected: i32) {
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(
        code,
        expected,
        "unexpected exit code.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_ok(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command should execute");
    assert!(
        output.status.success(),
        "git command failed: git {}\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command should execute");
    assert!(
        output.status.success(),
        "git command failed: git {}\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn setup_with_standard_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    h.setup_mock_backends(&script)
        .expect("setup_mock_backends failed");
    h.create_project(project_id, "Rollback Test Project", "Rollback test prompt")
        .expect("create_project failed");
    h.ralph_ok(["run", "--loops", "2"])
        .expect("run --loops 2 should succeed");
}

fn setup_remote_tracking(h: &RalphHarness, project_id: &str) -> PathBuf {
    let remote_path = h.temp_dir.path().join("origin.git");
    git_ok(&h.repo_root, &["init", "--bare", remote_path.to_string_lossy().as_ref()]);
    git_ok(&h.repo_root, &["remote", "add", "origin", remote_path.to_string_lossy().as_ref()]);
    git_ok(&h.repo_root, &["push", "-u", "origin", "master"]);

    let project_branch = format!("ralph/{project_id}");
    git_ok(&h.repo_root, &["checkout", &project_branch]);
    git_ok(
        &h.repo_root,
        &["push", "-u", "origin", project_branch.as_str()],
    );

    remote_path
}

#[test]
fn soft_rollback_writes_marker_without_git_history_mutation() {
    let harness = RalphHarness::new(ralph_bin_absolute()).expect("create harness");
    let project_id = "soft-marker";
    setup_with_standard_mock(&harness, project_id);

    let project_dir = harness.project_dir(project_id);
    let marker_path = project_dir.join(".rollback-target");
    assert!(!marker_path.exists(), "marker should not exist before rollback");

    let head_before = git_output(&harness.repo_root, &["rev-parse", "HEAD"]);
    let output = harness
        .ralph(["rollback", "1"])
        .expect("rollback command should execute");
    assert_exit_code(&output, 0);
    let head_after = git_output(&harness.repo_root, &["rev-parse", "HEAD"]);

    assert_eq!(head_after, head_before, "soft rollback should not rewrite HEAD");
    let marker = fs::read_to_string(&marker_path).expect("marker should be written");
    assert_eq!(marker.trim(), "1");

    let state = harness.load_state(project_id).expect("load state");
    let loops = state["loops"].as_array().expect("loops should be an array");
    assert_eq!(loops.len(), 1, "expected one loop after soft rollback");
}

#[test]
fn hard_rollback_deletes_marker_on_success() {
    let harness = RalphHarness::new(ralph_bin_absolute()).expect("create harness");
    let project_id = "hard-marker-delete";
    setup_with_standard_mock(&harness, project_id);
    setup_remote_tracking(&harness, project_id);

    harness
        .ralph_ok(["rollback", "1"])
        .expect("soft rollback should succeed");
    let marker_path = harness.project_dir(project_id).join(".rollback-target");
    assert!(marker_path.exists(), "soft rollback should create marker");

    let tag_name = format!("ralph/{project_id}/loop-1");
    let loop1_commit = git_output(&harness.repo_root, &["rev-parse", &tag_name]);

    let output = harness
        .ralph(["rollback", "--hard", "1"])
        .expect("hard rollback command should execute");
    assert_exit_code(&output, 0);
    assert!(
        !marker_path.exists(),
        "hard rollback success should delete marker"
    );

    let head = git_output(&harness.repo_root, &["rev-parse", "HEAD"]);
    assert_eq!(head, loop1_commit, "hard rollback should reset local HEAD");

    let project_branch = format!("refs/heads/ralph/{project_id}");
    let remote = git_output(
        &harness.repo_root,
        &["ls-remote", "--heads", "origin", &project_branch],
    );
    let remote_commit = remote
        .split_whitespace()
        .next()
        .expect("remote branch should exist")
        .to_owned();
    assert_eq!(
        remote_commit, loop1_commit,
        "hard rollback should force-push upstream branch"
    );
}

#[test]
fn hard_rollback_push_failure_reverts_head_and_writes_soft_fallback_marker() {
    let harness = RalphHarness::new(ralph_bin_absolute()).expect("create harness");
    let project_id = "hard-push-failure";
    setup_with_standard_mock(&harness, project_id);
    setup_remote_tracking(&harness, project_id);

    let original_head = git_output(&harness.repo_root, &["rev-parse", "HEAD"]);
    let broken_remote = harness.temp_dir.path().join("missing").join("origin.git");
    git_ok(
        &harness.repo_root,
        &[
            "remote",
            "set-url",
            "origin",
            broken_remote.to_string_lossy().as_ref(),
        ],
    );

    let output = harness
        .ralph(["rollback", "--hard", "1"])
        .expect("hard rollback command should execute");
    assert_exit_code(&output, 1);

    let head_after = git_output(&harness.repo_root, &["rev-parse", "HEAD"]);
    assert_eq!(
        head_after, original_head,
        "failed hard push should restore local HEAD to original commit"
    );

    let marker_path = harness.project_dir(project_id).join(".rollback-target");
    let marker = fs::read_to_string(&marker_path).expect("fallback should write marker");
    assert_eq!(marker.trim(), "1");

    let state = harness.load_state(project_id).expect("load state after fallback");
    let loops = state["loops"].as_array().expect("loops should be an array");
    assert_eq!(loops.len(), 1, "fallback cleanup should keep only loop 1");
    assert_eq!(
        state["current_phase"].as_str().unwrap_or_default(),
        "planning",
        "fallback cleanup should reset phase to planning"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("soft fallback rollback"),
        "expected soft fallback error context, got:\n{stderr}"
    );
}

#[test]
fn rollback_dry_run_has_zero_side_effects_and_mode_specific_output() {
    let harness = RalphHarness::new(ralph_bin_absolute()).expect("create harness");
    let project_id = "dry-run";
    setup_with_standard_mock(&harness, project_id);

    let project_dir = harness.project_dir(project_id);
    let state_path = project_dir.join("state.json");
    let marker_path = project_dir.join(".rollback-target");
    let loop2_dir = harness.loop_dir(project_id, 2).expect("loop_dir");
    assert!(loop2_dir.is_some(), "loop 2 artifacts should exist before dry-run");

    let head_before = git_output(&harness.repo_root, &["rev-parse", "HEAD"]);
    let state_before = fs::read_to_string(&state_path).expect("read state before dry-run");
    assert!(
        !marker_path.exists(),
        "marker should not exist before dry-run checks"
    );

    let soft = harness
        .ralph(["rollback", "--dry-run", "1"])
        .expect("soft dry-run should execute");
    assert_exit_code(&soft, 0);
    let soft_stdout = String::from_utf8_lossy(&soft.stdout);
    assert!(soft_stdout.contains("soft rollback"));
    assert!(soft_stdout.contains("no git reset/push"));
    assert!(soft_stdout.contains(".rollback-target"));

    let hard = harness
        .ralph(["rollback", "--hard", "--dry-run", "1"])
        .expect("hard dry-run should execute");
    assert_exit_code(&hard, 0);
    let hard_stdout = String::from_utf8_lossy(&hard.stdout);
    assert!(hard_stdout.contains("hard rollback"));
    assert!(hard_stdout.contains("force-push"));

    let head_after = git_output(&harness.repo_root, &["rev-parse", "HEAD"]);
    let state_after = fs::read_to_string(&state_path).expect("read state after dry-run");
    assert_eq!(
        head_after, head_before,
        "dry-run should not mutate git history"
    );
    assert_eq!(
        state_after, state_before,
        "dry-run should not mutate state.json"
    );
    assert!(
        !marker_path.exists(),
        "dry-run should not write or delete marker"
    );
    assert!(
        harness.loop_dir(project_id, 2).expect("loop_dir").is_some(),
        "dry-run should not delete loop artifacts"
    );
}

#[test]
fn successful_new_checkpoint_commit_removes_stale_marker() {
    let harness = RalphHarness::new(ralph_bin_absolute()).expect("create harness");
    let project_id = "marker-clears-after-commit";
    harness.init_workspace().expect("init failed");
    let script = harness
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    harness
        .setup_mock_backends(&script)
        .expect("setup_mock_backends failed");
    harness
        .create_project(project_id, "Rollback Test Project", "Rollback test prompt")
        .expect("create_project failed");
    harness
        .ralph_ok(["run", "--loops", "1"])
        .expect("run --loops 1 should succeed");

    let marker_path = harness.project_dir(project_id).join(".rollback-target");
    fs::write(&marker_path, "1\n").expect("write stale marker");

    harness
        .ralph_ok(["run", "--loops", "1"])
        .expect("run should succeed and produce a new checkpoint commit");
    assert!(
        !marker_path.exists(),
        "successful checkpoint commit should remove stale rollback marker"
    );
}

#[test]
fn failed_checkpoint_attempt_keeps_marker() {
    let harness = RalphHarness::new(ralph_bin_absolute()).expect("create harness");
    let project_id = "marker-kept-on-commit-failure";
    harness.init_workspace().expect("init failed");
    let script = harness
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    harness
        .setup_mock_backends(&script)
        .expect("setup_mock_backends failed");
    harness
        .create_project(project_id, "Rollback Test Project", "Rollback test prompt")
        .expect("create_project failed");
    harness
        .ralph_ok(["run", "--loops", "1"])
        .expect("run --loops 1 should succeed");

    let marker_path = harness.project_dir(project_id).join(".rollback-target");
    fs::write(&marker_path, "1\n").expect("write stale marker");

    let hook_path = harness.repo_root.join(".git").join("hooks").join("pre-commit");
    fs::write(&hook_path, "#!/bin/sh\nexit 1\n").expect("write pre-commit hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path).expect("stat pre-commit hook").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms).expect("chmod pre-commit hook");
    }

    let output = harness
        .ralph(["run", "--loops", "1"])
        .expect("run command should execute");
    assert_exit_code(&output, 1);
    assert!(
        marker_path.exists(),
        "failed checkpoint attempt should not remove rollback marker"
    );
}
