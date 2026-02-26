use super::*;

use std::fs;
use std::path::Path;

use crate::validate::assertions::{
    assert_exit_code, assert_file_contains, assert_file_exists, assert_json_array_len,
    assert_json_field, assert_no_loop_artifacts, assert_stderr_contains,
};
use crate::validate::harness::RalphHarness;
use serde_json::{json, Value};

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
            name: "qa::iteration_limit_fails",
            func: iteration_limit_fails,
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
        ConformanceTest {
            name: "qa::acceptance_gate_multi_backend_one_fails",
            func: acceptance_gate_multi_backend_one_fails,
        },
        ConformanceTest {
            name: "qa::acceptance_gate_multi_backend_independent",
            func: acceptance_gate_multi_backend_independent,
        },
        ConformanceTest {
            name: "qa::acceptance_gate_qa_backend_override_no_duplicate",
            func: acceptance_gate_qa_backend_override_no_duplicate,
        },
        ConformanceTest {
            name: "qa::acceptance_gate_qa_backend_override_opposite_family",
            func: acceptance_gate_qa_backend_override_opposite_family,
        },
        ConformanceTest {
            name: "qa::acceptance_gate_all_feedback_on_failure",
            func: acceptance_gate_all_feedback_on_failure,
        },
        ConformanceTest {
            name: "qa::history_verbose_shows_qa",
            func: history_verbose_shows_qa,
        },
        ConformanceTest {
            name: "qa::status_shows_qa_info",
            func: status_shows_qa_info,
        },
    ]
}

fn disabled_skips_phase(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-301";
        setup_with_mock_script(h, project_id, "qa-pass.sh", &qa_pass_mock_script());

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("config set qa_enabled failed");

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
        assert_has_ralph_checkpoint_commit(&h.repo_root, project_id);
    })
}

fn enabled_pass_proceeds_to_review(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-302";
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
        assert_has_ralph_checkpoint_commit(&h.repo_root, project_id);
    })
}

fn fail_retries_then_passes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-303";
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
        assert_has_ralph_checkpoint_commit(&h.repo_root, project_id);
    })
}

fn iteration_limit_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-304";
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

        // After QA limit rollback, artifacts are removed and reconstruction
        // derives "pending" status.  The non-zero exit code is the failure signal.
        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("pending"));
        assert_json_array_len(&state, "loops", 0);
        assert_no_loop_artifacts(&h.project_dir(project_id));
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
            "true",
            "expected workflow.qa_enabled default to be true"
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
    h.setup_mock_backends_stable(&script_path)
        .expect("setup_mock_backends_stable failed");
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

## Manual Testing
- acceptance manual check: passed

## Automated Tests
- acceptance check: passed

## Acceptance Criteria Verification
All project-level acceptance criteria verified.
EOF
else
  cat <<'EOF'
# QA: PASS

## Manual Testing
- ran binary with test args: ok
- verified CLI output matches spec

## Automated Tests
- cargo check: ok
- cargo test: 10 passed, 0 failed

## Acceptance Criteria Verification
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

## Manual Testing
- ran binary: all features work

## Automated Tests
- cargo test: all passed

## Acceptance Criteria Verification
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
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
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
        let project_id = "issue-305";
        setup_with_mock_script(h, project_id, "qa-accept-pass.sh", &qa_pass_mock_script());
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
            .expect("config set workflow.final_review_enabled false failed");

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
        let acceptance_results = first_attempt["artifacts"]["acceptance_results"]
            .as_array()
            .expect("acceptance_results should be an array");
        assert_acceptance_results_cover_both_families(acceptance_results);
        let passed_count = acceptance_results
            .iter()
            .filter(|result| result["passed"] == json!(true))
            .count();
        assert_eq!(
            passed_count, 2,
            "expected both acceptance QA backends to pass"
        );

        let project_dir = h.project_dir(project_id);
        for result in acceptance_results {
            let acceptance_result_rel = result["artifact"]
                .as_str()
                .expect("acceptance artifact path should exist");
            let acceptance_path = project_dir.join(acceptance_result_rel);
            assert_file_exists(&acceptance_path);
            assert_file_contains(&acceptance_path, "# QA: PASS");
        }
    })
}

fn acceptance_gate_fail_forces_continue(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-306";
        let planner_counter = h.temp_dir.path().join("planner-counter.txt");
        let acceptance_counter = h.temp_dir.path().join("acceptance-counter.txt");
        let script = acceptance_fail_then_pass_mock_script(&planner_counter, &acceptance_counter);
        setup_with_mock_script(h, project_id, "qa-accept-fail.sh", &script);
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
            .expect("config set workflow.final_review_enabled false failed");

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

        // First completion attempt should include both backend families and mixed pass/fail.
        let first_attempt = &attempts[0];
        let first_acceptance_results = first_attempt["artifacts"]["acceptance_results"]
            .as_array()
            .expect("first attempt acceptance_results should be an array");
        assert_acceptance_results_cover_both_families(first_acceptance_results);
        let first_pass_count = first_acceptance_results
            .iter()
            .filter(|result| result["passed"] == json!(true))
            .count();
        let first_fail_count = first_acceptance_results
            .iter()
            .filter(|result| result["passed"] == json!(false))
            .count();
        assert_eq!(
            first_pass_count, 1,
            "expected first completion attempt to have one acceptance pass"
        );
        assert_eq!(
            first_fail_count, 1,
            "expected first completion attempt to have one acceptance failure"
        );
        let project_dir = h.project_dir(project_id);
        for result in first_acceptance_results {
            let first_acceptance_rel = result["artifact"]
                .as_str()
                .expect("first acceptance artifact should exist");
            let first_acceptance_path = project_dir.join(first_acceptance_rel);
            assert_file_exists(&first_acceptance_path);
            if result["passed"] == json!(true) {
                assert_file_contains(&first_acceptance_path, "# QA: PASS");
            } else {
                assert_file_contains(&first_acceptance_path, "# QA: FAIL");
            }
        }

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

        // Last completion attempt should include both backends and both passing.
        let last_attempt = &attempts[attempts.len() - 1];
        let last_acceptance_results = last_attempt["artifacts"]["acceptance_results"]
            .as_array()
            .expect("last attempt acceptance_results should be an array");
        assert_acceptance_results_cover_both_families(last_acceptance_results);
        let last_pass_count = last_acceptance_results
            .iter()
            .filter(|result| result["passed"] == json!(true))
            .count();
        assert_eq!(
            last_pass_count, 2,
            "expected final completion attempt to have two passing acceptance results"
        );
    })
}

fn acceptance_gate_multi_backend_one_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-307";
        let codex_fail_counter = h.temp_dir.path().join("acceptance-codex-fail-counter.txt");
        let script = acceptance_one_backend_fail_then_pass_mock_script(&codex_fail_counter);
        setup_with_mock_script(h, project_id, "qa-accept-one-fails.sh", &script);
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
            .expect("config set workflow.final_review_enabled false failed");

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
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

        let first_attempt = &attempts[0];
        assert_eq!(
            first_attempt["verdict"],
            json!("continue"),
            "expected first completion attempt verdict to be forced to continue"
        );
        let first_acceptance_results = first_attempt["artifacts"]["acceptance_results"]
            .as_array()
            .expect("first attempt acceptance_results should be an array");
        assert_acceptance_results_cover_both_families(first_acceptance_results);
        assert_eq!(
            acceptance_result_for_family(first_acceptance_results, "claude")["passed"],
            json!(true),
            "expected claude acceptance QA to pass on first completion attempt"
        );
        assert_eq!(
            acceptance_result_for_family(first_acceptance_results, "codex")["passed"],
            json!(false),
            "expected codex acceptance QA to fail on first completion attempt"
        );
    })
}

fn acceptance_gate_multi_backend_independent(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-308";
        let planner_counter = h.temp_dir.path().join("planner-counter-independent.txt");
        let qa_invocations = h.temp_dir.path().join("acceptance-invocations.log");
        let script = acceptance_independent_mock_script(&planner_counter, &qa_invocations);
        setup_with_mock_script(h, project_id, "qa-accept-independent.sh", &script);
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("in_progress"));

        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be an array");
        assert_eq!(
            attempts.len(),
            1,
            "expected exactly one completion attempt for independence test"
        );

        let first_attempt = &attempts[0];
        assert_eq!(
            first_attempt["verdict"],
            json!("continue"),
            "expected completion verdict to be forced to continue after acceptance failure"
        );
        let acceptance_results = first_attempt["artifacts"]["acceptance_results"]
            .as_array()
            .expect("acceptance_results should be an array");
        assert_acceptance_results_cover_both_families(acceptance_results);
        assert_eq!(
            acceptance_result_for_family(acceptance_results, "claude")["passed"],
            json!(false),
            "expected claude acceptance QA to fail"
        );
        assert_eq!(
            acceptance_result_for_family(acceptance_results, "codex")["passed"],
            json!(true),
            "expected codex acceptance QA to pass"
        );

        let invocation_log = fs::read_to_string(&qa_invocations)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", qa_invocations.display()));
        let invocations = invocation_log
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        assert_eq!(
            invocations.len(),
            2,
            "expected exactly two acceptance QA invocations, got {:?}",
            invocations
        );
        assert!(
            invocations.iter().any(|entry| entry.starts_with("claude")),
            "expected acceptance QA invocation for claude backend, got {:?}",
            invocations
        );
        assert!(
            invocations.iter().any(|entry| entry.starts_with("codex")),
            "expected acceptance QA invocation for codex backend, got {:?}",
            invocations
        );
    })
}

fn acceptance_gate_qa_backend_override_no_duplicate(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-309";
        setup_with_mock_script(
            h,
            project_id,
            "qa-accept-override-same-family.sh",
            &qa_pass_mock_script(),
        );
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
            .expect("config set workflow.final_review_enabled false failed");
        h.ralph_ok(["config", "set", "workflow.completer_backend", "codex"])
            .expect("config set workflow.completer_backend failed");
        h.ralph_ok(["config", "set", "workflow.qa_backend", "codex"])
            .expect("config set workflow.qa_backend failed");

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
            "expected exactly one completion attempt for override test"
        );

        let acceptance_results = attempts[0]["artifacts"]["acceptance_results"]
            .as_array()
            .expect("acceptance_results should be an array");
        assert_acceptance_results_cover_both_families(acceptance_results);
        let pass_count = acceptance_results
            .iter()
            .filter(|result| result["passed"] == json!(true))
            .count();
        assert_eq!(
            pass_count, 2,
            "expected both acceptance QA backends to pass"
        );
    })
}

fn acceptance_gate_qa_backend_override_opposite_family(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-310";
        setup_with_mock_script(
            h,
            project_id,
            "qa-accept-override-opposite-family.sh",
            &qa_pass_mock_script(),
        );
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
            .expect("config set workflow.final_review_enabled false failed");
        h.ralph_ok(["config", "set", "workflow.completer_backend", "codex"])
            .expect("config set workflow.completer_backend failed");
        h.ralph_ok(["config", "set", "workflow.qa_backend", "claude"])
            .expect("config set workflow.qa_backend failed");

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
            "expected exactly one completion attempt for override test"
        );

        let acceptance_results = attempts[0]["artifacts"]["acceptance_results"]
            .as_array()
            .expect("acceptance_results should be an array");
        assert_acceptance_results_cover_both_families(acceptance_results);
        let pass_count = acceptance_results
            .iter()
            .filter(|result| result["passed"] == json!(true))
            .count();
        assert_eq!(
            pass_count, 2,
            "expected both acceptance QA backends to pass"
        );
    })
}

fn acceptance_gate_all_feedback_on_failure(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-311";
        let planner_counter = h.temp_dir.path().join("planner-counter-all-feedback.txt");
        let planner_prompt_dir = h.temp_dir.path().join("planner-prompts");
        let script =
            acceptance_all_fail_feedback_mock_script(&planner_counter, &planner_prompt_dir);
        setup_with_mock_script(h, project_id, "qa-accept-all-feedback.sh", &script);
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("in_progress"));

        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be an array");
        assert_eq!(
            attempts.len(),
            1,
            "expected exactly one completion attempt for all-feedback test"
        );
        let first_attempt = &attempts[0];
        assert_eq!(
            first_attempt["verdict"],
            json!("continue"),
            "expected completion verdict to be forced to continue after acceptance failures"
        );
        let acceptance_results = first_attempt["artifacts"]["acceptance_results"]
            .as_array()
            .expect("acceptance_results should be an array");
        assert_acceptance_results_cover_both_families(acceptance_results);
        let fail_count = acceptance_results
            .iter()
            .filter(|result| result["passed"] == json!(false))
            .count();
        assert_eq!(
            fail_count, 2,
            "expected both acceptance QA backends to fail"
        );

        let second_planner_prompt = planner_prompt_dir.join("planner-2.md");
        assert_file_exists(&second_planner_prompt);
        let second_planner_prompt_content = fs::read_to_string(&second_planner_prompt)
            .unwrap_or_else(|err| {
                panic!("failed to read {}: {err}", second_planner_prompt.display())
            });
        assert!(
            second_planner_prompt_content.contains("Claude acceptance blocker."),
            "expected planner feedback to include claude failure artifact, got:\n{}",
            second_planner_prompt_content
        );
        assert!(
            second_planner_prompt_content.contains("Codex acceptance blocker."),
            "expected planner feedback to include codex failure artifact, got:\n{}",
            second_planner_prompt_content
        );
        assert!(
            second_planner_prompt_content.contains("Acceptance QA Failure Artifact 1 (backend:")
                && second_planner_prompt_content
                    .contains("Acceptance QA Failure Artifact 2 (backend:"),
            "expected planner feedback to include numbered acceptance failure sections, got:\n{}",
            second_planner_prompt_content
        );
        assert!(
            second_planner_prompt_content.contains("backend: claude")
                && second_planner_prompt_content.contains("backend: codex"),
            "expected planner feedback to identify both failing backend families, got:\n{}",
            second_planner_prompt_content
        );
    })
}

fn history_verbose_shows_qa(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-312";
        setup_with_mock_script(
            h,
            project_id,
            "qa-history-verbose.sh",
            &qa_pass_mock_script(),
        );
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let stdout = h
            .ralph_ok(["history", "--verbose"])
            .expect("ralph history --verbose should succeed");

        // With the new checkpoint-based history, verbose output shows phase
        // transitions with commit hashes rather than QA-specific fields.
        // Verify the output contains loop entries and phase transition info.
        assert!(
            stdout.contains("loop 1") || stdout.contains("Loop 1"),
            "expected verbose history output to contain loop entries, got:\n{}",
            stdout
        );
        // Verbose mode includes commit hashes in parentheses
        let default_stdout = h
            .ralph_ok(["history"])
            .expect("ralph history should succeed");
        assert!(
            stdout.len() > default_stdout.len(),
            "expected verbose history to be richer than default history"
        );
    })
}

fn status_shows_qa_info(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-313";
        setup_with_mock_script(h, project_id, "qa-status.sh", &qa_pass_mock_script());
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let stdout = h.ralph_ok(["status"]).expect("ralph status should succeed");

        assert!(
            stdout.contains("Latest QA (iteration 1): PASS"),
            "expected status output to contain latest QA verdict information, got:\n{}",
            stdout
        );
        assert!(
            stdout.contains("qa="),
            "expected status output to contain qa backend field, got:\n{}",
            stdout
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

## Manual Testing
- acceptance manual check: passed

## Automated Tests
- acceptance check: passed

## Acceptance Criteria Verification
All project-level acceptance criteria verified.
EOF
    fi
  else
    cat <<'EOF'
# QA: PASS

## Manual Testing
- ran binary with test args: ok
- verified CLI output matches spec

## Automated Tests
- cargo check: ok
- cargo test: 10 passed, 0 failed

## Acceptance Criteria Verification
All acceptance criteria from the spec have been verified.
EOF
  fi
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###,
        planner_counter = planner_counter.to_string_lossy(),
        acceptance_counter = acceptance_counter.to_string_lossy(),
    )
}

fn assert_acceptance_results_cover_both_families(results: &[Value]) {
    assert_eq!(
        results.len(),
        2,
        "expected exactly 2 acceptance QA results, got {}",
        results.len()
    );

    let backends = results
        .iter()
        .map(|result| {
            result["backend"]
                .as_str()
                .unwrap_or_else(|| panic!("acceptance result backend should be a string: {result}"))
                .to_owned()
        })
        .collect::<Vec<_>>();

    assert_ne!(
        backends[0], backends[1],
        "expected distinct acceptance QA backends, got {:?}",
        backends
    );
    assert!(
        backends.iter().any(|backend| backend.starts_with("claude")),
        "expected one acceptance QA backend from claude family, got {:?}",
        backends
    );
    assert!(
        backends.iter().any(|backend| backend.starts_with("codex")),
        "expected one acceptance QA backend from codex family, got {:?}",
        backends
    );
}

fn acceptance_result_for_family<'a>(results: &'a [Value], family: &str) -> &'a Value {
    results
        .iter()
        .find(|result| {
            result["backend"]
                .as_str()
                .map(|backend| backend.starts_with(family))
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("missing acceptance QA result for backend family '{family}'"))
}

fn acceptance_one_backend_fail_then_pass_mock_script(codex_fail_counter: &Path) -> String {
    let qa_branch = format!(
        r#"if echo "$INPUT" | grep -q "overall project acceptance"; then
  if echo "$INPUT" | grep -q "QA Backend: claude"; then
    cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance manual check (claude): passed

## Automated Tests
- acceptance check (claude): passed

## Acceptance Criteria Verification
Claude acceptance check passed.
EOF
  elif echo "$INPUT" | grep -q "QA Backend: codex"; then
    CCOUNT="$(cat "{codex_fail_counter}" 2>/dev/null || echo 0)"
    CCOUNT=$((CCOUNT + 1))
    echo "$CCOUNT" > "{codex_fail_counter}"
    if [ "$CCOUNT" -le 1 ]; then
      cat <<'EOF'
# QA: FAIL

## Failures
1. Codex acceptance gate intentionally fails on first attempt.

## Suggested Fixes
1. Retry after planner feedback.
EOF
    else
      cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance manual check (codex): passed

## Automated Tests
- acceptance check (codex): passed

## Acceptance Criteria Verification
Codex acceptance check passed.
EOF
    fi
  else
    cat <<'EOF'
# QA: FAIL

## Failures
1. Unknown QA backend in acceptance prompt.

## Suggested Fixes
1. Ensure QA Backend context is present.
EOF
  fi
else
  cat <<'EOF'
# QA: PASS

## Manual Testing
- ran binary manually: ok

## Automated Tests
- cargo check: ok

## Acceptance Criteria Verification
Feature QA passed.
EOF
fi"#,
        codex_fail_counter = codex_fail_counter.to_string_lossy(),
    );

    mock_script_with_qa_branch(&qa_branch)
}

fn acceptance_independent_mock_script(planner_counter: &Path, qa_invocations: &Path) -> String {
    format!(
        r###"#!/bin/sh
set -eu

INPUT="$(cat)"
PLANNER_COUNTER="{planner_counter}"
QA_INVOCATIONS="{qa_invocations}"

if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  PCOUNT="$(cat "$PLANNER_COUNTER" 2>/dev/null || echo 0)"
  PCOUNT=$((PCOUNT + 1))
  echo "$PCOUNT" > "$PLANNER_COUNTER"
  if [ "$PCOUNT" -eq 1 ]; then
    cat <<'EOF'
# Project Completion Request

## Rationale
Validation of acceptance gate behavior.

## Summary of Work
- Initial behavior complete.

## Remaining Items
- None
EOF
  else
    cat <<'EOF'
# Feature: Post-Acceptance Followup

## Description
Feature returned after forced continue.

## Acceptance Criteria
- [ ] Followup file is created

## Files to Modify/Create
- `followup_file.txt` - file created by the mock implementer

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
- Created a followup implementation artifact.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  fi
  echo "implemented" > followup_file.txt
  git add followup_file.txt
elif echo "$INPUT" | grep -q "You are a code reviewer ensuring implementations match specifications."; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Followup file is created

## Notes
Looks good.

## Commit Message
feat: add followup artifact
EOF
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Completion criteria satisfied.
EOF
elif echo "$INPUT" | grep -q "You are a QA engineer validating"; then
  if echo "$INPUT" | grep -q "overall project acceptance"; then
    if ! echo "$INPUT" | grep -q '"acceptance_results": \[\]'; then
      cat <<'EOF'
# QA: FAIL

## Failures
1. Prompt contamination detected from prior acceptance results.

## Suggested Fixes
1. Snapshot state before acceptance QA loop.
EOF
      exit 0
    fi
    if echo "$INPUT" | grep -q "QA Backend: claude"; then
      echo "claude" >> "$QA_INVOCATIONS"
      cat <<'EOF'
# QA: FAIL

## Failures
1. Claude acceptance gate intentionally fails.

## Suggested Fixes
1. Retry planning after continuation.
EOF
    elif echo "$INPUT" | grep -q "QA Backend: codex"; then
      echo "codex" >> "$QA_INVOCATIONS"
      cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance manual check (codex): passed

## Automated Tests
- acceptance check (codex): passed

## Acceptance Criteria Verification
Codex acceptance check passed independently.
EOF
    else
      echo "unknown" >> "$QA_INVOCATIONS"
      cat <<'EOF'
# QA: FAIL

## Failures
1. Unknown acceptance backend.

## Suggested Fixes
1. Ensure QA Backend context is present.
EOF
    fi
  else
    cat <<'EOF'
# QA: PASS

## Manual Testing
- ran binary manually: ok

## Automated Tests
- cargo check: ok

## Acceptance Criteria Verification
Feature QA passed.
EOF
  fi
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###,
        planner_counter = planner_counter.to_string_lossy(),
        qa_invocations = qa_invocations.to_string_lossy(),
    )
}

fn acceptance_all_fail_feedback_mock_script(
    planner_counter: &Path,
    planner_prompt_dir: &Path,
) -> String {
    format!(
        r###"#!/bin/sh
set -eu

INPUT="$(cat)"
PLANNER_COUNTER="{planner_counter}"
PLANNER_PROMPT_DIR="{planner_prompt_dir}"

if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  PCOUNT="$(cat "$PLANNER_COUNTER" 2>/dev/null || echo 0)"
  PCOUNT=$((PCOUNT + 1))
  echo "$PCOUNT" > "$PLANNER_COUNTER"
  mkdir -p "$PLANNER_PROMPT_DIR"
  echo "$INPUT" > "$PLANNER_PROMPT_DIR/planner-$PCOUNT.md"
  if [ "$PCOUNT" -eq 1 ]; then
    cat <<'EOF'
# Project Completion Request

## Rationale
Acceptance feedback aggregation test.

## Summary of Work
- Initial implementation complete.

## Remaining Items
- None
EOF
  else
    cat <<'EOF'
# Feature: Recovery Feature

## Description
Feature added after acceptance QA failures.

## Acceptance Criteria
- [ ] Recovery file is created

## Files to Modify/Create
- `feedback_recovery.txt` - file created by the mock implementer

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
- Created a recovery implementation artifact.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  fi
  echo "implemented" > feedback_recovery.txt
  git add feedback_recovery.txt
elif echo "$INPUT" | grep -q "You are a code reviewer ensuring implementations match specifications."; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Recovery file is created

## Notes
Looks good.

## Commit Message
feat: add recovery artifact
EOF
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Completion criteria satisfied.
EOF
elif echo "$INPUT" | grep -q "You are a QA engineer validating"; then
  if echo "$INPUT" | grep -q "overall project acceptance"; then
    if echo "$INPUT" | grep -q "QA Backend: claude"; then
      cat <<'EOF'
# QA: FAIL

## Failures
1. Claude acceptance blocker.

## Suggested Fixes
1. Fix claude-related blocker.
EOF
    elif echo "$INPUT" | grep -q "QA Backend: codex"; then
      cat <<'EOF'
# QA: FAIL

## Failures
1. Codex acceptance blocker.

## Suggested Fixes
1. Fix codex-related blocker.
EOF
    else
      cat <<'EOF'
# QA: FAIL

## Failures
1. Unknown acceptance backend.

## Suggested Fixes
1. Ensure QA Backend context is present.
EOF
    fi
  else
    cat <<'EOF'
# QA: PASS

## Manual Testing
- ran binary manually: ok

## Automated Tests
- cargo check: ok

## Acceptance Criteria Verification
Feature QA passed.
EOF
  fi
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###,
        planner_counter = planner_counter.to_string_lossy(),
        planner_prompt_dir = planner_prompt_dir.to_string_lossy(),
    )
}

fn assert_has_ralph_checkpoint_commit(repo_root: &Path, project_id: &str) {
    let output = std::process::Command::new("git")
        .args(["log", "--format=%s"])
        .current_dir(repo_root)
        .output()
        .expect("git log should execute");
    assert!(
        output.status.success(),
        "git log failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let subjects = String::from_utf8_lossy(&output.stdout);
    assert!(
        subjects
            .lines()
            .any(|line| line.starts_with(&format!("ralph({project_id}):"))),
        "expected at least one Ralph checkpoint commit for project '{project_id}'"
    );
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
