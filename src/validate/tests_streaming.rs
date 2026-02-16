use super::*;

use std::fs;

use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    planner_parse_fail_then_pass_mock_script, standard_mock_script,
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
            content.contains("Feature") || content.contains("feature") || content.contains("not a valid"),
            "planner log should contain backend output, got:\n{content}"
        );

        // Implementer log should also exist (after planner succeeded on retry)
        let impl_log = h
            .project_dir(project_id)
            .join("loops")
            .join("001")
            .join("agent-output-implementer.log");
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
