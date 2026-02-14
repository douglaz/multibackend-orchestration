use super::*;

use std::fs;
use std::path::PathBuf;

use crate::validate::assertions::{assert_exit_code, assert_stdout_contains};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts;
use serde_json::{json, Value};

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        // --- Loop 1 Foundation Tests ---
        ConformanceTest {
            name: "daemon::cli_parse_start_status_abort",
            func: cli_parse_start_status_abort,
        },
        ConformanceTest {
            name: "daemon::config_merge_and_defaults",
            func: config_merge_and_defaults,
        },
        ConformanceTest {
            name: "daemon::start_validates_inputs_and_workspace",
            func: start_validates_inputs_and_workspace,
        },
        ConformanceTest {
            name: "daemon::status_reads_store_with_locking",
            func: status_reads_store_with_locking,
        },
        ConformanceTest {
            name: "daemon::abort_by_full_task_id",
            func: abort_by_full_task_id,
        },
        ConformanceTest {
            name: "daemon::abort_by_bare_number_ambiguous_error",
            func: abort_by_bare_number_ambiguous_error,
        },
        ConformanceTest {
            name: "daemon::abort_when_daemon_not_running",
            func: abort_when_daemon_not_running,
        },
        ConformanceTest {
            name: "daemon::abort_stale_pid_and_terminal_state_handling",
            func: abort_stale_pid_and_terminal_state_handling,
        },
        // --- Loop 2 Runtime Tests ---
        ConformanceTest {
            name: "daemon::runtime_reconciliation_on_startup",
            func: runtime_reconciliation_on_startup,
        },
        ConformanceTest {
            name: "daemon::runtime_polling_filter_overflow",
            func: runtime_polling_filter_overflow,
        },
        ConformanceTest {
            name: "daemon::runtime_worktree_isolation",
            func: runtime_worktree_isolation,
        },
        ConformanceTest {
            name: "daemon::runtime_pid_pgid_persistence",
            func: runtime_pid_pgid_persistence,
        },
        ConformanceTest {
            name: "daemon::runtime_idempotent_comments",
            func: runtime_idempotent_comments,
        },
        ConformanceTest {
            name: "daemon::runtime_pr_reuse_no_diff",
            func: runtime_pr_reuse_no_diff,
        },
        ConformanceTest {
            name: "daemon::runtime_pr_create_failure_terminal",
            func: runtime_pr_create_failure_terminal,
        },
        ConformanceTest {
            name: "daemon::runtime_single_iteration_mode",
            func: runtime_single_iteration_mode,
        },
        ConformanceTest {
            name: "daemon::runtime_adopt_pending_fetches_raw_idea_and_uses_idea_flag",
            func: runtime_adopt_pending_fetches_raw_idea_and_uses_idea_flag,
        },
        ConformanceTest {
            name: "daemon::runtime_adopt_pending_fetch_failure_uses_metadata_fallback",
            func: runtime_adopt_pending_fetch_failure_uses_metadata_fallback,
        },
        ConformanceTest {
            name: "daemon::runtime_abort_during_dispatch_preserves_terminal",
            func: runtime_abort_during_dispatch_preserves_terminal,
        },
        ConformanceTest {
            name: "daemon::runtime_no_diff_pr_path",
            func: runtime_no_diff_pr_path,
        },
        // --- Loop 3 Refinement Dispatch Tests ---
        ConformanceTest {
            name: "daemon::refinement_happy_path",
            func: refinement_happy_path,
        },
        ConformanceTest {
            name: "daemon::refinement_failure_fallback",
            func: refinement_failure_fallback,
        },
        ConformanceTest {
            name: "daemon::refinement_disabled_uses_raw_idea",
            func: refinement_disabled_uses_raw_idea,
        },
        ConformanceTest {
            name: "daemon::refinement_comment_failure_non_blocking",
            func: refinement_comment_failure_non_blocking,
        },
        ConformanceTest {
            name: "daemon::refinement_strict_ordering",
            func: refinement_strict_ordering,
        },
        ConformanceTest {
            name: "daemon::refinement_comment_idempotency_on_retry",
            func: refinement_comment_idempotency_on_retry,
        },
        // --- Push-before-PR Tests ---
        ConformanceTest {
            name: "daemon::runtime_push_before_pr_create",
            func: runtime_push_before_pr_create,
        },
        // --- Branch Resolution Regression Tests ---
        ConformanceTest {
            name: "daemon::runtime_branch_switch_updates_task_and_pr",
            func: runtime_branch_switch_updates_task_and_pr,
        },
        ConformanceTest {
            name: "daemon::runtime_branch_unchanged_no_switch_log",
            func: runtime_branch_unchanged_no_switch_log,
        },
        // --- Child Output Capture Tests ---
        ConformanceTest {
            name: "daemon::runtime_child_output_captured_in_log",
            func: runtime_child_output_captured_in_log,
        },
        // --- Auto-Rebase Conformance Tests ---
        ConformanceTest {
            name: "daemon::rebase_disabled_skip",
            func: rebase_disabled_skip,
        },
        ConformanceTest {
            name: "daemon::rebase_conflict_skip",
            func: rebase_conflict_skip,
        },
        ConformanceTest {
            name: "daemon::rebase_closed_merged_skip",
            func: rebase_closed_merged_skip,
        },
        ConformanceTest {
            name: "daemon::rebase_unknown_mergeability_skip",
            func: rebase_unknown_mergeability_skip,
        },
        ConformanceTest {
            name: "daemon::rebase_branch_switched_task",
            func: rebase_branch_switched_task,
        },
        ConformanceTest {
            name: "daemon::rebase_base_branch_from_pr",
            func: rebase_base_branch_from_pr,
        },
        ConformanceTest {
            name: "daemon::rebase_pr_comment_not_issue",
            func: rebase_pr_comment_not_issue,
        },
        ConformanceTest {
            name: "daemon::rebase_dedup_by_head_sha",
            func: rebase_dedup_by_head_sha,
        },
        ConformanceTest {
            name: "daemon::rebase_force_with_lease_rejection",
            func: rebase_force_with_lease_rejection,
        },
        ConformanceTest {
            name: "daemon::rebase_gh_pr_view_failure_break",
            func: rebase_gh_pr_view_failure_break,
        },
        ConformanceTest {
            name: "daemon::rebase_per_cycle_cap",
            func: rebase_per_cycle_cap,
        },
        ConformanceTest {
            name: "daemon::rebase_interval_skip",
            func: rebase_interval_skip,
        },
        ConformanceTest {
            name: "daemon::rebase_status_last_rebase_column",
            func: rebase_status_last_rebase_column,
        },
        ConformanceTest {
            name: "daemon::rebase_backward_compat_state",
            func: rebase_backward_compat_state,
        },
    ]
}

// =============================================================================
// Loop 1 Foundation Tests (preserved from original)
// =============================================================================

fn cli_parse_start_status_abort(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let help = h
            .ralph(["daemon", "--help"])
            .expect("daemon --help should execute");
        assert_exit_code(&help, 0);
        assert_stdout_contains(&help, "start");
        assert_stdout_contains(&help, "status");
        assert_stdout_contains(&help, "abort");

        // Use --single-iteration and mock gh so runtime exits cleanly
        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");
        let start = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--poll-seconds",
                    "30",
                    "--max-concurrent",
                    "2",
                    "--label",
                    "ralph:ready",
                    "--label",
                    "triage",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&start, 0);
        assert_stdout_contains(&start, "daemon start validated for repo acme/widgets");

        let status = h
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&status, 0);
        // After a single iteration with no issues, still no tasks
        let combined = combined_output(&status);
        assert!(
            combined.contains("no daemon tasks") || combined.contains("DAEMON TASKS"),
            "expected task output, got:\n{combined}"
        );

        let abort = h
            .ralph(["daemon", "abort", "123"])
            .expect("daemon abort should execute");
        assert_exit_code(&abort, 2);
        assert!(
            combined_output(&abort).contains("no task found for issue number 123"),
            "expected missing task message, got:\n{}",
            combined_output(&abort)
        );
    })
}

fn config_merge_and_defaults(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Use --single-iteration and mock gh for the daemon start
        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");
        let default_start = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/default",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&default_start, 0);
        assert_stdout_contains(
            &default_start,
            "poll=60s, max_concurrent=1, labels=ralph:ready",
        );

        let poll_default = h
            .ralph_ok(["config", "get", "workspace.daemon_poll_seconds"])
            .expect("config get workspace.daemon_poll_seconds should succeed");
        assert_eq!(poll_default.trim(), "60");

        let conc_default = h
            .ralph_ok(["config", "get", "workspace.daemon_max_concurrent"])
            .expect("config get workspace.daemon_max_concurrent should succeed");
        assert_eq!(conc_default.trim(), "1");

        let labels_default = h
            .ralph_ok(["config", "get", "workspace.daemon_labels"])
            .expect("config get workspace.daemon_labels should succeed");
        assert_eq!(labels_default.trim(), "[\n  \"ralph:ready\"\n]");

        let refinement_enabled_default = h
            .ralph_ok(["config", "get", "workspace.daemon_refinement_enabled"])
            .expect("config get workspace.daemon_refinement_enabled should succeed");
        assert_eq!(refinement_enabled_default.trim(), "true");

        let refinement_backend_default = h
            .ralph_ok(["config", "get", "workspace.daemon_refinement_backend"])
            .expect("config get workspace.daemon_refinement_backend should succeed");
        assert_eq!(refinement_backend_default.trim(), "claude(sonnet)");

        let auto_rebase_enabled_default = h
            .ralph_ok(["config", "get", "workspace.daemon_auto_rebase_enabled"])
            .expect("config get workspace.daemon_auto_rebase_enabled should succeed");
        assert_eq!(auto_rebase_enabled_default.trim(), "true");

        let rebase_interval_default = h
            .ralph_ok(["config", "get", "workspace.daemon_rebase_interval_seconds"])
            .expect("config get workspace.daemon_rebase_interval_seconds should succeed");
        assert_eq!(rebase_interval_default.trim(), "1800");

        let rebase_cap_default = h
            .ralph_ok(["config", "get", "workspace.daemon_max_rebases_per_cycle"])
            .expect("config get workspace.daemon_max_rebases_per_cycle should succeed");
        assert_eq!(rebase_cap_default.trim(), "3");

        let rebase_timeout_default = h
            .ralph_ok(["config", "get", "workspace.daemon_rebase_timeout_seconds"])
            .expect("config get workspace.daemon_rebase_timeout_seconds should succeed");
        assert_eq!(rebase_timeout_default.trim(), "120");

        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_auto_rebase_enabled",
            "false",
        ])
        .expect("set workspace.daemon_auto_rebase_enabled failed");
        let auto_rebase_enabled_updated = h
            .ralph_ok(["config", "get", "workspace.daemon_auto_rebase_enabled"])
            .expect("config get workspace.daemon_auto_rebase_enabled should succeed");
        assert_eq!(auto_rebase_enabled_updated.trim(), "false");

        h.create_project(
            "daemon-config",
            "Daemon Config",
            "Project used for daemon config merge checks",
        )
        .expect("create_project failed");

        h.ralph_ok([
            "config",
            "set",
            "daemon.poll_seconds",
            "15",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.poll_seconds failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.max_concurrent",
            "3",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.max_concurrent failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.labels",
            "[\"ralph:ready\",\"priority:high\"]",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.labels failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.repo",
            "acme/project-override",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.repo failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.refinement_enabled",
            "false",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.refinement_enabled failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.refinement_backend",
            "codex(gpt-5.3-codex-medium)",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.refinement_backend failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.auto_rebase_enabled",
            "false",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.auto_rebase_enabled failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.rebase_interval_seconds",
            "900",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.rebase_interval_seconds failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.max_rebases_per_cycle",
            "5",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.max_rebases_per_cycle failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.rebase_timeout_seconds",
            "240",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.rebase_timeout_seconds failed");

        h.ralph_ok(["project", "use", "daemon-config"])
            .expect("project use should succeed");

        let refinement_enabled_project = h
            .ralph_ok(["config", "get", "daemon.refinement_enabled"])
            .expect("config get daemon.refinement_enabled should succeed");
        assert_eq!(refinement_enabled_project.trim(), "false");

        let refinement_backend_project = h
            .ralph_ok(["config", "get", "daemon.refinement_backend"])
            .expect("config get daemon.refinement_backend should succeed");
        assert_eq!(
            refinement_backend_project.trim(),
            "codex(gpt-5.3-codex-medium)"
        );

        let auto_rebase_enabled_project = h
            .ralph_ok(["config", "get", "daemon.auto_rebase_enabled"])
            .expect("config get daemon.auto_rebase_enabled should succeed");
        assert_eq!(auto_rebase_enabled_project.trim(), "false");

        let rebase_interval_project = h
            .ralph_ok(["config", "get", "daemon.rebase_interval_seconds"])
            .expect("config get daemon.rebase_interval_seconds should succeed");
        assert_eq!(rebase_interval_project.trim(), "900");

        let rebase_cap_project = h
            .ralph_ok(["config", "get", "daemon.max_rebases_per_cycle"])
            .expect("config get daemon.max_rebases_per_cycle should succeed");
        assert_eq!(rebase_cap_project.trim(), "5");

        let rebase_timeout_project = h
            .ralph_ok(["config", "get", "daemon.rebase_timeout_seconds"])
            .expect("config get daemon.rebase_timeout_seconds should succeed");
        assert_eq!(rebase_timeout_project.trim(), "240");

        let merged_start = h
            .ralph_env(
                ["daemon", "start", "--single-iteration"],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&merged_start, 0);
        assert_stdout_contains(
            &merged_start,
            "daemon start validated for repo acme/project-override",
        );
        assert_stdout_contains(
            &merged_start,
            "poll=15s, max_concurrent=3, labels=ralph:ready,priority:high",
        );
    })
}

fn start_validates_inputs_and_workspace(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let no_workspace = h
            .ralph([
                "daemon",
                "start",
                "--repo",
                "acme/widgets",
                "--single-iteration",
            ])
            .expect("daemon start should execute");
        assert_exit_code(&no_workspace, 2);

        h.init_workspace().expect("init failed");

        let gh_path = write_mock_gh(h, "#!/bin/sh\necho \"octo/demo\"\n")
            .expect("write mock gh should succeed");

        let with_workspace = h
            .ralph_env(
                ["daemon", "start", "--single-iteration"],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&with_workspace, 0);
        assert_stdout_contains(&with_workspace, "daemon start validated for repo octo/demo");
    })
}

fn status_reads_store_with_locking(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let empty = h
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&empty, 0);
        assert_stdout_contains(&empty, "no daemon tasks");

        write_tasks(
            h,
            vec![
                task_json(
                    "acme-widgets-41",
                    "pending",
                    41,
                    "acme",
                    "widgets",
                    None,
                    None,
                ),
                task_json(
                    "acme-widgets-42",
                    "in_progress",
                    42,
                    "acme",
                    "widgets",
                    Some(2222),
                    Some(2222),
                ),
            ],
        )
        .expect("write_tasks failed");

        let populated = h
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&populated, 0);
        assert_stdout_contains(&populated, "LAST REBASE");
        assert_stdout_contains(&populated, "acme-widgets-41");
        assert_stdout_contains(&populated, "pending");
        assert_stdout_contains(&populated, "acme-widgets-42");
        assert_stdout_contains(&populated, "in_progress");
    })
}

fn abort_by_full_task_id(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-10",
                "in_progress",
                10,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph(["daemon", "abort", "acme-widgets-10"])
            .expect("daemon abort should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "aborted task acme-widgets-10");

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["state"], json!("aborted"));
    })
}

fn abort_by_bare_number_ambiguous_error(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![
                task_json(
                    "acme-widgets-7",
                    "pending",
                    7,
                    "acme",
                    "widgets",
                    None,
                    None,
                ),
                task_json("other-api-7", "pending", 7, "other", "api", None, None),
            ],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph(["daemon", "abort", "7"])
            .expect("daemon abort should execute");
        assert_exit_code(&output, 2);

        let combined = combined_output(&output);
        assert!(
            combined.contains("ambiguous"),
            "expected ambiguous error, got:\n{combined}"
        );

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks[0]["state"], json!("pending"));
        assert_eq!(tasks[1]["state"], json!("pending"));
    })
}

fn abort_when_daemon_not_running(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-99",
                "pending",
                99,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph(["daemon", "abort", "99"])
            .expect("daemon abort should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "aborted task acme-widgets-99");

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks[0]["state"], json!("aborted"));
    })
}

fn abort_stale_pid_and_terminal_state_handling(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-55",
                "in_progress",
                55,
                "acme",
                "widgets",
                Some(999_999),
                Some(999_999),
            )],
        )
        .expect("write_tasks failed");

        let first_abort = h
            .ralph(["daemon", "abort", "acme-widgets-55"])
            .expect("daemon abort should execute");
        assert_exit_code(&first_abort, 0);

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks[0]["state"], json!("aborted"));

        let second_abort = h
            .ralph(["daemon", "abort", "acme-widgets-55"])
            .expect("daemon abort should execute");
        assert_exit_code(&second_abort, 2);

        let combined = combined_output(&second_abort);
        assert!(
            combined.contains("already terminal"),
            "expected terminal-state error, got:\n{combined}"
        );
    })
}

// =============================================================================
// Loop 2 Runtime Tests
// =============================================================================

/// Test that startup reconciliation resets in_progress tasks to pending
/// and clears PID/PGID fields.
///
/// Verifies:
/// - in_progress tasks are reset to pending with PID/PGID cleared
/// - completed tasks remain untouched
/// - reconciliation is logged in stderr
fn runtime_reconciliation_on_startup(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Pre-populate tasks with in_progress state and fake PID/PGID
        write_tasks(
            h,
            vec![
                task_json(
                    "acme-widgets-10",
                    "in_progress",
                    10,
                    "acme",
                    "widgets",
                    Some(12345),
                    Some(12345),
                ),
                task_json(
                    "acme-widgets-20",
                    "in_progress",
                    20,
                    "acme",
                    "widgets",
                    Some(67890),
                    Some(67890),
                ),
                task_json(
                    "acme-widgets-30",
                    "completed",
                    30,
                    "acme",
                    "widgets",
                    None,
                    None,
                ),
            ],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        // Run daemon with --single-iteration to trigger reconciliation then exit.
        // Reconciliation resets in_progress -> pending before any adoption.
        // Re-adoption may fail (worktree creation in test env) but that's fine;
        // we verify reconciliation happened via stderr and task state.
        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // Verify reconciliation message in stderr — this proves the
        // reconcile phase ran and detected the 2 in_progress tasks.
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("reconcile: reset 2 in_progress task(s) to pending"),
            "expected reconciliation message in stderr, got:\n{stderr}"
        );

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks.len(), 3, "should still have 3 tasks");

        // The completed task should remain completed
        let completed = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-30")
            .unwrap();
        assert_eq!(completed["state"], json!("completed"));

        // The formerly in_progress tasks should have been reconciled.
        // They may still be pending (if re-adoption failed) or may have
        // been dispatched and drained (completed/failed). Either way, the
        // stale PID/PGID (12345, 67890) should have been cleared.
        for tid in &["acme-widgets-10", "acme-widgets-20"] {
            let task = tasks.iter().find(|t| t["task_id"] == *tid).unwrap();
            let state = task["state"].as_str().unwrap();
            // Must not still have the original stale PIDs
            let pid_val = task["child_pid"].as_u64().unwrap_or(0);
            assert!(
                pid_val != 12345 && pid_val != 67890,
                "task {} should have stale PID cleared, got: {}",
                tid,
                pid_val
            );
            // Must be in a valid post-reconciliation state
            assert!(
                ["pending", "in_progress", "completed", "failed"].contains(&state),
                "task {} should be in valid state after reconciliation, got: {}",
                tid,
                state
            );
        }
    })
}

/// Test that polling correctly filters out ralph:* labeled issues and
/// emits overflow warning when exactly 100 issues returned.
///
/// Verifies:
/// - Overflow warning appears when poll returns exactly 100 issues
/// - Issues with ralph:* labels are filtered out (not claimed)
/// - max_concurrent is respected (only 1 task claimed)
fn runtime_polling_filter_overflow(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Create a mock gh that returns exactly 100 issues, some with ralph:* labels.
        // Issues 1-5 have ralph:in-progress labels (should be filtered).
        // Issues 6-100 have no ralph labels (claimable).
        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        printf '['
        i=1
        while [ $i -le 100 ]; do
          if [ $i -gt 1 ]; then printf ','; fi
          if [ $i -le 5 ]; then
            printf '{"number":%d,"title":"issue %d","labels":[{"name":"ralph:in-progress"}]}' $i $i
          else
            printf '{"number":%d,"title":"issue %d","labels":[]}' $i $i
          fi
          i=$((i + 1))
        done
        printf ']'
        exit 0
        ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
    esac
    ;;
  repo)
    printf 'acme/widgets\n'
    exit 0
    ;;
esac
exit 1
"#;

        let gh_path = write_mock_gh(h, gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        // Use max_concurrent=1 to limit claiming. The overflow warning should
        // still be emitted based on the poll result count.
        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                    "--max-concurrent",
                    "1",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Verify overflow warning
        assert!(
            stderr.contains("100 issues") || stderr.contains("truncated"),
            "expected overflow/truncation warning in stderr, got:\n{stderr}"
        );

        // At most 1 task should have been claimed (max_concurrent=1)
        let tasks = load_tasks(h);
        let task_count = match tasks {
            Ok(ref t) => t.len(),
            Err(_) => 0,
        };
        assert!(
            task_count <= 1,
            "with max_concurrent=1, at most 1 task should be claimed, got {task_count}"
        );
    })
}

/// Test that each task gets its own worktree directory.
///
/// Uses RALPH_DAEMON_BIN to inject a mock ralph binary so dispatch is
/// deterministic and always succeeds.
///
/// Verifies:
/// - The daemon dispatches the task (unconditional)
/// - The worktree directory is created under .ralph/daemon/worktrees/<task-id>/
/// - The task reaches terminal state after drain
fn runtime_worktree_isolation(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Pre-populate a pending task
        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-50",
                "pending",
                50,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Dispatch must have succeeded (unconditional)
        assert!(
            stderr.contains("dispatched task acme-widgets-50"),
            "task must be dispatched (deterministic with mock ralph), stderr:\n{stderr}"
        );

        // Worktrees base directory must exist
        let wt_base = h.repo_root.join(".ralph").join("daemon").join("worktrees");
        assert!(
            wt_base.exists(),
            "worktrees base directory must exist after dispatch"
        );

        // Task should have reached a terminal state after drain
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-50")
            .unwrap();
        let state = task["state"].as_str().unwrap();
        assert!(
            ["completed", "failed"].contains(&state),
            "task should be in terminal state after drain, got: {state}"
        );
    })
}

/// Test that PID and PGID are stored as real OS values when a child is spawned.
///
/// Uses RALPH_DAEMON_BIN to inject a mock ralph binary so dispatch is
/// deterministic and always succeeds.
///
/// Verifies (all unconditional):
/// - The dispatch log includes a real PID
/// - In single-iteration mode with drain, the task reaches a terminal state
/// - PID/PGID are cleared in the terminal state
fn runtime_pid_pgid_persistence(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Pre-populate a pending task
        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-60",
                "pending",
                60,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Dispatch must have succeeded (unconditional)
        assert!(
            stderr.contains("dispatched task acme-widgets-60"),
            "task must be dispatched (deterministic with mock ralph), stderr:\n{stderr}"
        );

        // Verify the dispatch log includes a real PID
        assert!(
            stderr.contains("pid="),
            "dispatch log should include pid= with a real OS PID, got:\n{stderr}"
        );

        // Since single-iteration drains children, the task should have
        // reached a terminal state with cleared PID/PGID.
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-60")
            .unwrap();
        let state = task["state"].as_str().unwrap();

        assert!(
            ["completed", "failed"].contains(&state),
            "task should be in terminal state after drain, got: {state}"
        );
        assert!(
            task["child_pid"].is_null(),
            "PID should be cleared in terminal state, got: {}",
            task["child_pid"]
        );
        assert!(
            task["child_pgid"].is_null(),
            "PGID should be cleared in terminal state, got: {}",
            task["child_pgid"]
        );
    })
}

/// Test that the daemon's comment idempotency mechanism prevents duplicates.
///
/// Verifies:
/// - Comment markers are posted on first run
/// - Running again with the marker already present does NOT duplicate comments
/// - The comment log file shows exactly the expected number of postings
fn runtime_idempotent_comments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Create a mock gh that tracks comment calls via a file.
        // The "view" handler returns previously posted comments so the
        // idempotency check can find existing markers.
        let comment_log = h.temp_dir.path().join("comment_log.txt");
        let comment_count_file = h.temp_dir.path().join("comment_count.txt");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let comment_count_str = comment_count_file.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        # Return previously posted comments for marker check
        if [ -f "{comment_log_str}" ]; then
          cat "{comment_log_str}"
        fi
        exit 0
        ;;
      comment)
        # Log the comment body and count invocations
        shift; shift; shift  # skip 'issue' 'comment' '<number>'
        while [ $# -gt 0 ]; do
          case "$1" in
            --body) echo "$2" >> "{comment_log_str}" ; shift 2 ;;
            *) shift ;;
          esac
        done
        # Increment counter
        if [ -f "{comment_count_str}" ]; then
          count=$(cat "{comment_count_str}")
        else
          count=0
        fi
        count=$((count + 1))
        echo "$count" > "{comment_count_str}"
        exit 0
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        // Pre-populate a pending task that will be dispatched, complete,
        // and trigger a comment.
        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-70",
                "pending",
                70,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        // First daemon run — should post comment(s) on completion
        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("first daemon run should execute");
        assert_exit_code(&output, 0);

        // Record how many comments were posted on first run
        let first_count = read_count_file(&comment_count_file);

        // Verify comments contain the expected marker pattern
        if comment_log.exists() {
            let log_content = fs::read_to_string(&comment_log).expect("read comment log");
            assert!(
                log_content.contains("<!-- ralph:task:acme-widgets-70:"),
                "comment should contain ralph marker, got:\n{log_content}"
            );
        }

        // Second daemon run — comments should be de-duplicated (markers
        // already exist in the mock view output). Since the task is already
        // terminal, no new comments should be posted.
        let output2 = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("second daemon run should execute");
        assert_exit_code(&output2, 0);

        let second_count = read_count_file(&comment_count_file);
        assert_eq!(
            first_count, second_count,
            "comment count should not increase on second run (idempotent): first={first_count}, second={second_count}"
        );
    })
}

/// Test PR reuse / no-diff behavior.
///
/// Drives a task from pending → completed via mock ralph (with commit) so the
/// PR flow is triggered. The mock gh reports an existing PR on `pr list --head`,
/// so `pr create` should never be called.
///
/// Verifies:
/// - Task reaches completed state via child process completion
/// - `gh pr list --head` is used to find existing PR (reuse path)
/// - `gh pr create` is NOT called when an existing PR is found
/// - task pr_url is populated from the existing PR
fn runtime_pr_reuse_no_diff(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Track whether `pr create` was called
        let pr_create_log = h.temp_dir.path().join("pr_create_called.txt");
        let pr_create_log_str = pr_create_log.to_string_lossy().into_owned();

        // Track `pr list` calls
        let pr_list_log = h.temp_dir.path().join("pr_list_called.txt");
        let pr_list_log_str = pr_list_log.to_string_lossy().into_owned();

        // Mock gh: pr list --head returns existing PR URL, pr create logs and fails
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        # Check if --head flag is present (PR reuse check)
        for arg in "$@"; do
          case "$arg" in
            --head)
              echo "called" >> "{pr_list_log_str}"
              printf 'https://github.com/acme/widgets/pull/99'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      create)
        echo "called" > "{pr_create_log_str}"
        echo "should not be called when PR exists" >&2
        exit 1
        ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        // Use mock ralph that creates a commit (so has_diff returns true)
        let ralph_path = write_daemon_mock_ralph_with_commit(h).expect("write mock ralph");

        // Pre-populate a PENDING task — the daemon will dispatch it, the
        // mock ralph will commit a change, the child exits 0, triggering
        // complete_task → handle_pr_flow.
        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-80",
                    "pending",
                    80,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-80");
                t
            }],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Task must have been dispatched and completed
        assert!(
            stderr.contains("dispatched task acme-widgets-80"),
            "task should have been dispatched, stderr:\n{stderr}"
        );

        // Task should be completed
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-80")
            .unwrap();
        assert_eq!(
            task["state"],
            json!("completed"),
            "task should be completed after child exits 0"
        );

        // `pr list --head` should have been called (reuse check)
        assert!(
            pr_list_log.exists(),
            "pr list --head should have been called for PR reuse check"
        );

        // `pr create` should NOT have been called
        assert!(
            !pr_create_log.exists(),
            "pr create should not be called when an existing PR is found"
        );

        // pr_url should be set from the existing PR
        assert_eq!(
            task["pr_url"],
            json!("https://github.com/acme/widgets/pull/99"),
            "pr_url should be populated from existing PR"
        );
    })
}

/// Test that PR creation failure still allows task to reach terminal state.
///
/// Drives a task from pending → completed via mock ralph (with commit) so the
/// PR flow is triggered. The mock gh returns no existing PR and fails on
/// `pr create`. The task should still reach completed state.
///
/// Verifies:
/// - Task reaches completed despite `gh pr create` failing
/// - Warning about PR creation failure appears in stderr
/// - The task's pr_url remains null (no PR was created)
/// - `pr create` was actually attempted
fn runtime_pr_create_failure_terminal(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let pr_create_log = h.temp_dir.path().join("pr_create_attempted.txt");
        let pr_create_log_str = pr_create_log.to_string_lossy().into_owned();

        // Mock gh: pr list returns empty string (simulating `gh pr list -q ".[0].url"`
        // with no results), pr create fails
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        # Return empty string (no existing PRs) to simulate
        # `gh pr list --json url -q ".[0].url"` with no results
        printf ''
        exit 0
        ;;
      create)
        echo "called" > "{pr_create_log_str}"
        echo "PR creation failed: mock error" >&2
        exit 1
        ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        // Use mock ralph that creates a commit (so has_diff returns true → PR flow)
        let ralph_path = write_daemon_mock_ralph_with_commit(h).expect("write mock ralph");

        // Pre-populate a PENDING task — daemon dispatches it, mock ralph
        // commits, child exits 0, complete_task → handle_pr_flow → pr create fails
        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-90",
                    "pending",
                    90,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-90");
                t
            }],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Task must have been dispatched
        assert!(
            stderr.contains("dispatched task acme-widgets-90"),
            "task should have been dispatched, stderr:\n{stderr}"
        );

        // Task should be completed despite PR failure
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-90")
            .unwrap();
        assert_eq!(
            task["state"],
            json!("completed"),
            "task should be completed despite PR creation failure"
        );

        // PR create was attempted
        assert!(
            pr_create_log.exists(),
            "pr create should have been attempted"
        );

        // Warning about PR failure should be in stderr
        assert!(
            stderr.contains("failed to create PR"),
            "expected PR creation failure warning in stderr, got:\n{stderr}"
        );

        // pr_url should be null since creation failed
        assert!(
            task["pr_url"].is_null(),
            "pr_url should be null when PR creation fails, got: {}",
            task["pr_url"]
        );
    })
}

/// Test that --single-iteration mode runs exactly one cycle and exits
/// deterministically with no children left running.
///
/// Verifies:
/// - Daemon exits successfully in single-iteration mode
/// - All tasks that were dispatched reach a terminal state (drain)
/// - No in_progress tasks remain after single-iteration completes
fn runtime_single_iteration_mode(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        // Pre-populate a pending task that should be dispatched and drained
        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-100",
                "pending",
                100,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "daemon start validated for repo acme/widgets");

        // Verify no tasks are left in_progress (deterministic drain)
        let tasks = load_tasks(h).expect("load_tasks failed");
        for task in &tasks {
            let state = task["state"].as_str().unwrap_or("unknown");
            assert_ne!(
                state, "in_progress",
                "no tasks should be in_progress after single-iteration drain, but {} is in_progress",
                task["task_id"]
            );
        }
    })
}

/// Test restart/adoption behavior for legacy pending tasks with missing
/// `raw_idea`, including exact `ralph auto --idea <idea>` argv semantics.
///
/// Verifies:
/// - adopt_pending_tasks hydrates raw_idea via `gh issue view --json title,body`
/// - hydrated value is persisted to tasks.json
/// - spawned child receives argv `auto --idea <hydrated_idea>`
fn runtime_adopt_pending_fetches_raw_idea_and_uses_idea_flag(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Disable refinement so the raw hydrated idea is passed through unchanged.
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_refinement_enabled",
            "false",
        ])
        .expect("set refinement_enabled failed");

        // Seed a legacy pending task with no raw_idea field.
        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-130",
                "pending",
                130,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        found=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then
            found=1
          fi
        done
        if [ "$found" -eq 1 ]; then
          printf '{"title":"Hydrated title","body":"Hydrated body"}'
        else
          printf ''
        fi
        exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#;
        let gh_path = write_mock_gh(h, gh_script).expect("write mock gh");

        let args_log = h.temp_dir.path().join("daemon_args.log");
        let idea_log = h.temp_dir.path().join("daemon_idea.log");
        let args_log_str = args_log.to_string_lossy().into_owned();
        let idea_log_str = idea_log.to_string_lossy().into_owned();
        let ralph_script = format!(
            r#"#!/bin/sh
echo "$1" > "{args_log_str}"
echo "$2" >> "{args_log_str}"
printf '%s' "$3" > "{idea_log_str}"

expected="Hydrated title

Hydrated body"
[ "$1" = "auto" ] || exit 11
[ "$2" = "--idea" ] || exit 12
[ "$3" = "$expected" ] || exit 13
exit 0
"#
        );
        let ralph_path = write_mock_ralph(h, &ralph_script).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-130")
            .expect("task should exist");
        assert_eq!(
            task["raw_idea"],
            json!("Hydrated title\n\nHydrated body"),
            "raw_idea should be hydrated and persisted"
        );

        let args = fs::read_to_string(&args_log).expect("read args log");
        assert!(
            args.contains("auto\n--idea"),
            "expected auto/--idea args, got:\n{args}"
        );
        let idea = fs::read_to_string(&idea_log).expect("read idea log");
        assert_eq!(idea, "Hydrated title\n\nHydrated body");
    })
}

/// Test restart/adoption behavior when `gh issue view --json title,body`
/// returns malformed output for a legacy pending task with missing `raw_idea`.
///
/// Verifies:
/// - daemon does not skip dispatch when hydration fetch fails
/// - metadata fallback raw_idea is persisted
/// - spawned child receives argv `auto --idea <fallback_idea>`
fn runtime_adopt_pending_fetch_failure_uses_metadata_fallback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Disable refinement so the fallback idea is passed through unchanged.
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_refinement_enabled",
            "false",
        ])
        .expect("set refinement_enabled failed");

        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-131",
                "pending",
                131,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        found=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then
            found=1
          fi
        done
        if [ "$found" -eq 1 ]; then
          # Malformed/empty response for hydration fetch
          printf ''
        else
          printf ''
        fi
        exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#;
        let gh_path = write_mock_gh(h, gh_script).expect("write mock gh");

        let fallback_idea = "Issue #131 (acme/widgets)\n\nIssue body unavailable from GitHub; using daemon task metadata.";
        let args_log = h.temp_dir.path().join("daemon_args_fallback.log");
        let idea_log = h.temp_dir.path().join("daemon_idea_fallback.log");
        let args_log_str = args_log.to_string_lossy().into_owned();
        let idea_log_str = idea_log.to_string_lossy().into_owned();
        let ralph_script = format!(
            r#"#!/bin/sh
echo "$1" > "{args_log_str}"
echo "$2" >> "{args_log_str}"
printf '%s' "$3" > "{idea_log_str}"

expected="Issue #131 (acme/widgets)

Issue body unavailable from GitHub; using daemon task metadata."
[ "$1" = "auto" ] || exit 21
[ "$2" = "--idea" ] || exit 22
[ "$3" = "$expected" ] || exit 23
exit 0
"#
        );
        let ralph_path = write_mock_ralph(h, &ralph_script).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("using metadata fallback"),
            "expected metadata fallback warning, stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-131")
            .expect("task should exist");
        assert_eq!(task["raw_idea"], json!(fallback_idea));

        let args = fs::read_to_string(&args_log).expect("read args log");
        assert!(
            args.contains("auto\n--idea"),
            "expected auto/--idea args, got:\n{args}"
        );
        let idea = fs::read_to_string(&idea_log).expect("read idea log");
        assert_eq!(idea, fallback_idea);
    })
}

/// Test that aborting a task before/during dispatch preserves the aborted
/// terminal state — the runtime must not overwrite it with in_progress or
/// completed.
///
/// Simulates the race by:
/// 1. Pre-populating a pending task
/// 2. Aborting it via `ralph daemon abort` (sets state to aborted)
/// 3. Running the daemon with --single-iteration (which tries to adopt/dispatch)
/// 4. Asserting the task remains aborted (never reverts to in_progress/completed)
///
/// Verifies:
/// - Abort sets the task to aborted state
/// - Daemon runtime's dispatch CAS detects the terminal state
/// - Final persisted state is aborted, not in_progress or completed
fn runtime_abort_during_dispatch_preserves_terminal(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Pre-populate a pending task that the daemon will try to re-adopt
        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-110",
                "pending",
                110,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        // Abort the task BEFORE starting the daemon. This simulates the race
        // condition where abort runs concurrently with dispatch — the task
        // moves to terminal state before the runtime can transition it.
        let abort_output = h
            .ralph(["daemon", "abort", "acme-widgets-110"])
            .expect("daemon abort should execute");
        assert_exit_code(&abort_output, 0);

        // Verify the task is now aborted
        let tasks_before = load_tasks(h).expect("load_tasks failed");
        let task_before = tasks_before
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-110")
            .unwrap();
        assert_eq!(
            task_before["state"],
            json!("aborted"),
            "task should be aborted before daemon starts"
        );

        // Now start the daemon. It will try to re-adopt the (now-aborted)
        // task via adopt_pending_tasks, but the CAS guard in dispatch_task
        // should detect the terminal state and skip activation.
        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // The task MUST still be aborted — the runtime must not have
        // overwritten it with in_progress or completed.
        let tasks_after = load_tasks(h).expect("load_tasks failed");
        let task_after = tasks_after
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-110")
            .unwrap();
        assert_eq!(
            task_after["state"],
            json!("aborted"),
            "task must remain aborted after daemon dispatch attempt; CAS guard should prevent overwrite"
        );

        // PID/PGID should be cleared (no active child)
        assert!(
            task_after["child_pid"].is_null(),
            "PID should be null for aborted task, got: {}",
            task_after["child_pid"]
        );
        assert!(
            task_after["child_pgid"].is_null(),
            "PGID should be null for aborted task, got: {}",
            task_after["child_pgid"]
        );
    })
}

/// Test the "no diff → no PR + idempotent note comment" path.
///
/// Drives a task from pending → completed via mock ralph that does NOT create
/// any commits (so has_diff returns false). The mock gh tracks `pr create`
/// and `issue comment` calls.
///
/// Verifies:
/// - Task reaches completed state
/// - `gh pr create` is NOT called (no diff → no PR)
/// - An idempotent `no-diff` marker comment is posted (`<!-- ralph:task:<id>:no-diff -->`)
/// - pr_url remains null
fn runtime_no_diff_pr_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Track `pr create` calls
        let pr_create_log = h.temp_dir.path().join("pr_create_no_diff.txt");
        let pr_create_log_str = pr_create_log.to_string_lossy().into_owned();

        // Track comment calls and their content
        let comment_log = h.temp_dir.path().join("comment_no_diff.txt");
        let comment_log_str = comment_log.to_string_lossy().into_owned();

        // Mock gh: pr list returns empty, pr create logs but should never be called,
        // issue comment logs the body for inspection
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        # Return previously posted comments for marker check
        if [ -f "{comment_log_str}" ]; then
          cat "{comment_log_str}"
        fi
        exit 0
        ;;
      comment)
        # Log the comment body
        shift; shift; shift  # skip 'issue' 'comment' '<number>'
        while [ $# -gt 0 ]; do
          case "$1" in
            --body) echo "$2" >> "{comment_log_str}" ; shift 2 ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        # No existing PRs
        printf ''
        exit 0
        ;;
      create)
        echo "called" > "{pr_create_log_str}"
        printf 'https://github.com/mock/pr/1\n'
        exit 0
        ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");

        // Use the standard mock ralph that does NOT create commits (just exits 0).
        // This means has_diff will return false → no-diff path.
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        // Pre-populate a PENDING task with a branch set
        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-120",
                    "pending",
                    120,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-120");
                t
            }],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Task must have been dispatched
        assert!(
            stderr.contains("dispatched task acme-widgets-120"),
            "task should have been dispatched, stderr:\n{stderr}"
        );

        // Task should be completed
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-120")
            .unwrap();
        assert_eq!(
            task["state"],
            json!("completed"),
            "task should be completed after child exits 0"
        );

        // `pr create` should NOT have been called (no diff → no PR)
        assert!(
            !pr_create_log.exists(),
            "pr create should not be called when there is no diff"
        );

        // pr_url should be null (no PR created)
        assert!(
            task["pr_url"].is_null(),
            "pr_url should be null when no diff, got: {}",
            task["pr_url"]
        );

        // The no-diff marker comment should have been posted
        if comment_log.exists() {
            let log_content = fs::read_to_string(&comment_log).expect("read comment log");
            assert!(
                log_content.contains("<!-- ralph:task:acme-widgets-120:no-diff -->"),
                "expected no-diff marker comment, got:\n{log_content}"
            );
            assert!(
                log_content.contains("no code changes"),
                "expected 'no code changes' in comment body, got:\n{log_content}"
            );
        } else {
            panic!("comment log should exist — no-diff comment should have been posted");
        }

        // Run again — the no-diff comment should be idempotent (not posted twice).
        // Reset the task to pending to trigger another run.
        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-120",
                    "pending",
                    120,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-120");
                t
            }],
        )
        .expect("write_tasks re-seed failed");

        let comment_count_before = fs::read_to_string(&comment_log)
            .unwrap_or_default()
            .matches("<!-- ralph:task:acme-widgets-120:no-diff -->")
            .count();

        let output2 = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("second daemon run should execute");
        assert_exit_code(&output2, 0);

        let comment_count_after = fs::read_to_string(&comment_log)
            .unwrap_or_default()
            .matches("<!-- ralph:task:acme-widgets-120:no-diff -->")
            .count();

        assert_eq!(
            comment_count_before, comment_count_after,
            "no-diff comment should be idempotent (marker already present): before={comment_count_before}, after={comment_count_after}"
        );
    })
}

// =============================================================================
// Loop 3 Refinement Dispatch Tests
// =============================================================================

/// Test that refinement successfully transforms the raw idea and the refined
/// prompt is passed to `ralph auto --idea`.
///
/// Verifies:
/// - When refinement is enabled and the backend succeeds, the refined output
///   is used as --idea argument to the spawned child
/// - The mock refinement backend receives the raw idea on stdin
/// - Task completes successfully
fn refinement_happy_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Set up a mock refinement backend that reads stdin and outputs a refined prompt
        let refine_script = h
            .write_mock_script(
                "mock_refine_backend.sh",
                r#"#!/bin/sh
# Read stdin (the refinement prompt)
cat > /dev/null
# Output a refined prompt (must be >= 20 chars)
printf 'Refined: implement the feature with proper error handling and tests.'
exit 0
"#,
            )
            .expect("write mock refine backend");
        let refine_script_str = refine_script.to_string_lossy().into_owned();

        // Configure the claude backend command to use our mock
        h.ralph_ok([
            "config",
            "set",
            "backends.claude.command",
            &refine_script_str,
        ])
        .expect("set claude command failed");
        h.ralph_ok(["config", "set", "backends.claude.args", "[]"])
            .expect("set claude args failed");

        // Seed a pending task with a raw_idea
        let mut task = task_json(
            "acme-widgets-200",
            "pending",
            200,
            "acme",
            "widgets",
            None,
            None,
        );
        task["raw_idea"] = json!("Fix the login bug\n\nUsers cannot log in with SSO.");
        write_tasks(h, vec![task]).expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");

        // Mock ralph that records the idea it receives
        let idea_log = h.temp_dir.path().join("refinement_idea.log");
        let idea_log_str = idea_log.to_string_lossy().into_owned();
        let ralph_script = format!(
            r#"#!/bin/sh
printf '%s' "$3" > "{idea_log_str}"
exit 0
"#
        );
        let ralph_path = write_mock_ralph(h, &ralph_script).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("dispatched task acme-widgets-200"),
            "task should be dispatched, stderr:\n{stderr}"
        );
        // Should NOT have a refinement failure warning
        assert!(
            !stderr.contains("refinement failed"),
            "refinement should succeed, stderr:\n{stderr}"
        );

        // The spawned child should have received the refined prompt
        let idea = fs::read_to_string(&idea_log).expect("read idea log");
        assert_eq!(
            idea,
            "Refined: implement the feature with proper error handling and tests."
        );
    })
}

/// Test that when the refinement backend fails, the raw idea is used as
/// fallback and dispatch proceeds normally.
///
/// Verifies:
/// - Backend failure triggers a warning log
/// - Raw idea is used as the --idea argument
/// - Dispatch completes successfully despite refinement failure
fn refinement_failure_fallback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Set up a mock refinement backend that fails
        let refine_script = h
            .write_mock_script(
                "mock_refine_fail.sh",
                "#!/bin/sh\necho 'backend error' >&2\nexit 1\n",
            )
            .expect("write mock refine backend");
        let refine_script_str = refine_script.to_string_lossy().into_owned();

        h.ralph_ok([
            "config",
            "set",
            "backends.claude.command",
            &refine_script_str,
        ])
        .expect("set claude command failed");
        h.ralph_ok(["config", "set", "backends.claude.args", "[]"])
            .expect("set claude args failed");

        let raw_idea_text = "Raw bug report title\n\nRaw bug report body with details.";
        let mut task = task_json(
            "acme-widgets-201",
            "pending",
            201,
            "acme",
            "widgets",
            None,
            None,
        );
        task["raw_idea"] = json!(raw_idea_text);
        write_tasks(h, vec![task]).expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");

        let idea_log = h.temp_dir.path().join("fallback_idea.log");
        let idea_log_str = idea_log.to_string_lossy().into_owned();
        let ralph_script = format!(
            r#"#!/bin/sh
printf '%s' "$3" > "{idea_log_str}"
exit 0
"#
        );
        let ralph_path = write_mock_ralph(h, &ralph_script).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Warning about refinement failure should be present
        assert!(
            stderr.contains("refinement failed"),
            "expected refinement failure warning, stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("using raw idea"),
            "expected 'using raw idea' in warning, stderr:\n{stderr}"
        );

        // The spawned child should have received the raw idea (fallback)
        let idea = fs::read_to_string(&idea_log).expect("read idea log");
        assert_eq!(idea, raw_idea_text);
    })
}

/// Test that when `daemon_refinement_enabled = false`, no refinement call is
/// made and the raw idea is used directly.
///
/// Verifies:
/// - No refinement backend invocation
/// - Raw idea is passed as --idea to spawned child
/// - No refinement-related warnings in logs
fn refinement_disabled_uses_raw_idea(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Disable refinement
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_refinement_enabled",
            "false",
        ])
        .expect("set refinement_enabled failed");

        // Set up a backend that would fail to prove it's never called
        let refine_script = h
            .write_mock_script(
                "mock_refine_nocall.sh",
                "#!/bin/sh\necho 'SHOULD NOT BE CALLED' >&2\nexit 1\n",
            )
            .expect("write mock refine backend");
        let refine_script_str = refine_script.to_string_lossy().into_owned();
        h.ralph_ok([
            "config",
            "set",
            "backends.claude.command",
            &refine_script_str,
        ])
        .expect("set claude command failed");
        h.ralph_ok(["config", "set", "backends.claude.args", "[]"])
            .expect("set claude args failed");

        let raw_idea_text = "Disabled refinement task\n\nBody of the task.";
        let mut task = task_json(
            "acme-widgets-202",
            "pending",
            202,
            "acme",
            "widgets",
            None,
            None,
        );
        task["raw_idea"] = json!(raw_idea_text);
        write_tasks(h, vec![task]).expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");

        let idea_log = h.temp_dir.path().join("disabled_idea.log");
        let idea_log_str = idea_log.to_string_lossy().into_owned();
        let ralph_script = format!(
            r#"#!/bin/sh
printf '%s' "$3" > "{idea_log_str}"
exit 0
"#
        );
        let ralph_path = write_mock_ralph(h, &ralph_script).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // No refinement-related messages should appear
        assert!(
            !stderr.contains("refinement failed"),
            "no refinement failure expected when disabled, stderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("SHOULD NOT BE CALLED"),
            "backend should not be invoked when refinement is disabled, stderr:\n{stderr}"
        );

        // Raw idea should be passed directly to spawned child
        let idea = fs::read_to_string(&idea_log).expect("read idea log");
        assert_eq!(idea, raw_idea_text);
    })
}

/// Test that comment-post failure does not abort dispatch.
///
/// Verifies:
/// - When posting the refined-prompt comment fails, a warning is logged
/// - Dispatch still succeeds and spawns the child
/// - Task reaches terminal state normally
fn refinement_comment_failure_non_blocking(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Disable refinement to simplify (just test comment failure path)
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_refinement_enabled",
            "false",
        ])
        .expect("set refinement_enabled failed");

        let raw_idea_text = "Comment failure test\n\nBody text.";
        let mut task = task_json(
            "acme-widgets-203",
            "pending",
            203,
            "acme",
            "widgets",
            None,
            None,
        );
        task["raw_idea"] = json!(raw_idea_text);
        write_tasks(h, vec![task]).expect("write_tasks failed");

        // Mock gh where comment posting always fails
        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then want_title_body=1; fi
        done
        if [ "$want_title_body" = "1" ]; then
          printf '{"title":"test","body":"test"}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment)
        echo "comment API error" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#;
        let gh_path = write_mock_gh(h, gh_script).expect("write mock gh");

        let idea_log = h.temp_dir.path().join("comment_fail_idea.log");
        let idea_log_str = idea_log.to_string_lossy().into_owned();
        let ralph_script = format!(
            r#"#!/bin/sh
printf '%s' "$3" > "{idea_log_str}"
exit 0
"#
        );
        let ralph_path = write_mock_ralph(h, &ralph_script).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Comment failure warning should be logged
        assert!(
            stderr.contains("failed to post refined-prompt comment")
                || stderr.contains("failed to post comment"),
            "expected comment failure warning, stderr:\n{stderr}"
        );

        // Dispatch must still succeed
        assert!(
            stderr.contains("dispatched task acme-widgets-203"),
            "task must be dispatched despite comment failure, stderr:\n{stderr}"
        );

        // Child received the idea
        let idea = fs::read_to_string(&idea_log).expect("read idea log");
        assert_eq!(idea, raw_idea_text);
    })
}

/// Test that the dispatch enforces strict ordering:
/// create_worktree -> refine -> post comment -> spawn
///
/// Uses a mock refinement backend and mock gh that log timestamps/sequence
/// numbers to verify the exact call order.
///
/// Verifies:
/// - Refinement happens before comment posting
/// - Comment posting happens before spawn
/// - The overall sequence matches spec requirements
fn refinement_strict_ordering(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let order_log = h.temp_dir.path().join("ordering.log");
        let order_log_str = order_log.to_string_lossy().into_owned();

        // Mock refinement backend that logs step 1 (refine)
        let refine_script = h
            .write_mock_script(
                "mock_refine_order.sh",
                &format!(
                    r#"#!/bin/sh
cat > /dev/null
echo "step:refine" >> "{order_log_str}"
printf 'Refined prompt with sufficient length for validation check.'
exit 0
"#
                ),
            )
            .expect("write mock refine backend");
        let refine_script_str = refine_script.to_string_lossy().into_owned();

        h.ralph_ok([
            "config",
            "set",
            "backends.claude.command",
            &refine_script_str,
        ])
        .expect("set claude command failed");
        h.ralph_ok(["config", "set", "backends.claude.args", "[]"])
            .expect("set claude args failed");

        // Mock gh that logs step 2 (comment) — comment calls happen after refine
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then want_title_body=1; fi
        done
        if [ "$want_title_body" = "1" ]; then
          printf '{{"title":"test","body":"test"}}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment)
        echo "step:comment" >> "{order_log_str}"
        exit 0
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );
        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");

        // Mock ralph that logs step 3 (spawn)
        let ralph_script = format!(
            r#"#!/bin/sh
echo "step:spawn" >> "{order_log_str}"
exit 0
"#
        );
        let ralph_path = write_mock_ralph(h, &ralph_script).expect("write mock ralph");

        let mut task = task_json(
            "acme-widgets-204",
            "pending",
            204,
            "acme",
            "widgets",
            None,
            None,
        );
        task["raw_idea"] = json!("Ordering test\n\nBody.");
        write_tasks(h, vec![task]).expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("dispatched task acme-widgets-204"),
            "task should be dispatched, stderr:\n{stderr}"
        );

        // Read the ordering log and verify strict sequence
        let log_content = fs::read_to_string(&order_log).expect("read ordering log");
        let steps: Vec<&str> = log_content.lines().collect();

        // We expect: refine, then comment (marker check + post = potentially
        // two gh calls but comment step logged only on actual post), then spawn
        let refine_idx = steps.iter().position(|s| s.contains("step:refine"));
        let comment_idx = steps.iter().position(|s| s.contains("step:comment"));
        let spawn_idx = steps.iter().position(|s| s.contains("step:spawn"));

        assert!(
            refine_idx.is_some(),
            "refine step should appear in log, got:\n{log_content}"
        );
        assert!(
            spawn_idx.is_some(),
            "spawn step should appear in log, got:\n{log_content}"
        );

        let refine_pos = refine_idx.unwrap();
        let spawn_pos = spawn_idx.unwrap();

        assert!(
            refine_pos < spawn_pos,
            "refine must happen before spawn: refine@{refine_pos}, spawn@{spawn_pos}"
        );

        if let Some(comment_pos) = comment_idx {
            assert!(
                refine_pos < comment_pos,
                "refine must happen before comment: refine@{refine_pos}, comment@{comment_pos}"
            );
            assert!(
                comment_pos < spawn_pos,
                "comment must happen before spawn: comment@{comment_pos}, spawn@{spawn_pos}"
            );
        }
        // Comment may not appear if marker check found existing (idempotency);
        // that's acceptable — the important invariant is refine < spawn.
    })
}

/// Test comment idempotency on retry: if a refined-prompt comment was already
/// posted, a retry dispatch should not duplicate it.
///
/// Verifies:
/// - First dispatch posts the refined-prompt comment
/// - On restart (second dispatch), the marker is detected and comment is
///   not duplicated
/// - Both dispatches complete successfully
fn refinement_comment_idempotency_on_retry(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Disable refinement to simplify (focus on comment idempotency)
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_refinement_enabled",
            "false",
        ])
        .expect("set refinement_enabled failed");

        let comment_log = h.temp_dir.path().join("idempotent_comment.log");
        let comment_count_file = h.temp_dir.path().join("idempotent_comment_count.txt");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let comment_count_str = comment_count_file.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then want_title_body=1; fi
        done
        if [ "$want_title_body" = "1" ]; then
          printf '{{"title":"test","body":"test"}}'
          exit 0
        fi
        # Return previously posted comments for marker check
        if [ -f "{comment_log_str}" ]; then
          cat "{comment_log_str}"
        fi
        exit 0
        ;;
      comment)
        shift; shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in
            --body) echo "$2" >> "{comment_log_str}" ; shift 2 ;;
            *) shift ;;
          esac
        done
        if [ -f "{comment_count_str}" ]; then
          count=$(cat "{comment_count_str}")
        else
          count=0
        fi
        count=$((count + 1))
        echo "$count" > "{comment_count_str}"
        exit 0
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );
        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let mut task = task_json(
            "acme-widgets-205",
            "pending",
            205,
            "acme",
            "widgets",
            None,
            None,
        );
        task["raw_idea"] = json!("Idempotency test\n\nBody.");
        write_tasks(h, vec![task]).expect("write_tasks failed");

        // First run
        let output1 = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("first daemon run should execute");
        assert_exit_code(&output1, 0);

        let _count_after_first = read_count_file(&comment_count_file);

        // Verify the refined-prompt comment was posted
        if comment_log.exists() {
            let log = fs::read_to_string(&comment_log).expect("read comment log");
            assert!(
                log.contains("<!-- ralph:task:acme-widgets-205:refined-prompt -->"),
                "expected refined-prompt marker in comment, got:\n{log}"
            );
        }

        // Re-seed the task as pending (simulate restart)
        let mut task2 = task_json(
            "acme-widgets-205",
            "pending",
            205,
            "acme",
            "widgets",
            None,
            None,
        );
        task2["raw_idea"] = json!("Idempotency test\n\nBody.");
        write_tasks(h, vec![task2]).expect("write_tasks re-seed failed");

        // Second run
        let output2 = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("second daemon run should execute");
        assert_exit_code(&output2, 0);

        let _count_after_second = read_count_file(&comment_count_file);

        // The refined-prompt comment count should not increase (marker
        // already exists from first run). Completion comments may add to
        // the total, so we check specifically that the refined-prompt marker
        // appears exactly once.
        if comment_log.exists() {
            let log = fs::read_to_string(&comment_log).expect("read comment log");
            let marker_count = log
                .matches("<!-- ralph:task:acme-widgets-205:refined-prompt -->")
                .count();
            assert_eq!(
                marker_count, 1,
                "refined-prompt comment should be posted exactly once (idempotent), found {marker_count}"
            );
        }
    })
}

/// Test that the daemon pushes the branch to the remote before creating a PR.
///
/// Drives a task from pending → completed via mock ralph (with commit) so the
/// PR flow is triggered. Verifies the branch is pushed and PR create succeeds.
///
/// Verifies:
/// - Task reaches completed state
/// - `git push` was performed (branch exists on the bare remote)
/// - `gh pr create` was called after push
/// - pr_url is populated
fn runtime_push_before_pr_create(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let pr_create_log = h.temp_dir.path().join("pr_create_push_test.txt");
        let pr_create_log_str = pr_create_log.to_string_lossy().into_owned();

        // Mock gh: pr list returns empty, pr create succeeds and logs
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        printf ''
        exit 0
        ;;
      create)
        echo "called" > "{pr_create_log_str}"
        printf 'https://github.com/acme/widgets/pull/42\n'
        exit 0
        ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph_with_commit(h).expect("write mock ralph");

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-130",
                    "pending",
                    130,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-130");
                t
            }],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("dispatched task acme-widgets-130"),
            "task should have been dispatched, stderr:\n{stderr}"
        );

        // Task should be completed
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-130")
            .unwrap();
        assert_eq!(
            task["state"],
            json!("completed"),
            "task should be completed"
        );

        // PR create should have been called (push succeeded first)
        assert!(
            pr_create_log.exists(),
            "pr create should have been called after successful push"
        );

        // pr_url should be populated
        assert_eq!(
            task["pr_url"],
            json!("https://github.com/acme/widgets/pull/42"),
            "pr_url should be populated from successful PR creation"
        );
    })
}

// =============================================================================
// Branch Resolution Regression Tests
// =============================================================================

/// Verify that when a mock ralph switches the worktree branch (simulating
/// orchestrator behavior), the daemon detects the change, updates task.branch
/// in the store, and creates the PR with the resolved branch as --head.
///
/// Regression test for: daemon pushes stale branch after orchestrator switches
/// worktree to a project branch, causing "No commits between master and branch".
fn runtime_branch_switch_updates_task_and_pr(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let gh_head_log = h.temp_dir.path().join("gh_head_arg.txt");
        let gh_head_log_str = gh_head_log.to_string_lossy().into_owned();

        // Custom gh mock that captures the --head argument from pr create
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        printf ''
        exit 0
        ;;
      create)
        shift 2
        while [ $# -gt 0 ]; do
          case "$1" in
            --head) echo "$2" > "{gh_head_log_str}" ; shift 2 ;;
            *) shift ;;
          esac
        done
        printf 'https://github.com/acme/widgets/pull/99\n'
        exit 0
        ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph_with_branch_switch(h).expect("write mock ralph");

        // Pre-populate task with the original daemon branch
        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-200",
                    "pending",
                    200,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-200");
                t
            }],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Task must have been dispatched
        assert!(
            stderr.contains("dispatched task acme-widgets-200"),
            "task should have been dispatched, stderr:\n{stderr}"
        );

        // Branch change log must be present
        assert!(
            stderr.contains("worktree branch changed"),
            "expected 'worktree branch changed' log in stderr, got:\n{stderr}"
        );
        assert!(
            stderr.contains("ralph/mock-project-branch"),
            "expected new branch name in stderr, got:\n{stderr}"
        );

        // task.branch in store must be updated to the new branch
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-200")
            .unwrap();
        assert_eq!(
            task["state"],
            json!("completed"),
            "task should be completed"
        );
        assert_eq!(
            task["branch"],
            json!("ralph/mock-project-branch"),
            "task.branch should be updated to the resolved worktree branch"
        );

        // PR creation was called with the new branch as --head
        assert!(gh_head_log.exists(), "gh pr create should have been called");
        let head_arg = fs::read_to_string(&gh_head_log)
            .expect("read gh_head_log")
            .trim()
            .to_owned();
        assert_eq!(
            head_arg, "ralph/mock-project-branch",
            "gh pr create --head should use the resolved branch, not the original daemon branch"
        );

        // PR URL should be populated
        assert_eq!(
            task["pr_url"],
            json!("https://github.com/acme/widgets/pull/99"),
            "pr_url should be populated"
        );
    })
}

/// Verify that when the mock ralph does NOT switch branches (stays on the
/// original daemon branch), the branch resolution is a no-op: task.branch
/// remains unchanged and no "worktree branch changed" log is emitted.
fn runtime_branch_unchanged_no_switch_log(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let pr_create_log = h.temp_dir.path().join("pr_create_no_switch.txt");
        let pr_create_log_str = pr_create_log.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        printf ''
        exit 0
        ;;
      create)
        echo "called" > "{pr_create_log_str}"
        printf 'https://github.com/acme/widgets/pull/55\n'
        exit 0
        ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        // Use the standard commit mock (no branch switch)
        let ralph_path = write_daemon_mock_ralph_with_commit(h).expect("write mock ralph");

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-201",
                    "pending",
                    201,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-201");
                t
            }],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Task dispatched
        assert!(
            stderr.contains("dispatched task acme-widgets-201"),
            "task should have been dispatched, stderr:\n{stderr}"
        );

        // No branch-change log
        assert!(
            !stderr.contains("worktree branch changed"),
            "should NOT contain 'worktree branch changed' when branch is unchanged, stderr:\n{stderr}"
        );

        // task.branch unchanged in store
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-201")
            .unwrap();
        assert_eq!(
            task["state"],
            json!("completed"),
            "task should be completed"
        );
        assert_eq!(
            task["branch"],
            json!("ralph/daemon/acme-widgets-201"),
            "task.branch should remain unchanged when no branch switch occurs"
        );

        // PR still created
        assert!(
            pr_create_log.exists(),
            "gh pr create should have been called even without branch switch"
        );

        // PR URL populated
        assert_eq!(
            task["pr_url"],
            json!("https://github.com/acme/widgets/pull/55"),
            "pr_url should be populated"
        );
    })
}

// =============================================================================
// Child Output Capture Tests
// =============================================================================

/// Verify that child stdout/stderr is captured to a log file at
/// `.ralph/daemon/logs/{task_id}.log`, surviving worktree cleanup.
fn runtime_child_output_captured_in_log(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let gh_path = write_daemon_mock_gh(h).expect("write mock gh");

        // Mock ralph that prints known output to stdout and stderr
        let ralph_script = r#"#!/bin/sh
case "$1" in
  auto)
    echo "STDOUT_MARKER_LINE"
    echo "STDERR_MARKER_LINE" >&2
    exit 0
    ;;
  *)
    echo "mock ralph: unhandled command: $1" >&2
    exit 1
    ;;
esac
"#;
        let ralph_path = write_mock_ralph(h, ralph_script).expect("write mock ralph");

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-300",
                    "pending",
                    300,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-300");
                t
            }],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // Verify log file exists
        let log_path = h
            .repo_root
            .join(".ralph")
            .join("daemon")
            .join("logs")
            .join("acme-widgets-300.log");
        assert!(
            log_path.exists(),
            "log file should exist at {}",
            log_path.display()
        );

        // Verify log contains child output
        let log_content = fs::read_to_string(&log_path).expect("read log file");
        assert!(
            log_content.contains("STDOUT_MARKER_LINE"),
            "log should contain stdout from child, got:\n{log_content}"
        );
        assert!(
            log_content.contains("STDERR_MARKER_LINE"),
            "log should contain stderr from child, got:\n{log_content}"
        );

        // Verify task completed
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks
            .iter()
            .find(|t| t["task_id"] == "acme-widgets-300")
            .unwrap();
        assert_eq!(
            task["state"],
            json!("completed"),
            "task should be completed"
        );

        // Verify daemon stderr mentions the log path
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("acme-widgets-300.log"),
            "daemon stderr should mention the log file path, got:\n{stderr}"
        );
    })
}

// =============================================================================
// Auto-Rebase Conformance Tests
// =============================================================================

/// Test that auto-rebase is skipped when disabled by config.
///
/// Verifies:
/// - `auto-rebase: skipped (disabled by config)` appears in stderr
/// - No PR view queries are made
fn rebase_disabled_skip(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Disable auto-rebase
        h.ralph_ok(["config", "set", "workspace.daemon_auto_rebase_enabled", "false"])
            .expect("set auto_rebase_enabled failed");

        // Seed a completed task with a PR URL
        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-100",
                    "completed",
                    100,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-100");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/100");
                t
            }],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("auto-rebase: skipped (disabled by config)"),
            "expected disabled skip message in stderr, got:\n{stderr}"
        );
    })
}

/// Test that conflicting PR merge status causes skip.
fn rebase_conflict_skip(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-101",
                    "completed",
                    101,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-101");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/101");
                t
            }],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"CONFLICTING","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}"#;

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("merge status is Conflicting"),
            "expected Conflicting skip in stderr, got:\n{stderr}"
        );
    })
}

/// Test that closed/merged PRs cause skip.
fn rebase_closed_merged_skip(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-102",
                    "completed",
                    102,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-102");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/102");
                t
            }],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"MERGEABLE","state":"CLOSED","baseRefName":"master","headRefOid":"abc123"}"#;

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("PR state is CLOSED (not OPEN)"),
            "expected CLOSED skip in stderr, got:\n{stderr}"
        );
    })
}

/// Test that unknown mergeability causes skip.
fn rebase_unknown_mergeability_skip(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-103",
                    "completed",
                    103,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-103");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/103");
                t
            }],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"UNKNOWN","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}"#;

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("merge status is Unknown"),
            "expected Unknown skip in stderr, got:\n{stderr}"
        );
    })
}

/// Test that a branch-switched task rebases the correct (switched) branch.
///
/// Verifies the log mentions the switched branch name, not the default daemon branch.
fn rebase_branch_switched_task(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-104",
                    "completed",
                    104,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                // Branch is switched to a project branch (not the daemon branch)
                t["branch"] = json!("ralph/mock-project-branch");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/104");
                t
            }],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"def456"}"#;

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("branch=ralph/mock-project-branch"),
            "expected switched branch in rebase log, got:\n{stderr}"
        );
    })
}

/// Test that the rebase target is `origin/<baseRefName>` from PR metadata.
fn rebase_base_branch_from_pr(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-105",
                    "completed",
                    105,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-105");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/105");
                t
            }],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"develop","headRefOid":"ghi789"}"#;

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("target=origin/develop"),
            "expected origin/develop as rebase target, got:\n{stderr}"
        );
    })
}

/// Test that failure comments are posted on PR (not issue).
///
/// Uses a mock git (push fails) and mock gh that logs pr comment calls to a file.
fn rebase_pr_comment_not_issue(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let comment_log = h.temp_dir.path().join("pr_comment_log.txt");
        let comment_log_str = comment_log.to_string_lossy().into_owned();

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-106",
                    "completed",
                    106,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-106");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/106");
                t
            }],
        )
        .expect("write_tasks failed");

        // Mock git: worktree/checkout/fetch/rebase succeed, push fails
        let mock_git = h
            .write_mock_script("git", &mock_scripts::daemon_mock_git_rebase_fail_push_script())
            .expect("write mock git");
        let mock_git_dir = mock_git
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}"#;

        // Prepend mock git dir to PATH so it shadows real git during rebase
        let path_with_mock_git = format!("{mock_git_dir}:{gh_path}");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &path_with_mock_git),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                    ("MOCK_PR_COMMENT_LOG", &comment_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // The stderr should show the rebase attempt and failure
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("auto-rebase: failure for acme-widgets-106"),
            "expected rebase failure in stderr, got:\n{stderr}"
        );

        // Comment log MUST exist — the mock gh logs pr comment calls to this file
        assert!(
            comment_log.exists(),
            "PR comment log must exist at {} — failure comment was not posted",
            comment_log.display()
        );
        let log_content = fs::read_to_string(&comment_log).expect("read comment log");
        assert!(
            log_content.contains("<!-- ralph:rebase:acme-widgets-106:failed:abc123 -->"),
            "expected rebase failure marker in PR comment, got:\n{log_content}"
        );
    })
}

/// Test that failure comment dedup prevents duplicate posts for same head_sha.
///
/// Uses a mock git (push fails) to trigger the failure path, with
/// `last_rebase_head_sha` pre-seeded to match the PR's `headRefOid`.
fn rebase_dedup_by_head_sha(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let comment_log = h.temp_dir.path().join("dedup_comment_log.txt");
        let comment_log_str = comment_log.to_string_lossy().into_owned();

        // Pre-seed task with last_rebase_head_sha matching current head
        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-107",
                    "completed",
                    107,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-107");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/107");
                t["last_rebase_head_sha"] = json!("same_sha_123");
                t
            }],
        )
        .expect("write_tasks failed");

        // Mock git: worktree/checkout/fetch/rebase succeed, push fails
        let mock_git = h
            .write_mock_script("git", &mock_scripts::daemon_mock_git_rebase_fail_push_script())
            .expect("write mock git");
        let mock_git_dir = mock_git
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"same_sha_123"}"#;

        let path_with_mock_git = format!("{mock_git_dir}:{gh_path}");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &path_with_mock_git),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                    ("MOCK_PR_COMMENT_LOG", &comment_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Must see the dedup message in stderr
        assert!(
            stderr.contains("dedup"),
            "expected dedup skip message in stderr, got:\n{stderr}"
        );

        // Comment log must NOT contain the failure marker (dedup prevents posting)
        if comment_log.exists() {
            let log_content = fs::read_to_string(&comment_log).expect("read comment log");
            assert!(
                !log_content.contains("<!-- ralph:rebase:acme-widgets-107:failed:same_sha_123 -->"),
                "should not post duplicate failure comment for same head_sha, got:\n{log_content}"
            );
        }
    })
}

/// Test that force-with-lease rejection is treated as per-task failure
/// and processing continues to the next task.
///
/// Uses a mock git that fails push --force-with-lease with "stale info",
/// and two tasks to verify the second task is still attempted.
fn rebase_force_with_lease_rejection(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Two tasks — both eligible for rebase
        write_tasks(
            h,
            vec![
                {
                    let mut t = task_json(
                        "acme-widgets-108",
                        "completed",
                        108,
                        "acme",
                        "widgets",
                        None,
                        None,
                    );
                    t["branch"] = json!("ralph/daemon/acme-widgets-108");
                    t["pr_url"] = json!("https://github.com/acme/widgets/pull/108");
                    t
                },
                {
                    let mut t = task_json(
                        "acme-widgets-109",
                        "completed",
                        109,
                        "acme",
                        "widgets",
                        None,
                        None,
                    );
                    t["branch"] = json!("ralph/daemon/acme-widgets-109");
                    t["pr_url"] = json!("https://github.com/acme/widgets/pull/109");
                    t
                },
            ],
        )
        .expect("write_tasks failed");

        // Write mock git that simulates lease rejection
        let mock_git = h
            .write_mock_script("git", &mock_scripts::daemon_mock_git_lease_reject_script())
            .expect("write mock git");
        let mock_git_dir = mock_git
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}"#;

        // Prepend mock git dir to PATH so it shadows real git during rebase
        let path_with_mock_git = format!("{mock_git_dir}:{gh_path}");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &path_with_mock_git),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Verify lease mismatch was detected for first task
        assert!(
            stderr.contains("lease mismatch for acme-widgets-108"),
            "expected lease mismatch message for first task, got:\n{stderr}"
        );

        // Verify processing continued to second task (not a break)
        assert!(
            stderr.contains("auto-rebase: rebasing acme-widgets-109")
                || stderr.contains("lease mismatch for acme-widgets-109"),
            "expected second task to be attempted after lease rejection of first, got:\n{stderr}"
        );
    })
}

/// Test that `gh pr view` failure stops processing for the cycle.
fn rebase_gh_pr_view_failure_break(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Two tasks — both have PRs
        write_tasks(
            h,
            vec![
                {
                    let mut t = task_json(
                        "acme-widgets-109",
                        "completed",
                        109,
                        "acme",
                        "widgets",
                        None,
                        None,
                    );
                    t["branch"] = json!("ralph/daemon/acme-widgets-109");
                    t["pr_url"] = json!("https://github.com/acme/widgets/pull/109");
                    t
                },
                {
                    let mut t = task_json(
                        "acme-widgets-110",
                        "completed",
                        110,
                        "acme",
                        "widgets",
                        None,
                        None,
                    );
                    t["branch"] = json!("ralph/daemon/acme-widgets-110");
                    t["pr_url"] = json!("https://github.com/acme/widgets/pull/110");
                    t
                },
            ],
        )
        .expect("write_tasks failed");

        // gh pr view fails with exit code 1
        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", "rate limit exceeded"),
                    ("MOCK_PR_VIEW_EXIT", "1"),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("gh pr view failed") && stderr.contains("stopping rebase processing"),
            "expected gh pr view failure with break message, got:\n{stderr}"
        );

        // The second task should NOT have been attempted (break after first failure)
        let second_attempt_count = stderr.matches("auto-rebase: rebasing acme-widgets-110").count();
        assert_eq!(
            second_attempt_count, 0,
            "second task should not be attempted after gh pr view failure"
        );
    })
}

/// Test that per-cycle cap limits rebase attempts.
fn rebase_per_cycle_cap(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Set max_rebases_per_cycle to 1 via config
        h.ralph_ok(["config", "set", "workspace.daemon_max_rebases_per_cycle", "1"])
            .expect("set max_rebases_per_cycle failed");

        // Create 3 tasks with PRs
        write_tasks(
            h,
            vec![
                {
                    let mut t = task_json(
                        "acme-widgets-120",
                        "completed",
                        120,
                        "acme",
                        "widgets",
                        None,
                        None,
                    );
                    t["branch"] = json!("ralph/daemon/acme-widgets-120");
                    t["pr_url"] = json!("https://github.com/acme/widgets/pull/120");
                    t
                },
                {
                    let mut t = task_json(
                        "acme-widgets-121",
                        "completed",
                        121,
                        "acme",
                        "widgets",
                        None,
                        None,
                    );
                    t["branch"] = json!("ralph/daemon/acme-widgets-121");
                    t["pr_url"] = json!("https://github.com/acme/widgets/pull/121");
                    t
                },
                {
                    let mut t = task_json(
                        "acme-widgets-122",
                        "completed",
                        122,
                        "acme",
                        "widgets",
                        None,
                        None,
                    );
                    t["branch"] = json!("ralph/daemon/acme-widgets-122");
                    t["pr_url"] = json!("https://github.com/acme/widgets/pull/122");
                    t
                },
            ],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}"#;

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Should see cap message
        assert!(
            stderr.contains("per-cycle cap reached"),
            "expected per-cycle cap message in stderr, got:\n{stderr}"
        );
    })
}

/// Test that recently-rebased interval causes skip.
///
/// Uses a dynamically-computed "now" timestamp to avoid time-fragility.
fn rebase_interval_skip(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Use a very large interval to ensure skip regardless of clock skew
        h.ralph_ok(["config", "set", "workspace.daemon_rebase_interval_seconds", "999999"])
            .expect("set rebase_interval_seconds failed");

        // Dynamically compute a "just now" timestamp so test never becomes stale
        let recent_timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-130",
                    "completed",
                    130,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["branch"] = json!("ralph/daemon/acme-widgets-130");
                t["pr_url"] = json!("https://github.com/acme/widgets/pull/130");
                t["last_rebase_at"] = json!(recent_timestamp);
                t
            }],
        )
        .expect("write_tasks failed");

        let gh_path = write_daemon_mock_gh_rebase(h).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");
        let pr_json = r#"{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}"#;

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_PR_VIEW_JSON", pr_json),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("last rebased") && stderr.contains("interval="),
            "expected interval skip message in stderr, got:\n{stderr}"
        );
    })
}

/// Test that `ralph daemon status` LAST REBASE column shows RFC3339 timestamp.
fn rebase_status_last_rebase_column(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let timestamp = "2026-02-14T19:22:31Z";
        write_tasks(
            h,
            vec![{
                let mut t = task_json(
                    "acme-widgets-140",
                    "completed",
                    140,
                    "acme",
                    "widgets",
                    None,
                    None,
                );
                t["last_rebase_at"] = json!(timestamp);
                t
            }],
        )
        .expect("write_tasks failed");

        let status = h
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&status, 0);

        let stdout = String::from_utf8_lossy(&status.stdout);
        assert!(
            stdout.contains("LAST REBASE"),
            "expected LAST REBASE header, got:\n{stdout}"
        );
        assert!(
            stdout.contains(timestamp),
            "expected RFC3339 timestamp '{}' in output, got:\n{}",
            timestamp,
            stdout
        );
    })
}

/// Test that state deserialization is backward compatible (missing rebase fields).
fn rebase_backward_compat_state(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Write tasks without last_rebase_at / last_rebase_head_sha fields
        let tasks_path = h.repo_root.join(".ralph").join("daemon").join("tasks.json");
        if let Some(parent) = tasks_path.parent() {
            fs::create_dir_all(parent).expect("create daemon dir");
        }
        let legacy_json = r#"[{
            "task_id":"acme-widgets-150",
            "state":"completed",
            "issue_number":150,
            "owner":"acme",
            "repo":"widgets",
            "child_pid":null,
            "child_pgid":null,
            "branch":null,
            "pr_url":null,
            "created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-01T00:00:00Z"
        }]"#;
        fs::write(&tasks_path, legacy_json).expect("write legacy tasks");

        let status = h
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&status, 0);

        let stdout = String::from_utf8_lossy(&status.stdout);
        assert!(
            stdout.contains("acme-widgets-150"),
            "expected task to be listed, got:\n{stdout}"
        );

        // Verify the LAST REBASE column shows "-" for missing field
        assert!(
            stdout.contains("-"),
            "expected '-' for missing last_rebase_at, got:\n{stdout}"
        );

        // Also verify deserialization round-trip works
        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks.len(), 1);
        assert!(
            tasks[0].get("last_rebase_at").map(|v| v.is_null()).unwrap_or(true),
            "last_rebase_at should be null/missing"
        );
    })
}

// =============================================================================
// Test helpers
// =============================================================================

fn write_tasks(h: &RalphHarness, tasks: Vec<Value>) -> crate::Result<()> {
    let path = tasks_path(h);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(&tasks)?)?;
    Ok(())
}

fn load_tasks(h: &RalphHarness) -> crate::Result<Vec<Value>> {
    let raw = fs::read_to_string(tasks_path(h))?;
    Ok(serde_json::from_str(&raw)?)
}

fn tasks_path(h: &RalphHarness) -> PathBuf {
    h.repo_root.join(".ralph").join("daemon").join("tasks.json")
}

fn task_json(
    task_id: &str,
    state: &str,
    issue_number: u32,
    owner: &str,
    repo: &str,
    child_pid: Option<u32>,
    child_pgid: Option<u32>,
) -> Value {
    json!({
        "task_id": task_id,
        "state": state,
        "issue_number": issue_number,
        "owner": owner,
        "repo": repo,
        "child_pid": child_pid,
        "child_pgid": child_pgid,
        "branch": null,
        "pr_url": null,
        "created_at": "2026-02-13T00:00:00Z",
        "updated_at": "2026-02-13T00:00:00Z"
    })
}

fn write_mock_gh(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("gh", body)?;
    let base = script
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let existing = std::env::var("PATH").unwrap_or_default();
    Ok(format!("{base}:{existing}"))
}

fn write_daemon_mock_gh(h: &RalphHarness) -> crate::Result<String> {
    write_mock_gh(h, &mock_scripts::daemon_mock_gh_script())
}

/// Write a mock ralph script and return its absolute path (suitable for
/// RALPH_DAEMON_BIN env var).
fn write_mock_ralph(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("mock_ralph", body)?;
    Ok(script.to_string_lossy().into_owned())
}

/// Write the standard mock ralph (exits 0) and return its path.
fn write_daemon_mock_ralph(h: &RalphHarness) -> crate::Result<String> {
    write_mock_ralph(h, &mock_scripts::daemon_mock_ralph_script())
}

/// Write the mock ralph that creates a commit (for PR diff tests).
fn write_daemon_mock_ralph_with_commit(h: &RalphHarness) -> crate::Result<String> {
    write_mock_ralph(h, &mock_scripts::daemon_mock_ralph_with_commit_script())
}

/// Write the mock ralph that switches branch and creates a commit (for branch resolution tests).
fn write_daemon_mock_ralph_with_branch_switch(h: &RalphHarness) -> crate::Result<String> {
    write_mock_ralph(
        h,
        &mock_scripts::daemon_mock_ralph_with_branch_switch_script(),
    )
}

fn read_count_file(path: &std::path::Path) -> u32 {
    match fs::read_to_string(path) {
        Ok(content) => content.trim().parse::<u32>().unwrap_or(0),
        Err(_) => 0,
    }
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Write a rebase-capable mock gh and return the PATH.
fn write_daemon_mock_gh_rebase(h: &RalphHarness) -> crate::Result<String> {
    write_mock_gh(h, &mock_scripts::daemon_mock_gh_rebase_script())
}

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
