use super::*;

use std::path::PathBuf;

use crate::validate::assertions::{
    assert_exit_code, assert_file_contains, assert_file_exists, assert_path_not_exists,
    assert_stderr_contains, parse_yaml_frontmatter,
};
use crate::validate::harness::RalphHarness;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "prompt_review_panel::multi_validator_accept_path",
            func: multi_validator_accept_path,
        },
        ConformanceTest {
            name: "prompt_review_panel::multi_validator_reject_path",
            func: multi_validator_reject_path,
        },
        ConformanceTest {
            name: "prompt_review_panel::mixed_accept_reject_aggregation",
            func: mixed_accept_reject_aggregation,
        },
        ConformanceTest {
            name: "prompt_review_panel::optional_validator_skipping",
            func: optional_validator_skipping,
        },
        ConformanceTest {
            name: "prompt_review_panel::singular_alias_compatibility",
            func: singular_alias_compatibility,
        },
        ConformanceTest {
            name: "prompt_review_panel::min_reviewers_enforcement",
            func: min_reviewers_enforcement,
        },
    ]
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

fn panel_backend_script(validator_verdict: &str) -> String {
    format!(
        r###"#!/bin/sh
set -eu

INPUT="$(cat)"

if printf '%s' "$INPUT" | grep -q "You are a prompt review validator."; then
  cat <<'EOF'
{validator_verdict}
EOF
elif printf '%s' "$INPUT" | grep -q "You are a prompt reviewer."; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- tighten acceptance criteria

## Refined Prompt
This is the refined prompt from the panel refiner.
EOF
elif printf '%s' "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  cat <<'EOF'
# Feature: Panel Feature

## Description
Mock feature used by prompt-review-panel conformance tests.

## Acceptance Criteria
- [ ] Mock implementation file is created

## Files to Modify/Create
- `mock_file.txt` - file created by the mock implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
elif printf '%s' "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created a mock implementation artifact.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  echo "implemented" > mock_file.txt
  git add mock_file.txt
elif printf '%s' "$INPUT" | grep -q "You are a code reviewer ensuring implementations match specifications."; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif printf '%s' "$INPUT" | grep -q "You are a QA engineer"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif printf '%s' "$INPUT" | grep -q "You are a final reviewer evaluating a completed project for quality and correctness."; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
The project is complete and requires no further amendments.
EOF
elif printf '%s' "$INPUT" | grep -q "You are a project completion validator."; then
  cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Additional feature remains.

## Recommended Next Features
1. Implement another mock feature.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    )
}

fn write_wrapped_mock(h: &RalphHarness, name: &str, content: &str) -> PathBuf {
    let script = h
        .write_mock_script(name, content)
        .expect("write mock script");
    let wrapper_name = format!("{}-wrapper.sh", name.trim_end_matches(".sh"));
    let wrapper_content = format!("#!/bin/sh\nexec sh \"{}\"\n", script.to_string_lossy());
    h.write_mock_script(&wrapper_name, &wrapper_content)
        .expect("write wrapper script")
}

fn setup_panel_mocks(
    h: &RalphHarness,
    project_id: &str,
    claude_verdict: &str,
    codex_verdict: &str,
) {
    h.init_workspace().expect("init failed");

    let claude_wrapper = write_wrapped_mock(
        h,
        "prompt-panel-claude.sh",
        &panel_backend_script(claude_verdict),
    );
    let codex_wrapper = write_wrapped_mock(
        h,
        "prompt-panel-codex.sh",
        &panel_backend_script(codex_verdict),
    );

    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.claude.command".to_owned(),
        claude_wrapper.to_string_lossy().into_owned(),
        "--global".to_owned(),
    ])
    .expect("set claude command");
    h.ralph_ok(["config", "set", "backends.claude.args", "[]", "--global"])
        .expect("set claude args");

    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.codex.command".to_owned(),
        codex_wrapper.to_string_lossy().into_owned(),
        "--global".to_owned(),
    ])
    .expect("set codex command");
    h.ralph_ok(["config", "set", "backends.codex.args", "[]", "--global"])
        .expect("set codex args");

    h.ralph_ok([
        "config",
        "set",
        "backends.gemini.enabled",
        "false",
        "--global",
    ])
    .expect("disable gemini");

    h.create_project(
        project_id,
        "Prompt Review Panel Project",
        "Prompt review panel test prompt",
    )
    .expect("create project failed");
}

fn configure_gemini_mock(h: &RalphHarness, validator_verdict: &str) {
    let gemini_wrapper = write_wrapped_mock(
        h,
        "prompt-panel-gemini.sh",
        &panel_backend_script(validator_verdict),
    );
    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.gemini.command".to_owned(),
        gemini_wrapper.to_string_lossy().into_owned(),
        "--global".to_owned(),
    ])
    .expect("set gemini command");
    h.ralph_ok(["config", "set", "backends.gemini.args", "[]", "--global"])
        .expect("set gemini args");
    h.ralph_ok([
        "config",
        "set",
        "backends.gemini.enabled",
        "true",
        "--global",
    ])
    .expect("enable gemini");
}

fn multi_validator_accept_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-panel-accept";
        setup_panel_mocks(h, project_id, "ACCEPT", "ACCEPT");

        h.ralph_ok([
            "config",
            "set",
            "workflow.prompt_review_backends",
            "[\"claude\",\"codex\",\"claude(opus)\"]",
        ])
        .expect("set prompt_review_backends");
        h.ralph_ok(["config", "set", "workflow.prompt_review_min_reviewers", "2"])
            .expect("set prompt_review_min_reviewers");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("run should succeed on accept path");

        let project_dir = h.project_dir(project_id);
        let codex_validator = project_dir.join("prompt-review-validator-codex.md");
        let claude_validator = project_dir.join("prompt-review-validator-claude-opus.md");
        assert_file_exists(&project_dir.join("prompt-review.md"));
        assert_file_exists(&codex_validator);
        assert_file_exists(&claude_validator);
        assert_file_contains(&codex_validator, "ACCEPT");
        assert_file_contains(&claude_validator, "ACCEPT");

        let fm = parse_yaml_frontmatter(&codex_validator);
        assert_eq!(fm["artifact"].as_str(), Some("prompt-review-validator"));
        assert_eq!(fm["role"].as_str(), Some("prompt_review_validator"));

        let state = h.load_state(project_id).expect("load state");
        assert_eq!(state["prompt_review_completed"], json!(true));
    })
}

fn multi_validator_reject_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-panel-reject";
        setup_panel_mocks(
            h,
            project_id,
            "ACCEPT",
            "REJECT(missing acceptance criteria)",
        );

        h.ralph_ok([
            "config",
            "set",
            "workflow.prompt_review_backends",
            "[\"claude\",\"codex\"]",
        ])
        .expect("set prompt_review_backends");
        h.ralph_ok(["config", "set", "workflow.prompt_review_min_reviewers", "1"])
            .expect("set prompt_review_min_reviewers");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("run should execute");
        assert_exit_code(&output, 1);
        assert_stderr_contains(&output, "prompt review rejected by validator(s)");
        assert_stderr_contains(&output, "missing acceptance criteria");

        let project_dir = h.project_dir(project_id);
        let validator = project_dir.join("prompt-review-validator-codex.md");
        assert_file_exists(&project_dir.join("prompt-review.md"));
        assert_file_exists(&validator);
        assert_file_contains(&validator, "REJECT(missing acceptance criteria)");
    })
}

fn mixed_accept_reject_aggregation(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-panel-mixed";
        setup_panel_mocks(h, project_id, "ACCEPT", "REJECT(reason-from-codex)");
        configure_gemini_mock(h, "REJECT(reason-from-gemini)");

        h.ralph_ok([
            "config",
            "set",
            "workflow.prompt_review_backends",
            "[\"claude\",\"codex\",\"claude(opus)\",\"gemini\"]",
        ])
        .expect("set prompt_review_backends");
        h.ralph_ok(["config", "set", "workflow.prompt_review_min_reviewers", "2"])
            .expect("set prompt_review_min_reviewers");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("run should execute");
        assert_exit_code(&output, 1);
        assert_stderr_contains(&output, "prompt review rejected by validator(s)");
        assert_stderr_contains(&output, "reason-from-codex");
        assert_stderr_contains(&output, "reason-from-gemini");

        let project_dir = h.project_dir(project_id);
        assert_file_exists(&project_dir.join("prompt-review-validator-codex.md"));
        assert_file_exists(&project_dir.join("prompt-review-validator-claude-opus.md"));
        assert_file_exists(&project_dir.join("prompt-review-validator-gemini.md"));
    })
}

fn optional_validator_skipping(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-panel-optional-skip";
        setup_panel_mocks(h, project_id, "ACCEPT", "ACCEPT");

        h.ralph_ok([
            "config",
            "set",
            "workflow.prompt_review_backends",
            "[\"claude\",\"codex\",\"?gemini\"]",
        ])
        .expect("set prompt_review_backends");
        h.ralph_ok(["config", "set", "workflow.prompt_review_min_reviewers", "1"])
            .expect("set prompt_review_min_reviewers");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("run should succeed with optional validator skipped");

        let project_dir = h.project_dir(project_id);
        assert_file_exists(&project_dir.join("prompt-review-validator-codex.md"));
        assert_path_not_exists(&project_dir.join("prompt-review-validator-gemini.md"));
    })
}

fn singular_alias_compatibility(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-panel-alias";
        setup_panel_mocks(h, project_id, "ACCEPT", "ACCEPT");

        h.ralph_ok(["config", "set", "workflow.prompt_review_backend", "claude"])
            .expect("set singular prompt_review_backend alias");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("run should succeed with singular alias");

        let project_dir = h.project_dir(project_id);
        let fm = parse_yaml_frontmatter(&project_dir.join("prompt-review.md"));
        assert_eq!(fm["backend"].as_str(), Some("claude"));
        assert_path_not_exists(&project_dir.join("prompt-review-validator-codex.md"));
    })
}

fn min_reviewers_enforcement(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-panel-min-reviewers";
        setup_panel_mocks(h, project_id, "ACCEPT", "ACCEPT");

        h.ralph_ok([
            "config",
            "set",
            "workflow.prompt_review_backends",
            "[\"claude\",\"?gemini\"]",
        ])
        .expect("set prompt_review_backends");
        h.ralph_ok(["config", "set", "workflow.prompt_review_min_reviewers", "1"])
            .expect("set prompt_review_min_reviewers");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("run should execute");
        assert_exit_code(&output, 1);
        assert_stderr_contains(&output, "prompt_review_min_reviewers requires 1");
    })
}
