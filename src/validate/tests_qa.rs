use super::*;

use std::path::Path;

use crate::validate::assertions::{
    assert_exit_code, assert_file_contains, assert_file_exists, assert_git_tag_exists,
    assert_git_tag_not_exists, assert_json_array_len, assert_json_field,
    assert_no_loop_artifacts, assert_stderr_contains,
};
use crate::validate::harness::RalphHarness;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "qa::disabled_skips_phase",
            func: disabled_skips_phase,
        },
        ConformanceTest {
            name: "qa::enabled_pass_proceeds_to_review",
            func: enabled_pass_proceeds_to_review,
        },
        ConformanceTest {
            name: "qa::fail_retries_then_passes",
            func: fail_retries_then_passes,
        },
        ConformanceTest {
            name: "qa::iteration_limit_rolls_back",
            func: iteration_limit_rolls_back,
        },
        ConformanceTest {
            name: "qa::config_get_set",
            func: config_get_set,
        },
        ConformanceTest {
            name: "qa::acceptance_gate_pass",
            func: acceptance_gate_pass,
        },
        ConformanceTest {
            name: "qa::acceptance_gate_fail_forces_continue",
            func: acceptance_gate_fail_forces_continue,
        },
    ]
}

fn disabled_skips_phase(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qa-disabled";
        setup_with_mock_script(h, project_id, "qa-pass.sh", &qa_pass_mock_script());

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected exactly one completed loop");
        let loop_state = &loops[0];

        assert_json_array_len(loop_state, "artifacts.qa_results", 0);

        let artifact_names = loop_artifact_names(h, project_id, 1);
        let qa_artifacts = artifact_names
            .iter()
            .filter(|name| name.contains("qa"))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            qa_artifacts.is_empty(),
            "expected no QA artifacts when QA is disabled, got: {qa_artifacts:?}"
        );

        assert!(
            loop_state["commit"].as_str().is_some(),
            "expected loop to commit normally when QA is disabled"
        );
        assert_git_tag_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn enabled_pass_proceeds_to_review(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qa-pass";
        setup_with_mock_script(h, project_id, "qa-pass.sh", &qa_pass_mock_script());
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected exactly one completed loop");
        let loop_state = &loops[0];
        let project_dir = h.project_dir(project_id);

        assert_json_array_len(loop_state, "artifacts.qa_results", 1);
        let qa_results = loop_state["artifacts"]["qa_results"]
            .as_array()
            .expect("qa_results should be an array");
        assert_eq!(
            qa_results[0]["passed"],
            json!(true),
            "expected first QA result to be PASS"
        );

        let qa_report_rel = qa_results[0]["report"]
            .as_str()
            .expect("qa report path should exist");
        assert_file_exists(&project_dir.join(qa_report_rel));

        let qa_backend = loop_state["backends"]["qa"]
            .as_str()
            .expect("backends.qa should be a string");
        assert!(
            !qa_backend.trim().is_empty(),
            "expected backends.qa to be populated"
        );

        assert!(
            loop_state["commit"].as_str().is_some(),
            "expected loop to commit normally after QA pass"
        );
        assert_git_tag_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn fail_retries_then_passes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qa-retry";
        let counter_file = h.temp_dir.path().join("qa-counter.txt");
        let script = qa_fail_then_pass_mock_script(&counter_file);
        setup_with_mock_script(h, project_id, "qa-fail-then-pass.sh", &script);
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected exactly one completed loop");
        let loop_state = &loops[0];
        let project_dir = h.project_dir(project_id);

        assert_json_array_len(loop_state, "artifacts.qa_results", 2);
        let qa_results = loop_state["artifacts"]["qa_results"]
            .as_array()
            .expect("qa_results should be an array");
        assert_eq!(
            qa_results[0]["passed"],
            json!(false),
            "expected first QA result to be FAIL"
        );
        assert_eq!(
            qa_results[1]["passed"],
            json!(true),
            "expected second QA result to be PASS"
        );

        let first_report = qa_results[0]["report"]
            .as_str()
            .expect("first qa report path should exist");
        let second_report = qa_results[1]["report"]
            .as_str()
            .expect("second qa report path should exist");
        assert_file_exists(&project_dir.join(first_report));
        assert_file_exists(&project_dir.join(second_report));

        let impl_qa_response_rel = qa_results[0]["implementer_response"]
            .as_str()
            .expect("first QA result should include implementer response artifact");
        assert_file_exists(&project_dir.join(impl_qa_response_rel));

        assert!(
            qa_results[1]["implementer_response"].is_null(),
            "second QA result should not include implementer response"
        );

        assert!(
            loop_state["commit"].as_str().is_some(),
            "expected loop to commit after QA retry then pass"
        );
        assert_git_tag_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn iteration_limit_rolls_back(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qa-limit";
        setup_with_mock_script(
            h,
            project_id,
            "qa-always-fail.sh",
            &qa_always_fail_mock_script(),
        );
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");
        h.ralph_ok(["config", "set", "workflow.max_qa_iterations", "1"])
            .expect("config set workflow.max_qa_iterations 1 failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert!(
            !output.status.success(),
            "expected run to fail after QA iteration limit is exceeded"
        );
        assert_stderr_contains(&output, "QA iteration limit exceeded");

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_array_len(&state, "loops", 0);
        assert_no_loop_artifacts(&h.project_dir(project_id));
        assert_git_tag_not_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn config_get_set(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let qa_enabled_default = h
            .ralph_ok(["config", "get", "workflow.qa_enabled"])
            .expect("config get workflow.qa_enabled failed");
        assert_eq!(
            qa_enabled_default.trim(),
            "false",
            "expected workflow.qa_enabled default to be false"
        );

        let max_qa_iterations_default = h
            .ralph_ok(["config", "get", "workflow.max_qa_iterations"])
            .expect("config get workflow.max_qa_iterations failed");
        assert_eq!(
            max_qa_iterations_default.trim(),
            "3",
            "expected workflow.max_qa_iterations default to be 3"
        );

        let qa_backend_default = h
            .ralph_ok(["config", "get", "workflow.qa_backend"])
            .expect("config get workflow.qa_backend failed");
        assert_eq!(
            qa_backend_default.trim(),
            "null",
            "expected workflow.qa_backend default to be null"
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");
        let qa_enabled = h
            .ralph_ok(["config", "get", "workflow.qa_enabled"])
            .expect("config get workflow.qa_enabled failed after set");
        assert_eq!(qa_enabled.trim(), "true");

        h.ralph_ok(["config", "set", "workflow.max_qa_iterations", "5"])
            .expect("config set workflow.max_qa_iterations 5 failed");
        let max_qa_iterations = h
            .ralph_ok(["config", "get", "workflow.max_qa_iterations"])
            .expect("config get workflow.max_qa_iterations failed after set");
        assert_eq!(max_qa_iterations.trim(), "5");

        h.ralph_ok(["config", "set", "workflow.qa_backend", "claude(opus)"])
            .expect("config set workflow.qa_backend failed");
        let qa_backend = h
            .ralph_ok(["config", "get", "workflow.qa_backend"])
            .expect("config get workflow.qa_backend failed after set");
        assert_eq!(qa_backend.trim(), "claude(opus)");

        h.ralph_ok(["config", "set", "qa_backend", "codex"])
            .expect("config set qa_backend alias failed");
        let qa_backend_alias = h
            .ralph_ok(["config", "get", "workflow.qa_backend"])
            .expect("config get workflow.qa_backend failed after alias set");
        assert_eq!(qa_backend_alias.trim(), "codex");
    })
}

fn setup_with_mock_script(h: &RalphHarness, project_id: &str, script_name: &str, script: &str) {
    h.init_workspace().expect("init failed");
    let script_path = h
        .write_mock_script(script_name, script)
        .expect("failed to write mock script");
    h.setup_mock_backends(&script_path)
        .expect("setup_mock_backends failed");
    h.create_project(project_id, "QA Conformance Project", "QA suite test prompt")
        .expect("create_project failed");
}

fn loop_artifact_names(h: &RalphHarness, project_id: &str, loop_number: u32) -> Vec<String> {
    h.list_artifacts(project_id, loop_number)
        .expect("list_artifacts should succeed")
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("artifact filename should be valid UTF-8")
                .to_owned()
        })
        .collect()
}

fn qa_pass_mock_script() -> String {
    mock_script_with_qa_branch(
        r#"if echo "$INPUT" | grep -q "overall project acceptance"; then
  cat <<'EOF'
# QA: PASS

## Tests Run
- acceptance check: passed

## Verification Summary
All project-level acceptance criteria verified.
EOF
else
  cat <<'EOF'
# QA: PASS

## Tests Run
- cargo check: ok
- cargo test: 10 passed, 0 failed

## Verification Summary
All acceptance criteria from the spec have been verified.
EOF
fi"#,
    )
}

fn qa_fail_then_pass_mock_script(counter_file: &Path) -> String {
    let qa_branch = format!(
        r#"COUNTER_FILE="{}"
COUNT="$(cat "$COUNTER_FILE" 2>/dev/null || echo 0)"
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNTER_FILE"
if [ "$COUNT" -le 1 ]; then
  cat <<'EOF'
# QA: FAIL

## Failures
1. cargo test failed: 2 tests failing

## Suggested Fixes
1. Fix the failing assertions in test_feature_x.
EOF
else
  cat <<'EOF'
# QA: PASS

## Tests Run
- cargo test: all passed

## Verification Summary
All tests passing after fixes.
EOF
fi"#,
        counter_file.to_string_lossy()
    );
    mock_script_with_qa_branch(&qa_branch)
}

fn qa_always_fail_mock_script() -> String {
    mock_script_with_qa_branch(
        r#"cat <<'EOF'
# QA: FAIL

## Failures
1. Acceptance criteria is still unmet.

## Suggested Fixes
1. Implement the missing behavior and re-run the test suite.
EOF"#,
    )
}

fn mock_script_with_qa_branch(qa_branch: &str) -> String {
    r###"#!/bin/sh
set -eu

INPUT="$(cat)"

if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  if [ "${RALPH_COMPLETE:-no}" = "yes" ]; then
    cat <<'EOF'
# Project Completion Request

## Rationale
All required behavior is complete.

## Summary of Work
- Prior loops implemented and reviewed successfully.

## Remaining Items
- None
EOF
  else
    cat <<'EOF'
# Feature: Demo Feature

## Description
Mock feature used by validate tests.

## Acceptance Criteria
- [ ] Mock implementation file is created

## Files to Modify/Create
- `mock_file.txt` - file created by the mock implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
  fi
elif echo "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if echo "$INPUT" | grep -q "## Review Feedback" && ! echo "$INPUT" | grep -q "(none)"; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed feedback in the mock implementation.

## Could Not Address
- None
EOF
  else
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created a mock implementation artifact.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  fi
  echo "implemented" > mock_file.txt
  git add mock_file.txt
elif echo "$INPUT" | grep -q "You are a code reviewer ensuring implementations match specifications."; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
  if [ "${RALPH_COMPLETE:-no}" = "yes" ]; then
    cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Mock requirement: satisfied
EOF
  else
    cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Additional feature remains.

## Recommended Next Features
1. Implement another mock feature.
EOF
  fi
elif echo "$INPUT" | grep -q "You are a QA engineer validating"; then
__QA_BRANCH__
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .replace("__QA_BRANCH__", qa_branch)
}

fn acceptance_gate_pass(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qa-accept-pass";
        setup_with_mock_script(h, project_id, "qa-accept-pass.sh", &qa_pass_mock_script());
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run with RALPH_COMPLETE should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be an array");
        assert_eq!(
            attempts.len(),
            1,
            "expected exactly one completion attempt, got {}",
            attempts.len()
        );

        let first_attempt = &attempts[0];
        assert_eq!(
            first_attempt["artifacts"]["acceptance_passed"],
            json!(true),
            "expected first completion attempt to have acceptance_passed == true"
        );

        let acceptance_result_rel = first_attempt["artifacts"]["acceptance_result"]
            .as_str()
            .expect("acceptance_result artifact path should exist");
        let project_dir = h.project_dir(project_id);
        let acceptance_path = project_dir.join(acceptance_result_rel);
        assert_file_exists(&acceptance_path);
        assert_file_contains(&acceptance_path, "# QA: PASS");
    })
}

fn acceptance_gate_fail_forces_continue(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qa-accept-fail";
        let planner_counter = h.temp_dir.path().join("planner-counter.txt");
        let acceptance_counter = h.temp_dir.path().join("acceptance-counter.txt");
        let script = acceptance_fail_then_pass_mock_script(&planner_counter, &acceptance_counter);
        setup_with_mock_script(h, project_id, "qa-accept-fail.sh", &script);
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        let output = h
            .ralph(["run", "--until-complete"])
            .expect("ralph run --until-complete should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be an array");
        assert!(
            attempts.len() >= 2,
            "expected at least 2 completion attempts, got {}",
            attempts.len()
        );

        // First completion attempt: acceptance_passed == false
        let first_attempt = &attempts[0];
        assert_eq!(
            first_attempt["artifacts"]["acceptance_passed"],
            json!(false),
            "expected first completion attempt to have acceptance_passed == false"
        );
        let first_acceptance_rel = first_attempt["artifacts"]["acceptance_result"]
            .as_str()
            .expect("first acceptance_result artifact should exist");
        let project_dir = h.project_dir(project_id);
        let first_acceptance_path = project_dir.join(first_acceptance_rel);
        assert_file_exists(&first_acceptance_path);
        assert_file_contains(&first_acceptance_path, "# QA: FAIL");

        // First attempt verdict should be overridden to continue
        assert_eq!(
            first_attempt["verdict"],
            json!("continue"),
            "expected first completion attempt verdict to be 'continue' (forced by acceptance failure)"
        );

        // At least 2 feature loops completed (forced continue caused another feature loop)
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert!(
            loops.len() >= 2,
            "expected at least 2 completed feature loops, got {}",
            loops.len()
        );

        // Last completion attempt: acceptance_passed == true
        let last_attempt = &attempts[attempts.len() - 1];
        assert_eq!(
            last_attempt["artifacts"]["acceptance_passed"],
            json!(true),
            "expected final completion attempt to have acceptance_passed == true"
        );
    })
}

/// Mock script for `acceptance_gate_fail_forces_continue`.
///
/// The planner alternates between Feature and CompletionRequest using a counter:
///   - Odd calls (1, 3, ...): Feature spec
///   - Even calls (2, 4, ...): Project Completion Request
///
/// This produces at least 2 feature loops and 2 completion attempts.
///
/// The acceptance QA uses a counter file:
///   - 1st call → QA: FAIL
///   - 2nd+ calls → QA: PASS
///
/// Feature QA always passes.
/// Completer always returns COMPLETE.
fn acceptance_fail_then_pass_mock_script(
    planner_counter: &Path,
    acceptance_counter: &Path,
) -> String {
    format!(
        r###"#!/bin/sh
set -eu

INPUT="$(cat)"

PLANNER_COUNTER="{planner_counter}"
ACCEPTANCE_COUNTER="{acceptance_counter}"

if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  PCOUNT="$(cat "$PLANNER_COUNTER" 2>/dev/null || echo 0)"
  PCOUNT=$((PCOUNT + 1))
  echo "$PCOUNT" > "$PLANNER_COUNTER"
  # Odd calls produce Feature, even calls produce CompletionRequest
  REMAINDER=$((PCOUNT % 2))
  if [ "$REMAINDER" -eq 1 ]; then
    cat <<'EOF'
# Feature: Recovery Feature

## Description
Feature added after acceptance QA failure forced continuation.

## Acceptance Criteria
- [ ] Recovery file is created

## Files to Modify/Create
- `recovery_file.txt` - file created by the mock implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
  else
    cat <<'EOF'
# Project Completion Request

## Rationale
All required behavior is complete.

## Summary of Work
- Prior loops implemented and reviewed successfully.

## Remaining Items
- None
EOF
  fi
elif echo "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if echo "$INPUT" | grep -q "## Review Feedback" && ! echo "$INPUT" | grep -q "(none)"; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed feedback in the mock implementation.

## Could Not Address
- None
EOF
  else
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created a mock implementation artifact.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  fi
  echo "implemented" > mock_file.txt
  git add mock_file.txt
elif echo "$INPUT" | grep -q "You are a code reviewer ensuring implementations match specifications."; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Mock requirement: satisfied
EOF
elif echo "$INPUT" | grep -q "You are a QA engineer validating"; then
  if echo "$INPUT" | grep -q "overall project acceptance"; then
    ACOUNT="$(cat "$ACCEPTANCE_COUNTER" 2>/dev/null || echo 0)"
    ACOUNT=$((ACOUNT + 1))
    echo "$ACOUNT" > "$ACCEPTANCE_COUNTER"
    if [ "$ACOUNT" -le 1 ]; then
      cat <<'EOF'
# QA: FAIL

## Failures
1. Project-level acceptance criteria not fully met.

## Suggested Fixes
1. Add the missing recovery feature and re-run acceptance.
EOF
    else
      cat <<'EOF'
# QA: PASS

## Tests Run
- acceptance check: passed

## Verification Summary
All project-level acceptance criteria verified.
EOF
    fi
  else
    cat <<'EOF'
# QA: PASS

## Tests Run
- cargo check: ok
- cargo test: 10 passed, 0 failed

## Verification Summary
All acceptance criteria from the spec have been verified.
EOF
  fi
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###,
        planner_counter = planner_counter.to_string_lossy(),
        acceptance_counter = acceptance_counter.to_string_lossy(),
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
