use super::*;

use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::validate::assertions::assert_exit_code;
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    active_streaming_planner_mock_script, hanging_after_partial_planner_mock_script,
    idle_timeout_reset_planner_mock_script, planner_parse_fail_then_pass_mock_script,
    slow_streaming_planner_mock_script, standard_mock_script, timeout_hanging_planner_mock_script,
};

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "streaming::retry_append_behavior",
            func: retry_append_behavior,
        },
        ConformanceTest {
            name: "streaming::prompt_reviewer_path",
            func: prompt_reviewer_path,
        },
        ConformanceTest {
            name: "streaming::mid_execution_visibility",
            func: mid_execution_visibility,
        },
        ConformanceTest {
            name: "streaming::timeout_cleanup",
            func: timeout_cleanup,
        },
        ConformanceTest {
            name: "streaming::active_stream_no_timeout",
            func: active_stream_no_timeout,
        },
        ConformanceTest {
            name: "streaming::hanging_stall_timeout",
            func: hanging_stall_timeout,
        },
        ConformanceTest {
            name: "streaming::idle_timeout_reset",
            func: idle_timeout_reset,
        },
    ]
}

/// Force a parse-retry/reformatter path for the planner role and verify
/// that the single log file contains multiple attempt separators (attempt=1,
/// attempt=2, etc.) with appended content across attempts.
fn retry_append_behavior(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "streaming-retry";
        let counter_file = h.temp_dir.path().join("planner-counter.txt");

        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "parse-fail-mock.sh",
                &planner_parse_fail_then_pass_mock_script(&counter_file),
            )
            .expect("failed to write parse-fail mock script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");
        h.create_project(
            project_id,
            "Streaming Retry Project",
            "Streaming retry test prompt",
        )
        .expect("create_project failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        // Planner log path: .ralph/projects/<id>/loops/001/agent-output-planner.log
        let planner_log = h
            .project_dir(project_id)
            .join("loops")
            .join("001")
            .join("agent-output-planner.log");

        assert!(
            planner_log.exists(),
            "planner log file should exist at {}",
            planner_log.display()
        );

        let content = fs::read_to_string(&planner_log).expect("read planner log");

        // The first attempt should have attempt=1 with fallback=false
        assert!(
            content.contains("attempt=1"),
            "planner log should contain attempt=1, got:\n{content}"
        );
        assert!(
            content.contains("fallback=false"),
            "first attempt should have fallback=false, got:\n{content}"
        );

        // The parse-retry path should produce at least one more attempt
        // (reformatter or format-reminder retry), so we expect attempt=2
        assert!(
            content.contains("attempt=2"),
            "planner log should contain attempt=2 from parse-retry/reformatter path, got:\n{content}"
        );

        // The second attempt should have fallback=true (since attempt > 0)
        // Count occurrences of fallback=true to verify retry attribution
        let fallback_true_count = content.matches("fallback=true").count();
        assert!(
            fallback_true_count >= 1,
            "expected at least one fallback=true entry from retry path, found {fallback_true_count}, got:\n{content}"
        );

        // Verify all attempts are in the same file with separator markers
        let separator_count = content.matches("--- attempt=").count();
        assert!(
            separator_count >= 2,
            "expected at least 2 separators (initial + retry), found {separator_count}, got:\n{content}"
        );

        // Should also contain actual output content from the successful attempt
        assert!(
            content.contains("Feature")
                || content.contains("feature")
                || content.contains("not a valid"),
            "planner log should contain backend output, got:\n{content}"
        );

        // Implementer log should also exist (after planner succeeded on retry)
        // The implementer log is in the slug-prefixed loop directory (e.g., 001-feature/)
        let loop_dir = h
            .loop_dir(project_id, 1)
            .expect("loop_dir should succeed")
            .expect("loop directory should exist");
        let impl_log = loop_dir.join("agent-output-implementer.log");
        assert!(
            impl_log.exists(),
            "implementer log file should exist at {}",
            impl_log.display()
        );

        let impl_content = fs::read_to_string(&impl_log).expect("read implementer log");
        assert!(
            impl_content.contains("attempt=1"),
            "implementer log should contain attempt separator, got:\n{impl_content}"
        );
    })
}

/// The prompt-reviewer log should be written at the project root level
/// (no loop subdirectory) with loop_number=None.
fn prompt_reviewer_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "streaming-prompt-review";
        setup_with_standard_mock(h, project_id);

        // Enable prompt review and run
        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "true"])
            .expect("enable prompt review");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        // Prompt reviewer log should be at project root, not in a loop dir
        let pr_log = h
            .project_dir(project_id)
            .join("agent-output-prompt-reviewer.log");

        assert!(
            pr_log.exists(),
            "prompt-reviewer log should exist at project root: {}",
            pr_log.display()
        );

        let content = fs::read_to_string(&pr_log).expect("read prompt-reviewer log");
        assert!(
            content.contains("attempt=1"),
            "prompt-reviewer log should contain attempt separator, got:\n{content}"
        );

        // It should NOT be inside any loop directory
        let loops_dir = h.project_dir(project_id).join("loops");
        if loops_dir.exists() {
            for entry in fs::read_dir(&loops_dir).expect("read loops dir") {
                let entry = entry.expect("dir entry");
                if entry.file_type().expect("file type").is_dir() {
                    let bad_path = entry.path().join("agent-output-prompt-reviewer.log");
                    assert!(
                        !bad_path.exists(),
                        "prompt-reviewer log should NOT exist inside loop dir: {}",
                        bad_path.display()
                    );
                }
            }
        }
    })
}

/// Verify planner output is visible in the log while backend execution is
/// still in progress (chunked streaming, not post-hoc write).
fn mid_execution_visibility(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "streaming-mid-visibility";

        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("slow-streaming.sh", &slow_streaming_planner_mock_script())
            .expect("failed to write slow streaming mock script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");
        h.create_project(
            project_id,
            "Streaming Mid-Execution Visibility",
            "Streaming visibility test prompt",
        )
        .expect("create_project failed");

        let planner_log = h
            .project_dir(project_id)
            .join("loops")
            .join("001")
            .join("agent-output-planner.log");

        let mut child = Command::new(&h.ralph_bin)
            .args(["run", "--loops", "1"])
            .current_dir(&h.repo_root)
            .spawn()
            .expect("spawn ralph run");

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut observed_size = 0_u64;
        let mut observed_while_running = false;

        while Instant::now() < deadline {
            if let Ok(meta) = fs::metadata(&planner_log) {
                observed_size = meta.len();
                if observed_size > 0 {
                    let status = child.try_wait().expect("try_wait should succeed");
                    if status.is_none() {
                        observed_while_running = true;
                        break;
                    }
                }
            }

            if child.try_wait().expect("try_wait should succeed").is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        assert!(
            observed_while_running,
            "expected planner log bytes while run process was still active; observed_size={observed_size}"
        );

        let output = child.wait_with_output().expect("wait_with_output");
        assert_exit_code(&output, 0);

        let final_size = fs::metadata(&planner_log)
            .expect("planner log metadata after completion")
            .len();
        assert!(
            final_size > observed_size,
            "planner log should grow after initial streamed bytes; initial={observed_size} final={final_size}"
        );
    })
}

/// Verify timeout behavior: partial output is preserved, timeout footer exists,
/// and the hanging planner child process is dead after retries.
fn timeout_cleanup(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "streaming-timeout-cleanup";
        let pid_file = h.temp_dir.path().join("streaming-timeout.pid");

        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "timeout-hanging.sh",
                &timeout_hanging_planner_mock_script(&pid_file),
            )
            .expect("failed to write timeout mock script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");
        h.ralph_ok(["config", "set", "backends.claude.timeout_seconds", "1"])
            .expect("set claude timeout");
        h.ralph_ok(["config", "set", "backends.codex.timeout_seconds", "1"])
            .expect("set codex timeout");
        h.create_project(
            project_id,
            "Streaming Timeout Cleanup",
            "Streaming timeout cleanup prompt",
        )
        .expect("create_project failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert_exit_code(&output, 1);

        let planner_log = h
            .project_dir(project_id)
            .join("loops")
            .join("001")
            .join("agent-output-planner.log");
        assert!(
            planner_log.exists(),
            "planner log should exist at {}",
            planner_log.display()
        );
        let content = fs::read_to_string(&planner_log).expect("read planner log");
        assert!(
            content.contains("planner-partial-before-timeout"),
            "planner log should contain partial output before timeout, got:\n{content}"
        );
        assert!(
            content.contains("--- timeout ts="),
            "planner log should contain timeout footer, got:\n{content}"
        );

        let pid_raw = fs::read_to_string(&pid_file).expect("read pid file");
        let pid: i32 = pid_raw.trim().parse().expect("pid should be numeric");
        let kill_rc = unsafe { libc::kill(pid, 0) };
        assert_eq!(kill_rc, -1, "timed-out planner process should be dead");
        let os_err = std::io::Error::last_os_error()
            .raw_os_error()
            .expect("raw os error should be present");
        assert_eq!(
            os_err,
            libc::ESRCH,
            "timed-out planner process should be fully reaped"
        );
    })
}

/// Active streaming beyond timeout_seconds without timeout: the planner mock
/// emits output at intervals shorter than timeout_seconds (0.3s < 1s), with total
/// runtime exceeding timeout_seconds (~2.4s > 1s). Must succeed (no timeout).
fn active_stream_no_timeout(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "streaming-active-no-timeout";

        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "active-streaming.sh",
                &active_streaming_planner_mock_script(),
            )
            .expect("failed to write active streaming mock script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");
        // Disable prompt review so mock scripts don't need to handle the prompt-reviewer prompt.
        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("config set workflow.prompt_review_enabled failed");
        // Set timeout to 1s -- total planner runtime ~2.4s > 1s, but each
        // chunk arrives every 0.3s < 1s, so inactivity timeout must NOT fire.
        h.ralph_ok(["config", "set", "backends.claude.timeout_seconds", "1"])
            .expect("set claude timeout");
        h.ralph_ok(["config", "set", "backends.codex.timeout_seconds", "1"])
            .expect("set codex timeout");
        h.create_project(
            project_id,
            "Active Stream No Timeout",
            "Active stream inactivity test",
        )
        .expect("create_project failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let planner_log = h
            .project_dir(project_id)
            .join("loops")
            .join("001")
            .join("agent-output-planner.log");
        assert!(
            planner_log.exists(),
            "planner log should exist at {}",
            planner_log.display()
        );
        let content = fs::read_to_string(&planner_log).expect("read planner log");
        // All 8 chunks should be present
        assert!(
            content.contains("chunk-8"),
            "planner log should contain all chunks: {content}"
        );
        // No timeout footer should appear
        assert!(
            !content.contains("--- timeout ts="),
            "planner log should NOT contain timeout footer: {content}"
        );
    })
}

/// Hanging-after-partial-output timeout with cleanup: the planner emits partial
/// output then stalls beyond timeout_seconds. Must timeout with cleanup and
/// retain partial output in the log.
fn hanging_stall_timeout(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "streaming-hanging-stall-timeout";
        let pid_file = h.temp_dir.path().join("hanging-stall.pid");

        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "hanging-stall.sh",
                &hanging_after_partial_planner_mock_script(&pid_file),
            )
            .expect("failed to write hanging stall mock script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");
        // Disable prompt review so mock scripts don't need to handle the prompt-reviewer prompt.
        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("config set workflow.prompt_review_enabled failed");
        h.ralph_ok(["config", "set", "backends.claude.timeout_seconds", "1"])
            .expect("set claude timeout");
        h.ralph_ok(["config", "set", "backends.codex.timeout_seconds", "1"])
            .expect("set codex timeout");
        h.create_project(
            project_id,
            "Hanging Stall Timeout",
            "Hanging stall inactivity timeout test",
        )
        .expect("create_project failed");

        let start = Instant::now();
        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        let elapsed = start.elapsed();
        assert_exit_code(&output, 1);

        // The timeout is 1s and the mock sleeps for 30s. With retries (up to 3
        // attempts), the run should finish well under 30s if the idle timeout
        // actually kills the process promptly.
        assert!(
            elapsed < Duration::from_secs(20),
            "hanging stall should be killed by idle timeout, not run for full 30s; elapsed={elapsed:?}"
        );

        let planner_log = h
            .project_dir(project_id)
            .join("loops")
            .join("001")
            .join("agent-output-planner.log");
        assert!(
            planner_log.exists(),
            "planner log should exist at {}",
            planner_log.display()
        );
        let content = fs::read_to_string(&planner_log).expect("read planner log");
        assert!(
            content.contains("partial-output-before-stall"),
            "planner log should contain partial output: {content}"
        );
        assert!(
            content.contains("--- timeout ts="),
            "planner log should contain timeout footer: {content}"
        );

        // Verify the hanging process was killed
        let pid_raw = fs::read_to_string(&pid_file).expect("read pid file");
        let pid: i32 = pid_raw.trim().parse().expect("pid should be numeric");
        let kill_rc = unsafe { libc::kill(pid, 0) };
        assert_eq!(kill_rc, -1, "stalled planner process should be dead");
        let os_err = std::io::Error::last_os_error()
            .raw_os_error()
            .expect("raw os error should be present");
        assert_eq!(
            os_err,
            libc::ESRCH,
            "stalled planner process should be fully reaped"
        );
    })
}

/// Verify idle timeout resets with periodic planner output chunks:
/// planner runtime exceeds nominal timeout, run succeeds, and no timeout footer.
fn idle_timeout_reset(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "streaming-idle-timeout-reset";

        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "idle-timeout-reset.sh",
                &idle_timeout_reset_planner_mock_script(),
            )
            .expect("failed to write idle-timeout-reset mock script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");
        h.ralph_ok(["config", "set", "backends.claude.timeout_seconds", "1"])
            .expect("set claude timeout");
        h.ralph_ok(["config", "set", "backends.codex.timeout_seconds", "1"])
            .expect("set codex timeout");
        h.create_project(
            project_id,
            "Streaming Idle Timeout Reset",
            "Streaming idle-timeout reset prompt",
        )
        .expect("create_project failed");

        let start = Instant::now();
        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        let elapsed = start.elapsed();
        assert_exit_code(&output, 0);
        assert!(
            elapsed >= Duration::from_millis(1200),
            "run should exceed nominal 1s timeout while still active; elapsed={elapsed:?}"
        );

        let planner_log = h
            .project_dir(project_id)
            .join("loops")
            .join("001")
            .join("agent-output-planner.log");
        assert!(
            planner_log.exists(),
            "planner log should exist at {}",
            planner_log.display()
        );
        let content = fs::read_to_string(&planner_log).expect("read planner log");
        assert!(
            content.contains("Idle Timeout Reset Feature"),
            "planner log should contain streamed planner content, got:\n{content}"
        );
        assert!(
            content.contains("periodic output chunks"),
            "planner log should contain delayed chunk content, got:\n{content}"
        );
        assert!(
            !content.contains("--- timeout ts="),
            "planner log should not contain timeout footer, got:\n{content}"
        );
    })
}


fn setup_with_standard_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    h.setup_mock_backends(&script)
        .expect("setup_mock_backends failed");
    h.create_project(
        project_id,
        "Streaming Conformance Project",
        "Streaming suite test prompt",
    )
    .expect("create_project failed");
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
