use super::*;

use std::fs;

use crate::daemon::runtime::{load_task_metadata, save_task_metadata, TaskMetadata};
use crate::validate::assertions::assert_exit_code;
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "daemon_orphan::reconciliation_skips_live_orphan",
            func: reconciliation_skips_live_orphan,
        },
        ConformanceTest {
            name: "daemon_orphan::reconciliation_resets_dead_orphan",
            func: reconciliation_resets_dead_orphan,
        },
        ConformanceTest {
            name: "daemon_orphan::pid_reuse_rejected_by_pgid_mismatch",
            func: pid_reuse_rejected_by_pgid_mismatch,
        },
        ConformanceTest {
            name: "daemon_orphan::adopted_orphan_counts_toward_max_concurrent",
            func: adopted_orphan_counts_toward_max_concurrent,
        },
        ConformanceTest {
            name: "daemon_orphan::no_duplicate_dispatch_for_adopted_orphan",
            func: no_duplicate_dispatch_for_adopted_orphan,
        },
        ConformanceTest {
            name: "daemon_orphan::pid_lifecycle_dispatch_to_collect",
            func: pid_lifecycle_dispatch_to_collect,
        },
        ConformanceTest {
            name: "daemon_orphan::abort_kills_adopted_orphan",
            func: abort_kills_adopted_orphan,
        },
        ConformanceTest {
            name: "daemon_orphan::orphan_terminalization_routes_through_complete_task",
            func: orphan_terminalization_routes_through_complete_task,
        },
        ConformanceTest {
            name: "daemon_orphan::crash_after_spawn_before_stage3",
            func: crash_after_spawn_before_stage3,
        },
        ConformanceTest {
            name: "daemon_orphan::dispatch_failure_clears_pid",
            func: dispatch_failure_clears_pid,
        },
    ]
}

// ---- helpers ----

fn write_mock_gh(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("gh", body)?;
    let base = script
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let existing = std::env::var("PATH").unwrap_or_default();
    Ok(format!("{base}:{existing}"))
}

fn write_mock_ralph(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("mock_ralph", body)?;
    Ok(script.to_string_lossy().into_owned())
}

fn write_daemon_mock_ralph(h: &RalphHarness) -> crate::Result<String> {
    write_mock_ralph(h, &mock_scripts::daemon_mock_ralph_script())
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
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

/// Write task metadata to the correct path for the daemon harness.
/// The daemon's workspace_root is `<data_dir>/<owner>/<repo>/.ralph`.
fn write_task_metadata(h: &RalphHarness, owner: &str, repo: &str, task_id: &str, meta: &TaskMetadata) {
    let workspace_root = h.data_dir().join(owner).join(repo).join(".ralph");
    save_task_metadata(&workspace_root, task_id, meta);
}

fn read_task_metadata(h: &RalphHarness, owner: &str, repo: &str, task_id: &str) -> TaskMetadata {
    let workspace_root = h.data_dir().join(owner).join(repo).join(".ralph");
    load_task_metadata(&workspace_root, task_id)
}

// ---- tests ----

/// Spawn a long-running `sleep` child via setsid, persist its PID/PGID to
/// TaskMetadata, label an issue ralph:in-progress, run reconciliation.
/// Assert: label is NOT reset, orphan is adopted (logged as such).
fn reconciliation_skips_live_orphan(h: &RalphHarness) -> TestResult {
    run_case(|| {
        use std::os::unix::process::CommandExt;

        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Spawn a session-leader child process
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .process_group(0)
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id();
        let pgid = pid; // session leader: pid == pgid

        // Persist PID/PGID into task metadata
        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(pid),
            pgid: Some(pgid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-10", &meta);

        let label_log = dh.temp_dir.path().join("orphan_live_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock GH: reconciliation sees issue 10 as in-progress, poll returns empty
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[{{"number":10,"title":"orphan issue","labels":[{{"name":"ralph:in-progress"}}],"body":"body"}}]'
              exit 0
              ;;
            ralph:ready)
              printf '[]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
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

        // Clean up child
        let _ = child.kill();
        let _ = child.wait();

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // Should see adoption message, NOT reset message
        assert!(
            combined.contains("reconcile: adopting orphan for issue #10"),
            "expected orphan adoption message for issue #10 in output:\n{combined}"
        );
        assert!(
            combined.contains("reconcile: adopted 1 surviving orphan(s)"),
            "expected adopted count in output:\n{combined}"
        );

        // The label log should NOT contain a reset to ralph:ready for this issue
        let log = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            !log.contains("ralph:ready"),
            "issue 10 should NOT be reset to ralph:ready:\n{log}"
        );
    })
}

/// Persist PID/PGID for a non-existent process, label issue ralph:in-progress,
/// run reconciliation. Assert: label IS reset to ralph:ready, PID/PGID cleared.
fn reconciliation_resets_dead_orphan(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Use a dead PID (very high value)
        let dead_pid = u32::MAX - 10;
        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(dead_pid),
            pgid: Some(dead_pid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-20", &meta);

        let label_log = dh.temp_dir.path().join("orphan_dead_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[{{"number":20,"title":"dead issue","labels":[{{"name":"ralph:in-progress"}}],"body":"body"}}]'
              exit 0
              ;;
            ralph:ready)
              printf '[]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
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

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // Should reset (not adopt)
        assert!(
            combined.contains("reconcile: reset 1 in-progress issue(s) to ready"),
            "expected reset message in output:\n{combined}"
        );

        // PID/PGID should be cleared from metadata
        let meta = read_task_metadata(&dh, "acme", "widgets", "acme-widgets-20");
        assert_eq!(meta.pid, None, "PID should be cleared after dead-process reconciliation");
        assert_eq!(meta.pgid, None, "PGID should be cleared after dead-process reconciliation");
    })
}

/// Persist metadata with pid set to the current process's PID but pgid set to a
/// non-existent value (simulating PID reuse where the new process has a different
/// PGID). Assert: reconciliation treats this as dead and resets the label.
fn pid_reuse_rejected_by_pgid_mismatch(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Use current process PID (alive!) but a mismatched PGID
        let live_pid = std::process::id();
        let fake_pgid = u32::MAX - 20;
        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(live_pid),
            pgid: Some(fake_pgid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-30", &meta);

        let label_log = dh.temp_dir.path().join("orphan_pgid_mismatch_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[{{"number":30,"title":"pgid mismatch issue","labels":[{{"name":"ralph:in-progress"}}],"body":"body"}}]'
              exit 0
              ;;
            ralph:ready)
              printf '[]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
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

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // Should be treated as dead due to pid != pgid (session-leader invariant)
        assert!(
            combined.contains("reconcile: reset 1 in-progress issue(s) to ready"),
            "expected reset due to PGID mismatch in output:\n{combined}"
        );
    })
}

/// Set max_concurrent=1, adopt one orphan, run poll_and_claim.
/// Assert: no new issues are claimed (0 available slots).
fn adopted_orphan_counts_toward_max_concurrent(h: &RalphHarness) -> TestResult {
    run_case(|| {
        use std::os::unix::process::CommandExt;

        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Spawn a session-leader child to be adopted
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .process_group(0)
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id();

        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(pid),
            pgid: Some(pid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-40", &meta);

        let label_log = dh.temp_dir.path().join("orphan_slots_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock GH: issue 40 in-progress (will be adopted), issue 50 ready (should NOT be claimed)
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[{{"number":40,"title":"adopted orphan","labels":[{{"name":"ralph:in-progress"}}],"body":"body"}}]'
              exit 0
              ;;
            ralph:ready)
              printf '[{{"number":50,"title":"ready issue","labels":[{{"name":"ralph:ready"}}],"body":"body"}}]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

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
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");

        // Clean up child
        let _ = child.kill();
        let _ = child.wait();

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // Orphan should be adopted
        assert!(
            combined.contains("reconcile: adopting orphan for issue #40"),
            "expected orphan adoption for issue #40:\n{combined}"
        );

        // Issue 50 should NOT be dispatched because orphan fills the slot
        assert!(
            !combined.contains("dispatch: task acme-widgets-50")
                && !combined.contains("dispatched task acme-widgets-50"),
            "issue 50 should NOT be dispatched when orphan fills max_concurrent=1:\n{combined}"
        );
    })
}

/// Adopt an orphan for issue #N, then run poll_and_claim with issue #N in
/// the ralph:in-progress poll results. Assert: issue #N is skipped, no dispatch.
fn no_duplicate_dispatch_for_adopted_orphan(h: &RalphHarness) -> TestResult {
    run_case(|| {
        use std::os::unix::process::CommandExt;

        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Spawn a session-leader child to be adopted as issue 60
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .process_group(0)
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id();

        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(pid),
            pgid: Some(pid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-60", &meta);

        let label_log = dh.temp_dir.path().join("orphan_nodup_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock GH: issue 60 in-progress (will be adopted), also appears as ready
        // (simulating a race or labeling quirk — should still be skipped)
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[{{"number":60,"title":"adopted","labels":[{{"name":"ralph:in-progress"}}],"body":"body"}}]'
              exit 0
              ;;
            ralph:ready)
              printf '[{{"number":60,"title":"adopted dup","labels":[{{"name":"ralph:ready"}}],"body":"body"}}]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                    "--max-concurrent",
                    "4",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");

        // Clean up child
        let _ = child.kill();
        let _ = child.wait();

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // Should be adopted, NOT dispatched as new
        assert!(
            combined.contains("reconcile: adopting orphan for issue #60"),
            "expected orphan adoption for issue #60:\n{combined}"
        );
        assert!(
            !combined.contains("dispatch: task acme-widgets-60")
                && !combined.contains("dispatched task acme-widgets-60"),
            "issue 60 should NOT be dispatched when already adopted as orphan:\n{combined}"
        );
    })
}

/// Run a full dispatch_task → collect_children cycle.
/// Assert: TaskMetadata has pid/pgid set after dispatch, and cleared after collect.
fn pid_lifecycle_dispatch_to_collect(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        let label_log = dh.temp_dir.path().join("pid_lifecycle_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Compute the metadata file and snapshot paths.
        // The daemon's workspace_root is <data_dir>/<owner>/<repo>/.ralph,
        // and task metadata lives at <workspace_root>/daemon/tasks/<task_id>.json.
        let workspace_root = dh.data_dir().join("acme").join("widgets").join(".ralph");
        let meta_path = workspace_root
            .join("daemon")
            .join("tasks")
            .join("acme-widgets-70.json");
        let snapshot_path = dh.temp_dir.path().join("meta_snapshot_70.json");

        // Mock GH: one ready issue that gets dispatched
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[]'
              exit 0
              ;;
            ralph:ready)
              printf '[{{"number":70,"title":"lifecycle issue","labels":[{{"name":"ralph:ready"}}],"body":"body"}}]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
            exit 0
          fi
        done
        printf ''
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  pr)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      *) exit 0 ;;
    esac
    ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");

        // Use a mock ralph that snapshots the metadata file mid-dispatch
        // (after dispatch_task writes PID/PGID, before collect_children clears them).
        let ralph_path = write_mock_ralph(
            &dh,
            &mock_scripts::daemon_mock_ralph_meta_snapshot_script(&meta_path, &snapshot_path),
        )
        .expect("write mock ralph");

        let output = dh
            .daemon_env(
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

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // Task should have been dispatched and completed
        assert!(
            combined.contains("dispatched task acme-widgets-70")
                || combined.contains("dispatch: task acme-widgets-70"),
            "issue 70 should be dispatched:\n{combined}"
        );

        // Phase 1: Assert PID/PGID were set during dispatch (intermediate state).
        // The mock ralph snapshots the metadata file while it is still alive,
        // after dispatch_task has written PID/PGID.
        assert!(
            snapshot_path.exists(),
            "metadata snapshot should have been created by mock ralph"
        );
        let snapshot_content = fs::read_to_string(&snapshot_path)
            .expect("read snapshot");
        let snapshot_meta: TaskMetadata = serde_json::from_str(&snapshot_content)
            .expect("parse snapshot metadata");
        assert!(
            snapshot_meta.pid.is_some(),
            "PID should be set in metadata during dispatch (snapshot: {snapshot_content})"
        );
        assert!(
            snapshot_meta.pgid.is_some(),
            "PGID should be set in metadata during dispatch (snapshot: {snapshot_content})"
        );

        // Phase 2: After completion, PID/PGID should be cleared
        let meta = read_task_metadata(&dh, "acme", "widgets", "acme-widgets-70");
        assert_eq!(
            meta.pid, None,
            "PID should be cleared after child completes"
        );
        assert_eq!(
            meta.pgid, None,
            "PGID should be cleared after child completes"
        );
    })
}

/// Adopt an orphan, swap its label away from ralph:in-progress externally,
/// run kill_aborted_children. Assert: orphan's process group is terminated.
fn abort_kills_adopted_orphan(h: &RalphHarness) -> TestResult {
    run_case(|| {
        use std::os::unix::process::CommandExt;

        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Spawn a session-leader child for issue 80
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .process_group(0)
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id();

        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(pid),
            pgid: Some(pid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-80", &meta);

        let label_log = dh.temp_dir.path().join("orphan_abort_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock GH: issue 80 in-progress for reconciliation, then when queried
        // for abort check returns ralph:failed (externally aborted)
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[{{"number":80,"title":"abort orphan","labels":[{{"name":"ralph:in-progress"}}],"body":"body"}}]'
              exit 0
              ;;
            ralph:ready)
              printf '[]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        # For abort check, return ralph:failed (simulating external abort)
        printf '{{"labels":[{{"name":"ralph:failed"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
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

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // The orphan should be adopted first, then killed by abort check
        assert!(
            combined.contains("reconcile: adopting orphan for issue #80"),
            "expected orphan adoption for issue #80:\n{combined}"
        );
        assert!(
            combined.contains("abort-check: killed adopted orphan acme-widgets-80"),
            "expected abort kill message for adopted orphan:\n{combined}"
        );

        // PID/PGID should be cleared from metadata
        let meta = read_task_metadata(&dh, "acme", "widgets", "acme-widgets-80");
        assert_eq!(meta.pid, None, "PID should be cleared after abort kill");
        assert_eq!(meta.pgid, None, "PGID should be cleared after abort kill");

        // Child should be dead (killed by terminate_process_group)
        // Give it a moment to die
        std::thread::sleep(std::time::Duration::from_millis(200));
        let status = child.try_wait().expect("check child status");
        assert!(
            status.is_some(),
            "orphan child process should be terminated after abort"
        );
    })
}

/// Spawn a live orphan so reconciliation adopts it, then kill the process
/// before poll_adopted_orphans runs. Assert: complete_task side effects fire
/// (completion comment posted, label swapped to terminal state, PID/PGID cleared).
fn orphan_terminalization_routes_through_complete_task(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Spawn a real session-leader child via setsid + exec so that the PID
        // captured in the pidfile is the actual long-lived process (not a
        // short-lived wrapper shell). Using `exec sleep` replaces the shell
        // with sleep, so the pidfile PID remains valid and `pid_exists` returns
        // true throughout the test.
        let pidfile = dh.temp_dir.path().join("orphan_terminalize.pid");
        let pidfile_str = pidfile.to_string_lossy().into_owned();
        let spawn_status = std::process::Command::new("sh")
            .args([
                "-c",
                &format!(
                    "setsid sh -c 'echo $$ > {pidfile_str}; exec sleep 300' </dev/null >/dev/null 2>&1 &"
                ),
            ])
            .status()
            .expect("spawn session-leader sleep via shell");
        assert!(spawn_status.success(), "setsid spawn command should succeed");

        // Wait for pidfile to appear (bounded)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let pid: u32 = loop {
            if let Ok(content) = fs::read_to_string(&pidfile) {
                if let Ok(p) = content.trim().parse::<u32>() {
                    break p;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for pidfile"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let pgid = pid; // setsid makes pid == pgid

        // Confirm the process is alive before proceeding
        assert!(
            crate::daemon::process::pid_exists(pid),
            "orphan process (pid={pid}) should be alive before test begins"
        );

        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(pid),
            pgid: Some(pgid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-90", &meta);

        let label_log = dh.temp_dir.path().join("orphan_terminalize_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();
        let comment_log = dh.temp_dir.path().join("orphan_terminalize_comment.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();

        // Mock GH: issue 90 in-progress for reconciliation.
        // The `issue view` handler kills the orphan process group so that by the
        // time poll_adopted_orphans checks liveness the process is dead.
        // kill_aborted_children calls `gh issue view` before poll_adopted_orphans.
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[{{"number":90,"title":"terminalize orphan","labels":[{{"name":"ralph:in-progress"}}],"body":"body"}}]'
              exit 0
              ;;
            ralph:ready)
              printf '[]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      comment)
        echo "$@" >> "{comment_log_str}"
        exit 0
        ;;
      view)
        # Kill the orphan process group so it is dead by poll_adopted_orphans.
        # Use SIGKILL for immediate effect; wait briefly for kernel cleanup.
        kill -9 -{pid} 2>/dev/null
        sleep 0.05
        printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      *) exit 0 ;;
    esac
    ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
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

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // Orphan should be adopted during reconciliation (process was alive)
        assert!(
            combined.contains("reconcile: adopting orphan for issue #90"),
            "expected orphan adoption for issue #90:\n{combined}"
        );
        // Then terminalized during poll_adopted_orphans (process killed by mock gh)
        assert!(
            combined.contains("orphan-poll: process dead for issue #90"),
            "expected orphan terminalization message for issue #90:\n{combined}"
        );

        // complete_task should post a completion comment (side effect)
        let comments = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            comments.contains("acme-widgets-90"),
            "expected completion comment for acme-widgets-90:\n{comments}\nfull output:\n{combined}"
        );

        // Label should be swapped from in-progress to terminal (ralph:failed since no merged PR)
        let labels = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            labels.contains("ralph:failed"),
            "expected terminal label swap to ralph:failed:\n{labels}\nfull output:\n{combined}"
        );

        // PID/PGID should be cleared from metadata after terminalization
        let meta = read_task_metadata(&dh, "acme", "widgets", "acme-widgets-90");
        assert_eq!(meta.pid, None, "PID should be cleared after terminalization");
        assert_eq!(meta.pgid, None, "PGID should be cleared after terminalization");

        // Clean up child in case kill didn't reach it
        unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
    })
}

/// Simulate the critical crash window: dispatch_task has spawned a child and
/// written PID/PGID to metadata, but the daemon crashes before Stage 3
/// inserts the ChildHandle into `children`.  On restart, reconciliation should
/// detect the live child via persisted PID/PGID and adopt it as an orphan
/// rather than resetting the label.
///
/// This test uses manual spawn + metadata write (not real `dispatch_task`)
/// because simulating a mid-dispatch crash is not possible through the daemon
/// binary.  The `pid_lifecycle_dispatch_to_collect` test validates that the
/// real `dispatch_task` path correctly persists PID/PGID.  This test focuses
/// on the reconciliation logic: given a live process with matching metadata,
/// the daemon should adopt it rather than resetting labels.
///
/// The metadata is written via the same `TaskMetadata` struct and
/// `save_task_metadata` function that `dispatch_task` uses, ensuring format
/// compatibility between the two tests.
fn crash_after_spawn_before_stage3(h: &RalphHarness) -> TestResult {
    run_case(|| {
        use std::os::unix::process::CommandExt;

        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Spawn a session-leader child (simulates the child spawned by dispatch_task).
        // Uses process_group(0) to mirror the setsid() behavior in dispatch_task.
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .process_group(0)
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id();
        let pgid = pid; // session leader: pid == pgid

        // Persist PID/PGID using the same struct/function as dispatch_task.
        // We do NOT insert into children (simulating crash before Stage 3).
        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(pid),
            pgid: Some(pgid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-100", &meta);

        let label_log = dh.temp_dir.path().join("crash_window_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock GH: issue 100 in-progress (left over from pre-crash dispatch)
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[{{"number":100,"title":"crash window issue","labels":[{{"name":"ralph:in-progress"}}],"body":"body"}}]'
              exit 0
              ;;
            ralph:ready)
              printf '[]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
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

        // Clean up child
        let _ = child.kill();
        let _ = child.wait();

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // The live child should be detected via persisted PID/PGID and adopted
        assert!(
            combined.contains("reconcile: adopting orphan for issue #100"),
            "expected orphan adoption for crash-window issue #100:\n{combined}"
        );
        assert!(
            combined.contains("reconcile: adopted 1 surviving orphan(s)"),
            "expected adopted count in output:\n{combined}"
        );

        // The label should NOT be reset to ralph:ready (process is still alive)
        let log = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            !log.contains("ralph:ready"),
            "issue 100 should NOT be reset to ralph:ready (crash-window adoption):\n{log}"
        );

        // PID/PGID should still be present in metadata (orphan is alive, not yet terminalized)
        let meta = read_task_metadata(&dh, "acme", "widgets", "acme-widgets-100");
        assert_eq!(meta.pid, Some(pid), "PID should still be set for live adopted orphan");
        assert_eq!(meta.pgid, Some(pgid), "PGID should still be set for live adopted orphan");
    })
}

/// Pre-set PID/PGID in task metadata, trigger a dispatch failure (via a mock
/// ralph that errors out), and assert PID/PGID are defensively cleared.
fn dispatch_failure_clears_pid(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement");

        // Pre-set PID/PGID in metadata (simulating a partial spawn that wrote PID before failing)
        let stale_pid = u32::MAX - 40;
        let meta = TaskMetadata {
            pr_url: None,
            pid: Some(stale_pid),
            pgid: Some(stale_pid),
        };
        write_task_metadata(&dh, "acme", "widgets", "acme-widgets-110", &meta);

        let label_log = dh.temp_dir.path().join("dispatch_fail_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock GH: no in-progress issues, one ready issue that will be dispatched (and fail)
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '[]'
              exit 0
              ;;
            ralph:ready)
              printf '[{{"number":110,"title":"dispatch fail issue","labels":[{{"name":"ralph:ready"}}],"body":"body"}}]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
        exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        // Mock ralph that exits with error to trigger dispatch failure
        let ralph_path = write_mock_ralph(&dh, "#!/bin/sh\nexit 1\n").expect("write failing mock ralph");

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");

        let output = dh
            .daemon_env(
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

        let combined = combined_output(&output);
        assert_exit_code(&output, 0);

        // PID/PGID should be defensively cleared from metadata after dispatch failure
        let meta = read_task_metadata(&dh, "acme", "widgets", "acme-widgets-110");
        assert_eq!(
            meta.pid, None,
            "PID should be cleared after dispatch failure:\n{combined}"
        );
        assert_eq!(
            meta.pgid, None,
            "PGID should be cleared after dispatch failure:\n{combined}"
        );
    })
}
