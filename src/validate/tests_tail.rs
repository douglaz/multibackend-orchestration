use super::*;

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
    ]
}

fn one_shot_shows_artifacts(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "tail-one-shot";
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
        let project_id = "tail-json";
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
        let project_id = "tail-last";
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

fn setup_with_standard_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    h.setup_mock_backends(&script)
        .expect("setup_mock_backends failed");
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
