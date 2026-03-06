use super::*;

use crate::validate::assertions::{
    assert_exit_code, assert_json_field, parse_yaml_frontmatter, strip_ansi,
};
use crate::validate::harness::RalphHarness;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Command;

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
        ConformanceTest {
            name: "resume_backend_resolution::completion_planner_drift_on_resume",
            func: completion_planner_drift_on_resume,
        },
        ConformanceTest {
            name: "resume_backend_resolution::completion_completer_panel_drift_on_resume",
            func: completion_completer_panel_drift_on_resume,
        },
        ConformanceTest {
            name: "resume_backend_resolution::final_review_planner_drift_on_resume",
            func: final_review_planner_drift_on_resume,
        },
        ConformanceTest {
            name: "resume_backend_resolution::same_run_completion_no_panel_reresolution",
            func: same_run_completion_no_panel_reresolution,
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
            .ralph_with_log(["run", "--loops", "1"], "warn")
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = strip_ansi(&String::from_utf8_lossy(&resumed.stderr));
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
            .ralph_with_log(["run", "--loops", "1"], "warn")
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = strip_ansi(&String::from_utf8_lossy(&resumed.stderr));
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
            .ralph_with_log(["run", "--loops", "1"], "warn")
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = strip_ansi(&String::from_utf8_lossy(&resumed.stderr));
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
            .ralph_with_log(["run", "--loops", "1"], "warn")
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = strip_ansi(&String::from_utf8_lossy(&resumed.stderr));
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

/// Build a direct fixture that parks state in the `Completing` phase.
///
/// Completer command failures are non-fatal (skipped votes) in the orchestrator,
/// so we cannot rely on them to crash the process mid-phase. Instead, this
/// function manually creates the completion loop directory with a
/// termination-request artifact and a git checkpoint commit that positions the
/// reconstructed state at `completing`.
///
/// If `verdict_backend` is `Some((backend, verdict))`, a per-backend completer
/// verdict artifact is also written so that `reconstructed_completers` is
/// non-empty (required for panel drift detection).
fn create_completing_fixture(
    h: &RalphHarness,
    project_id: &str,
    planner_backend: &str,
    verdict_backend: Option<(&str, &str)>,
) {
    let project_dir = h.project_dir(project_id);
    let completion_dir = project_dir.join("loops").join("002-completion");
    fs::create_dir_all(&completion_dir).expect("create completion loop directory");

    // Write termination-request artifact with frontmatter matching what
    // the orchestrator would produce.
    let termination_content = format!(
        "---\n\
         artifact: termination-request\n\
         loop: 2\n\
         project: {project_id}\n\
         backend: {planner_backend}\n\
         role: planner\n\
         created_at: 2026-03-05T00:00:00Z\n\
         ---\n\n\
         # Project Completion Request\n\n\
         ## Rationale\n\
         All required behavior is complete.\n\n\
         ## Summary of Work\n\
         - Prior loops implemented and reviewed successfully.\n\n\
         ## Remaining Items\n\
         - None\n"
    );
    fs::write(
        completion_dir.join("20260305000000-termination-request.md"),
        &termination_content,
    )
    .expect("write termination-request artifact");

    // Optionally write a partial completer verdict artifact so that
    // reconstructed_completers is non-empty (enabling the panel drift
    // detection path). The verdict body is intentionally left without a
    // parseable `# Verdict:` heading so that `parse_completion_verdict`
    // returns `None`, keeping the completion attempt status as InProgress.
    // An InProgress status is required because `has_in_progress_loop()`
    // would return false for a Completed attempt, resetting the phase to
    // Planning before the Completing branch is ever reached.
    if let Some((backend, _verdict)) = verdict_backend {
        let verdict_content = format!(
            "---\n\
             artifact: completer-verdict\n\
             loop: 2\n\
             project: {project_id}\n\
             backend: {backend}\n\
             role: completer\n\
             created_at: 2026-03-05T00:00:01Z\n\
             ---\n\n\
             Completer execution was interrupted before producing a verdict.\n"
        );
        fs::write(
            completion_dir.join(format!("20260305000001-completer-verdict-{backend}.md")),
            &verdict_content,
        )
        .expect("write partial completer-verdict artifact");
    }

    // Stage all new files and create a ralph checkpoint commit so that
    // derive_position returns (2, Phase::Completing).
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&h.repo_root)
            .output()
            .unwrap_or_else(|e| panic!("git {:?} failed to spawn: {e}", args));
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["add", "-A"]);
    let commit_msg = format!(
        "ralph({project_id}): loop 2 planning -> completing\n\n\
         Ralph-Project: {project_id}\n\
         Ralph-Loop: 2\n\
         Ralph-Phase: completing"
    );
    git(&["commit", "-m", &commit_msg]);
}

fn backend_from_frontmatter(path: &Path) -> String {
    parse_yaml_frontmatter(path)["backend"]
        .as_str()
        .unwrap_or_else(|| panic!("missing backend frontmatter at {}", path.display()))
        .to_owned()
}

/// Find the completion loop directory (e.g. `loops/002-completion`) inside a
/// project directory.  Panics if none exists.
fn find_completion_loop_dir(project_dir: &Path) -> std::path::PathBuf {
    let loops_dir = project_dir.join("loops");
    let mut candidates: Vec<_> = fs::read_dir(&loops_dir)
        .unwrap_or_else(|e| panic!("failed to read loops dir {}: {e}", loops_dir.display()))
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains("completion") {
                Some(entry.path())
            } else {
                None
            }
        })
        .collect();
    candidates.sort();
    candidates.last().cloned().unwrap_or_else(|| {
        panic!(
            "no completion loop directory found under {}",
            loops_dir.display()
        )
    })
}

/// Find the newest `completer-verdict-*.md` file in a loop directory by
/// timestamp prefix, falling back to `completer-verdict.md` if no per-backend
/// verdicts exist.  Panics if no verdict artifact is found.
fn find_newest_verdict_artifact(loop_dir: &Path) -> std::path::PathBuf {
    let mut verdicts: Vec<_> = fs::read_dir(loop_dir)
        .unwrap_or_else(|e| panic!("failed to read loop dir {}: {e}", loop_dir.display()))
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains("completer-verdict") && name.ends_with(".md") {
                Some(entry.path())
            } else {
                None
            }
        })
        .collect();
    verdicts.sort();
    verdicts.last().cloned().unwrap_or_else(|| {
        panic!(
            "no completer-verdict artifact found in {}",
            loop_dir.display()
        )
    })
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

/// Mock script for completion/final-review resume tests.
///
/// The planner uses a counter file to first emit a feature spec (loop 1),
/// then a Project Completion Request (loop 2 / second invocation).
/// `fail_role` can be "completer" or "final_reviewer" to simulate a crash
/// during that phase. If `fail_role` is "none", nothing fails.
fn completion_resume_mock_script(fail_role: &str, fail_marker: &Path) -> String {
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

# Use a per-repo counter file so the planner alternates between feature/completion
PLANNER_COUNTER=".ralph-planner-counter"

if grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- none

## Refined Prompt
No changes.
EOF
elif grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  COUNT=0
  if [ -f "$PLANNER_COUNTER" ]; then
    COUNT="$(cat "$PLANNER_COUNTER")"
  fi
  COUNT=$((COUNT + 1))
  echo "$COUNT" > "$PLANNER_COUNTER"
  if [ "$COUNT" -le 1 ]; then
    cat <<'EOF'
# Feature: Completion Resume Drift Feature

## Description
A feature used to validate completion/final-review backend re-resolution on resume.

## Acceptance Criteria
- [ ] Mock implementation file is created

## Files to Modify/Create
- `mock_file.txt` - written by implementer

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
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
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
- Implemented mock feature for completion resume tests.

## Spec Deviations
- None

## Testing
- Mock script execution only.
EOF
  fi
  echo "implemented" > mock_file.txt
  git add mock_file.txt
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
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: approve completion resume test feature
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if should_fail "completer"; then
    echo "forced completer failure for resume test" >&2
    exit 1
  fi
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Mock requirement: satisfied
EOF
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  if should_fail "final_reviewer"; then
    echo "forced final reviewer failure for resume test" >&2
    exit 1
  fi
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

/// Mock script for the final-review amendment resume test.
///
/// Identical to `completion_resume_mock_script` except the final reviewer
/// uses a counter file:
///  - first successful call → AMENDMENTS (forces planner re-execution)
///  - second successful call → NO AMENDMENTS (allows completion)
///
/// The planner counter is also extended:
///  - count 1 → feature spec (loop 1)
///  - count 2 → completion request (loop 2)
///  - count 3 → amendment feature spec (loop 3, triggered by AMENDMENTS)
///  - count ≥ 4 → completion request (loop 4+)
fn final_review_amendment_resume_mock_script(fail_marker: &Path) -> String {
    let fail_marker = shell_single_quote(&fail_marker.to_string_lossy());
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"
FAIL_MARKER={fail_marker}

# Use per-repo counter files for planner and final reviewer
PLANNER_COUNTER=".ralph-planner-counter"
FINAL_REVIEWER_COUNTER=".ralph-final-reviewer-counter"

if grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- none

## Refined Prompt
No changes.
EOF
elif grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  COUNT=0
  if [ -f "$PLANNER_COUNTER" ]; then
    COUNT="$(cat "$PLANNER_COUNTER")"
  fi
  COUNT=$((COUNT + 1))
  echo "$COUNT" > "$PLANNER_COUNTER"
  if [ "$COUNT" -eq 1 ] || [ "$COUNT" -eq 3 ]; then
    cat <<'EOF'
# Feature: Final Review Amendment Feature

## Description
A feature used to validate planner re-execution on final review amendments.

## Acceptance Criteria
- [ ] Mock implementation file is created

## Files to Modify/Create
- `mock_file.txt` - written by implementer

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
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
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
- Implemented mock feature for final review amendment resume tests.

## Spec Deviations
- None

## Testing
- Mock script execution only.
EOF
  fi
  echo "implemented" > mock_file.txt
  git add mock_file.txt
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
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: approve final review amendment test feature
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Mock requirement: satisfied
EOF
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  if [ -f "$FAIL_MARKER" ]; then
    echo "forced final reviewer failure for resume test" >&2
    exit 1
  fi
  COUNT=0
  if [ -f "$FINAL_REVIEWER_COUNTER" ]; then
    COUNT="$(cat "$FINAL_REVIEWER_COUNTER")"
  fi
  COUNT=$((COUNT + 1))
  echo "$COUNT" > "$FINAL_REVIEWER_COUNTER"
  if [ "$COUNT" -le 1 ]; then
    cat <<'EOF'
# Final Review: AMENDMENTS

## Amendment: DOC-001

### Problem
mock_file.txt lacks documentation comments.

### Proposed Change
Add a documentation header to mock_file.txt.

### Affected Files
- `mock_file.txt`
EOF
  else
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No amendments required.
EOF
  fi
elif grep -q "You are a technical evaluator assessing proposed amendments from final reviewers." <<< "$INPUT"; then
  cat <<'EOF'
# Planner Positions

## Amendment: DOC-001

### Position
ACCEPT

### Rationale
The amendment identifies a genuine documentation gap in mock_file.txt.
EOF
elif grep -q "You are a reviewer voting on proposed amendments after considering the planner's positions." <<< "$INPUT"; then
  cat <<'EOF'
# Vote Results

## Amendment: DOC-001

### Vote
ACCEPT

### Rationale
Agreed with planner assessment; documentation should be added.
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

fn completion_planner_drift_on_resume(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "resume-completion-planner-drift";
        // Use "none" as fail_role so the first feature loop succeeds normally.
        setup_resume_fixture(
            h,
            project_id,
            "resume-completion-planner-drift.sh",
            &completion_resume_mock_script("none", Path::new("/nonexistent")),
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable QA");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
            .expect("disable final review");
        h.ralph_ok(["config", "set", "workflow.starting_backend", "claude"])
            .expect("set starting_backend to claude");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\"]",
        ])
        .expect("set single completer");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "1"])
            .expect("set min completers to 1");

        // Complete one feature loop only (planner count=1 emits feature spec).
        let first = h
            .ralph(["run", "--loops", "1"])
            .expect("initial feature loop run should execute");
        assert_exit_code(&first, 0);

        // Build a direct fixture in Completing: create the completion loop
        // directory with a termination-request artifact and a git checkpoint
        // commit. Completer command failures are non-fatal (skipped votes),
        // so we cannot rely on them to park state at Completing.
        create_completing_fixture(h, project_id, "claude", None);

        let parked_state = h
            .load_state(project_id)
            .expect("load_state after completing fixture");
        assert_json_field(&parked_state, "current_phase", &json!("completing"));

        // Change starting_backend to alter planner resolution for the completion loop
        h.ralph_ok(["config", "set", "workflow.starting_backend", "codex"])
            .expect("set starting_backend to codex");

        let resumed = h
            .ralph_with_log(["run", "--until-complete"], "warn")
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = strip_ansi(&String::from_utf8_lossy(&resumed.stderr));
        // Find the specific planner drift warning line and verify concrete fields.
        let planner_drift_line = resumed_stderr
            .lines()
            .find(|l| {
                l.contains("role=\"planner\"")
                    && l.contains("backend drift detected on resume, using config-resolved value")
            })
            .expect("expected a single log line with planner drift warning");
        assert!(
            planner_drift_line.contains("original=claude"),
            "expected original=claude (bare fixture value), got: {planner_drift_line}"
        );
        // The resolved planner is planner_for_loop(2, "codex") = opposite("codex")
        // = "claude", but with default model injection → "claude(opus)" (or similar).
        // Assert resolved starts with "claude(" to verify model injection occurred.
        assert!(
            planner_drift_line.contains("resolved=claude("),
            "expected resolved planner to include model suffix, got: {planner_drift_line}"
        );
    })
}

fn completion_completer_panel_drift_on_resume(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "resume-completion-panel-drift";
        // Use "none" as fail_role so the first feature loop succeeds normally.
        setup_resume_fixture(
            h,
            project_id,
            "resume-completion-panel-drift.sh",
            &completion_resume_mock_script("none", Path::new("/nonexistent")),
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable QA");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
            .expect("disable final review");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\"]",
        ])
        .expect("set initial completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "1"])
            .expect("set min completers to 1");

        // Complete one feature loop only (planner count=1 emits feature spec).
        let first = h
            .ralph(["run", "--loops", "1"])
            .expect("initial feature loop run should execute");
        assert_exit_code(&first, 0);

        // Build a direct fixture in Completing with a prior completer verdict
        // so that reconstructed_completers is non-empty (needed for panel
        // drift detection). Completer command failures are non-fatal (skipped
        // votes), so we cannot rely on them to park state at Completing.
        create_completing_fixture(h, project_id, "claude", Some(("claude", "CONTINUE")));

        let parked_state = h
            .load_state(project_id)
            .expect("load_state after completing fixture");
        assert_json_field(&parked_state, "current_phase", &json!("completing"));

        // Change completion_backends to trigger completer panel drift
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"codex\"]",
        ])
        .expect("set new completion_backends");

        let resumed = h
            .ralph_with_log(["run", "--until-complete"], "warn")
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = strip_ansi(&String::from_utf8_lossy(&resumed.stderr));
        // Find the specific completer drift warning line.
        let completer_drift_line = resumed_stderr
            .lines()
            .find(|l| {
                l.contains("role=\"completer\"")
                    && l.contains("backend drift detected on resume, using config-resolved value")
            })
            .expect("expected a single log line with completer panel drift warning");
        assert!(
            completer_drift_line.contains("loop_number="),
            "expected loop_number field in completer panel drift warning, got: {completer_drift_line}"
        );
        assert!(
            completer_drift_line.contains("original="),
            "expected original field in completer panel drift warning, got: {completer_drift_line}"
        );
        assert!(
            completer_drift_line.contains("resolved="),
            "expected resolved field in completer panel drift warning, got: {completer_drift_line}"
        );

        // Execution proof: discover the newest completer-verdict artifact by
        // filename pattern in the completion loop directory, rather than relying
        // on reconstructed state pointers which may prefer stale per-backend
        // verdict artifacts.
        let project_dir = h.project_dir(project_id);
        let completion_loop_dir = find_completion_loop_dir(&project_dir);
        let newest_verdict = find_newest_verdict_artifact(&completion_loop_dir);
        let verdict_backend = backend_from_frontmatter(&newest_verdict);
        assert!(
            verdict_backend.starts_with("codex"),
            "completer verdict artifact backend should match re-resolved panel (codex), got: {verdict_backend}"
        );
    })
}

fn final_review_planner_drift_on_resume(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "resume-finalreview-planner-drift";
        let fail_marker = h.temp_dir.path().join("fail-final-reviewer.marker");
        fs::write(&fail_marker, "1").expect("write final reviewer fail marker");
        setup_resume_fixture(
            h,
            project_id,
            "resume-finalreview-planner-drift.sh",
            &final_review_amendment_resume_mock_script(&fail_marker),
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable QA");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review");
        h.ralph_ok(["config", "set", "workflow.starting_backend", "claude"])
            .expect("set starting_backend to claude");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\"]",
        ])
        .expect("set single completer");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "1"])
            .expect("set min completers to 1");

        // Use --until-complete so execution continues past the feature loop
        // count and reaches FinalReview.
        let first = h
            .ralph(["run", "--until-complete"])
            .expect("initial run should execute");
        assert!(
            !first.status.success(),
            "initial run should fail in final review phase; stderr:\n{}",
            String::from_utf8_lossy(&first.stderr)
        );

        let failed_state = h
            .load_state(project_id)
            .expect("load_state after final review failure");
        assert_json_field(&failed_state, "current_phase", &json!("final_review"));

        // Change starting_backend to alter planner resolution for the completion loop
        h.ralph_ok(["config", "set", "workflow.starting_backend", "codex"])
            .expect("set starting_backend to codex");
        fs::remove_file(&fail_marker).expect("remove final reviewer fail marker");

        // On resume, final reviewer returns AMENDMENTS which forces the planner
        // to execute (planning an amendment feature). The planner-produced
        // artifact should have the re-resolved backend in its frontmatter.
        let resumed = h
            .ralph_with_log(["run", "--until-complete"], "warn")
            .expect("resumed run should execute");
        assert_exit_code(&resumed, 0);

        let resumed_stderr = strip_ansi(&String::from_utf8_lossy(&resumed.stderr));
        // Find the specific planner drift warning line.
        let planner_drift_line = resumed_stderr
            .lines()
            .find(|l| {
                l.contains("role=\"planner\"")
                    && l.contains("backend drift detected on resume, using config-resolved value")
            })
            .expect("expected planner drift warning on final review resume");
        assert!(
            planner_drift_line.contains("original="),
            "expected original field in planner drift warning, got: {planner_drift_line}"
        );

        // Execution proof: the amendment feature spec artifact produced by the
        // planner on resume should have a backend field matching the re-resolved
        // planner backend (codex-based, since starting_backend was changed to codex).
        let state = h
            .load_state(project_id)
            .expect("load_state after final review amendment resume");
        let loops = state["loops"].as_array().expect("loops should be an array");
        // The amendment feature loop is the last one added after final review
        // triggered re-planning. Find a loop with loop_number > 2 (the original
        // feature loop was 1, completion was 2).
        let amendment_loop = loops
            .iter()
            .rev()
            .find(|l| l["loop_number"].as_u64().map(|n| n > 2).unwrap_or(false))
            .expect("should have an amendment feature loop after final review AMENDMENTS");
        let spec_rel = amendment_loop["artifacts"]["spec"]
            .as_str()
            .expect("amendment loop should have a spec artifact");
        let spec_backend = backend_from_frontmatter(&h.project_dir(project_id).join(spec_rel));
        assert!(
            spec_backend.starts_with("codex"),
            "amendment planner spec artifact backend should be codex-based (re-resolved), got: {spec_backend}"
        );
    })
}

fn same_run_completion_no_panel_reresolution(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "resume-samerun-completion";
        setup_resume_fixture(
            h,
            project_id,
            "resume-samerun-completion.sh",
            &completion_resume_mock_script("none", Path::new("/nonexistent")),
        );

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable QA");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "false"])
            .expect("disable final review");
        // Include an unavailable optional completer (?openrouter) alongside the
        // required completer. OpenRouter is disabled in the mock setup, so
        // resolve_completion_panel will skip it with a deterministic warning.
        // If the panel were re-resolved on same-run completion entry, the
        // skip warning would appear twice; asserting it appears exactly once
        // proves resolution happens only at planning time.
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\", \"?openrouter\"]",
        ])
        .expect("set completion_backends with optional unavailable backend");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "1"])
            .expect("set min completers to 1");

        // Run to completion in a single invocation (no resume).
        // is_resumed_state becomes false after the first outer-loop iteration,
        // so the Completing phase should NOT re-resolve the completer panel.
        let result = h
            .ralph_with_log(["run", "--until-complete"], "warn")
            .expect("single run should execute");
        assert_exit_code(&result, 0);

        // Verify that the completion phase was actually reached by checking
        // that the project completed (state.status == "completed").
        let state = h
            .load_state(project_id)
            .expect("load_state after same-run completion");
        assert_json_field(&state, "status", &json!("completed"));

        let stderr = strip_ansi(&String::from_utf8_lossy(&result.stderr));
        // No completer panel drift warning should appear because this is
        // a same-run entry (is_resumed_state == false).  RUST_LOG=warn
        // ensures this assertion is not vacuously true.
        assert!(
            !stderr.contains("role=\"completer\"")
                || !stderr
                    .contains("backend drift detected on resume, using config-resolved value"),
            "did not expect completer panel drift warning on same-run completion, stderr:\n{stderr}"
        );

        // The optional openrouter skip warning from resolve_completion_panel
        // should appear exactly once (at planning-time panel resolution),
        // proving that panel resolution does NOT happen again at same-run
        // completion entry.
        let skip_count = stderr
            .lines()
            .filter(|l| l.contains("optional completion backend unavailable, skipping"))
            .count();
        assert_eq!(
            skip_count, 1,
            "expected exactly 1 optional-backend skip warning (planning-time only), got {skip_count}; stderr:\n{stderr}"
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
