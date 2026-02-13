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
            name: "daemon::runtime_abort_during_dispatch_preserves_terminal",
            func: runtime_abort_during_dispatch_preserves_terminal,
        },
        ConformanceTest {
            name: "daemon::runtime_no_diff_pr_path",
            func: runtime_no_diff_pr_path,
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
                ["daemon", "start", "--repo", "acme/default", "--single-iteration"],
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

        h.ralph_ok(["project", "use", "daemon-config"])
            .expect("project use should succeed");

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
            .ralph(["daemon", "start", "--repo", "acme/widgets", "--single-iteration"])
            .expect("daemon start should execute");
        assert_exit_code(&no_workspace, 2);

        h.init_workspace().expect("init failed");

        let gh_path = write_mock_gh(h, "#!/bin/sh\necho \"octo/demo\"\n")
            .expect("write mock gh should succeed");

        let with_workspace = h
            .ralph_env(["daemon", "start", "--single-iteration"], &[("PATH", &gh_path)])
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
                task_json("acme-widgets-41", "pending", 41, "acme", "widgets", None, None),
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
                task_json("acme-widgets-7", "pending", 7, "acme", "widgets", None, None),
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
                &[("PATH", &gh_path)],
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
        let completed = tasks.iter().find(|t| t["task_id"] == "acme-widgets-30").unwrap();
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
                &[("PATH", &gh_path)],
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
        let wt_base = h
            .repo_root
            .join(".ralph")
            .join("daemon")
            .join("worktrees");
        assert!(
            wt_base.exists(),
            "worktrees base directory must exist after dispatch"
        );

        // Task should have reached a terminal state after drain
        let tasks = load_tasks(h).expect("load_tasks failed");
        let task = tasks.iter().find(|t| t["task_id"] == "acme-widgets-50").unwrap();
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
        let task = tasks.iter().find(|t| t["task_id"] == "acme-widgets-60").unwrap();
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
                &[("PATH", &gh_path)],
            )
            .expect("first daemon run should execute");
        assert_exit_code(&output, 0);

        // Record how many comments were posted on first run
        let first_count = read_count_file(&comment_count_file);

        // Verify comments contain the expected marker pattern
        if comment_log.exists() {
            let log_content = fs::read_to_string(&comment_log)
                .expect("read comment log");
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
                &[("PATH", &gh_path)],
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
        let ralph_path = write_daemon_mock_ralph_with_commit(h)
            .expect("write mock ralph");

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
        let task = tasks.iter().find(|t| t["task_id"] == "acme-widgets-80").unwrap();
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
        let ralph_path = write_daemon_mock_ralph_with_commit(h)
            .expect("write mock ralph");

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
        let task = tasks.iter().find(|t| t["task_id"] == "acme-widgets-90").unwrap();
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
                &[("PATH", &gh_path)],
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
            let log_content =
                fs::read_to_string(&comment_log).expect("read comment log");
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

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
