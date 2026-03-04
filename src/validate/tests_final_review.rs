use super::*;

use crate::validate::assertions::{assert_exit_code, assert_json_field, assert_stderr_contains};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::standard_mock_script;
use serde_json::json;
use std::fs;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "final_review::completion_no_amendments",
            func: completion_no_amendments,
        },
        ConformanceTest {
            name: "final_review::restart_round_then_complete",
            func: restart_round_then_complete,
        },
        ConformanceTest {
            name: "final_review::planner_completion_after_amendments_fails",
            func: planner_completion_after_amendments_fails,
        },
    ]
}

fn completion_no_amendments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "final-review-complete";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("final-review-standard.sh", &standard_mock_script())
            .expect("failed to write standard final-review script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Final Review Completion Project",
            "Final review completion prompt",
        )
        .expect("create_project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review");

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
        assert!(
            artifacts.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with("-final-review-exit-approved.md"))
            }),
            "expected final-review approved exit artifact in completion loop"
        );
    })
}

fn restart_round_then_complete(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "final-review-restart";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("final-review-restart.sh", &restart_then_complete_script())
            .expect("failed to write restart script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Final Review Restart Project",
            "Final review restart prompt",
        )
        .expect("create_project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review");
        h.ralph_ok(["config", "set", "workflow.max_final_review_restarts", "3"])
            .expect("set max final review restarts");

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));
        let completion_attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert_eq!(
            completion_attempts.len(),
            2,
            "expected restart flow to create a second completion attempt"
        );

        let amendments_path = h
            .project_dir(project_id)
            .join("final-review-amendments-applied.md");
        let amendments = fs::read_to_string(&amendments_path)
            .expect("final-review-amendments-applied.md should exist");
        assert!(amendments.contains("## Round 1"));
        assert!(amendments.contains("R1-1"));
    })
}

fn restart_then_complete_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"
counter_dir="${COUNTER_DIR:-.ralph/counters}"
mkdir -p "$counter_dir"

inc_counter() {
  local file="${counter_dir}/$1"
  local value=0
  if [ -f "$file" ]; then
    value=$(cat "$file")
  fi
  value=$((value + 1))
  echo "$value" > "$file"
  echo "$value"
}

collect_ids() {
  printf "%s\n" "$prompt" | sed -n 's/^## Amendment: //p'
}

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  planner_call=$(inc_counter "planner_calls")
  if [ "$planner_call" -eq 1 ] || [ "$planner_call" -ge 3 ]; then
    cat <<'EOF'
# Project Completion Request

## Rationale
Ready for completion.

## Summary of Work
- Baseline done.

## Remaining Items
- None
EOF
  else
    cat <<'EOF'
# Feature: Address Amendments

## Description
Implement changes required by final review amendments.

## Acceptance Criteria
- [ ] Amendments addressed

## Files to Modify/Create
- `mock_file.txt` - address amendments

## Dependencies
- Requires: none
- Blocks: none
EOF
  fi
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Requirement: satisfied
EOF
elif [[ "$prompt" == *"You are a QA engineer validating overall project acceptance."* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance passed

## Automated Tests
- acceptance passed

## Acceptance Criteria Verification
All good.
EOF
elif [[ "$prompt" == *"You are a final reviewer auditing a completed project for correctness, safety, and robustness."* ]]; then
  total=$(inc_counter "final_reviewer_total")
  round=$(( (total - 1) / 2 + 1 ))
  slot=$(( (total - 1) % 2 + 1 ))
  if [ "$round" -eq 1 ]; then
    cat <<EOF
# Final Review: AMENDMENTS

## Amendment: R1-${slot}

### Problem
Round-one issue ${slot}.

### Proposed Change
Apply round-one change ${slot}.

### Affected Files
- \`README.md\` - update
EOF
  else
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No additional amendments in round two.
EOF
  fi
elif [[ "$prompt" == *"You are a technical evaluator assessing proposed amendments from final reviewers."* ]]; then
  call=$(inc_counter "planner_position_calls")
  cat <<'EOF'
# Planner Positions
EOF
  if [ "$call" -eq 1 ]; then
    ids=("R1-1" "R1-2")
  else
    ids=()
  fi
  for id in "${ids[@]}"; do
    cat <<EOF

## Amendment: $id

### Position
ACCEPT

### Rationale
Planner accepts $id.
EOF
  done
elif [[ "$prompt" == *"You are a reviewer voting on proposed amendments after considering the planner's positions."* ]]; then
  call=$(inc_counter "vote_calls")
  round=$(( (call - 1) / 2 + 1 ))
  cat <<'EOF'
# Vote Results
EOF
  if [ "$round" -eq 1 ]; then
    ids=("R1-1" "R1-2")
  else
    ids=()
  fi
  for id in "${ids[@]}"; do
    cat <<EOF

## Amendment: $id

### Vote
ACCEPT

### Rationale
Vote accepts $id.
EOF
  done
elif [[ "$prompt" == *"You are a software developer implementing a feature specification."* ]]; then
  if echo "$prompt" | grep -q "## Review Feedback" && ! echo "$prompt" | grep -q "(none)"; then
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
- Addressed final review amendments.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  fi
  echo "amended" > mock_file.txt
  git add mock_file.txt
elif [[ "$prompt" == *"You are a code reviewer ensuring implementations match specifications."* ]]; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Amendments addressed

## Notes
Looks good.

## Commit Message
feat: address final review amendments
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- none

## Refined Prompt
No changes.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

fn planner_completion_after_amendments_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "final-review-guard";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "final-review-guard.sh",
                &always_completion_after_amendments_script(),
            )
            .expect("failed to write guard script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Final Review Guard Project",
            "Final review guard prompt",
        )
        .expect("create_project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review");
        h.ralph_ok(["config", "set", "workflow.max_final_review_restarts", "3"])
            .expect("set max final review restarts");

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 1);
        assert_stderr_contains(
            &output,
            "planner requested completion without addressing final review amendments",
        );

        let state = h.load_state(project_id).expect("load_state failed");
        let completion_attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert_eq!(
            completion_attempts.len(),
            1,
            "guard should fire before a second completion attempt is registered"
        );
    })
}

/// Script where the planner always returns CompletionRequest (even after amendments).
/// Final reviewer always returns AMENDMENTS on the first round.
fn always_completion_after_amendments_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"
counter_dir="${COUNTER_DIR:-.ralph/counters}"
mkdir -p "$counter_dir"

inc_counter() {
  local file="${counter_dir}/$1"
  local value=0
  if [ -f "$file" ]; then
    value=$(cat "$file")
  fi
  value=$((value + 1))
  echo "$value" > "$file"
  echo "$value"
}

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  cat <<'EOF'
# Project Completion Request

## Rationale
Ready for completion.

## Summary of Work
- Baseline done.

## Remaining Items
- None
EOF
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Requirement: satisfied
EOF
elif [[ "$prompt" == *"You are a QA engineer validating overall project acceptance."* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance passed

## Automated Tests
- acceptance passed

## Acceptance Criteria Verification
All good.
EOF
elif [[ "$prompt" == *"You are a final reviewer auditing a completed project for correctness, safety, and robustness."* ]]; then
  total=$(inc_counter "final_reviewer_total")
  slot=$(( (total - 1) % 2 + 1 ))
  cat <<EOF
# Final Review: AMENDMENTS

## Amendment: R1-${slot}

### Problem
Issue ${slot}.

### Proposed Change
Apply change ${slot}.

### Affected Files
- \`README.md\` - update
EOF
elif [[ "$prompt" == *"You are a technical evaluator assessing proposed amendments from final reviewers."* ]]; then
  cat <<'EOF'
# Planner Positions

## Amendment: R1-1

### Position
ACCEPT

### Rationale
Planner accepts R1-1.

## Amendment: R1-2

### Position
ACCEPT

### Rationale
Planner accepts R1-2.
EOF
elif [[ "$prompt" == *"You are a reviewer voting on proposed amendments after considering the planner's positions."* ]]; then
  cat <<'EOF'
# Vote Results

## Amendment: R1-1

### Vote
ACCEPT

### Rationale
Vote accepts R1-1.

## Amendment: R1-2

### Vote
ACCEPT

### Rationale
Vote accepts R1-2.
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- none

## Refined Prompt
No changes.
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
