use super::*;

use std::fs;
use std::process::{Command, Stdio};

use crate::validate::assertions::assert_exit_code;
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "daemon_concurrency::concurrent_dispatch_two_issues",
            func: concurrent_dispatch_two_issues,
        },
        ConformanceTest {
            name: "daemon_concurrency::drain_terminates_all_tasks_uniformly",
            func: drain_terminates_all_tasks_uniformly,
        },
        ConformanceTest {
            name: "daemon_concurrency::single_iteration_prd_inline_only",
            func: single_iteration_prd_inline_only,
        },
        ConformanceTest {
            name: "daemon_concurrency::concurrent_rebase_dispatch_no_lock_contention",
            func: concurrent_rebase_dispatch_no_lock_contention,
        },
        ConformanceTest {
            name: "daemon_concurrency::drain_marks_all_tasks_failed",
            func: drain_marks_all_tasks_failed,
        },
        ConformanceTest {
            name: "daemon_concurrency::concurrent_dispatch_evidence",
            func: concurrent_dispatch_evidence,
        },
        ConformanceTest {
            name: "daemon_concurrency::drain_cancellation_terminalization",
            func: drain_cancellation_terminalization,
        },
        ConformanceTest {
            name: "daemon_concurrency::execution_failure_terminalization",
            func: execution_failure_terminalization,
        },
        ConformanceTest {
            name: "daemon_concurrency::mixed_outcome_claim_isolation",
            func: mixed_outcome_claim_isolation,
        },
        ConformanceTest {
            name: "daemon_concurrency::per_task_log_isolation",
            func: per_task_log_isolation,
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

fn write_daemon_mock_gh(h: &RalphHarness) -> crate::Result<String> {
    write_mock_gh(h, &mock_scripts::daemon_mock_gh_script())
}

fn write_daemon_mock_gh_concurrency(h: &RalphHarness) -> crate::Result<String> {
    write_mock_gh(h, &mock_scripts::daemon_mock_gh_concurrency_script())
}

/// Like `enable_fast_daemon_refinement` but uses a backend that writes
/// per-invocation start/end nanosecond timestamps to `barrier_dir`, enabling
/// callers to assert temporal overlap across concurrent task executions.
fn enable_timed_daemon_refinement(
    h: &RalphHarness,
    barrier_dir: &std::path::Path,
) -> crate::Result<()> {
    let barrier_dir_str = barrier_dir.to_string_lossy();
    let script_body = format!(
        r#"#!/bin/sh
BARRIER_DIR="{barrier_dir_str}"
mkdir -p "$BARRIER_DIR"
START=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
echo "$START" > "$BARRIER_DIR/task_$$.start"
# Sleep briefly so concurrent tasks have an overlap window
sleep 0.3
END=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
echo "$END" > "$BARRIER_DIR/task_$$.end"
printf 'TITLE: Refined task execution\n'
printf '---\n'
printf 'Refined task body with explicit steps.\n'
"#
    );
    let refine_script = h.write_mock_script("mock_refine_timed.sh", &script_body)?;
    let refine_script_str = refine_script.to_string_lossy().into_owned();
    h.ralph_ok([
        "config",
        "set",
        "backends.claude.command",
        &refine_script_str,
    ])?;
    h.ralph_ok(["config", "set", "backends.claude.args", "[]"])?;
    h.ralph_ok([
        "config",
        "set",
        "workspace.daemon_refinement_enabled",
        "true",
    ])?;
    Ok(())
}

fn enable_fast_daemon_refinement(h: &RalphHarness) -> crate::Result<()> {
    let refine_script = h.write_mock_script(
        "mock_refine_fast.sh",
        &mock_scripts::daemon_mock_fast_refinement_script(),
    )?;
    let refine_script_str = refine_script.to_string_lossy().into_owned();
    h.ralph_ok([
        "config",
        "set",
        "backends.claude.command",
        &refine_script_str,
    ])?;
    h.ralph_ok(["config", "set", "backends.claude.args", "[]"])?;
    h.ralph_ok([
        "config",
        "set",
        "workspace.daemon_refinement_enabled",
        "true",
    ])?;
    Ok(())
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

/// Count non-overlapping occurrences of `needle` in `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

// ---- tests ----

/// Verifies that two ralph:ready issues are both claimed and dispatched
/// concurrently in a single poll cycle. Both should produce worktrees and
/// complete successfully.
fn concurrent_dispatch_two_issues(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("concurrent_dispatch_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock gh returns TWO ralph:ready issues
        let issues = r#"[{"number":200,"title":"issue A","labels":[{"name":"ralph:ready"}],"body":"body A"},{"number":201,"title":"issue B","labels":[{"name":"ralph:ready"}],"body":"body B"}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");

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
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");

        // Daemon must exit successfully in single-iteration mode
        assert_exit_code(&output, 0);

        let combined = combined_output(&output);

        // Both issues should have been dispatched (in-process)
        assert!(
            combined.contains("dispatched task acme-widgets-200")
                || combined.contains("dispatch: task acme-widgets-200"),
            "issue 200 should be dispatched: {combined}"
        );
        assert!(
            combined.contains("dispatched task acme-widgets-201")
                || combined.contains("dispatch: task acme-widgets-201"),
            "issue 201 should be dispatched: {combined}"
        );

        // Label log must exist — if it doesn't, label transitions never happened
        assert!(
            label_log.exists(),
            "label log should exist at {}: {combined}",
            label_log.display(),
        );

        let log_content = fs::read_to_string(&label_log).expect("read label log");
        // Both issues should have in-progress claims
        assert!(
            log_content.contains("200"),
            "issue 200 should appear in label operations: {log_content}"
        );
        assert!(
            log_content.contains("201"),
            "issue 201 should appear in label operations: {log_content}"
        );
    })
}

/// Verifies that drain-induced cancellation uniformly terminalizes all
/// dispatched tasks with independent label transitions.
///
/// Two issues are claimed and dispatched as concurrent in-process tasks.
/// In single-iteration mode, drain_all_children cancels all tasks and each
/// reaches terminal state independently with its own label transitions.
///
/// NOTE: Both tasks receive the same cancellation treatment (both fail),
/// so this test proves uniform drain behavior, not partial rollback or
/// sibling isolation. See `execution_failure_terminalization` for
/// execution-error failures and `mixed_outcome_claim_isolation` for
/// claim-level isolation.
///
/// Asserts:
/// - Both issues are dispatched (in-process)
/// - Both issues reach terminal state with independent label transitions
/// - Each issue's label transitions reference only its own issue number
fn drain_terminates_all_tasks_uniformly(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("partial_rollback_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let issues = r#"[{"number":300,"title":"good issue","labels":[{"name":"ralph:ready"}],"body":"good body"},{"number":301,"title":"bad issue","labels":[{"name":"ralph:ready"}],"body":"bad body"}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");

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
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");

        let combined = combined_output(&output);

        // Both issues should be dispatched (in-process)
        assert!(
            combined.contains("dispatched task acme-widgets-300")
                || combined.contains("dispatch: task acme-widgets-300"),
            "issue 300 should be dispatched: {combined}"
        );
        assert!(
            combined.contains("dispatched task acme-widgets-301")
                || combined.contains("dispatch: task acme-widgets-301"),
            "issue 301 should be dispatched: {combined}"
        );

        // Verify per-issue label transitions in the label log
        assert!(
            label_log.exists(),
            "label log should exist at {}",
            label_log.display()
        );
        let log_content = fs::read_to_string(&label_log).expect("read label log");
        let log_lines: Vec<&str> = log_content.lines().collect();

        // Both issues should have been claimed: ready -> in-progress
        assert!(
            log_lines.iter().any(|l| l.contains("300")
                && l.contains("--add-label")
                && l.contains("ralph:in-progress")),
            "issue 300 should have been claimed (add ralph:in-progress): {log_content}"
        );
        assert!(
            log_lines.iter().any(|l| l.contains("301")
                && l.contains("--add-label")
                && l.contains("ralph:in-progress")),
            "issue 301 should have been claimed (add ralph:in-progress): {log_content}"
        );

        // Both issues should reach terminal state independently.
        // In single-iteration mode, drain_all_children cancels in-process
        // tasks, producing ralph:failed for each.
        assert!(
            log_lines.iter().any(|l| l.contains("300")
                && l.contains("--add-label")
                && l.contains("ralph:failed")),
            "issue 300 should reach terminal state (ralph:failed): {log_content}"
        );
        assert!(
            log_lines.iter().any(|l| l.contains("301")
                && l.contains("--add-label")
                && l.contains("ralph:failed")),
            "issue 301 should reach terminal state (ralph:failed): {log_content}"
        );
    })
}

/// Verifies that in single-iteration mode with PRD enabled (default),
/// exactly one inline PRD tick runs and no PRD background task is spawned.
///
/// Verification strategy:
/// 1. Use the concurrency mock GH script which logs `prd-tick` to
///    `MOCK_PRD_TICK_LOG` every time `gh issue list` is called with a
///    `ralph:prd` or `ralph:prd-active` label. This provides direct,
///    causal observability of how many PRD poll ticks executed.
/// 2. Assert that **exactly** 2 `prd-tick` log lines are recorded.
///    `poll_and_advance_prd` issues 2 `gh issue list` calls per tick
///    (one for `ralph:prd`, one for `ralph:prd-active`), so exactly 2
///    lines proves exactly 1 tick ran. Anything else is a regression.
/// 3. Assert no PRD background task shutdown messages appear.
/// 4. Assert daemon exits cleanly with code 0.
fn single_iteration_prd_inline_only(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("prd_inline_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let prd_tick_log = dh.temp_dir.path().join("prd_tick.log");
        let prd_tick_log_str = prd_tick_log.to_string_lossy().into_owned();

        // No issues — we just want to verify PRD phase runs inline
        let issues = r#"[]"#;

        let gh_path = write_daemon_mock_gh_concurrency(&dh).expect("write mock gh");

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
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                    ("MOCK_PRD_TICK_LOG", &prd_tick_log_str),
                ],
            )
            .expect("daemon start should execute");

        // Daemon should exit successfully in single-iteration mode
        assert_exit_code(&output, 0);

        let combined = combined_output(&output);

        // 1. No PRD background task shutdown messages should appear.
        //    These markers are only emitted by Phase 4 (PRD shutdown) which
        //    only runs when a background PRD JoinHandle exists.
        assert!(
            !combined.contains("PRD background task panicked"),
            "single-iteration mode must not produce PRD background panic message: {combined}"
        );
        assert!(
            !combined.contains("PRD background task did not stop"),
            "single-iteration mode must not produce PRD background timeout message: {combined}"
        );

        // 2. The background PRD tick failure marker is only emitted inside
        //    the spawned continuous-mode task. Its absence proves no background
        //    task was spawned.
        assert!(
            !combined.contains("PRD background tick failed"),
            "single-iteration mode must not emit background tick log: {combined}"
        );

        // 3. Verify PRD tick count via the mock GH logging file.
        //    `poll_and_advance_prd` issues 2 `gh issue list` calls per tick
        //    (one for ralph:prd, one for ralph:prd-active), each producing a
        //    `prd-tick` log line. Exactly 2 lines = exactly 1 inline tick.
        assert!(
            prd_tick_log.exists(),
            "PRD tick log must exist — inline PRD tick should have fired: {combined}"
        );
        let tick_log_content = fs::read_to_string(&prd_tick_log).expect("read prd tick log");
        let tick_count = tick_log_content
            .lines()
            .filter(|l| l.contains("prd-tick"))
            .count();
        assert!(
            tick_count == 2,
            "expected exactly 2 prd-tick log lines (1 inline tick = 2 gh calls), got {tick_count}. \
             PRD is enabled by default and single-iteration mode must run exactly one inline tick: {combined}"
        );

        // 4. If the inline PRD phase ran and failed, the distinct
        //    "interactive PRD phase failed" marker is used. Count occurrences
        //    to prove at most one tick ran.
        let inline_prd_failures = count_occurrences(&combined, "interactive PRD phase failed");
        assert!(
            inline_prd_failures <= 1,
            "at most one inline PRD tick should run in single-iteration mode, but saw {inline_prd_failures} failures: {combined}"
        );

        // 5. Daemon exited 0 without hanging — this would not happen if a
        //    background PRD task was spawned and kept running.
    })
}

/// Verifies that concurrent rebase and dispatch operations do not produce
/// git lock contention errors when the repo-root semaphore serializes
/// root-level git operations.
///
/// Strategy: Run the daemon in continuous mode (not single-iteration) with a
/// bounded-run approach. The mock GH returns one claimable issue on the first
/// `issue list` call and empty thereafter. A long-running mock ralph keeps the
/// child alive across iteration boundaries. In iteration 2+, the `auto_rebase_phase`
/// discovers the active child, looks up its PR URL and merge metadata, validating
/// the rebase candidate discovery code path. The test asserts:
/// - The issue was dispatched successfully (dispatch path worked)
/// - The `auto_rebase_phase` entered and attempted rebase candidate discovery
///   (via `MOCK_REBASE_ATTEMPT_LOG`)
/// - No git lock contention errors occurred
/// - No unhandled mock GH commands
///
/// The daemon is killed via SIGTERM after a bounded wait, and output is captured.
fn concurrent_rebase_dispatch_no_lock_contention(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("rebase_dispatch_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let issue_counter = dh.temp_dir.path().join("issue_list_counter");
        let issue_counter_str = issue_counter.to_string_lossy().into_owned();

        let rebase_log = dh.temp_dir.path().join("rebase_attempt.log");
        let rebase_log_str = rebase_log.to_string_lossy().into_owned();

        // One issue to claim and dispatch. The bounded mock returns this only
        // on the first `issue list` call; subsequent calls return `[]`.
        let issues = r#"[{"number":400,"title":"rebase test A","labels":[{"name":"ralph:ready"}],"body":"rebase test body A"}]"#;

        // Use the bounded concurrency mock that returns issues once and
        // supports rebase PR lookup + merge metadata queries.
        let gh_path = write_mock_gh(
            &dh,
            &mock_scripts::daemon_mock_gh_bounded_concurrency_script(),
        )
        .expect("write mock gh");
        // Disable PRD to keep the test focused on rebase + dispatch.
        // PRD config keys are rejected by `config set` CLI; write TOML directly.
        // Insert immediately after the [workspace] header so the key stays in
        // the correct TOML section even when other sections follow.
        {
            let config_path = dh.repo_root.join(".ralph").join("ralph.toml");
            let mut toml_content = fs::read_to_string(&config_path).unwrap_or_default();
            if let Some(pos) = toml_content.find("[workspace]") {
                let insert_pos = toml_content[pos..]
                    .find('\n')
                    .map(|p| pos + p + 1)
                    .unwrap_or(toml_content.len());
                toml_content.insert_str(insert_pos, "daemon_prd_enabled = false\n");
            } else {
                toml_content.push_str("\n[workspace]\ndaemon_prd_enabled = false\n");
            }
            fs::write(&config_path, toml_content).expect("write config to disable PRD");
        }

        let data_dir_str = dh.data_dir().to_string_lossy().into_owned();

        // Spawn daemon in continuous mode (non-blocking).
        // We construct args manually since `prepare_cli_args` is private;
        // inject `--data-dir` after `daemon start` as the harness would.
        let child = Command::new(&dh.ralph_bin)
            .args([
                "daemon",
                "start",
                "--data-dir",
                &data_dir_str,
                "--repo",
                "acme/widgets",
                "--max-concurrent",
                "4",
                "--poll-seconds",
                "1",
            ])
            .current_dir(dh.data_dir())
            .env("PATH", &gh_path)
            .env("MOCK_GH_ISSUES", issues)
            .env("MOCK_GH_LABEL_LOG", &label_log_str)
            .env("MOCK_GH_ISSUE_LIST_COUNTER", &issue_counter_str)
            .env("MOCK_REBASE_ATTEMPT_LOG", &rebase_log_str)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn daemon in continuous mode");

        // Wait long enough for at least 2 iterations (poll_seconds=1, so
        // iteration 1 dispatches child at ~0s, iteration 2 at ~1s runs
        // auto_rebase_phase with active child). 8s is generous.
        std::thread::sleep(std::time::Duration::from_secs(8));

        // Send SIGTERM to stop the daemon gracefully
        let pid = nix::unistd::Pid::from_raw(child.id() as i32);
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);

        // Wait for the process to exit and capture output
        let output = child.wait_with_output().expect("wait for daemon output");

        let combined = combined_output(&output);

        // 1. Issue should have been dispatched in iteration 1
        assert!(
            combined.contains("dispatched task acme-widgets-400")
                || combined.contains("dispatch: task acme-widgets-400"),
            "issue 400 should be dispatched: {combined}"
        );

        // 2. auto_rebase_phase should have discovered the child as a rebase
        //    candidate in iteration 2+. The bounded mock logs each `pr list --head`
        //    call to MOCK_REBASE_ATTEMPT_LOG.
        assert!(
            rebase_log.exists(),
            "rebase attempt log must exist — auto_rebase_phase should have looked up PR for child: {combined}"
        );
        let rebase_log_content = fs::read_to_string(&rebase_log).expect("read rebase attempt log");
        let rebase_attempts = rebase_log_content
            .lines()
            .filter(|l| l.contains("rebase-pr-lookup"))
            .count();
        assert!(
            rebase_attempts >= 1,
            "expected at least 1 rebase PR lookup attempt (auto_rebase_phase exercised), got {rebase_attempts}: {combined}"
        );

        // 3. No git lock contention errors
        assert!(
            !combined.contains("index.lock"),
            "should not have git lock contention: {combined}"
        );

        // 4. No unhandled mock gh commands
        assert!(
            !combined.contains("mock gh: unhandled"),
            "mock gh should handle all commands without errors: {combined}"
        );
    })
}

/// Verifies that drain marks all cancelled tasks as failed with
/// deterministic per-task failure markers.
///
/// Two issues are claimed and dispatched as in-process tasks. In
/// single-iteration mode, drain_all_children cancels all tasks and each
/// reaches terminal ralph:failed state. The mock GH logs explicit
/// `dispatch-failure:<issue>` markers when it detects a label swap to
/// ralph:failed.
///
/// NOTE: Both tasks are cancelled during drain (same outcome). This test
/// proves each task independently produces its failure marker. See
/// `execution_failure_terminalization` for the execution-error path and
/// `mixed_outcome_claim_isolation` for mixed claim outcomes.
///
/// Asserts:
/// - Both issues were dispatched (in-process)
/// - Each issue produced its own dispatch-failure marker in the failure log
fn drain_marks_all_tasks_failed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("dispatch_fail_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let failure_log = dh.temp_dir.path().join("dispatch_failure.log");
        let failure_log_str = failure_log.to_string_lossy().into_owned();

        let issues = r#"[{"number":500,"title":"good dispatch","labels":[{"name":"ralph:ready"}],"body":"body A"},{"number":501,"title":"bad dispatch","labels":[{"name":"ralph:ready"}],"body":"body B"}]"#;

        let gh_path = write_mock_gh(&dh, &mock_scripts::daemon_mock_gh_dispatch_failure_script())
            .expect("write mock gh");

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
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                    ("MOCK_DISPATCH_FAILURE_LOG", &failure_log_str),
                ],
            )
            .expect("daemon start should execute");

        let combined = combined_output(&output);

        // Both issues should be dispatched (in-process)
        assert!(
            combined.contains("dispatched task acme-widgets-500")
                || combined.contains("dispatch: task acme-widgets-500"),
            "issue 500 should be dispatched: {combined}"
        );
        assert!(
            combined.contains("dispatched task acme-widgets-501")
                || combined.contains("dispatch: task acme-widgets-501"),
            "issue 501 should be dispatched: {combined}"
        );

        // Failure log should contain dispatch-failure markers for both issues
        // (both reach terminal ralph:failed via drain_all_children cancellation)
        assert!(
            failure_log.exists(),
            "dispatch failure log should exist at {}: {combined}",
            failure_log.display()
        );
        let failure_content = fs::read_to_string(&failure_log).expect("read failure log");
        assert!(
            failure_content.contains("dispatch-failure:501"),
            "dispatch-failure:501 marker expected in failure log: {failure_content}"
        );
        assert!(
            failure_content.contains("dispatch-failure:500"),
            "dispatch-failure:500 marker expected in failure log: {failure_content}"
        );
    })
}

/// Verifies concurrent in-process dispatch of multiple issues.
///
/// Two issues are dispatched as concurrent tokio tasks. Concurrency is
/// proven by asserting temporal overlap: the timed mock backend records
/// per-invocation start/end nanosecond timestamps, and the test verifies
/// that at least two backend invocations were alive at the same time
/// (i.e., one started before the other ended).
fn concurrent_dispatch_evidence(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let barrier_dir = dh.temp_dir.path().join("concurrency_barrier");
        fs::create_dir_all(&barrier_dir).expect("create barrier dir");
        enable_timed_daemon_refinement(&dh, &barrier_dir)
            .expect("configure timed refinement backend for test");

        let issues = r#"[{"number":600,"title":"concurrent A","labels":[{"name":"ralph:ready"}],"body":"body A"},{"number":601,"title":"concurrent B","labels":[{"name":"ralph:ready"}],"body":"body B"}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");

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
                &[("PATH", &gh_path), ("MOCK_GH_ISSUES", issues)],
            )
            .expect("daemon start should execute");

        let combined = combined_output(&output);

        // Both issues should be dispatched as in-process tasks
        assert!(
            combined.contains("dispatched task acme-widgets-600 (in-process)")
                || combined.contains("dispatch: task acme-widgets-600"),
            "issue 600 should be dispatched (in-process): {combined}"
        );
        assert!(
            combined.contains("dispatched task acme-widgets-601 (in-process)")
                || combined.contains("dispatch: task acme-widgets-601"),
            "issue 601 should be dispatched (in-process): {combined}"
        );

        // Both tasks should reach terminal state
        assert!(
            combined.contains("acme-widgets-600 completed")
                || combined.contains("collect: task acme-widgets-600"),
            "issue 600 should reach terminal state: {combined}"
        );
        assert!(
            combined.contains("acme-widgets-601 completed")
                || combined.contains("collect: task acme-widgets-601"),
            "issue 601 should reach terminal state: {combined}"
        );

        // --- Concurrency evidence: timestamp overlap ---
        // Collect (start, end) intervals from the timed mock backend.
        // Each backend invocation writes task_<pid>.start and task_<pid>.end
        // files with nanosecond timestamps. True concurrency requires that
        // at least two intervals overlap (A.start < B.end AND B.start < A.end).
        let mut intervals: Vec<(u128, u128)> = Vec::new();
        for entry in fs::read_dir(&barrier_dir).expect("read barrier dir") {
            let entry = entry.expect("read dir entry");
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".start") {
                let pid = name_str.trim_end_matches(".start");
                let end_file = barrier_dir.join(format!("{pid}.end"));
                if end_file.exists() {
                    let start_ns: u128 = fs::read_to_string(entry.path())
                        .expect("read start timestamp")
                        .trim()
                        .parse()
                        .expect("parse start timestamp");
                    let end_ns: u128 = fs::read_to_string(&end_file)
                        .expect("read end timestamp")
                        .trim()
                        .parse()
                        .expect("parse end timestamp");
                    intervals.push((start_ns, end_ns));
                }
            }
        }

        assert!(
            intervals.len() >= 2,
            "expected at least 2 timed backend invocations, got {}; \
             barrier_dir contents: {:?}\n{combined}",
            intervals.len(),
            fs::read_dir(&barrier_dir)
                .map(|rd| rd
                    .filter_map(|e| e.ok().map(|e| e.file_name()))
                    .collect::<Vec<_>>())
                .unwrap_or_default(),
        );

        // Check for at least one pair with temporal overlap
        let mut found_overlap = false;
        'outer: for i in 0..intervals.len() {
            for j in (i + 1)..intervals.len() {
                let (a_start, a_end) = intervals[i];
                let (b_start, b_end) = intervals[j];
                // Overlap: A started before B ended AND B started before A ended
                if a_start < b_end && b_start < a_end {
                    found_overlap = true;
                    break 'outer;
                }
            }
        }
        assert!(
            found_overlap,
            "no temporal overlap detected among {} backend invocations; \
             intervals (start_ns, end_ns): {intervals:?}\n\
             This indicates tasks ran sequentially, not concurrently.\n{combined}",
            intervals.len(),
        );

        // Secondary sanity: log ordering (both dispatches before any terminal)
        let last_dispatch_pos = ["acme-widgets-600", "acme-widgets-601"]
            .iter()
            .filter_map(|id| {
                combined
                    .find(&format!("dispatched task {id} (in-process)"))
                    .or_else(|| combined.find(&format!("dispatch: task {id}")))
            })
            .max();

        let first_terminal_pos = ["acme-widgets-600", "acme-widgets-601"]
            .iter()
            .filter_map(|id| {
                combined
                    .find(&format!("collect: task {id}"))
                    .or_else(|| {
                        combined.find(&format!(
                            "complete-task-terminal: preserving worktree for {id}"
                        ))
                    })
                    .or_else(|| combined.find(&format!("verbose: task terminal task_id={id}")))
            })
            .min();

        if let (Some(last_dispatch), Some(first_terminal)) = (last_dispatch_pos, first_terminal_pos)
        {
            assert!(
                last_dispatch < first_terminal,
                "concurrency ordering violated: last dispatch at byte {last_dispatch} \
                 but first terminal at byte {first_terminal} — \
                 both dispatches must precede any terminal event.\n{combined}"
            );
        }
    })
}

/// Verifies that drain-induced cancellation in single-iteration mode
/// correctly terminalizes in-progress tasks to `ralph:failed`.
///
/// Strategy:
/// - Dispatch one issue as an in-process task.
/// - In single-iteration mode, `drain_all_children` cancels all active
///   tasks' `CancellationToken`s, causing them to return `Err(Cancelled)`.
/// - `collect_children` picks up the cancelled task and transitions
///   `ralph:in-progress` -> `ralph:failed`.
/// - Assert the label log contains the `ralph:failed` terminal transition.
///
/// NOTE: This test exercises the *cancellation* failure path (via drain),
/// not a true backend execution failure. See
/// `execution_failure_terminalization` for the execution failure path.
fn drain_cancellation_terminalization(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("completion_fail_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let issues = r#"[{"number":700,"title":"failing task","labels":[{"name":"ralph:ready"}],"body":"will fail"}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");

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
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");

        let combined = combined_output(&output);

        // 1. Issue 700 should have been dispatched
        assert!(
            combined.contains("dispatched task acme-widgets-700")
                || combined.contains("dispatch: task acme-widgets-700"),
            "issue 700 should be dispatched: {combined}"
        );

        // 2. Issue 700 should be terminalized as ralph:failed
        assert!(
            combined.contains("task acme-widgets-700 completed with label: ralph:failed"),
            "issue 700 should reach terminal ralph:failed state: {combined}"
        );

        // 3. Verify label log contains terminal transition for issue 700
        assert!(
            label_log.exists(),
            "label log should exist at {}: {combined}",
            label_log.display()
        );
        let log_content = fs::read_to_string(&label_log).expect("read label log");
        let log_lines: Vec<&str> = log_content.lines().collect();

        // Terminal transition: remove ralph:in-progress + add ralph:failed
        // This comes from complete_task -> swap_lifecycle_label
        assert!(
            log_lines.iter().any(|l| l.contains("700")
                && l.contains("--add-label")
                && l.contains("ralph:failed")),
            "issue 700 should have terminal ralph:failed label in log: {log_content}"
        );

        // 4. Issue should NOT remain as ralph:in-progress (no stuck state)
        // The last label operation for 700 should be the terminal transition.
        // Verify the terminal failed label was applied (already checked above).
        // Additionally ensure the daemon logged the terminal state message.
        assert!(
            combined.contains("ralph:failed"),
            "daemon output should indicate terminal failed state: {combined}"
        );
    })
}

/// Verifies that a genuine backend execution failure (non-zero exit)
/// causes the issue to be terminalized to `ralph:failed` through the
/// normal `collect_children` path — not via drain-induced cancellation.
///
/// Strategy:
/// - Configure the `claude` backend to a script that always exits 1.
/// - Run the daemon in continuous mode so `collect_children` naturally
///   discovers the failed task without `drain_all_children` masking the
///   failure signal.
/// - The in-process task fails during quick-prd (BackendCommandFailed)
///   because the backend exits non-zero.
/// - Assert `"collect: task ... failed:"` appears in output (the execution
///   failure path, not the `"cancelled"` path).
/// - Assert the label log contains `ralph:failed` for the issue.
fn execution_failure_terminalization(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Configure claude backend to always fail (exit 1).
        // Health check (which::which) still passes because the script exists.
        let fail_script = dh
            .write_mock_script("fail_backend.sh", "#!/bin/sh\nexit 1\n")
            .expect("write fail script");
        let fail_script_str = fail_script.to_string_lossy().into_owned();
        dh.ralph_ok(["config", "set", "backends.claude.command", &fail_script_str])
            .expect("set failing backend command");
        dh.ralph_ok(["config", "set", "backends.claude.args", "[]"])
            .expect("set backend args");
        // Refinement will fail gracefully (non-fatal), falling back to raw idea.
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_refinement_enabled",
            "true",
        ])
        .expect("enable refinement");

        // Disable PRD to keep the test focused on execution failure.
        {
            let config_path = dh.repo_root.join(".ralph").join("ralph.toml");
            let mut toml_content = fs::read_to_string(&config_path).unwrap_or_default();
            if let Some(pos) = toml_content.find("[workspace]") {
                let insert_pos = toml_content[pos..]
                    .find('\n')
                    .map(|p| pos + p + 1)
                    .unwrap_or(toml_content.len());
                toml_content.insert_str(insert_pos, "daemon_prd_enabled = false\n");
            } else {
                toml_content.push_str("\n[workspace]\ndaemon_prd_enabled = false\n");
            }
            fs::write(&config_path, toml_content).expect("write config to disable PRD");
        }

        let label_log = dh.temp_dir.path().join("exec_fail_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let issue_counter = dh.temp_dir.path().join("exec_fail_counter");
        let issue_counter_str = issue_counter.to_string_lossy().into_owned();

        let issues = r#"[{"number":750,"title":"will fail from execution","labels":[{"name":"ralph:ready"}],"body":"fail body"}]"#;

        let gh_path = write_mock_gh(
            &dh,
            &mock_scripts::daemon_mock_gh_bounded_concurrency_script(),
        )
        .expect("write mock gh");

        let data_dir_str = dh.data_dir().to_string_lossy().into_owned();

        // Continuous mode: collect_children discovers the failed task
        // naturally, without drain_all_children cancellation.
        let child = Command::new(&dh.ralph_bin)
            .args([
                "daemon",
                "start",
                "--data-dir",
                &data_dir_str,
                "--repo",
                "acme/widgets",
                "--max-concurrent",
                "1",
                "--poll-seconds",
                "1",
            ])
            .current_dir(dh.data_dir())
            .env("PATH", &gh_path)
            .env("MOCK_GH_ISSUES", issues)
            .env("MOCK_GH_LABEL_LOG", &label_log_str)
            .env("MOCK_GH_ISSUE_LIST_COUNTER", &issue_counter_str)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn daemon in continuous mode");

        // Wait for: dispatch (iteration 1) + task failure + collection
        // (iteration 2). 8 seconds is generous for poll_seconds=1.
        std::thread::sleep(std::time::Duration::from_secs(8));

        // SIGTERM to stop the daemon gracefully
        let pid = nix::unistd::Pid::from_raw(child.id() as i32);
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);

        let output = child.wait_with_output().expect("wait for daemon output");
        let combined = combined_output(&output);

        // 1. Issue 750 should have been dispatched
        assert!(
            combined.contains("dispatched task acme-widgets-750")
                || combined.contains("dispatch: task acme-widgets-750"),
            "issue 750 should be dispatched: {combined}"
        );

        // 2. Task should fail from execution error (not cancellation).
        // collect_children logs "collect: task ... failed: {err}" for
        // execution errors and "collect: task ... cancelled" for
        // CancellationToken-induced failures.
        assert!(
            combined.contains("collect: task acme-widgets-750 failed:"),
            "issue 750 should fail from execution error path \
             (\"collect: task ... failed:\"), not cancellation: {combined}"
        );

        // 3. Should NOT show "cancelled" for this task (it was a real failure)
        assert!(
            !combined.contains("collect: task acme-widgets-750 cancelled"),
            "issue 750 failure should NOT be from cancellation: {combined}"
        );

        // 4. Label log should have ralph:failed for issue 750
        assert!(
            label_log.exists(),
            "label log should exist at {}: {combined}",
            label_log.display()
        );
        let log_content = fs::read_to_string(&label_log).expect("read label log");
        assert!(
            log_content.lines().any(|l| l.contains("750")
                && l.contains("--add-label")
                && l.contains("ralph:failed")),
            "issue 750 should have terminal ralph:failed label: {log_content}"
        );
    })
}

/// Verifies mixed-outcome isolation: one issue fails to claim while
/// the sibling is claimed, dispatched, and reaches terminal state
/// independently.
///
/// Uses a mock GH script where `MOCK_GH_CLAIM_FAIL_ISSUE=901` causes
/// the label claim (`ralph:ready` -> `ralph:in-progress`) to fail for
/// issue 901, while issue 900 proceeds normally.
///
/// Asserts:
/// 1. Issue 900 is dispatched (in-process) — the claim succeeded
/// 2. Issue 901 is NOT dispatched — the claim failed
/// 3. Issue 900 reaches terminal state independently
/// 4. Label log contains `ralph:in-progress` claim for issue 900 only
/// 5. Issue 901's claim failure does not cause a rollback or label
///    transition (no `ralph:failed` for 901 — it was never claimed)
fn mixed_outcome_claim_isolation(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("mixed_outcome_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let issues = r#"[{"number":900,"title":"good issue","labels":[{"name":"ralph:ready"}],"body":"good body"},{"number":901,"title":"fail claim","labels":[{"name":"ralph:ready"}],"body":"fail body"}]"#;

        let gh_path = write_mock_gh(&dh, &mock_scripts::daemon_mock_gh_mixed_outcome_script())
            .expect("write mock gh");

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
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                    ("MOCK_GH_CLAIM_FAIL_ISSUE", "901"),
                ],
            )
            .expect("daemon start should execute");

        let combined = combined_output(&output);

        // 1. Issue 900 should be dispatched (claim succeeded)
        assert!(
            combined.contains("dispatched task acme-widgets-900")
                || combined.contains("dispatch: task acme-widgets-900"),
            "issue 900 should be dispatched: {combined}"
        );

        // 2. Issue 901 should NOT be dispatched (claim failed)
        assert!(
            !combined.contains("dispatched task acme-widgets-901")
                && !combined.contains("dispatch: task acme-widgets-901"),
            "issue 901 should NOT be dispatched (claim failed): {combined}"
        );

        // 3. Issue 900 should reach terminal state
        assert!(
            combined.contains("collect: task acme-widgets-900")
                || combined.contains("acme-widgets-900 completed"),
            "issue 900 should reach terminal state: {combined}"
        );

        // 4. Verify label log isolation
        assert!(
            label_log.exists(),
            "label log should exist at {}: {combined}",
            label_log.display()
        );
        let log_content = fs::read_to_string(&label_log).expect("read label log");
        let log_lines: Vec<&str> = log_content.lines().collect();

        // Issue 900 should have been claimed (add ralph:in-progress)
        assert!(
            log_lines.iter().any(|l| l.contains("900")
                && l.contains("--add-label")
                && l.contains("ralph:in-progress")),
            "issue 900 should have been claimed: {log_content}"
        );

        // 5. Issue 901 should NOT have ralph:failed — it was never
        //    successfully claimed, so no rollback label swap occurs.
        assert!(
            !log_lines.iter().any(|l| l.contains("901")
                && l.contains("--add-label")
                && l.contains("ralph:failed")),
            "issue 901 should NOT have ralph:failed (never claimed): {log_content}"
        );

        // Issue 901's claim failure is logged but does not affect 900
        assert!(
            combined.contains("failed to claim issue #901") || combined.contains("claim failure"),
            "claim failure for 901 should be logged: {combined}"
        );
    })
}

/// Validates per-task log isolation for concurrent in-process daemon tasks.
///
/// Dispatches two issues concurrently.  Each in-process task emits a
/// deterministic `RALPH_TASK_STARTED` tracing marker (with its unique
/// project_id) at the very start of the entry point, before any async work
/// or cancellation checks.  The per-task tracing subscriber routes each
/// task's output to its own log file.
///
/// Asserts:
/// - Both task log files exist
/// - Each log file contains the deterministic `RALPH_TASK_STARTED` marker
///   with its own task's project_id
/// - No cross-task contamination occurs (no other task's project_id appears)
fn per_task_log_isolation(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // A simple no-op backend — we don't rely on backend output for markers.
        // The deterministic RALPH_TASK_STARTED tracing markers emitted by the
        // task entry points (before any backend invocation) are sufficient.
        let script_body = r#"#!/bin/sh
printf 'TITLE: noop\n---\nnoop refinement\n'
"#;
        let refine_script = dh
            .write_mock_script("mock_log_isolation.sh", script_body)
            .expect("write mock log-isolation script");
        let refine_script_str = refine_script.to_string_lossy().into_owned();
        dh.ralph_ok([
            "config",
            "set",
            "backends.claude.command",
            &refine_script_str,
        ])
        .expect("set backend command");
        dh.ralph_ok(["config", "set", "backends.claude.args", "[]"])
            .expect("set backend args");
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_refinement_enabled",
            "true",
        ])
        .expect("enable daemon refinement");

        let label_log = dh.temp_dir.path().join("log_isolation_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Two concurrent issues
        let issues = r#"[{"number":800,"title":"log test A","labels":[{"name":"ralph:ready"}],"body":"log isolation A"},{"number":801,"title":"log test B","labels":[{"name":"ralph:ready"}],"body":"log isolation B"}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");

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
                &[
                    ("PATH", &gh_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let combined = combined_output(&output);

        // Both issues should be dispatched
        assert!(
            combined.contains("dispatched task acme-widgets-800")
                || combined.contains("dispatch: task acme-widgets-800"),
            "issue 800 should be dispatched: {combined}"
        );
        assert!(
            combined.contains("dispatched task acme-widgets-801")
                || combined.contains("dispatch: task acme-widgets-801"),
            "issue 801 should be dispatched: {combined}"
        );

        // Locate task log files: <repo_root>/.ralph/tmp/logs/<task_id>.log
        let log_dir = dh.repo_root.join(".ralph").join("tmp").join("logs");
        let log_800 = log_dir.join("acme-widgets-800.log");
        let log_801 = log_dir.join("acme-widgets-801.log");

        assert!(
            log_800.exists(),
            "task log for issue 800 should exist at {}: {combined}",
            log_800.display()
        );
        assert!(
            log_801.exists(),
            "task log for issue 801 should exist at {}: {combined}",
            log_801.display()
        );

        let content_800 = fs::read_to_string(&log_800).expect("read log 800");
        let content_801 = fs::read_to_string(&log_801).expect("read log 801");

        // Each log file must contain the deterministic RALPH_TASK_STARTED
        // marker emitted at the very start of the task entry point.
        assert!(
            content_800.contains("RALPH_TASK_STARTED"),
            "log 800 should contain RALPH_TASK_STARTED marker; content:\n{content_800}"
        );
        assert!(
            content_801.contains("RALPH_TASK_STARTED"),
            "log 801 should contain RALPH_TASK_STARTED marker; content:\n{content_801}"
        );

        // Each log must contain its own project_id (issue-800 / issue-801).
        assert!(
            content_800.contains("issue-800"),
            "log 800 should contain its own project_id (issue-800); content:\n{content_800}"
        );
        assert!(
            content_801.contains("issue-801"),
            "log 801 should contain its own project_id (issue-801); content:\n{content_801}"
        );

        // Cross-contamination check: neither log should contain the
        // other task's project_id.
        assert!(
            !content_800.contains("issue-801"),
            "cross-contamination: issue-801 found in log 800.\nlog 800:\n{content_800}\nlog 801:\n{content_801}"
        );
        assert!(
            !content_801.contains("issue-800"),
            "cross-contamination: issue-800 found in log 801.\nlog 800:\n{content_800}\nlog 801:\n{content_801}"
        );
    })
}
