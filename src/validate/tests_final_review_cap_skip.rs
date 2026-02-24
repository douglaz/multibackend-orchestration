use super::*;

use crate::validate::assertions::{assert_exit_code, assert_file_exists, assert_json_field};
use crate::validate::harness::RalphHarness;
use serde_json::json;
use std::fs;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "final_review_cap_skip::cap_reached_skips_deliberation_and_force_completes",
            func: cap_reached_skips_deliberation_and_force_completes,
        },
        ConformanceTest {
            name: "final_review_cap_skip::cap_boundary_force_completes_even_if_no_amendments_would_be_found",
            func: cap_boundary_force_completes_even_if_no_amendments_would_be_found,
        },
    ]
}

fn cap_reached_skips_deliberation_and_force_completes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "final-review-cap-skip";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "final-review-cap-skip.sh",
                &deliberation_must_be_skipped_script(),
            )
            .expect("failed to write cap-skip script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Final Review Cap Skip",
            "Final review cap skip prompt",
        )
        .expect("create_project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review");
        h.ralph_ok(["config", "set", "workflow.max_final_review_restarts", "1"])
            .expect("set max final review restarts");

        seed_restart_artifact(h, project_id);

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let combined_output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined_output.contains("skipping deliberation"),
            "expected output to mention deliberation skip, got:\n{combined_output}"
        );

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));
        assert_json_field(&state, "current_phase", &json!("completing"));
        assert_json_field(&state, "phase_iteration", &json!(1));

        let completion_attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert_eq!(completion_attempts.len(), 1);
        let loop_number = completion_attempts[0]["loop_number"]
            .as_u64()
            .expect("loop_number should be u64") as u32;
        let artifacts = h
            .list_artifacts(project_id, loop_number)
            .expect("list_artifacts should succeed");
        let artifact_names = artifacts
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>();

        assert!(
            !artifact_names
                .iter()
                .any(|name| name.ends_with("-final-review-exit-approved.md")),
            "did not expect approved final-review exit artifact when cap is reached; artifacts:\n{}",
            artifact_names.join("\n")
        );

        for forbidden in [
            "final-review-proposals",
            "final-review-planner-positions",
            "final-review-votes",
            "final-review-arbiter-ruling",
        ] {
            assert!(
                !artifact_names.iter().any(|name| name.contains(forbidden)),
                "did not expect deliberation artifact containing '{forbidden}' when cap is reached; artifacts:\n{}",
                artifact_names.join("\n")
            );
        }

        let force_complete_path = h
            .project_dir(project_id)
            .join("final-review-force-complete.md");
        assert_file_exists(&force_complete_path);
    })
}

fn cap_boundary_force_completes_even_if_no_amendments_would_be_found(
    h: &RalphHarness,
) -> TestResult {
    run_case(|| {
        let project_id = "final-review-cap-boundary";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "final-review-cap-boundary.sh",
                &no_amendments_if_called_script(),
            )
            .expect("failed to write cap-boundary script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Final Review Cap Boundary",
            "Final review cap boundary prompt",
        )
        .expect("create_project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review");
        h.ralph_ok(["config", "set", "workflow.max_final_review_restarts", "1"])
            .expect("set max final review restarts");

        seed_restart_artifact(h, project_id);

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));
        assert_json_field(&state, "current_phase", &json!("completing"));

        let completion_attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert_eq!(completion_attempts.len(), 1);
        let loop_number = completion_attempts[0]["loop_number"]
            .as_u64()
            .expect("loop_number should be u64") as u32;
        let artifacts = h
            .list_artifacts(project_id, loop_number)
            .expect("list_artifacts should succeed");
        let artifact_names = artifacts
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>();

        assert!(
            !artifact_names
                .iter()
                .any(|name| name.ends_with("-final-review-exit-approved.md")),
            "did not expect approved final-review exit artifact at restart-cap boundary; artifacts:\n{}",
            artifact_names.join("\n")
        );

        let force_complete_path = h
            .project_dir(project_id)
            .join("final-review-force-complete.md");
        assert_file_exists(&force_complete_path);
    })
}

fn seed_restart_artifact(h: &RalphHarness, project_id: &str) {
    let seeded_loop_dir = h.project_dir(project_id).join("loops").join("000-seeded");
    fs::create_dir_all(&seeded_loop_dir).expect("create seeded loop dir");
    fs::write(
        seeded_loop_dir.join("20260224000000-final-review-exit-restart.md"),
        "# seeded final review restart artifact\n",
    )
    .expect("write seeded restart artifact");
}

fn deliberation_must_be_skipped_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  cat <<'EOF'
# Project Completion Request

## Rationale
Ready for completion.

## Summary of Work
- Completed implementation.

## Remaining Items
- None
EOF
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Requirement: satisfied
EOF
elif [[ "$prompt" == *"You are a QA engineer"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance passed

## Automated Tests
- acceptance passed

## Acceptance Criteria Verification
All good.
EOF
elif [[ "$prompt" == *"You are a final reviewer evaluating a completed project for quality and correctness."* ]] \
  || [[ "$prompt" == *"You are the project planner evaluating proposed amendments from final reviewers."* ]] \
  || [[ "$prompt" == *"You are a reviewer voting on proposed amendments after considering the planner's positions."* ]] \
  || [[ "$prompt" == *"You are the arbiter resolving disputed amendments where reviewers and planner disagree."* ]]; then
  echo "final review deliberation should be skipped when restart cap is reached" >&2
  exit 91
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

fn no_amendments_if_called_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  cat <<'EOF'
# Project Completion Request

## Rationale
Ready for completion.

## Summary of Work
- Completed implementation.

## Remaining Items
- None
EOF
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Requirement: satisfied
EOF
elif [[ "$prompt" == *"You are a QA engineer"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance passed

## Automated Tests
- acceptance passed

## Acceptance Criteria Verification
All good.
EOF
elif [[ "$prompt" == *"You are a final reviewer evaluating a completed project for quality and correctness."* ]]; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No additional amendments needed.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
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
