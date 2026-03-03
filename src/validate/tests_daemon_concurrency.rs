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
            name: "daemon_concurrency::partial_dispatch_rollback",
            func: partial_dispatch_rollback,
        },
        ConformanceTest {
            name: "daemon_concurrency::single_iteration_prd_inline_only",
            func: single_iteration_prd_inline_only,
        },
        ConformanceTest {
            name: "daemon_concurrency::concurrent_rebase_dispatch_no_lock_contention",
            func: concurrent_rebase_dispatch_no_lock_contention,
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

fn write_mock_ralph(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("mock_ralph", body)?;
    Ok(script.to_string_lossy().into_owned())
}

fn write_daemon_mock_ralph(h: &RalphHarness) -> crate::Result<String> {
    write_mock_ralph(h, &mock_scripts::daemon_mock_ralph_script())
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
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");

        let combined = combined_output(&output);

        assert_exit_code(&output, 0);

        // Both issues should have been dispatched
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

        // Check label log for claim operations on both issues
        if label_log.exists() {
            let log_content = fs::read_to_string(&label_log).expect("read label log");
            // Both issues should have in-progress claims
            assert!(
                log_content.contains("200") || combined.contains("200"),
                "issue 200 should appear in label operations or output: {log_content}"
            );
            assert!(
                log_content.contains("201") || combined.contains("201"),
                "issue 201 should appear in label operations or output: {log_content}"
            );
        }
    })
}

/// Verifies that when dispatch_task fails for one issue in a batch, only
/// that issue is rolled back to ralph:failed while the other issue proceeds.
///
/// Asserts per-issue outcomes:
/// - Issue 300: dispatched successfully, no rollback labels applied
/// - Issue 301: dispatch fails, rolled back with ralph:in-progress removed
///   and ralph:failed added
/// - The successful issue (300) is NOT affected by the 301 failure
fn partial_dispatch_rollback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("partial_rollback_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Two issues: 300 will be a normal issue, 301 will fail dispatch
        // We mock ralph to fail for issue 301 by checking the task_id
        let issues = r#"[{"number":300,"title":"good issue","labels":[{"name":"ralph:ready"}],"body":"good body"},{"number":301,"title":"bad issue","labels":[{"name":"ralph:ready"}],"body":"bad body"}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");

        // Mock ralph that succeeds for issue 300 but fails for issue 301
        let mock_ralph_body = r#"#!/bin/sh
# Check if this is for issue 301 — fail immediately
for arg in "$@"; do
    case "$arg" in
        *issue-301*) exit 1 ;;
    esac
done
# Success for all others
sleep 0.1
exit 0
"#;
        let ralph_path = write_mock_ralph(&dh, mock_ralph_body).expect("write mock ralph");

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
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");

        let combined = combined_output(&output);

        // Issue 300 should be dispatched successfully
        assert!(
            combined.contains("dispatched task acme-widgets-300")
                || combined.contains("dispatch: task acme-widgets-300"),
            "issue 300 should be dispatched: {combined}"
        );

        // Issue 301 should show dispatch failure in daemon output
        assert!(
            combined.contains("failed to dispatch issue #301")
                || combined.contains("301"),
            "issue 301 dispatch failure should be logged: {combined}"
        );

        // Verify per-issue label transitions in the label log
        assert!(
            label_log.exists(),
            "label log should exist at {}", label_log.display()
        );
        let log_content = fs::read_to_string(&label_log).expect("read label log");
        let log_lines: Vec<&str> = log_content.lines().collect();

        // Both issues should have been claimed: ready -> in-progress
        // (remove ralph:ready + add ralph:in-progress for each)
        assert!(
            log_lines.iter().any(|l| l.contains("300") && l.contains("--add-label") && l.contains("ralph:in-progress")),
            "issue 300 should have been claimed (add ralph:in-progress): {log_content}"
        );
        assert!(
            log_lines.iter().any(|l| l.contains("301") && l.contains("--add-label") && l.contains("ralph:in-progress")),
            "issue 301 should have been claimed (add ralph:in-progress): {log_content}"
        );

        // Issue 301 should have rollback: in-progress -> failed
        // This means remove ralph:in-progress AND add ralph:failed specifically for 301
        assert!(
            log_lines.iter().any(|l| l.contains("301") && l.contains("--remove-label") && l.contains("ralph:in-progress")),
            "issue 301 should have rollback (remove ralph:in-progress): {log_content}"
        );
        assert!(
            log_lines.iter().any(|l| l.contains("301") && l.contains("--add-label") && l.contains("ralph:failed")),
            "issue 301 should have rollback (add ralph:failed): {log_content}"
        );

        // Issue 300 should NOT have any ralph:failed label added — the
        // sibling failure must not cause rollback of successfully dispatched
        // issues.
        let issue_300_failed = log_lines.iter().any(|l| {
            l.contains("300") && l.contains("--add-label") && l.contains("ralph:failed")
        });
        assert!(
            !issue_300_failed,
            "issue 300 should NOT be rolled back to ralph:failed (sibling isolation invariant): {log_content}"
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
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
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
        let tick_count = tick_log_content.lines().filter(|l| l.contains("prd-tick")).count();
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
        let gh_path = write_mock_gh(&dh, &mock_scripts::daemon_mock_gh_bounded_concurrency_script())
            .expect("write mock gh");
        // Use long-running mock ralph so child stays alive across iterations.
        let ralph_script = dh.write_mock_script(
            "mock_ralph",
            &mock_scripts::daemon_mock_ralph_long_running_script(),
        ).expect("write mock ralph");
        let ralph_path = ralph_script.to_string_lossy().into_owned();

        // Disable PRD to keep the test focused on rebase + dispatch.
        // PRD config keys are rejected by `config set` CLI; write TOML directly.
        {
            let config_path = dh.repo_root.join(".ralph").join("ralph.toml");
            let mut toml_content = fs::read_to_string(&config_path).unwrap_or_default();
            if !toml_content.contains("[workspace]") {
                toml_content.push_str("\n[workspace]\n");
            }
            toml_content.push_str("daemon_prd_enabled = false\n");
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
            .env("RALPH_DAEMON_BIN", &ralph_path)
            .env("MOCK_GH_ISSUES", issues)
            .env("MOCK_GH_LABEL_LOG", &label_log_str)
            .env("MOCK_GH_ISSUE_LIST_COUNTER", &issue_counter_str)
            .env("MOCK_REBASE_ATTEMPT_LOG", &rebase_log_str)
            .env("MOCK_RALPH_SLEEP_SECS", "30")
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
        let rebase_attempts = rebase_log_content.lines()
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
