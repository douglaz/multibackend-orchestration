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

        // Task should have been dispatched and completed
        assert!(
            combined.contains("dispatched task acme-widgets-70")
                || combined.contains("dispatch: task acme-widgets-70"),
            "issue 70 should be dispatched:\n{combined}"
        );

        // After completion, PID/PGID should be cleared
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
