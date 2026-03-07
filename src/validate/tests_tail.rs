use super::*;

use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::Duration;

use crate::validate::assertions::{assert_exit_code, assert_stdout_contains};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::standard_mock_script;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "tail::one_shot_shows_artifacts",
            func: one_shot_shows_artifacts,
        },
        ConformanceTest {
            name: "tail::json_output_valid",
            func: json_output_valid,
        },
        ConformanceTest {
            name: "tail::last_flag_limits_output",
            func: last_flag_limits_output,
        },
        ConformanceTest {
            name: "tail::no_project_fails_gracefully",
            func: no_project_fails_gracefully,
        },
        ConformanceTest {
            name: "tail::follow_flag_accepted",
            func: follow_flag_accepted,
        },
    ]
}

fn one_shot_shows_artifacts(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-101";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let output = h.ralph(["tail"]).expect("ralph tail should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "--- [");
        assert_stdout_contains(&output, "artifact=");
        assert_stdout_contains(&output, "started");
    })
}

fn json_output_valid(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-102";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let output = h
            .ralph(["tail", "--json"])
            .expect("ralph tail --json should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut saw_artifact = false;
        let mut saw_state = false;
        let mut parsed_count = 0usize;

        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let event: serde_json::Value =
                serde_json::from_str(line).expect("tail --json line should be valid JSON");
            assert!(
                event.get("project_id").is_some(),
                "expected event to include project_id: {event}"
            );
            assert!(
                event.get("event_type").is_some(),
                "expected event to include event_type: {event}"
            );
            assert!(
                event.get("timestamp").is_some(),
                "expected event to include timestamp: {event}"
            );

            match event.get("event_type").and_then(|v| v.as_str()) {
                Some("artifact") => saw_artifact = true,
                Some("state") => saw_state = true,
                _ => {}
            }
            parsed_count += 1;
        }

        assert!(parsed_count > 0, "expected at least one tail event");
        assert!(saw_artifact, "expected at least one artifact event");
        assert!(saw_state, "expected at least one state event");
    })
}

fn last_flag_limits_output(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-103";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let full_output = h.ralph_ok(["tail"]).expect("ralph tail should succeed");
        let limited_output = h
            .ralph_ok(["tail", "--last", "1"])
            .expect("ralph tail --last 1 should succeed");

        assert!(
            limited_output.len() < full_output.len(),
            "expected --last 1 output to be shorter than full output.\nfull={} limited={}",
            full_output.len(),
            limited_output.len()
        );
    })
}

fn no_project_fails_gracefully(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let output = h.ralph(["tail"]).expect("ralph tail should execute");
        assert_exit_code(&output, 2);

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.to_lowercase().contains("active project"),
            "expected no-project error to mention active project, got:\n{combined}"
        );
    })
}

fn follow_flag_accepted(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-104";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let child = Command::new(&h.ralph_bin)
            .args(["tail", "--follow"])
            .current_dir(&h.repo_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn ralph tail --follow");
        let mut child_guard = ChildGuard::new(child);

        thread::sleep(Duration::from_millis(500));

        let liveness = child_guard
            .child_mut()
            .try_wait()
            .expect("try_wait should succeed");
        assert!(
            liveness.is_none(),
            "expected `tail --follow` to still be running after 500ms, got: {liveness:?}"
        );

        let output = child_guard.kill_and_wait_with_output();
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        assert!(
            !stderr.contains("unrecognized")
                && !stderr.contains("unknown option")
                && !stderr.contains("unexpected argument '--follow'"),
            "expected no unknown/unrecognized flag error for --follow, got stderr:\n{stderr}"
        );
    })
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child process already consumed")
    }

    fn kill_and_wait_with_output(&mut self) -> Output {
        let mut child = self.child.take().expect("child process already consumed");
        let _ = child.kill();
        child
            .wait_with_output()
            .expect("wait_with_output should succeed")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn setup_with_standard_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends_stable failed");
    h.create_project(
        project_id,
        "Tail Conformance Project",
        "Tail suite test prompt",
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
