use super::*;

use crate::validate::assertions::{assert_exit_code, assert_json_field};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::standard_mock_script;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "gemini_backend::optional_backend_skips_when_unavailable",
            func: optional_backend_skips_when_unavailable,
        },
        ConformanceTest {
            name: "gemini_backend::required_backend_fails_when_unavailable",
            func: required_backend_fails_when_unavailable,
        },
        ConformanceTest {
            name: "gemini_backend::guardrails_reject_disallowed_surfaces",
            func: guardrails_reject_disallowed_surfaces,
        },
    ]
}

fn optional_backend_skips_when_unavailable(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "gemini-optional-skip";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("gemini-optional-standard.sh", &standard_mock_script())
            .expect("failed to write standard mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup mock backends failed");
        h.create_project(
            project_id,
            "Gemini Optional Skip",
            "Validate optional gemini backend skip",
        )
        .expect("create project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review failed");
        h.ralph_ok(["config", "set", "workflow.final_review_min_reviewers", "2"])
            .expect("set final_review_min_reviewers failed");
        h.ralph_ok([
            "config",
            "set",
            "workflow.final_review_backends",
            "[\"claude\",\"codex\",\"?gemini\"]",
        ])
        .expect("set final_review_backends failed");
        h.ralph_ok([
            "config",
            "set",
            "backends.gemini.enabled",
            "false",
            "--global",
        ])
        .expect("disable gemini backend failed");

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
            .expect("run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load state failed");
        assert_json_field(&state, "status", &json!("completed"));
    })
}

fn required_backend_fails_when_unavailable(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "gemini-required-fails";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("gemini-required-standard.sh", &standard_mock_script())
            .expect("failed to write standard mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup mock backends failed");
        h.create_project(
            project_id,
            "Gemini Required Failure",
            "Validate required gemini backend failure",
        )
        .expect("create project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review failed");
        h.ralph_ok(["config", "set", "workflow.final_review_min_reviewers", "1"])
            .expect("set final_review_min_reviewers failed");
        h.ralph_ok([
            "config",
            "set",
            "workflow.final_review_backends",
            "[\"gemini\"]",
        ])
        .expect("set final_review_backends failed");
        h.ralph_ok([
            "config",
            "set",
            "backends.gemini.enabled",
            "false",
            "--global",
        ])
        .expect("disable gemini backend failed");

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
            .expect("run should execute");
        assert!(
            output.status.code().unwrap_or(-1) != 0,
            "required unavailable backend should fail"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .to_lowercase();
        assert!(
            combined.contains("gemini"),
            "failure output should mention gemini, got:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    })
}

fn guardrails_reject_disallowed_surfaces(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "gemini-guardrails";
        h.init_workspace().expect("init failed");
        h.create_project(
            project_id,
            "Gemini Guardrails",
            "Validate gemini guardrail validation",
        )
        .expect("create project failed");

        h.ralph_ok(["config", "set", "workflow.starting_backend", "gemini"])
            .expect("set starting backend failed");
        let output = h
            .ralph(["run", "--dry-run"])
            .expect("dry-run should execute");
        assert!(
            output.status.code().unwrap_or(-1) != 0,
            "gemini starting backend should be rejected"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("gemini backend is not supported for starting backend"),
            "expected starting-backend guardrail message, got:\n{combined}"
        );

        h.ralph_ok(["config", "set", "workflow.starting_backend", "claude"])
            .expect("reset starting backend failed");
        h.ralph_ok(["config", "set", "workflow.planner_backend", "?gemini"])
            .expect("set planner backend optional syntax failed");
        let output = h
            .ralph(["run", "--dry-run"])
            .expect("dry-run should execute");
        assert!(
            output.status.code().unwrap_or(-1) != 0,
            "optional syntax on required surface should be rejected"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains(
                "optional backend specs (?backend) are not supported for planner backend override"
            ),
            "expected optional syntax rejection message, got:\n{combined}"
        );
    })
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
