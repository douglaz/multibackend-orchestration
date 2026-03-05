use super::*;

use crate::validate::assertions::{assert_exit_code, assert_json_field, parse_yaml_frontmatter};
use crate::validate::harness::RalphHarness;
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "resume_backend_resolution::implementing_uses_reresolved_backend",
            func: implementing_uses_reresolved_backend,
        },
        ConformanceTest {
            name: "resume_backend_resolution::qa_uses_reresolved_backend",
            func: qa_uses_reresolved_backend,
        },
        ConformanceTest {
            name: "resume_backend_resolution::reviewing_uses_reresolved_backend",
            func: reviewing_uses_reresolved_backend,
        },
        ConformanceTest {
            name: "resume_backend_resolution::no_drift_emits_no_warning",
            func: no_drift_emits_no_warning,
        },
    ]
}

fn implementing_uses_reresolved_backend(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "resume-backend-impl";
        let fail_marker = h.temp_dir.path().join("fail-implementer.marker");
        fs::write(&fail_marker, "1").expect("write implementer fail marker");
        setup_resume_fixture(
            h,
            project_id,
            "resume-backend-impl.sh",
            &resume_phase_mock_script("implementer", &fail_marker),
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable QA for implementer drift case");
        h.ralph_ok([
            "config",
            "set",
            "workflow.implementer_backend",
            "codex(gpt-5-old)",
        ])
        .expect("set old implementer backend");

        let first = h.ralph(["run"]).expect("initial run should execute");
        assert!(
            !first.status.success(),
            "initial run should fail in implementing phase; stderr:\n{}",
            String::from_utf8_lossy(&first.stderr)
        );

        let failed_state = h
            .load_state(project_id)
            .expect("load_state after implementing failure");
        assert_json_field(&failed_state, "current_phase", &json!("implementing"));

        h.ralph_ok([
            "config",
            "set",
            "workflow.implementer_backend",
            "codex(gpt-5-new)",
        ])
        .expect("set new implementer backend");
        fs::remove_file(&fail_marker).expect("remove implementer fail marker");

        let resumed = h
            .ralph(["run", "--loops", "1"])
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = String::from_utf8_lossy(&resumed.stderr);
        assert!(
            resumed_stderr
                .contains("backend drift detected on resume, using config-resolved value"),
            "expected drift warning on resume, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("role=\"implementer\""),
            "expected implementer role field in drift warning, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("loop_number=1"),
            "expected loop_number field in drift warning, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("original=")
                && resumed_stderr.contains("resolved=codex(gpt-5-new)"),
            "expected original/resolved backend specs in drift warning, stderr:\n{resumed_stderr}"
        );

        let state = h.load_state(project_id).expect("load_state after resume");
        let loops = state["loops"].as_array().expect("loops should be an array");
        let impl_notes_rel = loops[0]["artifacts"]["impl_notes"]
            .as_str()
            .expect("impl-notes artifact should exist");
        let impl_notes_backend =
            backend_from_frontmatter(&h.project_dir(project_id).join(impl_notes_rel));
        assert_eq!(
            impl_notes_backend, "codex(gpt-5-new)",
            "implementer execution should use re-resolved backend on resume"
        );
    })
}

fn qa_uses_reresolved_backend(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "resume-backend-qa";
        let fail_marker = h.temp_dir.path().join("fail-qa.marker");
        fs::write(&fail_marker, "1").expect("write QA fail marker");
        setup_resume_fixture(
            h,
            project_id,
            "resume-backend-qa.sh",
            &resume_phase_mock_script("qa", &fail_marker),
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("enable QA for qa drift case");
        h.ralph_ok(["config", "set", "workflow.qa_backend", "codex(gpt-5-old)"])
            .expect("set old QA backend");

        let first = h.ralph(["run"]).expect("initial run should execute");
        assert!(
            !first.status.success(),
            "initial run should fail in QA phase; stderr:\n{}",
            String::from_utf8_lossy(&first.stderr)
        );

        let failed_state = h
            .load_state(project_id)
            .expect("load_state after QA failure");
        assert_json_field(&failed_state, "current_phase", &json!("qa"));

        h.ralph_ok(["config", "set", "workflow.qa_backend", "codex(gpt-5-new)"])
            .expect("set new QA backend");
        fs::remove_file(&fail_marker).expect("remove QA fail marker");

        let resumed = h
            .ralph(["run", "--loops", "1"])
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = String::from_utf8_lossy(&resumed.stderr);
        assert!(
            resumed_stderr
                .contains("backend drift detected on resume, using config-resolved value"),
            "expected drift warning on resume, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("role=\"qa\""),
            "expected qa role field in drift warning, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("loop_number=1"),
            "expected loop_number field in drift warning, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("original=")
                && resumed_stderr.contains("resolved=codex(gpt-5-new)"),
            "expected original/resolved backend specs in drift warning, stderr:\n{resumed_stderr}"
        );

        let state = h.load_state(project_id).expect("load_state after resume");
        let loops = state["loops"].as_array().expect("loops should be an array");
        let qa_results = loops[0]["artifacts"]["qa_results"]
            .as_array()
            .expect("qa_results should be an array");
        assert!(
            !qa_results.is_empty(),
            "expected at least one qa report after resume"
        );
        let qa_report_rel = qa_results[0]["report"]
            .as_str()
            .expect("qa report artifact path should exist");
        let qa_backend = backend_from_frontmatter(&h.project_dir(project_id).join(qa_report_rel));
        assert_eq!(
            qa_backend, "codex(gpt-5-new)",
            "QA execution should use re-resolved backend on resume"
        );
    })
}

fn reviewing_uses_reresolved_backend(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "resume-backend-review";
        let fail_marker = h.temp_dir.path().join("fail-reviewer.marker");
        fs::write(&fail_marker, "1").expect("write reviewer fail marker");
        setup_resume_fixture(
            h,
            project_id,
            "resume-backend-review.sh",
            &resume_phase_mock_script("reviewer", &fail_marker),
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable QA for reviewer drift case");
        h.ralph_ok([
            "config",
            "set",
            "workflow.reviewer_backend",
            "codex(gpt-5-old)",
        ])
        .expect("set old reviewer backend");

        let first = h.ralph(["run"]).expect("initial run should execute");
        assert!(
            !first.status.success(),
            "initial run should fail in reviewing phase; stderr:\n{}",
            String::from_utf8_lossy(&first.stderr)
        );

        let failed_state = h
            .load_state(project_id)
            .expect("load_state after reviewing failure");
        assert_json_field(&failed_state, "current_phase", &json!("reviewing"));

        h.ralph_ok([
            "config",
            "set",
            "workflow.reviewer_backend",
            "codex(gpt-5-new)",
        ])
        .expect("set new reviewer backend");
        fs::remove_file(&fail_marker).expect("remove reviewer fail marker");

        let resumed = h
            .ralph(["run", "--loops", "1"])
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = String::from_utf8_lossy(&resumed.stderr);
        assert!(
            resumed_stderr
                .contains("backend drift detected on resume, using config-resolved value"),
            "expected drift warning on resume, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("role=\"reviewer\""),
            "expected reviewer role field in drift warning, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("loop_number=1"),
            "expected loop_number field in drift warning, stderr:\n{resumed_stderr}"
        );
        assert!(
            resumed_stderr.contains("original=")
                && resumed_stderr.contains("resolved=codex(gpt-5-new)"),
            "expected original/resolved backend specs in drift warning, stderr:\n{resumed_stderr}"
        );

        let state = h.load_state(project_id).expect("load_state after resume");
        let loops = state["loops"].as_array().expect("loops should be an array");
        let approval_rel = loops[0]["artifacts"]["approval"]
            .as_str()
            .expect("review approval artifact should exist");
        let reviewer_backend =
            backend_from_frontmatter(&h.project_dir(project_id).join(approval_rel));
        assert_eq!(
            reviewer_backend, "codex(gpt-5-new)",
            "reviewer execution should use re-resolved backend on resume"
        );
    })
}

fn no_drift_emits_no_warning(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "resume-backend-no-drift";
        let fail_marker = h.temp_dir.path().join("fail-no-drift.marker");
        let reviewer_counter = h.temp_dir.path().join("no-drift-reviewer.counter");
        fs::write(&fail_marker, "1").expect("write no-drift fail marker");
        setup_resume_fixture(
            h,
            project_id,
            "resume-backend-no-drift.sh",
            &review_feedback_then_fail_implementer_response_script(&fail_marker, &reviewer_counter),
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable QA for no-drift case");
        h.ralph_ok([
            "config",
            "set",
            "workflow.implementer_backend",
            "codex(gpt-5-stable)",
        ])
        .expect("set stable implementer backend");

        let first = h.ralph(["run"]).expect("initial run should execute");
        assert!(
            !first.status.success(),
            "initial run should fail in implementing response phase; stderr:\n{}",
            String::from_utf8_lossy(&first.stderr)
        );
        let failed_state = h
            .load_state(project_id)
            .expect("load_state after no-drift failure");
        assert_json_field(&failed_state, "current_phase", &json!("implementing"));

        fs::remove_file(&fail_marker).expect("remove no-drift fail marker");
        let resumed = h
            .ralph(["run", "--loops", "1"])
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = String::from_utf8_lossy(&resumed.stderr);
        assert!(
            !resumed_stderr
                .contains("backend drift detected on resume, using config-resolved value"),
            "did not expect drift warning when reconstructed and resolved specs match, stderr:\n{resumed_stderr}"
        );

        let state = h.load_state(project_id).expect("load_state after resume");
        let loops = state["loops"].as_array().expect("loops should be an array");
        let reviews = loops[0]["artifacts"]["reviews"]
            .as_array()
            .expect("reviews should be an array");
        assert!(
            !reviews.is_empty(),
            "expected at least one review exchange in no-drift case"
        );
        let response_rel = reviews[0]["response"]
            .as_str()
            .expect("impl-response artifact should exist");
        let backend = backend_from_frontmatter(&h.project_dir(project_id).join(response_rel));
        assert_eq!(
            backend, "codex(gpt-5-stable)",
            "backend selection should remain unchanged when no drift exists"
        );
    })
}

fn setup_resume_fixture(h: &RalphHarness, project_id: &str, script_name: &str, script: &str) {
    h.init_workspace().expect("init failed");
    let script_path = h
        .write_mock_script(script_name, script)
        .expect("write resume mock script");
    h.setup_mock_backends_stable(&script_path)
        .expect("setup mock backends");
    h.create_project(
        project_id,
        "Resume Backend Resolution Project",
        "Resume backend resolution prompt",
    )
    .expect("create project");
    h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
        .expect("disable prompt review");
}

fn backend_from_frontmatter(path: &Path) -> String {
    parse_yaml_frontmatter(path)["backend"]
        .as_str()
        .unwrap_or_else(|| panic!("missing backend frontmatter at {}", path.display()))
        .to_owned()
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn resume_phase_mock_script(fail_role: &str, fail_marker: &Path) -> String {
    let fail_role = shell_single_quote(fail_role);
    let fail_marker = shell_single_quote(&fail_marker.to_string_lossy());
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"
FAIL_ROLE={fail_role}
FAIL_MARKER={fail_marker}

should_fail() {{
  local role="$1"
  [[ "$FAIL_ROLE" == "$role" && -f "$FAIL_MARKER" ]]
}}

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  cat <<'EOF'
# Feature: Resume Drift Feature

## Description
A feature used to validate backend re-resolution on resume.

## Acceptance Criteria
- [ ] Mock implementation file is created

## Files to Modify/Create
- `mock_file.txt` - written by implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if should_fail "implementer"; then
    echo "forced implementer failure for resume test" >&2
    exit 1
  fi
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed reviewer feedback.

## Could Not Address
- None
EOF
  else
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Implemented mock feature for resume backend tests.

## Spec Deviations
- None

## Testing
- Mock script execution only.
EOF
  fi
  echo "implemented" > mock_file.txt
  git add mock_file.txt
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  if should_fail "qa"; then
    echo "forced qa failure for resume test" >&2
    exit 1
  fi
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check passed

## Automated Tests
- mock test suite passed

## Acceptance Criteria Verification
All acceptance criteria verified.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  if should_fail "reviewer"; then
    echo "forced reviewer failure for resume test" >&2
    exit 1
  fi
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: approve resume backend test feature
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Additional work remains.

## Recommended Next Features
1. Add another feature.
EOF
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- none

## Refined Prompt
No changes.
EOF
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No amendments required.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    )
}

fn review_feedback_then_fail_implementer_response_script(
    fail_marker: &Path,
    reviewer_counter: &Path,
) -> String {
    let fail_marker = shell_single_quote(&fail_marker.to_string_lossy());
    let reviewer_counter = shell_single_quote(&reviewer_counter.to_string_lossy());
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"
FAIL_MARKER={fail_marker}
REVIEWER_COUNTER={reviewer_counter}

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  cat <<'EOF'
# Feature: Resume No Drift Feature

## Description
A feature used to validate no-warning behavior when backend resolution does not drift.

## Acceptance Criteria
- [ ] Mock implementation file is created

## Files to Modify/Create
- `mock_file.txt` - written by implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
    if [ -f "$FAIL_MARKER" ]; then
      echo "forced implementer response failure for no-drift test" >&2
      exit 1
    fi
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed reviewer feedback.

## Could Not Address
- None
EOF
  else
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Implemented mock feature for no-drift resume test.

## Spec Deviations
- None

## Testing
- Mock script execution only.
EOF
  fi
  echo "implemented" > mock_file.txt
  git add mock_file.txt
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  count=0
  if [ -f "$REVIEWER_COUNTER" ]; then
    count="$(cat "$REVIEWER_COUNTER")"
  fi
  count=$((count + 1))
  echo "$count" > "$REVIEWER_COUNTER"
  if [ "$count" -eq 1 ]; then
    cat <<'EOF'
# Review: SUGGESTIONS

## Required Changes
1. Update mock_file.txt to satisfy reviewer.

## Acceptance Criteria Checklist
- [ ] Mock implementation file is created
EOF
  else
    cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: approve no-drift resume test feature
EOF
  fi
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check passed

## Automated Tests
- mock test suite passed

## Acceptance Criteria Verification
All acceptance criteria verified.
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Additional work remains.

## Recommended Next Features
1. Add another feature.
EOF
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- none

## Refined Prompt
No changes.
EOF
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No amendments required.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
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
