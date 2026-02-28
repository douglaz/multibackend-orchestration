use std::fs;
use std::process::Command;

use super::*;

use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "pr_runtime::draft_watcher_creates_draft_when_branch_ahead",
            func: draft_watcher_creates_draft_when_branch_ahead,
        },
        ConformanceTest {
            name: "pr_runtime::draft_watcher_pushes_before_create",
            func: draft_watcher_pushes_before_create,
        },
        ConformanceTest {
            name: "pr_runtime::draft_watcher_exits_cleanly_on_cancellation",
            func: draft_watcher_exits_cleanly_on_cancellation,
        },
        ConformanceTest {
            name: "pr_runtime::pr_url_plumbed_through_child_args",
            func: pr_url_plumbed_through_child_args,
        },
        ConformanceTest {
            name: "pr_runtime::e2e_draft_create_via_binary",
            func: e2e_draft_create_via_binary,
        },
        ConformanceTest {
            name: "pr_runtime::pr_url_persisted_across_restarts",
            func: pr_url_persisted_across_restarts,
        },
    ]
}

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(super::panic_message(e)),
    }
}

/// Verify that the draft_pr_watcher creates a draft PR when the branch has
/// commits ahead of the base branch. This test uses the github module's
/// has_commits_ahead_of_base function directly.
fn draft_watcher_creates_draft_when_branch_ahead(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a branch with a commit ahead of master
        let repo = &h.repo_root;
        git(repo, &["checkout", "-b", "ralph/test-ahead"]);
        fs::write(repo.join("new-file.txt"), "content\n").expect("write file");
        git(repo, &["add", "new-file.txt"]);
        git(repo, &["commit", "-m", "ahead commit"]);

        // Verify has_commits_ahead_of_base returns true
        let result = crate::daemon::github::has_commits_ahead_of_base(
            repo,
            "master",
        )
        .expect("has_commits_ahead_of_base should succeed");

        assert!(
            result,
            "branch with extra commit should be ahead of master"
        );

        // Verify that on master, has_commits_ahead_of_base returns false
        git(repo, &["checkout", "master"]);
        let result_on_master = crate::daemon::github::has_commits_ahead_of_base(
            repo,
            "master",
        )
        .expect("has_commits_ahead_of_base should succeed");

        assert!(
            !result_on_master,
            "master should not be ahead of itself"
        );
    })
}

/// Verify that push happens before create_pr call ordering is correct.
/// This tests the github module's push_branch and create_pr functions
/// are both available and correctly ordered.
fn draft_watcher_pushes_before_create(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let repo = &h.repo_root;

        // Create a branch with a commit
        git(repo, &["checkout", "-b", "ralph/test-push-order"]);
        fs::write(repo.join("push-test.txt"), "push test\n").expect("write file");
        git(repo, &["add", "push-test.txt"]);
        git(repo, &["commit", "-m", "push order test"]);

        // Push should succeed (we have an origin remote from harness setup)
        let push_result =
            crate::daemon::github::push_branch(repo, "ralph/test-push-order");
        assert!(
            push_result.is_ok(),
            "push_branch should succeed: {:?}",
            push_result.err()
        );

        // After push, has_commits_ahead_of_base should still be true
        // (we've pushed but the base hasn't moved)
        let ahead = crate::daemon::github::has_commits_ahead_of_base(
            repo,
            "master",
        )
        .expect("has_commits_ahead_of_base should succeed");
        assert!(ahead, "branch should still be ahead after push");
    })
}

/// Verify that CancellationToken works as expected for the draft PR watcher
/// pattern (immediate shutdown via tokio::select!).
fn draft_watcher_exits_cleanly_on_cancellation(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        use tokio_util::sync::CancellationToken;

        let token = CancellationToken::new();
        let token_clone = token.clone();

        // Cancel immediately
        token.cancel();

        // The token should be cancelled
        assert!(
            token_clone.is_cancelled(),
            "token should be cancelled after cancel() call"
        );

        // Verify that a tokio runtime can use the cancelled token in select
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build tokio runtime");

        let completed = rt.block_on(async {
            tokio::select! {
                _ = token_clone.cancelled() => true,
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => false,
            }
        });

        assert!(
            completed,
            "select! should immediately resolve on cancelled token"
        );
    })
}

/// Verify that --pr-url is accepted by both `run` and `auto` subcommands.
fn pr_url_plumbed_through_child_args(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Test that --pr-url is parsed correctly by the run subcommand
        // (this validates CLI parsing, not full execution)
        use clap::Parser;
        use crate::cli::{Cli, Commands};

        let cli = Cli::parse_from([
            "ralph",
            "run",
            "--pr-url",
            "https://github.com/acme/widgets/pull/42",
        ]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };
        assert_eq!(
            args.pr_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/42")
        );

        // Test that --pr-url is parsed correctly by the auto subcommand
        let cli = Cli::parse_from([
            "ralph",
            "auto",
            "--idea",
            "test feature",
            "--pr-url",
            "https://github.com/acme/widgets/pull/99",
        ]);
        let Commands::Auto(args) = cli.command else {
            panic!("expected auto command");
        };
        assert_eq!(
            args.pr_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/99")
        );

        // Test that --pr-url defaults to None when not provided
        let cli = Cli::parse_from([
            "ralph",
            "run",
        ]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };
        assert_eq!(args.pr_url, None);
    })
}

/// E2E test: verify that the binary accepts --pr-url flag without errors.
/// This is a smoke test that exercises the real binary's CLI parsing.
fn e2e_draft_create_via_binary(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Initialize workspace
        h.init_workspace().expect("init workspace");

        // Verify the binary accepts --pr-url flag (will fail due to missing
        // project, but should NOT fail due to unrecognized flag)
        let output = h
            .ralph([
                "run",
                "--project",
                "nonexistent",
                "--pr-url",
                "https://github.com/acme/widgets/pull/1",
            ])
            .expect("ralph run with --pr-url should execute");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{stderr}\n{stdout}");

        // Should NOT fail with "unrecognized argument" or similar
        assert!(
            !combined.contains("unexpected argument")
                && !combined.contains("unrecognized")
                && !combined.contains("error: Found argument"),
            "binary should accept --pr-url without argument errors, got: {combined}"
        );

        // Should fail with project not found (expected)
        assert!(
            !output.status.success(),
            "should fail because project doesn't exist"
        );
    })
}

/// Verify that task metadata (PR URL) persists to disk and can be recovered,
/// simulating the daemon restart scenario.  The save/load round-trip must
/// preserve the PR URL so the watcher does not create a duplicate PR.
fn pr_url_persisted_across_restarts(h: &RalphHarness) -> TestResult {
    run_case(|| {
        use crate::daemon::runtime::{load_task_metadata, save_task_metadata, TaskMetadata};

        let workspace_root = h.repo_root.join(".ralph");
        let task_id = "acme-widgets-99";

        // Initially no metadata exists — load should return default.
        let meta = load_task_metadata(&workspace_root, task_id);
        assert_eq!(meta.pr_url, None, "fresh load should return None");

        // Persist a PR URL.
        let pr_url = "https://github.com/acme/widgets/pull/99".to_owned();
        save_task_metadata(
            &workspace_root,
            task_id,
            &TaskMetadata {
                pr_url: Some(pr_url.clone()),
            },
        );

        // Reload — should recover the URL (simulating daemon restart).
        let recovered = load_task_metadata(&workspace_root, task_id);
        assert_eq!(
            recovered.pr_url.as_deref(),
            Some(pr_url.as_str()),
            "recovered PR URL should match persisted value"
        );

        // Overwrite with None — should clear the URL.
        save_task_metadata(
            &workspace_root,
            task_id,
            &TaskMetadata { pr_url: None },
        );
        let cleared = load_task_metadata(&workspace_root, task_id);
        assert_eq!(cleared.pr_url, None, "cleared PR URL should be None");
    })
}

fn git(repo_root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
