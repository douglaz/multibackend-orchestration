use super::*;

use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "pre_commit_checks::config_get_set",
            func: config_get_set,
        },
        ConformanceTest {
            name: "pre_commit_checks::disabled_skips_checks",
            func: disabled_skips_checks,
        },
        ConformanceTest {
            name: "pre_commit_checks::enabled_no_cargo_toml_passes",
            func: enabled_no_cargo_toml_passes,
        },
    ]
}

fn config_get_set(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Check defaults
        let fmt = h
            .ralph_ok(["config", "get", "workflow.pre_commit_fmt"])
            .expect("config get workflow.pre_commit_fmt failed");
        assert_eq!(fmt.trim(), "true", "pre_commit_fmt should default to true");

        let clippy = h
            .ralph_ok(["config", "get", "workflow.pre_commit_clippy"])
            .expect("config get workflow.pre_commit_clippy failed");
        assert_eq!(
            clippy.trim(),
            "true",
            "pre_commit_clippy should default to true"
        );

        let nix_build = h
            .ralph_ok(["config", "get", "workflow.pre_commit_nix_build"])
            .expect("config get workflow.pre_commit_nix_build failed");
        assert_eq!(
            nix_build.trim(),
            "false",
            "pre_commit_nix_build should default to false"
        );

        let fmt_auto_fix = h
            .ralph_ok(["config", "get", "workflow.pre_commit_fmt_auto_fix"])
            .expect("config get workflow.pre_commit_fmt_auto_fix failed");
        assert_eq!(
            fmt_auto_fix.trim(),
            "false",
            "pre_commit_fmt_auto_fix should default to false"
        );

        // Set and verify round-trip
        h.ralph_ok(["config", "set", "workflow.pre_commit_fmt", "false"])
            .expect("config set pre_commit_fmt failed");
        let fmt_after = h
            .ralph_ok(["config", "get", "workflow.pre_commit_fmt"])
            .expect("config get pre_commit_fmt after set failed");
        assert_eq!(fmt_after.trim(), "false");

        h.ralph_ok(["config", "set", "workflow.pre_commit_nix_build", "true"])
            .expect("config set pre_commit_nix_build failed");
        let nix_after = h
            .ralph_ok(["config", "get", "workflow.pre_commit_nix_build"])
            .expect("config get pre_commit_nix_build after set failed");
        assert_eq!(nix_after.trim(), "true");

        h.ralph_ok(["config", "set", "workflow.pre_commit_clippy", "false"])
            .expect("config set pre_commit_clippy failed");
        let clippy_after = h
            .ralph_ok(["config", "get", "workflow.pre_commit_clippy"])
            .expect("config get pre_commit_clippy after set failed");
        assert_eq!(clippy_after.trim(), "false");

        h.ralph_ok(["config", "set", "workflow.pre_commit_fmt_auto_fix", "true"])
            .expect("config set pre_commit_fmt_auto_fix failed");
        let auto_fix_after = h
            .ralph_ok(["config", "get", "workflow.pre_commit_fmt_auto_fix"])
            .expect("config get pre_commit_fmt_auto_fix after set failed");
        assert_eq!(auto_fix_after.trim(), "true");
    })
}

fn disabled_skips_checks(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-172-disabled";
        setup_project(h, project_id);

        // Disable all pre-commit checks
        h.ralph_ok(["config", "set", "workflow.pre_commit_fmt", "false"])
            .expect("config set pre_commit_fmt failed");
        h.ralph_ok(["config", "set", "workflow.pre_commit_clippy", "false"])
            .expect("config set pre_commit_clippy failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected exactly one completed loop");
        let loop_state = &loops[0];

        // Should have committed successfully (no pre-commit check failures)
        assert!(
            loop_state["commit"].as_str().is_some(),
            "expected loop to commit normally when pre-commit checks are disabled"
        );

        // Verify no pre-commit failure artifacts
        let artifact_names = loop_artifact_names(h, project_id, 1);
        let pre_commit_artifacts: Vec<_> = artifact_names
            .iter()
            .filter(|name| name.contains("pre-commit"))
            .collect();
        assert!(
            pre_commit_artifacts.is_empty(),
            "expected no pre-commit artifacts when checks disabled, got: {pre_commit_artifacts:?}"
        );
    })
}

fn enabled_no_cargo_toml_passes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-172-no-cargo";
        setup_project(h, project_id);

        // Enable pre-commit checks (fmt and clippy are already default true)
        // The test worktree has no Cargo.toml, so cargo checks should be skipped

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed with no Cargo.toml");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected exactly one completed loop");
        let loop_state = &loops[0];

        // Should have committed successfully (cargo checks skipped due to no Cargo.toml)
        assert!(
            loop_state["commit"].as_str().is_some(),
            "expected loop to commit normally when no Cargo.toml exists"
        );
    })
}

// ---------------------------------------------------------------------------

fn setup_project(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script_path = h
        .write_mock_script("pre-commit-test.sh", &mock_script())
        .expect("failed to write mock script");
    h.setup_mock_backends_stable(&script_path)
        .expect("setup_mock_backends_stable failed");
    h.create_project(
        project_id,
        "Pre-Commit Checks Test",
        "Test prompt for pre-commit checks",
    )
    .expect("create_project failed");

    // Disable QA and final review to keep the test simple
    h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
        .expect("config set qa_enabled failed");
    h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
        .expect("config set final_review_enabled failed");
}

fn mock_script() -> String {
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
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
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

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
