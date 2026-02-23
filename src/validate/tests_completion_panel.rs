use super::*;

use crate::validate::assertions::{
    assert_exit_code, assert_json_field, assert_no_uncommitted_ralph_files,
};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::standard_mock_script;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "completion_panel::two_completer_consensus_complete",
            func: two_completer_consensus_complete,
        },
        ConformanceTest {
            name: "completion_panel::single_completer_backward_compat",
            func: single_completer_backward_compat,
        },
        ConformanceTest {
            name: "completion_panel::panel_continue_verdict",
            func: panel_continue_verdict,
        },
        ConformanceTest {
            name: "completion_panel::per_backend_verdict_artifacts",
            func: per_backend_verdict_artifacts,
        },
        ConformanceTest {
            name: "completion_panel::optional_backend_skip",
            func: optional_backend_skip,
        },
        ConformanceTest {
            name: "completion_panel::required_backend_failure",
            func: required_backend_failure,
        },
        ConformanceTest {
            name: "completion_panel::partial_threshold_consensus",
            func: partial_threshold_consensus,
        },
        ConformanceTest {
            name: "completion_panel::insufficient_min_completers",
            func: insufficient_min_completers,
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

/// Generate a complete mock backend script that handles all orchestrator phases.
/// The `verdict` parameter controls the completion validator response: "COMPLETE"
/// or "CONTINUE".  Uses POSIX `/bin/sh` so it works in Nix sandboxes.
fn complete_mock_script(verdict: &str) -> String {
    let verdict_body = if verdict == "COMPLETE" {
        "# Verdict: COMPLETE\n\nThe project satisfies all requirements:\n- Mock requirement: satisfied"
    } else {
        "# Verdict: CONTINUE\n\n## Missing Requirements\n1. Additional work remains.\n\n## Recommended Next Features\n1. Implement another mock feature."
    };
    format!(
        r###"#!/bin/sh
set -eu
INPUT="$(cat)"
if printf '%s' "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif printf '%s' "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  cat <<'EOF'
# Project Completion Request

## Rationale
All required behavior is complete.

## Summary of Work
- Prior loops implemented and reviewed successfully.

## Remaining Items
- None
EOF
elif printf '%s' "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if printf '%s' "$INPUT" | grep -q "## Review Feedback" && ! printf '%s' "$INPUT" | grep -q "(none)"; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed reviewer feedback in the mock implementation.

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
elif printf '%s' "$INPUT" | grep -q "You are a final reviewer evaluating a completed project for quality and correctness."; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
The project is complete and requires no further amendments.
EOF
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
elif printf '%s' "$INPUT" | grep -q "You are a project completion validator."; then
  cat <<'EOF'
{verdict_body}
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    )
}

/// Like `complete_mock_script` but takes a `counter_file` path.  The planner
/// returns a completion request on the **first** invocation, then a regular
/// feature spec on all subsequent calls.  This prevents infinite loops when
/// the completion verdict is CONTINUE — the next planner call produces a
/// feature that the orchestrator can process as a normal loop.
fn complete_mock_script_with_counter(verdict: &str, counter_file: &std::path::Path) -> String {
    let verdict_body = if verdict == "COMPLETE" {
        "# Verdict: COMPLETE\n\nThe project satisfies all requirements:\n- Mock requirement: satisfied"
    } else {
        "# Verdict: CONTINUE\n\n## Missing Requirements\n1. Additional work remains.\n\n## Recommended Next Features\n1. Implement another mock feature."
    };
    format!(
        r###"#!/bin/sh
set -eu
INPUT="$(cat)"
if printf '%s' "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif printf '%s' "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  COUNTER_FILE="{counter}"
  COUNT="$(cat "$COUNTER_FILE" 2>/dev/null || echo 0)"
  COUNT=$((COUNT + 1))
  echo "$COUNT" > "$COUNTER_FILE"
  if [ "$COUNT" -le 1 ]; then
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
elif printf '%s' "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if printf '%s' "$INPUT" | grep -q "## Review Feedback" && ! printf '%s' "$INPUT" | grep -q "(none)"; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed reviewer feedback in the mock implementation.

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
elif printf '%s' "$INPUT" | grep -q "You are a final reviewer evaluating a completed project for quality and correctness."; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
The project is complete and requires no further amendments.
EOF
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
elif printf '%s' "$INPUT" | grep -q "You are a project completion validator."; then
  cat <<'EOF'
{verdict_body}
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###,
        counter = counter_file.to_string_lossy(),
    )
}

/// Write a mock script and create a `/bin/sh` wrapper that execs `bash <script>`
/// (matching what `setup_mock_backends_stable` does) so the script works in Nix
/// sandboxes where `/usr/bin/env` is absent.
fn write_wrapped_mock(h: &RalphHarness, name: &str, content: &str) -> std::path::PathBuf {
    let script = h
        .write_mock_script(name, content)
        .expect("write mock script");
    let wrapper_content = format!("#!/bin/sh\nexec sh \"{}\"\n", script.to_string_lossy());
    let wrapper_name = format!("{}-wrapper.sh", name.trim_end_matches(".sh"));
    h.write_mock_script(&wrapper_name, &wrapper_content)
        .expect("write wrapper script")
}

fn setup_panel_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("panel-mock.sh", &standard_mock_script())
        .expect("failed to write panel mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends_stable failed");
    h.create_project(project_id, "Completion Panel Project", "Panel test prompt")
        .expect("create_project failed");
}

/// Two completers (claude, codex), both return COMPLETE, min_completers=2,
/// threshold=1.0 → consensus reached, project completes.
fn two_completer_consensus_complete(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-complete-1";
        setup_panel_mock(h, project_id);

        // Configure 2-completer panel with strict consensus
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\",\"codex\"]",
        ])
        .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_consensus_threshold",
            "1.0",
        ])
        .expect("set completion_consensus_threshold");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        // Verify completion attempt has panel verdict
        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert!(
            !attempts.is_empty(),
            "should have at least one completion attempt"
        );
        let attempt = &attempts[attempts.len() - 1];
        assert_eq!(
            attempt["verdict"].as_str().unwrap(),
            "complete",
            "panel should reach COMPLETE consensus"
        );

        // Verify completers list has 2 entries
        let completers = attempt["backends"]["completers"]
            .as_array()
            .expect("completers should be array");
        assert_eq!(completers.len(), 2, "should have 2 completers in panel");

        assert_no_uncommitted_ralph_files(&h.repo_root);
    })
}

/// Single completer (only claude configured), min_completers=1,
/// threshold=1.0 → falls back to single completer behavior, uses legacy
/// artifact name.
fn single_completer_backward_compat(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-single-1";
        setup_panel_mock(h, project_id);

        // Configure single-completer panel
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\"]",
        ])
        .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "1"])
            .expect("set completion_min_completers");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_consensus_threshold",
            "1.0",
        ])
        .expect("set completion_consensus_threshold");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert!(!attempts.is_empty());
        let attempt = &attempts[attempts.len() - 1];
        assert_eq!(attempt["verdict"].as_str().unwrap(), "complete");

        // Single completer uses legacy artifact name (completer-verdict.md)
        let verdict_path = attempt["artifacts"]["verdict"]
            .as_str()
            .expect("verdict artifact should exist");
        assert!(
            verdict_path.contains("completer-verdict"),
            "verdict artifact should contain completer-verdict: {verdict_path}"
        );

        assert_no_uncommitted_ralph_files(&h.repo_root);
    })
}

/// Two completers, one returns CONTINUE, one returns COMPLETE,
/// with threshold=1.0 → consensus NOT reached, project continues.
fn panel_continue_verdict(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-continue-1";
        h.init_workspace().expect("init failed");

        // Create separate scripts: claude returns COMPLETE, codex returns CONTINUE.
        // Use counter files so planner returns a completion request on the first
        // call, then a feature spec on subsequent calls (preventing infinite loops
        // when the completion verdict is CONTINUE).
        let claude_counter = h.temp_dir.path().join("claude-panel-counter");
        let codex_counter = h.temp_dir.path().join("codex-panel-counter");
        let claude_script = write_wrapped_mock(
            h,
            "claude-panel.sh",
            &complete_mock_script_with_counter("COMPLETE", &claude_counter),
        );
        let codex_script = write_wrapped_mock(
            h,
            "codex-panel.sh",
            &complete_mock_script_with_counter("CONTINUE", &codex_counter),
        );
        h.setup_separate_mock_backends(&claude_script, &codex_script)
            .expect("setup_separate_mock_backends failed");

        h.create_project(
            project_id,
            "Panel Continue Project",
            "Panel continue prompt",
        )
        .expect("create_project failed");

        // Configure 2-completer panel with strict consensus
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\",\"codex\"]",
        ])
        .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_consensus_threshold",
            "1.0",
        ])
        .expect("set completion_consensus_threshold");

        // Run 2 loops: first triggers completion check (CONTINUE due to 1/2 < threshold),
        // second does a normal feature loop to satisfy --loops.
        let output = h
            .ralph(["run", "--loops", "2"])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        // With consensus failure, the completion attempt records CONTINUE
        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        if !attempts.is_empty() {
            let attempt = &attempts[attempts.len() - 1];
            assert_eq!(
                attempt["verdict"].as_str().unwrap(),
                "continue",
                "panel should not reach consensus with 1/2 COMPLETE votes at threshold=1.0"
            );
        }
    })
}

/// With 2 completers, verify per-backend verdict artifact files are created
/// with the expected naming pattern.
fn per_backend_verdict_artifacts(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-artifacts-1";
        setup_panel_mock(h, project_id);

        // Configure 2-completer panel
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\",\"codex\"]",
        ])
        .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_consensus_threshold",
            "1.0",
        ])
        .expect("set completion_consensus_threshold");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        // Find the completion loop directory and check for per-backend verdict files
        let project_dir = h.project_dir(project_id);
        let loops_dir = project_dir.join("loops");
        assert!(loops_dir.exists(), "loops directory should exist");

        // Find the completion loop directory
        let mut found_panel_verdicts = false;
        if let Ok(entries) = std::fs::read_dir(&loops_dir) {
            for entry in entries.flatten() {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                if dir_name.contains("completion") {
                    // Check for per-backend verdict artifacts
                    if let Ok(files) = std::fs::read_dir(entry.path()) {
                        let verdict_files: Vec<String> = files
                            .flatten()
                            .filter_map(|f| {
                                let name = f.file_name().to_string_lossy().to_string();
                                if name.contains("completer-verdict-") {
                                    Some(name)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if verdict_files.len() == 2 {
                            found_panel_verdicts = true;
                        }
                    }
                }
            }
        }

        assert!(
            found_panel_verdicts,
            "should find 2 per-backend verdict artifacts in completion loop directory"
        );
    })
}

/// Optional backend (`?gemini`) is unavailable → skipped with warning,
/// remaining completers satisfy min_completers and consensus proceeds.
fn optional_backend_skip(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-optional-skip-1";
        // setup_panel_mock uses setup_mock_backends_stable which already sets
        // backends.gemini.enabled = false at global scope.
        setup_panel_mock(h, project_id);

        // Configure panel with an optional backend that will not be available.
        // `?gemini` is optional; gemini is already disabled by setup_panel_mock.
        // Claude and codex are mocked and available.
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\",\"codex\",\"?gemini\"]",
        ])
        .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_consensus_threshold",
            "1.0",
        ])
        .expect("set completion_consensus_threshold");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        // Verify completion attempt succeeded with 2 effective completers
        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert!(
            !attempts.is_empty(),
            "should have at least one completion attempt"
        );
        let attempt = &attempts[attempts.len() - 1];
        assert_eq!(
            attempt["verdict"].as_str().unwrap(),
            "complete",
            "panel should reach COMPLETE despite optional gemini being skipped"
        );

        // Verify only 2 completers were effective (gemini skipped)
        let completers = attempt["backends"]["completers"]
            .as_array()
            .expect("completers should be array");
        assert_eq!(
            completers.len(),
            2,
            "should have 2 effective completers (gemini skipped)"
        );
    })
}

/// Required backend that is unavailable → run fails with error.
fn required_backend_failure(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-required-fail-1";
        // setup_panel_mock uses setup_mock_backends_stable which already sets
        // backends.gemini.enabled = false at global scope.
        setup_panel_mock(h, project_id);

        // Configure panel with a required (non-optional) backend that is disabled.
        // `gemini` (without ?) is required; gemini is already disabled by
        // setup_panel_mock, so the run should fail when resolving the panel.
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\",\"gemini\"]",
        ])
        .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_consensus_threshold",
            "1.0",
        ])
        .expect("set completion_consensus_threshold");

        // The run should fail because a required backend (gemini) is unavailable
        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");

        // The run should exit with non-zero status due to required backend failure
        assert!(
            output.status.code().unwrap_or(0) != 0
                || String::from_utf8_lossy(&output.stderr)
                    .to_lowercase()
                    .contains("unavailable"),
            "run should fail or report error when required completion backend is unavailable"
        );
    })
}

/// Two completers, one COMPLETE one CONTINUE, with threshold=0.5 →
/// consensus IS reached (1/2 >= 0.5 and 1 >= min_completers=1).
fn partial_threshold_consensus(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-partial-thresh-1";
        h.init_workspace().expect("init failed");

        // claude returns COMPLETE, codex returns CONTINUE.
        let claude_script =
            write_wrapped_mock(h, "claude-partial.sh", &complete_mock_script("COMPLETE"));
        let codex_script =
            write_wrapped_mock(h, "codex-partial.sh", &complete_mock_script("CONTINUE"));
        h.setup_separate_mock_backends(&claude_script, &codex_script)
            .expect("setup_separate_mock_backends failed");

        h.create_project(
            project_id,
            "Partial Threshold Project",
            "Partial threshold prompt",
        )
        .expect("create_project failed");

        // Configure panel: threshold=0.5 means 1/2 COMPLETE votes suffice,
        // and min_completers=1 so a single COMPLETE vote meets the minimum.
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\",\"codex\"]",
        ])
        .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "1"])
            .expect("set completion_min_completers");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_consensus_threshold",
            "0.5",
        ])
        .expect("set completion_consensus_threshold");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        // With threshold=0.5, 1/2 COMPLETE votes should reach consensus
        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert!(!attempts.is_empty(), "should have completion attempts");
        let attempt = &attempts[attempts.len() - 1];
        assert_eq!(
            attempt["verdict"].as_str().unwrap(),
            "complete",
            "panel should reach COMPLETE with 1/2 votes at threshold=0.5"
        );
    })
}

/// Two completers, one COMPLETE one CONTINUE, min_completers=2, threshold=0.5 →
/// consensus NOT reached because complete_votes (1) < min_completers (2).
fn insufficient_min_completers(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-min-fail-1";
        h.init_workspace().expect("init failed");

        // claude returns COMPLETE, codex returns CONTINUE.
        // Use counter files so planner returns completion request only on
        // the first call, then a feature spec (avoiding infinite loop).
        let claude_counter = h.temp_dir.path().join("claude-minfail-counter");
        let codex_counter = h.temp_dir.path().join("codex-minfail-counter");
        let claude_script = write_wrapped_mock(
            h,
            "claude-minfail.sh",
            &complete_mock_script_with_counter("COMPLETE", &claude_counter),
        );
        let codex_script = write_wrapped_mock(
            h,
            "codex-minfail.sh",
            &complete_mock_script_with_counter("CONTINUE", &codex_counter),
        );
        h.setup_separate_mock_backends(&claude_script, &codex_script)
            .expect("setup_separate_mock_backends failed");

        h.create_project(project_id, "Min Fail Project", "Min completer fail prompt")
            .expect("create_project failed");

        // threshold=0.5 would allow 1/2, but min_completers=2 requires 2 COMPLETE votes
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\",\"codex\"]",
        ])
        .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_consensus_threshold",
            "0.5",
        ])
        .expect("set completion_consensus_threshold");

        // Run 2 loops: first triggers completion check (CONTINUE because
        // 1 COMPLETE < min_completers=2), second does a regular feature loop.
        let output = h
            .ralph(["run", "--loops", "2"])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        if !attempts.is_empty() {
            let attempt = &attempts[attempts.len() - 1];
            assert_eq!(
                attempt["verdict"].as_str().unwrap(),
                "continue",
                "panel should NOT reach consensus: 1 COMPLETE < min_completers=2 even with threshold=0.5"
            );
        }
    })
}
