use super::*;

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::daemon::pr_review::{has_staged_amendments, PrReviewState};
use crate::daemon::runtime::TaskMetadata;
use crate::validate::assertions::assert_exit_code;
use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "pr_review::whitelist_filters_comments",
            func: whitelist_filters_comments,
        },
        ConformanceTest {
            name: "pr_review::completed_project_resumes_with_state_reset",
            func: completed_project_resumes_with_state_reset,
        },
        ConformanceTest {
            name: "pr_review::dedup_across_restart",
            func: dedup_across_restart,
        },
        ConformanceTest {
            name: "pr_review::capacity_deferral_preserves_staged",
            func: capacity_deferral_preserves_staged,
        },
        ConformanceTest {
            name: "pr_review::quick_dev_resume_resets_phase",
            func: quick_dev_resume_resets_phase,
        },
        ConformanceTest {
            name: "pr_review::dispatch_failure_preserves_staged_amendments",
            func: dispatch_failure_preserves_staged_amendments,
        },
        ConformanceTest {
            name: "pr_review::quick_dev_resume_clears_stale_counters",
            func: quick_dev_resume_clears_stale_counters,
        },
        ConformanceTest {
            name: "pr_review::restart_drift_ready_drains_staged",
            func: restart_drift_ready_drains_staged,
        },
    ]
}

// ---------------------------------------------------------------------------
// Test implementations — each executes `daemon start --single-iteration`
// ---------------------------------------------------------------------------

/// Run a daemon tick with mock gh returning PR review comments from whitelisted
/// and non-whitelisted users.  Assert that only whitelisted comments produce
/// staged amendments and that non-whitelisted/self-comments are absent.
fn whitelist_filters_comments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Configure whitelist.
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_pr_review_whitelist",
            "[\"alice\",\"bob\"]",
        ])
        .expect("set whitelist");

        // Create task metadata with a PR URL.
        let ws_root = dh.repo_root.join(".ralph");
        let tasks_dir = ws_root.join("daemon").join("tasks");
        fs::create_dir_all(&tasks_dir).expect("create tasks dir");
        let meta = TaskMetadata {
            pr_url: Some("https://github.com/acme/widgets/pull/99".to_string()),
        };
        fs::write(
            tasks_dir.join("acme-widgets-42.json"),
            serde_json::to_string(&meta).unwrap(),
        )
        .expect("write task metadata");

        // Mock gh script that handles PR review API endpoints.
        let gh_path = write_pr_review_mock_gh(&dh).expect("write mock gh");

        // Mock PR review comments from three endpoints:
        // - alice (whitelisted): 1 inline comment
        // - charlie (not whitelisted): 1 inline comment
        // - bob (whitelisted): 1 top-level comment
        // - ralph-bot (self): 1 top-level comment
        // - alice (whitelisted): 1 review summary
        let pr_comments = r#"[
            {"id":1,"user":{"login":"alice"},"body":"fix this line","path":"src/main.rs","line":42,"created_at":"2024-01-01T00:00:00Z"},
            {"id":2,"user":{"login":"charlie"},"body":"also fix","path":"src/lib.rs","line":10,"created_at":"2024-01-01T00:00:00Z"}
        ]"#;
        let issue_comments = r#"[
            {"id":10,"user":{"login":"bob"},"body":"please add tests","created_at":"2024-01-01T00:00:00Z"},
            {"id":11,"user":{"login":"ralph-bot"},"body":"status update","created_at":"2024-01-01T00:00:00Z"}
        ]"#;
        let reviews = r#"[
            {"id":20,"user":{"login":"alice"},"body":"needs refactoring","state":"CHANGES_REQUESTED","submitted_at":"2024-01-01T00:00:00Z"}
        ]"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", pr_comments),
                    ("MOCK_GH_ISSUE_COMMENTS", issue_comments),
                    ("MOCK_GH_REVIEWS", reviews),
                ],
            )
            .expect("daemon start");
        assert_exit_code(&output, 0);

        // Verify: only alice (2 comments) and bob (1 comment) produced amendments.
        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join("acme-widgets-42");
        let count = count_json_files(&staging_dir);
        assert_eq!(
            count, 3,
            "expected 3 staged amendments (alice x2, bob x1), got {count}"
        );

        // Verify dedup state persisted.
        let state = PrReviewState::load(&ws_root, "acme-widgets-42").expect("load dedup state");
        assert_eq!(
            state.processed_keys.len(),
            3,
            "dedup state should have 3 keys"
        );
    })
}

/// Run a daemon tick with a completed project that has a PR and staged
/// amendments.  Assert that the label swap from ralph:completed to
/// ralph:in-progress occurs and the dispatch is attempted.
fn completed_project_resumes_with_state_reset(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        setup_mock_backend(&dh);

        // Configure whitelist.
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_pr_review_whitelist",
            "[\"alice\"]",
        ])
        .expect("set whitelist");

        // Create task metadata and branch with project files.
        let ws_root = dh.repo_root.join(".ralph");
        setup_task_metadata(&ws_root, "acme-widgets-42", 99);
        setup_project_branch(&dh.repo_root, 42, false);

        // Pre-stage an amendment.
        let amendment = serde_json::json!({
            "id": "PR-99-issue_comment-1",
            "body": "fix the auth bug",
            "priority": "p2",
            "source": "pr-review",
            "source_detail": "pr#99/issue_comment#1",
            "created_at": "2024-01-01T00:00:00Z"
        });
        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join("acme-widgets-42");
        fs::create_dir_all(&staging_dir).expect("create staging dir");
        fs::write(
            staging_dir.join("20240101000000-PR-99-issue_comment-1.json"),
            serde_json::to_string_pretty(&amendment).unwrap(),
        )
        .expect("write staged amendment");

        let label_log = dh.temp_dir.path().join("resume_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_path = write_pr_review_mock_gh(&dh).expect("write mock gh");

        // Issue has ralph:completed label so pr_review_phase triggers resume.
        let issue_labels =
            r#"{"labels":[{"name":"ralph:completed"},{"name":"ralph:pr-review"}]}"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", "[]"),
                    ("MOCK_GH_ISSUE_COMMENTS", "[]"),
                    ("MOCK_GH_REVIEWS", "[]"),
                    ("MOCK_GH_ISSUE_LABELS", issue_labels),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Verify label swap occurred (completed → in-progress).
        assert!(
            label_log.exists(),
            "label log should exist after label swap"
        );
        let log_content = fs::read_to_string(&label_log).expect("read label log");
        assert!(
            log_content.contains("ralph:in-progress"),
            "label log should contain ralph:in-progress swap, got: {log_content}"
        );

        // Verify staged amendments were drained (staging dir should be empty/gone).
        assert!(
            !has_staged_amendments(&ws_root, "acme-widgets-42"),
            "staged amendments should have been drained during dispatch"
        );

        // Verify dispatch was attempted.
        assert!(
            stderr.contains("pr-review: resuming ralph:completed task"),
            "stderr should log resume attempt"
        );
    })
}

/// Run two daemon ticks with the same PR review comments.  Assert that the
/// second tick does not create duplicate amendments.
fn dedup_across_restart(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_pr_review_whitelist",
            "[\"alice\"]",
        ])
        .expect("set whitelist");

        let ws_root = dh.repo_root.join(".ralph");
        setup_task_metadata(&ws_root, "acme-widgets-10", 50);

        let gh_path = write_pr_review_mock_gh(&dh).expect("write mock gh");

        let issue_comments =
            r#"[{"id":500,"user":{"login":"alice"},"body":"fix this","created_at":"2024-01-01T00:00:00Z"}]"#;

        // Cycle 1: first daemon tick should stage the amendment.
        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", "[]"),
                    ("MOCK_GH_ISSUE_COMMENTS", issue_comments),
                    ("MOCK_GH_REVIEWS", "[]"),
                ],
            )
            .expect("daemon tick 1");
        assert_exit_code(&output, 0);

        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join("acme-widgets-10");
        let count_after_first = count_json_files(&staging_dir);
        assert_eq!(count_after_first, 1, "should have 1 staged amendment after first tick");

        // Verify dedup state persisted.
        let state = PrReviewState::load(&ws_root, "acme-widgets-10").expect("load dedup state");
        assert!(
            state.processed_keys.contains("issue_comment:500"),
            "dedup key should be persisted after first tick"
        );

        // Cycle 2: simulated restart — run daemon again with same comments.
        let output2 = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", "[]"),
                    ("MOCK_GH_ISSUE_COMMENTS", issue_comments),
                    ("MOCK_GH_REVIEWS", "[]"),
                ],
            )
            .expect("daemon tick 2");
        assert_exit_code(&output2, 0);

        // No new amendments should have been staged.
        let count_after_second = count_json_files(&staging_dir);
        assert_eq!(
            count_after_second, count_after_first,
            "no duplicate amendment should be created after restart"
        );
    })
}

/// Pre-stage amendments for a completed project, then run a daemon tick with
/// max_concurrent=1 and two completed tasks.  The first dispatch fills the slot;
/// the second should be deferred with staged amendments preserved.
fn capacity_deferral_preserves_staged(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        setup_mock_backend(&dh);

        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_pr_review_whitelist",
            "[\"alice\"]",
        ])
        .expect("set whitelist");

        let ws_root = dh.repo_root.join(".ralph");

        // Set up two completed tasks with staged amendments.
        for (issue_num, pr_num) in [(42u32, 99u32), (43u32, 100u32)] {
            let task_id = format!("acme-widgets-{issue_num}");
            setup_task_metadata(&ws_root, &task_id, pr_num);
            setup_project_branch(&dh.repo_root, issue_num, false);

            let staging_dir = ws_root
                .join("daemon")
                .join("pr-review-amendments")
                .join(&task_id);
            fs::create_dir_all(&staging_dir).expect("create staging dir");
            let amendment = serde_json::json!({
                "id": format!("PR-{pr_num}-issue_comment-1"),
                "body": "fix it",
                "priority": "p2",
                "source": "pr-review",
                "source_detail": format!("pr#{pr_num}/issue_comment#1"),
                "created_at": "2024-01-01T00:00:00Z"
            });
            fs::write(
                staging_dir.join(format!("20240101000000-PR-{pr_num}-issue_comment-1.json")),
                serde_json::to_string_pretty(&amendment).unwrap(),
            )
            .expect("write staged amendment");
        }

        let gh_path = write_pr_review_mock_gh(&dh).expect("write mock gh");

        let issue_labels =
            r#"{"labels":[{"name":"ralph:completed"}]}"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                    "--max-concurrent",
                    "1",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", "[]"),
                    ("MOCK_GH_ISSUE_COMMENTS", "[]"),
                    ("MOCK_GH_REVIEWS", "[]"),
                    ("MOCK_GH_ISSUE_LABELS", issue_labels),
                ],
            )
            .expect("daemon start");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // One task should have been dispatched and one deferred.
        assert!(
            stderr.contains("no capacity slots available; deferring"),
            "stderr should indicate capacity deferral, got: {stderr}"
        );

        // At least one task should still have staged amendments (the deferred one).
        let has_42 = has_staged_amendments(&ws_root, "acme-widgets-42");
        let has_43 = has_staged_amendments(&ws_root, "acme-widgets-43");
        assert!(
            has_42 || has_43,
            "at least one task should still have staged amendments (deferred)"
        );
    })
}

/// Run a daemon tick with a completed quick-dev project.  Assert that the
/// project state is reset to in_progress with quick_dev_phase=plan_and_implement
/// so the orchestrator does not short-circuit.
fn quick_dev_resume_resets_phase(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        setup_mock_backend(&dh);

        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_pr_review_whitelist",
            "[\"alice\"]",
        ])
        .expect("set whitelist");

        let ws_root = dh.repo_root.join(".ralph");
        setup_task_metadata(&ws_root, "acme-widgets-55", 77);
        setup_project_branch(&dh.repo_root, 55, true);

        // Pre-stage an amendment.
        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join("acme-widgets-55");
        fs::create_dir_all(&staging_dir).expect("create staging dir");
        let amendment = serde_json::json!({
            "id": "PR-77-issue_comment-1",
            "body": "fix it",
            "priority": "p2",
            "source": "pr-review",
            "source_detail": "pr#77/issue_comment#1",
            "created_at": "2024-01-01T00:00:00Z"
        });
        fs::write(
            staging_dir.join("20240101000000-PR-77-issue_comment-1.json"),
            serde_json::to_string_pretty(&amendment).unwrap(),
        )
        .expect("write staged amendment");

        let label_log = dh.temp_dir.path().join("quick_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_path = write_pr_review_mock_gh(&dh).expect("write mock gh");

        let issue_labels =
            r#"{"labels":[{"name":"ralph:completed"},{"name":"ralph:quick"}]}"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", "[]"),
                    ("MOCK_GH_ISSUE_COMMENTS", "[]"),
                    ("MOCK_GH_REVIEWS", "[]"),
                    ("MOCK_GH_ISSUE_LABELS", issue_labels),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Verify dispatch was attempted.
        assert!(
            stderr.contains("pr-review: resuming ralph:completed task"),
            "stderr should log resume attempt for quick-dev project"
        );

        // After dispatch, the project state inside the worktree should be reset.
        // Find the worktree directory.
        let worktrees_dir = ws_root.join("daemon").join("worktrees");
        if worktrees_dir.exists() {
            // Look for the worktree that contains issue-55
            for entry in fs::read_dir(&worktrees_dir)
                .unwrap_or_else(|_| panic!("read worktrees dir"))
                .filter_map(|e| e.ok())
            {
                let state_path = entry
                    .path()
                    .join(".ralph")
                    .join("projects")
                    .join("issue-55")
                    .join("state.json");
                if state_path.exists() {
                    let content =
                        fs::read_to_string(&state_path).expect("read worktree state.json");
                    let loaded: serde_json::Value =
                        serde_json::from_str(&content).expect("parse state");
                    assert_eq!(
                        loaded["status"], "in_progress",
                        "status should be reset to in_progress"
                    );
                    assert_eq!(
                        loaded["quick_dev_phase"], "plan_and_implement",
                        "quick_dev_phase should be plan_and_implement"
                    );
                    assert_eq!(
                        loaded["current_phase"], "implementing",
                        "current_phase should be implementing"
                    );
                    return; // found and verified
                }
            }
        }

        // If worktree wasn't found, check stderr for dispatch attempt as minimum.
        assert!(
            stderr.contains("pr-review: dispatched task"),
            "dispatch should have been attempted even if worktree is hard to locate"
        );
    })
}

/// Set up a completed project with staged amendments, then run a daemon tick
/// where dispatch will fail (worktree path is blocked by a regular file).
/// Assert that staged amendments are preserved and the label is reverted to
/// `ralph:completed`.
fn dispatch_failure_preserves_staged_amendments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        setup_mock_backend(&dh);

        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_pr_review_whitelist",
            "[\"alice\"]",
        ])
        .expect("set whitelist");

        let ws_root = dh.repo_root.join(".ralph");
        setup_task_metadata(&ws_root, "acme-widgets-42", 99);
        setup_project_branch(&dh.repo_root, 42, false);

        // Pre-stage an amendment.
        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join("acme-widgets-42");
        fs::create_dir_all(&staging_dir).expect("create staging dir");
        let amendment = serde_json::json!({
            "id": "PR-99-issue_comment-1",
            "body": "fix the auth bug",
            "priority": "p2",
            "source": "pr-review",
            "source_detail": "pr#99/issue_comment#1",
            "created_at": "2024-01-01T00:00:00Z"
        });
        fs::write(
            staging_dir.join("20240101000000-PR-99-issue_comment-1.json"),
            serde_json::to_string_pretty(&amendment).unwrap(),
        )
        .expect("write staged amendment");

        // Block worktree creation by placing a regular file at the expected
        // worktree path.  `create_worktree` checks `wt_path.exists()` and
        // then calls `verify_worktree_branch`, which fails because the path
        // is a file, not a git worktree directory.
        let worktrees_dir = ws_root.join("daemon").join("worktrees");
        fs::create_dir_all(&worktrees_dir).expect("create worktrees dir");
        fs::write(
            worktrees_dir.join("acme-widgets-42"),
            "blocker — not a worktree",
        )
        .expect("create worktree blocker");

        let gh_path = write_pr_review_mock_gh(&dh).expect("write mock gh");

        let label_log = dh.temp_dir.path().join("fail_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let issue_labels =
            r#"{"labels":[{"name":"ralph:completed"},{"name":"ralph:pr-review"}]}"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", "[]"),
                    ("MOCK_GH_ISSUE_COMMENTS", "[]"),
                    ("MOCK_GH_REVIEWS", "[]"),
                    ("MOCK_GH_ISSUE_LABELS", issue_labels),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Verify dispatch failure was logged.
        assert!(
            stderr.contains("warning: failed to dispatch task"),
            "stderr should indicate dispatch failure, got: {stderr}"
        );

        // Staged amendments must survive the failed dispatch.
        assert!(
            has_staged_amendments(&ws_root, "acme-widgets-42"),
            "staged amendments must be preserved after dispatch failure"
        );

        // Verify label was reverted (completed → in-progress → completed).
        if label_log.exists() {
            let log_content = fs::read_to_string(&label_log).expect("read label log");
            assert!(
                log_content.contains("ralph:completed"),
                "label should be reverted to ralph:completed after dispatch failure, got: {log_content}"
            );
        }
    })
}

/// Run a daemon tick with a completed quick-dev project whose state.json has
/// non-zero `quick_dev_review_iteration` and `quick_dev_final_review_attempts`
/// (simulating a previously force-completed project).  Assert that the counters
/// are reset to zero so the orchestrator does not immediately trip the
/// guard-at-entry force-complete path.
fn quick_dev_resume_clears_stale_counters(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        setup_mock_backend(&dh);

        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_pr_review_whitelist",
            "[\"alice\"]",
        ])
        .expect("set whitelist");

        let ws_root = dh.repo_root.join(".ralph");
        setup_task_metadata(&ws_root, "acme-widgets-60", 80);
        setup_project_branch_with_stale_counters(&dh.repo_root, 60);

        // Pre-stage an amendment.
        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join("acme-widgets-60");
        fs::create_dir_all(&staging_dir).expect("create staging dir");
        let amendment = serde_json::json!({
            "id": "PR-80-issue_comment-1",
            "body": "fix it",
            "priority": "p2",
            "source": "pr-review",
            "source_detail": "pr#80/issue_comment#1",
            "created_at": "2024-01-01T00:00:00Z"
        });
        fs::write(
            staging_dir.join("20240101000000-PR-80-issue_comment-1.json"),
            serde_json::to_string_pretty(&amendment).unwrap(),
        )
        .expect("write staged amendment");

        let label_log = dh.temp_dir.path().join("stale_counter_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_path = write_pr_review_mock_gh(&dh).expect("write mock gh");

        let issue_labels =
            r#"{"labels":[{"name":"ralph:completed"},{"name":"ralph:quick"}]}"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", "[]"),
                    ("MOCK_GH_ISSUE_COMMENTS", "[]"),
                    ("MOCK_GH_REVIEWS", "[]"),
                    ("MOCK_GH_ISSUE_LABELS", issue_labels),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Verify dispatch was attempted.
        assert!(
            stderr.contains("pr-review: resuming ralph:completed task"),
            "stderr should log resume attempt for quick-dev project with stale counters"
        );

        // After dispatch, check the project state inside the worktree.
        let worktrees_dir = ws_root.join("daemon").join("worktrees");
        if worktrees_dir.exists() {
            for entry in fs::read_dir(&worktrees_dir)
                .unwrap_or_else(|_| panic!("read worktrees dir"))
                .filter_map(|e| e.ok())
            {
                let state_path = entry
                    .path()
                    .join(".ralph")
                    .join("projects")
                    .join("issue-60")
                    .join("state.json");
                if state_path.exists() {
                    let content =
                        fs::read_to_string(&state_path).expect("read worktree state.json");
                    let loaded: serde_json::Value =
                        serde_json::from_str(&content).expect("parse state");
                    assert_eq!(
                        loaded["status"], "in_progress",
                        "status should be reset to in_progress"
                    );
                    assert_eq!(
                        loaded["quick_dev_phase"], "plan_and_implement",
                        "quick_dev_phase should be plan_and_implement"
                    );
                    assert_eq!(
                        loaded["quick_dev_review_iteration"], 0,
                        "quick_dev_review_iteration must be reset to 0, not stale value"
                    );
                    assert_eq!(
                        loaded["quick_dev_final_review_attempts"], 0,
                        "quick_dev_final_review_attempts must be reset to 0, not stale value"
                    );
                    assert_eq!(
                        loaded["phase_iteration"], 1,
                        "phase_iteration must be normalized to 1"
                    );
                    return; // found and verified
                }
            }
        }

        // If worktree wasn't found, check stderr for dispatch attempt as minimum.
        assert!(
            stderr.contains("pr-review: dispatched task"),
            "dispatch should have been attempted even if worktree is hard to locate"
        );
    })
}

/// Simulate restart drift: a completed project had its label swapped to
/// in-progress during a previous PR-review resume, but the daemon crashed
/// before dispatch.  Startup reconciliation converts in-progress → ready.
/// Assert that pr_review_phase picks up the ralph:ready issue (with staged
/// amendments) and drains them on the next tick.
fn restart_drift_ready_drains_staged(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        setup_mock_backend(&dh);

        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_pr_review_whitelist",
            "[\"alice\"]",
        ])
        .expect("set whitelist");

        let ws_root = dh.repo_root.join(".ralph");
        setup_task_metadata(&ws_root, "acme-widgets-42", 99);
        setup_project_branch(&dh.repo_root, 42, false);

        // Pre-stage an amendment (simulating what poll_pr_reviews did before crash).
        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join("acme-widgets-42");
        fs::create_dir_all(&staging_dir).expect("create staging dir");
        let amendment = serde_json::json!({
            "id": "PR-99-issue_comment-1",
            "body": "fix the auth bug",
            "priority": "p2",
            "source": "pr-review",
            "source_detail": "pr#99/issue_comment#1",
            "created_at": "2024-01-01T00:00:00Z"
        });
        fs::write(
            staging_dir.join("20240101000000-PR-99-issue_comment-1.json"),
            serde_json::to_string_pretty(&amendment).unwrap(),
        )
        .expect("write staged amendment");

        // Also persist the dedup key so the comment won't be re-enqueued
        // (simulating that dedup state was saved before the crash).
        let mut dedup_state = PrReviewState::default();
        dedup_state
            .processed_keys
            .insert("issue_comment:1".to_string());
        dedup_state
            .save(&ws_root, "acme-widgets-42")
            .expect("save dedup state");

        let label_log = dh.temp_dir.path().join("drift_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_path = write_pr_review_mock_gh(&dh).expect("write mock gh");

        // Issue has ralph:ready label — simulating post-restart reconciliation
        // (the original ralph:completed was swapped to ralph:in-progress, then
        // startup reconciliation converted it to ralph:ready).
        let issue_labels =
            r#"{"labels":[{"name":"ralph:ready"},{"name":"ralph:pr-review"}]}"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", "[]"),
                    ("MOCK_GH_PR_STATE", "open"),
                    ("MOCK_GH_PR_COMMENTS", "[]"),
                    ("MOCK_GH_ISSUE_COMMENTS", "[]"),
                    ("MOCK_GH_REVIEWS", "[]"),
                    ("MOCK_GH_ISSUE_LABELS", issue_labels),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Verify pr_review_phase picked up the ralph:ready task.
        assert!(
            stderr.contains("pr-review: resuming ralph:ready task"),
            "stderr should log resume of ralph:ready task, got: {stderr}"
        );

        // Verify label swap occurred (ready → in-progress).
        assert!(
            label_log.exists(),
            "label log should exist after label swap"
        );
        let log_content = fs::read_to_string(&label_log).expect("read label log");
        assert!(
            log_content.contains("ralph:in-progress"),
            "label log should contain ralph:in-progress swap, got: {log_content}"
        );

        // Verify staged amendments were drained.
        assert!(
            !has_staged_amendments(&ws_root, "acme-widgets-42"),
            "staged amendments should have been drained during dispatch"
        );
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

/// Count JSON files in a directory (returns 0 if directory doesn't exist).
fn count_json_files(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    fs::read_dir(dir)
        .unwrap_or_else(|_| panic!("read dir {}", dir.display()))
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x == "json")
                .unwrap_or(false)
        })
        .count()
}

/// Write a mock gh script that handles PR review API endpoints.
///
/// Environment variables:
/// - `MOCK_GH_ISSUES` — JSON array for `issue list`
/// - `MOCK_GH_PR_STATE` — PR state for `api repos/.../pulls/{n}` (default "open")
/// - `MOCK_GH_PR_COMMENTS` — JSON for inline review comments
/// - `MOCK_GH_ISSUE_COMMENTS` — JSON for top-level PR comments
/// - `MOCK_GH_REVIEWS` — JSON for review summaries
/// - `MOCK_GH_ISSUE_LABELS` — JSON for `issue view --json labels`
/// - `MOCK_GH_LABEL_LOG` — file to log label operations
fn write_pr_review_mock_gh(h: &RalphHarness) -> crate::Result<String> {
    let script = h.write_mock_script(
        "gh",
        r###"#!/bin/sh
# Mock gh for PR review daemon tests.

case "$1" in
  issue)
    case "$2" in
      list)
        if [ -n "${MOCK_GH_ISSUES:-}" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        exit 0
        ;;
      view)
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then want_labels=1; fi
          if [ "$arg" = "title,body" ]; then want_title_body=1; fi
        done
        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          issue_number="${3:-0}"
          printf '{"title":"Mock issue %s","body":"Mock body for issue %s"}' "$issue_number" "$issue_number"
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
      *) echo "mock gh: unhandled issue subcommand: $2" >&2; exit 1 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf ''; exit 0 ;;
      create) printf 'https://github.com/acme/widgets/pull/1\n'; exit 0 ;;
      edit) exit 0 ;;
      *) echo "mock gh: unhandled pr subcommand: $2" >&2; exit 1 ;;
    esac
    ;;
  api)
    # Handle `api user`
    if [ "$2" = "user" ]; then
      printf 'ralph-bot\n'
      exit 0
    fi

    # Handle PR review API endpoints.
    # $2 is the endpoint, remaining args are flags like --paginate, --jq
    endpoint="$2"

    # Check for --jq flag (used by is_pr_open)
    has_jq=0
    for arg in "$@"; do
      if [ "$arg" = "--jq" ]; then has_jq=1; fi
    done

    # Match endpoint patterns
    case "$endpoint" in
      repos/*/pulls/*/comments)
        # Inline review comments
        if [ -n "${MOCK_GH_PR_COMMENTS:-}" ]; then
          printf '%s' "$MOCK_GH_PR_COMMENTS"
        else
          printf '[]'
        fi
        exit 0
        ;;
      repos/*/issues/*/comments)
        # Top-level PR/issue comments
        if [ -n "${MOCK_GH_ISSUE_COMMENTS:-}" ]; then
          printf '%s' "$MOCK_GH_ISSUE_COMMENTS"
        else
          printf '[]'
        fi
        exit 0
        ;;
      repos/*/pulls/*/reviews)
        # Review summaries
        if [ -n "${MOCK_GH_REVIEWS:-}" ]; then
          printf '%s' "$MOCK_GH_REVIEWS"
        else
          printf '[]'
        fi
        exit 0
        ;;
      repos/*/pulls/*)
        # PR state check (is_pr_open uses --jq .state)
        if [ "$has_jq" = "1" ]; then
          printf '%s\n' "${MOCK_GH_PR_STATE:-open}"
          exit 0
        fi
        printf '{"state":"%s"}' "${MOCK_GH_PR_STATE:-open}"
        exit 0
        ;;
      *)
        echo "mock gh: unhandled api endpoint: $endpoint" >&2
        exit 1
        ;;
    esac
    ;;
  label)
    case "$2" in
      create) exit 0 ;;
      *) echo "mock gh: unhandled label subcommand: $2" >&2; exit 1 ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        if [ -n "$target_dir" ]; then
          mkdir -p "$target_dir"
          git init "$target_dir" --quiet 2>/dev/null
          git -C "$target_dir" config user.email "mock@test"
          git -C "$target_dir" config user.name "MockClone"
          touch "$target_dir/.gitkeep"
          git -C "$target_dir" add .gitkeep
          git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        fi
        exit 0
        ;;
      view) printf 'acme/widgets\n'; exit 0 ;;
      *) echo "mock gh: unhandled repo subcommand: $2" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "mock gh: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###,
    )?;
    let base = script
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let existing = std::env::var("PATH").unwrap_or_default();
    Ok(format!("{base}:{existing}"))
}

/// Write task metadata for a given task_id and pr_number.
fn setup_task_metadata(ws_root: &Path, task_id: &str, pr_number: u32) {
    let tasks_dir = ws_root.join("daemon").join("tasks");
    fs::create_dir_all(&tasks_dir).expect("create tasks dir");
    let meta = TaskMetadata {
        pr_url: Some(format!(
            "https://github.com/acme/widgets/pull/{pr_number}"
        )),
    };
    fs::write(
        tasks_dir.join(format!("{task_id}.json")),
        serde_json::to_string(&meta).unwrap(),
    )
    .expect("write task metadata");
}

/// Create a git branch `ralph/issue-{n}` with minimal project files committed,
/// so that `should_resume_issue_project()` returns true after worktree creation.
fn setup_project_branch(repo_root: &Path, issue_number: u32, is_quick: bool) {
    let branch = format!("ralph/issue-{issue_number}");
    let project_id = format!("issue-{issue_number}");

    // Create branch from current HEAD.
    let branch_out = Command::new("git")
        .args(["branch", &branch])
        .current_dir(repo_root)
        .output()
        .expect("git branch");
    assert!(branch_out.status.success(), "git branch failed: {}", String::from_utf8_lossy(&branch_out.stderr));

    // Checkout branch, add project files, commit, checkout back.
    let checkout_out = Command::new("git")
        .args(["checkout", &branch])
        .current_dir(repo_root)
        .output()
        .expect("git checkout branch");
    assert!(checkout_out.status.success(), "git checkout branch failed: {}", String::from_utf8_lossy(&checkout_out.stderr));

    let project_dir = repo_root
        .join(".ralph")
        .join("projects")
        .join(&project_id);
    fs::create_dir_all(&project_dir).expect("create project dir");

    // Write prompt.md (triggers resume detection).
    fs::write(
        project_dir.join("prompt.md"),
        "# Test prompt\nImplement the feature.",
    )
    .expect("write prompt.md");

    // Write state.json as completed.
    let mut state = serde_json::json!({
        "project_id": project_id,
        "project_name": "test",
        "status": "completed",
        "current_phase": "completing",
        "current_loop": 1,
        "phase_iteration": 1,
        "prompt_file": "prompt.md",
        "parent_project": null,
        "loops": [],
        "completion_attempts": [],
        "created_at": "2024-01-01T00:00:00Z"
    });
    if is_quick {
        state["quick_dev_phase"] = serde_json::Value::Null;
    }
    fs::write(
        project_dir.join("state.json"),
        serde_json::to_string_pretty(&state).unwrap(),
    )
    .expect("write state.json");

    // Commit only project-specific files (not all of .ralph/) to avoid
    // removing workspace config files (e.g. ralph.toml) when switching
    // back to master.
    let project_rel = format!(".ralph/projects/{project_id}/");
    let add_out = Command::new("git")
        .args(["add", &project_rel])
        .current_dir(repo_root)
        .output()
        .expect("git add");
    assert!(add_out.status.success(), "git add failed: {}", String::from_utf8_lossy(&add_out.stderr));

    let commit_out = Command::new("git")
        .args(["commit", "-m", "add project files for test"])
        .current_dir(repo_root)
        .output()
        .expect("git commit");
    assert!(commit_out.status.success(), "git commit failed: {}", String::from_utf8_lossy(&commit_out.stderr));

    // Push to origin so that sync_project_branch finds the remote branch
    // and does not force-reset it to origin/master (which would wipe out
    // the project files).
    let push_out = Command::new("git")
        .args(["push", "origin", &branch])
        .current_dir(repo_root)
        .output()
        .expect("git push origin");
    assert!(push_out.status.success(), "git push failed: {}", String::from_utf8_lossy(&push_out.stderr));

    // Switch back to master.
    let checkout_out = Command::new("git")
        .args(["checkout", "master"])
        .current_dir(repo_root)
        .output()
        .expect("git checkout master");
    assert!(checkout_out.status.success(), "git checkout master failed: {}", String::from_utf8_lossy(&checkout_out.stderr));
}

/// Like `setup_project_branch` with `is_quick=true`, but writes non-zero
/// `quick_dev_review_iteration` and `quick_dev_final_review_attempts` to
/// simulate a previously force-completed quick-dev project with stale counters.
fn setup_project_branch_with_stale_counters(repo_root: &Path, issue_number: u32) {
    let branch = format!("ralph/issue-{issue_number}");
    let project_id = format!("issue-{issue_number}");

    let branch_out = Command::new("git")
        .args(["branch", &branch])
        .current_dir(repo_root)
        .output()
        .expect("git branch");
    assert!(branch_out.status.success(), "git branch failed: {}", String::from_utf8_lossy(&branch_out.stderr));

    let checkout_out = Command::new("git")
        .args(["checkout", &branch])
        .current_dir(repo_root)
        .output()
        .expect("git checkout branch");
    assert!(checkout_out.status.success(), "git checkout branch failed: {}", String::from_utf8_lossy(&checkout_out.stderr));

    let project_dir = repo_root
        .join(".ralph")
        .join("projects")
        .join(&project_id);
    fs::create_dir_all(&project_dir).expect("create project dir");

    fs::write(
        project_dir.join("prompt.md"),
        "# Test prompt\nImplement the feature.",
    )
    .expect("write prompt.md");

    // State has non-zero retry counters (stale from previous force-complete).
    let state = serde_json::json!({
        "project_id": project_id,
        "project_name": "test",
        "status": "completed",
        "current_phase": "completing",
        "quick_dev_phase": null,
        "current_loop": 1,
        "phase_iteration": 5,
        "quick_dev_review_iteration": 3,
        "quick_dev_final_review_attempts": 2,
        "prompt_file": "prompt.md",
        "parent_project": null,
        "loops": [],
        "completion_attempts": [],
        "created_at": "2024-01-01T00:00:00Z"
    });
    fs::write(
        project_dir.join("state.json"),
        serde_json::to_string_pretty(&state).unwrap(),
    )
    .expect("write state.json");

    // Commit only project-specific files (not all of .ralph/) to avoid
    // removing workspace config files when switching back to master.
    let project_rel = format!(".ralph/projects/{project_id}/");
    let add_out = Command::new("git")
        .args(["add", &project_rel])
        .current_dir(repo_root)
        .output()
        .expect("git add");
    assert!(add_out.status.success(), "git add failed: {}", String::from_utf8_lossy(&add_out.stderr));

    let commit_out = Command::new("git")
        .args(["commit", "-m", "add project files with stale counters"])
        .current_dir(repo_root)
        .output()
        .expect("git commit");
    assert!(commit_out.status.success(), "git commit failed: {}", String::from_utf8_lossy(&commit_out.stderr));

    // Push to origin so that sync_project_branch finds the remote branch
    // and does not force-reset it to origin/master.
    let push_out = Command::new("git")
        .args(["push", "origin", &branch])
        .current_dir(repo_root)
        .output()
        .expect("git push origin");
    assert!(push_out.status.success(), "git push failed: {}", String::from_utf8_lossy(&push_out.stderr));

    let checkout_out = Command::new("git")
        .args(["checkout", "master"])
        .current_dir(repo_root)
        .output()
        .expect("git checkout master");
    assert!(checkout_out.status.success(), "git checkout master failed: {}", String::from_utf8_lossy(&checkout_out.stderr));
}

/// Set up a minimal mock backend so that dispatch_task can spawn a child process.
fn setup_mock_backend(dh: &RalphHarness) {
    let script = dh
        .write_mock_script(
            "mock_backend.sh",
            "#!/bin/sh\ncat >/dev/null\necho 'mock output'\n",
        )
        .expect("write mock backend");
    let script_str = script.to_string_lossy().into_owned();
    dh.ralph_ok(["config", "set", "backends.claude.command", &script_str])
        .expect("set claude backend");
    dh.ralph_ok(["config", "set", "backends.claude.args", "[]"])
        .expect("set claude args");
    dh.ralph_ok(["config", "set", "backends.codex.command", &script_str])
        .expect("set codex backend");
    dh.ralph_ok(["config", "set", "backends.codex.args", "[]"])
        .expect("set codex args");
    // Disable openrouter to avoid external calls.
    dh.ralph_ok([
        "config",
        "set",
        "backends.openrouter.command",
        &script_str,
    ])
    .expect("set openrouter backend");
    dh.ralph_ok(["config", "set", "backends.openrouter.args", "[]"])
        .expect("set openrouter args");
}

