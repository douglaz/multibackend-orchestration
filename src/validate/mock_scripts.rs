use std::path::Path;

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// Shared PRD-stage response dispatcher for mock backends.
///
/// Expects an `INPUT` shell variable in scope containing full stdin.
pub fn prd_mock_response_body() -> String {
    r###"if grep -q "You are a product ideation specialist" <<< "$INPUT"; then
  cat <<'EOF'
## Core Concept
Clear idea framing for the proposed product.

## Target Users
- Internal team

## Key Problems Solved
- Reduced manual burden

## Proposed Features
- A focused set of starter features

## Success Metrics
- Faster task completion

## Constraints & Assumptions
- Basic implementation assumptions
EOF
elif grep -q "You are a technical research analyst" <<< "$INPUT"; then
  cat <<'EOF'
## Market Context
- Market overview

## Technical Landscape
- Existing approaches

## Comparable Solutions
- Baseline alternatives

## Technical Feasibility
- Feasible with current stack

## Risk Assessment
- Low risk scope
EOF
elif grep -q "You are a product strategist" <<< "$INPUT"; then
  cat <<'EOF'
## Product Vision
- Vision summary

## User Stories
- User story example

## Feature Prioritization
- P0: core features

## Architecture Overview
- High-level architecture notes

## MVP Scope
- Core scope only

## Open Questions
- None
EOF
elif grep -q "You are a technical product manager" <<< "$INPUT"; then
  cat <<'EOF'
## Executive Summary
- Summary

## Goals & Non-Goals
- Goals and boundaries

## User Stories
- User story

## Functional Requirements
- Feature requirements

## Non-Functional Requirements
- Performance and reliability

## Technical Architecture
- Architecture outline

## Data Model
- Data structures

## API Design
- API boundaries

## Security Considerations
- Secure auth

## Testing Strategy
- Unit and integration tests

## Rollout Plan
- Phased rollout

## Success Metrics
- Uptake and retention

## Open Questions
- None
EOF
elif grep -q "You are a requirements analyst" <<< "$INPUT"; then
  cat <<'EOF'
```json
{
  "missing_fields": [],
  "ambiguities": [],
  "questions": [],
  "suggested_defaults": []
}
```
EOF
elif grep -q "You are a PRD reviewer." <<< "$INPUT"; then
  cat <<'EOF'
```json
{
  "valid": true,
  "issues": []
}
```
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// POSIX backend script for `backend exec` tests; echoes stdin to stdout.
pub fn backend_exec_echo_script() -> String {
    r###"#!/bin/sh
set -eu
cat
"###
    .to_owned()
}

/// POSIX backend script that logs argv (one arg per line), consumes stdin, and exits 0.
pub fn openrouter_arg_logging_script(log_path: &Path) -> String {
    let quoted_log_path = shell_single_quote(&log_path.to_string_lossy());
    format!(
        r###"#!/bin/sh
set -eu

: > {quoted_log_path}
for arg in "$@"; do
  printf '%s\n' "$arg" >> {quoted_log_path}
done

cat >/dev/null
exit 0
"###
    )
}

/// Bash backend script that atomically increments a counter file and emits PRD output.
pub fn prd_invocation_counting_script(counter_path: &Path) -> String {
    let quoted_counter_path = shell_single_quote(&counter_path.to_string_lossy());
    let response_body = prd_mock_response_body();
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

COUNTER_PATH={quoted_counter_path}
LOCK_DIR="${{COUNTER_PATH}}.lock"

while ! mkdir "$LOCK_DIR" 2>/dev/null; do
  sleep 0.01
done
trap 'rmdir "$LOCK_DIR" 2>/dev/null || true' EXIT

count=0
if [ -f "$COUNTER_PATH" ]; then
  count="$(cat "$COUNTER_PATH" 2>/dev/null || echo 0)"
fi
case "$count" in
  ''|*[!0-9]*) count=0 ;;
esac
count=$((count + 1))
printf '%s\n' "$count" > "$COUNTER_PATH"

rmdir "$LOCK_DIR" 2>/dev/null || true
trap - EXIT

INPUT="$(cat)"
{response_body}
"###
    )
}

/// Bash backend script that captures stdin to unique files and emits PRD output.
pub fn prd_stdin_capturing_script(output_dir: &Path) -> String {
    let quoted_output_dir = shell_single_quote(&output_dir.to_string_lossy());
    let response_body = prd_mock_response_body();
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

OUTPUT_DIR={quoted_output_dir}
mkdir -p "$OUTPUT_DIR"

INPUT="$(cat)"
capture_file="$OUTPUT_DIR/stdin-$(date +%s%N)-$$.md"
suffix=0
while [ -e "$capture_file" ]; do
  suffix=$((suffix + 1))
  capture_file="$OUTPUT_DIR/stdin-$(date +%s%N)-$$-$suffix.md"
done
printf '%s' "$INPUT" > "$capture_file"

{response_body}
"###
    )
}

/// Bash mock script that mutates prompt.md once during planner invocation
/// and otherwise behaves like `standard_mock_script`.
pub fn prompt_mutating_mock_script(prompt_path: &Path) -> String {
    let quoted_prompt_path = shell_single_quote(&prompt_path.to_string_lossy());
    let sentinel_path = format!("{}.mutated-once", prompt_path.to_string_lossy());
    let quoted_sentinel_path = shell_single_quote(&sentinel_path);
    let mut script = standard_mock_script();
    let injection = format!(
        r###"INPUT="$(cat)"

PROMPT_PATH={quoted_prompt_path}
SENTINEL_PATH={quoted_sentinel_path}
planner_invocation=0
for arg in "$@"; do
  case "$arg" in
    *planner*) planner_invocation=1 ;;
  esac
done
if printf '%s' "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  planner_invocation=1
fi
if [ "$planner_invocation" -eq 1 ] && [ ! -f "$SENTINEL_PATH" ]; then
  printf '\n<!-- prompt mutated by validate mock -->\n' >> "$PROMPT_PATH"
  : > "$SENTINEL_PATH"
fi
"###
    );

    script = script.replacen("INPUT=\"$(cat)\"\n", &injection, 1);
    script
}

pub fn standard_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
The project is complete and requires no further amendments.
EOF
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Variant of `standard_mock_script` that also creates stray impl artifact
/// files at the worktree root during the implementation phase.  Used by
/// conformance tests to verify stray cleanup in the regular orchestrator.
pub fn standard_mock_with_stray_files_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed reviewer feedback in the mock implementation.

## Could Not Address
- None
EOF
    # Create stray files on review-response iteration too
    echo "stray response" > 20260304130000-impl-response-002.md
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
  # Create stray impl artifacts at worktree root
  echo "stray notes" > 20260304120000-impl-notes.md
  echo "stray response" > 20260304120000-impl-response-001.md
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
The project is complete and requires no further amendments.
EOF
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Auto-capable mock script that handles quick-PRD writer/reviewer prompts in
/// addition to the standard orchestration prompts, enabling `ralph auto`
/// conformance tests.
pub fn auto_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

# --- Quick-PRD prompts ---
if grep -q "You are a senior software engineer writing a focused engineering specification." <<< "$INPUT"; then
  cat <<'EOF'
## Summary
Auto-generated mock feature spec.

## Acceptance Criteria
- [ ] Mock file is created

## Technical Approach
Create a mock file.

## Files & Modules
- `mock_file.txt`

## Testing Strategy
Manual verification.

## Out of Scope
Nothing.
EOF
elif grep -q "You are a senior engineer reviewing an engineering specification" <<< "$INPUT"; then
  cat <<'EOF'
```json
{"approved": true, "issues": []}
```
EOF
elif grep -q "You are a senior software engineer revising an engineering specification" <<< "$INPUT"; then
  cat <<'EOF'
## Summary
Revised mock spec.

## Acceptance Criteria
- [ ] Mock file is created

## Technical Approach
Create a mock file.

## Files & Modules
- `mock_file.txt`

## Testing Strategy
Manual verification.

## Out of Scope
Nothing.
EOF
# --- Standard orchestration prompts ---
elif grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" && "${RALPH_E2E_FORCE_FEATURE:-no}" != "yes" ]]; then
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
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
The project is complete and requires no further amendments.
EOF
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a QA engineer validating" <<< "$INPUT"; then
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
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Mock script that records `pwd` to a known file before producing standard output.
/// Used by the working_directory conformance test to verify repo-root invariant.
/// Mirrors `standard_mock_script()` prompt coverage exactly (including prompt reviewer)
/// with the addition of pwd capture at the top.
pub fn pwd_recording_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

# Record pwd to a sidecar file for the conformance test to inspect.
pwd > "${RALPH_PWD_LOG:-/tmp/ralph-pwd.log}"

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
# Feature: CWD Test Feature

## Description
Feature to verify working directory invariant.

## Acceptance Criteria
- [ ] Working directory stays at repo root

## Files to Modify/Create
- `cwd_test.txt` - test file

## Dependencies
- Requires: none
- Blocks: none
EOF
  fi
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
- Verified cwd is at repo root.

## Spec Deviations
- None

## Testing
- pwd check only
EOF
  fi
  echo "cwd-ok" > cwd_test.txt
  git add cwd_test.txt
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
The project is complete and requires no further amendments.
EOF
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- cwd check: passed

## Automated Tests
- pwd matches repo root

## Acceptance Criteria Verification
All criteria verified.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Working directory stays at repo root

## Notes
CWD is correct.

## Commit Message
feat: verify cwd invariant
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
    cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- CWD requirement: satisfied
EOF
  else
    cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Additional features remain.

## Recommended Next Features
1. More features.
EOF
  fi
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Mock `ralph` script for validate E2E tests that always delegates to the
/// provided absolute `ralph` binary path and executes `auto`.
///
/// This avoids recursive PATH-based resolution and ensures the delegated
/// executable is exactly the caller-provided binary.
pub fn e2e_mock_ralph_script(ralph_bin: &Path) -> String {
    let absolute = ralph_bin
        .canonicalize()
        .unwrap_or_else(|_| ralph_bin.to_path_buf());
    let quoted = shell_single_quote(&absolute.to_string_lossy());
    format!(
        "#!/bin/sh\n\
set -eu\n\
if [ \"${{1:-}}\" = \"auto\" ]; then\n\
  shift\n\
fi\n\
exec {quoted} auto \"$@\"\n"
    )
}

/// Mock `gh` script for validate E2E tests that captures full `gh pr create`
/// arguments and `--body-file` contents to a log file.
///
/// Set `RALPH_E2E_GH_LOG` to control the output path.
pub fn e2e_mock_gh_logging_script() -> String {
    r###"#!/bin/sh
set -eu

log_path="${RALPH_E2E_GH_LOG:-${TMPDIR:-/tmp}/ralph-e2e-gh.log}"

if [ $# -ge 2 ] && [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  : > "$log_path"

  idx=0
  for arg in "$@"; do
    printf 'arg[%s]=%s\n' "$idx" "$arg" >> "$log_path"
    idx=$((idx + 1))
  done

  body_file=""
  prev=""
  for arg in "$@"; do
    if [ "$prev" = "--body-file" ]; then
      body_file="$arg"
      break
    fi
    prev="$arg"
  done

  if [ -n "$body_file" ] && [ -f "$body_file" ]; then
    printf 'body_file=%s\n' "$body_file" >> "$log_path"
    printf 'body_begin\n' >> "$log_path"
    cat "$body_file" >> "$log_path"
    printf '\nbody_end\n' >> "$log_path"
  fi

  printf 'https://github.com/mock/repo/pull/123\n'
  exit 0
fi

case "${1:-}" in
  label)
    # label create is best-effort; always succeed
    exit 0
    ;;
  issue)
    case "${2:-}" in
      list)
        # Check whether --label ralph:ready is among the args
        has_ready=0
        for arg in "$@"; do
          if [ "$arg" = "ralph:ready" ]; then
            has_ready=1
          fi
        done
        if [ "$has_ready" = "1" ] && [ -n "${RALPH_E2E_MOCK_ISSUES:-}" ]; then
          printf '%s' "$RALPH_E2E_MOCK_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit) exit 0 ;;
      view)
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
        if [ "$want_title_body" = "1" ]; then
          printf '{"title":"","body":"E2E issue context from mock gh."}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "${2:-}" in
      list) printf '' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo)
    case "${2:-}" in
      clone)
        target_dir="$4"
        mkdir -p "$target_dir"
        git init "$target_dir" --quiet 2>/dev/null
        git -C "$target_dir" config user.email "mock@test"
        git -C "$target_dir" config user.name "MockClone"
        touch "$target_dir/.gitkeep"
        git -C "$target_dir" add .gitkeep
        git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        exit 0
        ;;
      view) printf 'acme/widgets\n'; exit 0 ;;
    esac
    ;;
esac

echo "mock gh: unhandled command: $*" >&2
exit 1
"###
    .to_owned()
}

/// Mock backend script that consumes stdin and exits non-zero.
///
/// Optional logging:
/// - `RALPH_VALIDATE_BACKEND_LOG`: append one line per invocation
/// - `RALPH_VALIDATE_BACKEND_LABEL`: label prefix for logged lines
pub fn nonzero_exit_backend_script() -> String {
    r###"#!/bin/sh
set -eu

cat >/dev/null
if [ -n "${RALPH_VALIDATE_BACKEND_LOG:-}" ]; then
  printf '%s:nonzero\n' "${RALPH_VALIDATE_BACKEND_LABEL:-backend}" >> "${RALPH_VALIDATE_BACKEND_LOG}"
fi
echo "intentional backend failure" >&2
exit 17
"###
    .to_owned()
}

/// Mock backend script that consumes stdin and returns empty output with exit 0.
///
/// Optional logging:
/// - `RALPH_VALIDATE_BACKEND_LOG`: append one line per invocation
/// - `RALPH_VALIDATE_BACKEND_LABEL`: label prefix for logged lines
pub fn empty_output_backend_script() -> String {
    r###"#!/bin/sh
set -eu

input="$(cat)"
if [ -n "${RALPH_VALIDATE_BACKEND_LOG:-}" ]; then
  kind="normal"
  if printf '%s' "$input" | grep -q "CRITICAL: Your previous response could not be parsed."; then
    kind="reformatter_prompt"
  elif printf '%s' "$input" | grep -q "IMPORTANT: Format your response as parseable markdown."; then
    kind="format_reminder"
  fi
  printf '%s:%s\n' "${RALPH_VALIDATE_BACKEND_LABEL:-backend}" "$kind" >> "${RALPH_VALIDATE_BACKEND_LOG}"
fi

# Intentionally produce no stdout so orchestrator exercises empty-output handling.
exit 0
"###
    .to_owned()
}

/// Mock script whose prompt reviewer response includes nested `##` headings
/// inside the `## Refined Prompt` section, exercising the extract-to-EOF parser
/// semantics. All other roles respond identically to `standard_mock_script()`.
pub fn nested_heading_prompt_review_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Acceptance criteria could be more specific
- Missing error-handling requirements

## Refined Prompt
## Overview
Build a widget system that manages lifecycle events.

## Architecture
The system uses an event-driven model with pluggable handlers.

### Component Registry
Each component registers via a unique key.

## Acceptance Criteria
- [ ] Widget lifecycle events fire in order
- [ ] Handlers can be registered and removed dynamically
- [ ] Error boundaries prevent cascading failures

## Technical Notes
Use the observer pattern for event dispatch.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Mock `gh` script for daemon runtime tests. Handles:
/// - `gh issue list ...` — returns configurable JSON issues
/// - `gh issue edit ...` — logs label add/remove, always succeeds
/// - `gh issue view --json title,body ...` — returns title/body JSON
/// - `gh issue view --json labels ...` — returns labels JSON
/// - `gh issue view ... -q .comments[].body` — returns empty comments
/// - `gh issue comment ...` — no-op success
/// - `gh pr list ...` — returns empty
/// - `gh pr create ...` — returns a fake PR URL
/// - `gh repo view ...` — returns the configured owner/repo
///
/// Set `MOCK_GH_ISSUES` env var to a JSON array of issues for poll responses.
/// Set `MOCK_GH_OVERFLOW` to "true" to return exactly 100 issues.
/// Set `MOCK_GH_LABEL_LOG` to a file path to log label add/remove operations.
/// Set `MOCK_GH_ISSUE_LABELS` to JSON for `issue view --json labels` responses.
pub fn daemon_mock_gh_script() -> String {
    r###"#!/bin/sh
# Mock gh for daemon runtime tests.
# Env: MOCK_GH_ISSUES - JSON array of issues for `issue list`
# Env: MOCK_GH_OVERFLOW - if "true", return 100 identical issues
# Env: MOCK_GH_LABEL_LOG - file to log label add/remove operations
# Env: MOCK_GH_ISSUE_LABELS - JSON for `issue view --json labels`

case "$1" in
  issue)
    case "$2" in
      list)
        if [ "$MOCK_GH_OVERFLOW" = "true" ]; then
          # Generate exactly 100 issues
          printf '['
          i=1
          while [ $i -le 100 ]; do
            if [ $i -gt 1 ]; then printf ','; fi
            printf '{"number":%d,"title":"issue %d","labels":[]}' $i $i
            i=$((i + 1))
          done
          printf ']'
          exit 0
        fi
        if [ -n "$MOCK_GH_ISSUES" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        # Log label operations if logging enabled
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        # Claiming / label update — always succeed
        exit 0
        ;;
      view)
        # Check for --json labels query
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            want_labels=1
          fi
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done

        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi

        # Title/body fetch used by pending-task hydration.
        if [ "$want_title_body" = "1" ]; then
          issue_number="${3:-0}"
          printf '{"title":"Mock issue %s","body":"Mock body for issue %s"}' "$issue_number" "$issue_number"
          exit 0
        fi

        # Existing comments query path (`-q .comments[].body`): empty output.
        printf ''
        exit 0
        ;;
      comment)
        # Post comment — always succeed
        exit 0
        ;;
      *)
        echo "mock gh: unhandled issue subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        # No existing PRs — return empty (simulates -q ".[0].url" with no results)
        printf ''
        exit 0
        ;;
      create)
        printf 'https://github.com/mock/repo/pull/1\n'
        exit 0
        ;;
      edit)
        # PR edit — always succeed
        exit 0
        ;;
      *)
        echo "mock gh: unhandled pr subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  api)
    if [ "$2" = "user" ]; then
      printf 'ralph-bot\n'
      exit 0
    fi
    echo "mock gh: unhandled api subcommand: $2" >&2
    exit 1
    ;;
  label)
    case "$2" in
      create)
        exit 0
        ;;
      *)
        echo "mock gh: unhandled label subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        if [ -n "$target_dir" ]; then
          mkdir -p "$target_dir"
          git init "$target_dir" --quiet 2>/dev/null
          git -C "$target_dir" config user.email "mock@test"
          git -C "$target_dir" config user.name "MockClone"
          touch "$target_dir/.gitkeep"
          git -C "$target_dir" add .gitkeep
          git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        fi
        exit 0
        ;;
      view)
        printf 'acme/widgets\n'
        exit 0
        ;;
      *)
        echo "mock gh: unhandled repo subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "mock gh: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Mock `gh` script that returns an existing PR URL from `pr list --head`,
/// succeeds on `pr edit`, and logs `pr edit` and `pr create` calls via files.
///
/// Set `MOCK_GH_PR_EDIT_LOG` to a file path to record `pr edit` invocations.
/// Set `MOCK_GH_PR_CREATE_LOG` to a file path to record `pr create` invocations.
/// Set `MOCK_GH_PR_EDIT_FAIL` to "true" to make `pr edit` fail.
pub fn daemon_mock_gh_edit_pr_script() -> String {
    r###"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit)
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        exit 0
        ;;
      view)
        want_labels=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            want_labels=1
          fi
        done
        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi
        printf '' ; exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        # Return an existing PR URL to trigger edit path
        for arg in "$@"; do
          case "$arg" in
            --head)
              printf 'https://github.com/acme/widgets/pull/77'
              exit 0
              ;;
          esac
        done
        printf ''
        exit 0
        ;;
      create)
        if [ -n "$MOCK_GH_PR_CREATE_LOG" ]; then
          echo "called" > "$MOCK_GH_PR_CREATE_LOG"
        fi
        printf 'https://github.com/acme/widgets/pull/new\n'
        exit 0
        ;;
      edit)
        if [ -n "$MOCK_GH_PR_EDIT_LOG" ]; then
          echo "$@" > "$MOCK_GH_PR_EDIT_LOG"
        fi
        if [ "$MOCK_GH_PR_EDIT_FAIL" = "true" ]; then
          echo "mock edit failure" >&2
          exit 1
        fi
        exit 0
        ;;
    esac
    ;;
  label)
    case "$2" in
      create) exit 0 ;;
      *)
        echo "mock gh: unhandled label subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"###
    .to_owned()
}

/// Mock refinement backend output used by daemon runtime tests when refinement is
/// required but should remain fast and deterministic.
pub fn daemon_mock_fast_refinement_script() -> String {
    r###"#!/bin/sh
# Minimal deterministic refinement output used by validate tests.
printf 'TITLE: Refined task execution\n'
printf '---\n'
printf 'Refined task body with explicit steps and acceptance checks for safe deterministic execution in tests.\n'
"###
    .to_owned()
}

/// Mock `gh` script for daemon auto-rebase tests.
///
/// Environment variables:
/// - `MOCK_PR_VIEW_JSON` — JSON response for `gh pr view --json ...`
/// - `MOCK_PR_VIEW_EXIT` — exit code for `gh pr view` (default: 0)
/// - `MOCK_PR_COMMENT_LOG` — file path to log pr comment bodies
/// - `MOCK_GH_LABEL_LOG` — file path to log label add/remove operations
/// - `MOCK_GH_ISSUE_LABELS` — JSON for `issue view --json labels` responses
pub fn daemon_mock_gh_rebase_script() -> String {
    r###"#!/bin/sh
# Mock gh for daemon auto-rebase tests.
# Env: MOCK_GH_ISSUES - JSON array of issues for `issue list`
# Env: MOCK_PR_VIEW_JSON - JSON response for `pr view --json`
# Env: MOCK_PR_VIEW_EXIT - exit code for `pr view` (default 0)
# Env: MOCK_PR_COMMENT_LOG - file to log pr comment bodies
# Env: MOCK_GH_LABEL_LOG - file to log label add/remove operations
# Env: MOCK_GH_ISSUE_LABELS - JSON for `issue view --json labels`

case "$1" in
  issue)
    case "$2" in
      list)
        if [ -n "$MOCK_GH_ISSUES" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        exit 0
        ;;
      view)
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            want_labels=1
          fi
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          issue_number="${3:-0}"
          printf '{"title":"Mock issue %s","body":"Mock body for issue %s"}' "$issue_number" "$issue_number"
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled issue subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/repo/pull/1\n' ; exit 0 ;;
      view)
        # Check if asking for JSON merge info
        has_json=0
        for arg in "$@"; do
          if [ "$arg" = "mergeable,state,baseRefName,headRefOid" ]; then
            has_json=1
          fi
        done
        if [ "$has_json" = "1" ]; then
          if [ -n "$MOCK_PR_VIEW_JSON" ]; then
            printf '%s' "$MOCK_PR_VIEW_JSON"
          else
            printf '{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}'
          fi
          exit ${MOCK_PR_VIEW_EXIT:-0}
        fi
        printf ''
        exit 0
        ;;
      comment)
        # Log the comment body
        shift; shift # skip 'pr' 'comment'
        pr_number="$1"
        shift
        while [ $# -gt 0 ]; do
          case "$1" in
            --body)
              if [ -n "$MOCK_PR_COMMENT_LOG" ]; then
                echo "$2" >> "$MOCK_PR_COMMENT_LOG"
              fi
              shift 2
              ;;
            --repo) shift 2 ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
      *)
        echo "mock gh: unhandled pr subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  label)
    case "$2" in
      create) exit 0 ;;
      *)
        echo "mock gh: unhandled label subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  repo)
    case "$2" in
      view) printf 'acme/widgets\n' ; exit 0 ;;
      *)
        echo "mock gh: unhandled repo subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "mock gh: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Mock `git` script for rebase tests. All git operations succeed except push,
/// which fails with a generic error (triggering a failure comment).
pub fn daemon_mock_git_rebase_fail_push_script() -> String {
    r###"#!/bin/sh
# Mock git for rebase tests. All ops succeed except push.
case "$1" in
  fetch) exit 0 ;;
  rebase)
    case "$2" in
      --abort) exit 0 ;;
      *) exit 0 ;;
    esac
    ;;
  push)
    echo "error: could not push to remote" >&2
    exit 1
    ;;
  worktree)
    case "$2" in
      add)
        # Create the directory so the daemon sees it exist
        for arg in "$@"; do :; done
        # Find the path argument (3rd positional after 'add' and flags)
        shift; shift  # skip 'worktree' 'add'
        wt_path=""
        for arg in "$@"; do
          case "$arg" in
            --force|-b|HEAD) ;;
            /*) wt_path="$arg" ;;
          esac
        done
        if [ -n "$wt_path" ]; then
          mkdir -p "$wt_path"
          # Create a minimal .git file so git recognizes it
          echo "gitdir: /dev/null" > "$wt_path/.git"
        fi
        exit 0
        ;;
      remove) exit 0 ;;
      prune) exit 0 ;;
      *) exit 0 ;;
    esac
    ;;
  checkout) exit 0 ;;
  rev-parse)
    for arg in "$@"; do
      if [ "$arg" = "--show-toplevel" ]; then
        # Only succeed if CWD is actually inside a git repo (has .git).
        # Walk up from CWD to find .git; fail if not found.
        check_dir="$(pwd)"
        while true; do
          if [ -d "$check_dir/.git" ] || [ -f "$check_dir/.git" ]; then
            echo "$check_dir"
            exit 0
          fi
          parent="$(dirname "$check_dir")"
          if [ "$parent" = "$check_dir" ]; then
            echo "fatal: not a git repository" >&2
            exit 128
          fi
          check_dir="$parent"
        done
      fi
      if [ "$arg" = "--abbrev-ref" ]; then
        echo "mock-branch"
        exit 0
      fi
    done
    exit 0
    ;;
  *) exit 0 ;;
esac
"###
    .to_owned()
}

/// Mock `git` script for lease-rejection simulation. Succeeds on fetch and
/// rebase, but fails push --force-with-lease with a `stale info` message.
pub fn daemon_mock_git_lease_reject_script() -> String {
    r###"#!/bin/sh
# Mock git that simulates force-with-lease rejection.
# fetch and rebase succeed; push --force-with-lease fails with stale info.
case "$1" in
  fetch) exit 0 ;;
  rebase)
    case "$2" in
      --abort) exit 0 ;;
      *) exit 0 ;;
    esac
    ;;
  push)
    for arg in "$@"; do
      if [ "$arg" = "--force-with-lease" ]; then
        echo "error: failed to push some refs" >&2
        echo " ! [rejected] branch -> branch (stale info)" >&2
        exit 1
      fi
    done
    exit 0
    ;;
  worktree)
    case "$2" in
      add)
        shift; shift
        wt_path=""
        for arg in "$@"; do
          case "$arg" in
            --force|-b|HEAD) ;;
            /*) wt_path="$arg" ;;
          esac
        done
        if [ -n "$wt_path" ]; then
          mkdir -p "$wt_path"
          echo "gitdir: /dev/null" > "$wt_path/.git"
        fi
        exit 0
        ;;
      remove) exit 0 ;;
      prune) exit 0 ;;
      *) exit 0 ;;
    esac
    ;;
  checkout) exit 0 ;;
  rev-parse)
    for arg in "$@"; do
      if [ "$arg" = "--show-toplevel" ]; then
        # Only succeed if CWD is actually inside a git repo (has .git).
        check_dir="$(pwd)"
        while true; do
          if [ -d "$check_dir/.git" ] || [ -f "$check_dir/.git" ]; then
            echo "$check_dir"
            exit 0
          fi
          parent="$(dirname "$check_dir")"
          if [ "$parent" = "$check_dir" ]; then
            echo "fatal: not a git repository" >&2
            exit 128
          fi
          check_dir="$parent"
        done
      fi
      if [ "$arg" = "--abbrev-ref" ]; then
        echo "mock-branch"
        exit 0
      fi
    done
    exit 0
    ;;
  *) exit 0 ;;
esac
"###
    .to_owned()
}

/// Mock `gh` script for daemon clone tests. Handles `gh repo clone <slug> <dir>`
/// by creating a git repo at the target directory. All other commands behave like
/// `daemon_mock_gh_script()`.
///
/// Set `MOCK_GH_CLONE_FAIL` to "true" to simulate clone failure.
pub fn daemon_mock_gh_clone_script() -> String {
    r###"#!/bin/sh
# Mock gh for daemon clone + runtime tests.
# Env: MOCK_GH_ISSUES - JSON array of issues for `issue list`
# Env: MOCK_GH_CLONE_FAIL - if "true", `repo clone` fails

case "$1" in
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        if [ "$MOCK_GH_CLONE_FAIL" = "true" ]; then
          echo "error: Could not resolve to a Repository" >&2
          exit 1
        fi
        # Simulate clone by creating a git repo
        mkdir -p "$target_dir"
        git init "$target_dir" --quiet 2>/dev/null
        git -C "$target_dir" config user.email "mock@test"
        git -C "$target_dir" config user.name "MockClone"
        touch "$target_dir/.gitkeep"
        git -C "$target_dir" add .gitkeep
        git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        exit 0
        ;;
      view)
        printf 'acme/widgets\n'
        exit 0
        ;;
      *)
        echo "mock gh: unhandled repo subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  issue)
    case "$2" in
      list)
        if [ -n "$MOCK_GH_ISSUES" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        exit 0
        ;;
      view)
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            want_labels=1
          fi
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          issue_number="${3:-0}"
          printf '{"title":"Mock issue %s","body":"Mock body for issue %s"}' "$issue_number" "$issue_number"
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled issue subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/repo/pull/1\n' ; exit 0 ;;
      edit) exit 0 ;;
      *)
        echo "mock gh: unhandled pr subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "mock gh: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Mock script whose reviewer rejects on the first iteration and approves on
/// the second. This produces exactly one review-feedback cycle, generating
/// `*-impl-response-001.md` before final approval.
pub fn review_feedback_once_then_approve_script(review_counter: &std::path::Path) -> String {
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

REVIEW_COUNTER="{review_counter}"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  cat <<'EOF'
# Feature: Feedback Feature

## Description
Mock feature used by impl-response conformance tests.

## Acceptance Criteria
- [ ] Mock implementation file is created

## Files to Modify/Create
- `mock_file.txt` - file created by the mock implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed reviewer feedback: tightened mock validation.

## Could Not Address
- None

## Pending Changes (Pre-Commit)
- Updated mock_file.txt with validated content
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
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  RCOUNT="$(cat "$REVIEW_COUNTER" 2>/dev/null || echo 0)"
  RCOUNT=$((RCOUNT + 1))
  echo "$RCOUNT" > "$REVIEW_COUNTER"
  if [ "$RCOUNT" -le 1 ]; then
    cat <<'EOF'
# Review: SUGGESTIONS

## Required Changes
1. Tighten mock validation behavior.
EOF
  else
    cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Feedback addressed.

## Commit Message
feat: apply mock implementation after review feedback
EOF
  fi
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
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
"###,
        review_counter = review_counter.to_string_lossy(),
    )
}

pub fn always_reject_review_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  cat <<'EOF'
# Feature: Review Retry Feature

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
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: SUGGESTIONS

## Required Changes
1. Tighten mock validation behavior.
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Review never approves in this mock script.

## Recommended Next Features
1. Replace reviewer mock with an approving version.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Mock script that returns unparseable planner output on the first call,
/// triggering the parse-retry/reformatter path. Uses a counter file to
/// track invocations so the second call returns valid output.
/// All other roles respond identically to `standard_mock_script()`.
pub fn planner_parse_fail_then_pass_mock_script(counter_file: &Path) -> String {
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  COUNTER_FILE="{counter}"
  COUNT="$(cat "$COUNTER_FILE" 2>/dev/null || echo 0)"
  COUNT=$((COUNT + 1))
  echo "$COUNT" > "$COUNTER_FILE"
  if [ "$COUNT" -le 1 ]; then
    # First call: return unparseable output (no valid H1)
    echo "This is not a valid planner response."
    echo "It has no H1 heading and will fail parsing."
  else
    if [ "${{RALPH_COMPLETE:-no}}" = "yes" ]; then
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
  fi
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if [ "${{RALPH_COMPLETE:-no}}" = "yes" ]; then
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
elif grep -q "CRITICAL: Your previous response could not be parsed." <<< "$INPUT"; then
  # Reformatter/parse-retry: return valid planner output
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
elif grep -q "IMPORTANT: Format your response as parseable markdown." <<< "$INPUT"; then
  # Format reminder retry: return valid planner output
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
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###,
        counter = counter_file.to_string_lossy()
    )
}

/// Mock script that emits planner output with a delayed chunk; used to verify
/// real-time streaming behavior in the log writer.
pub fn slow_streaming_planner_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  printf '# Feature: Slow Streaming Feature\n'
  sleep 1
  cat <<'EOF'

## Description
Planner output emitted in delayed chunks.

## Acceptance Criteria
- [ ] mock file exists

## Files to Modify/Create
- `mock_file.txt` - created by implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Mock script that keeps planner output active with periodic chunks for longer
/// than a 1-second timeout window, validating idle-timeout reset behavior.
pub fn idle_timeout_reset_planner_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  printf '# Feature: Idle Timeout Reset Feature\n'
  sleep 0.45
  printf '\n## Description\n'
  sleep 0.45
  printf 'Planner stays active with periodic output chunks.\n'
  sleep 0.45
  cat <<'EOF'

## Acceptance Criteria
- [ ] mock file exists

## Files to Modify/Create
- `mock_file.txt` - created by implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  if [[ "${RALPH_COMPLETE:-no}" == "yes" ]]; then
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
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Mock script that emits partial planner output then hangs; used to verify
/// timeout cleanup and log footer behavior.
pub fn timeout_hanging_planner_mock_script(pid_file: &Path) -> String {
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  echo $$ > "{pid_file}"
  printf 'planner-partial-before-timeout'
  sleep 30
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created a mock implementation artifact.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
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
"###,
        pid_file = pid_file.to_string_lossy()
    )
}

/// Mock script that emits planner output at regular intervals for a total runtime
/// exceeding timeout_seconds, proving that inactivity timeout does NOT fire when
/// the stream is active.
pub fn active_streaming_planner_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  # Emit output every 0.2s for ~1.2s total (> 1s timeout configured in test)
  printf '# Feature: Active Stream Feature\n'
  for i in $(seq 1 6); do
    sleep 0.2
    printf 'chunk-%d\n' "$i"
  done
  cat <<'EOF'

## Description
Planner output emitted in slow but steady chunks.

## Acceptance Criteria
- [ ] mock file exists

## Files to Modify/Create
- `mock_file.txt` - created by implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  if grep -q "## Review Feedback" <<< "$INPUT" && ! grep -q "(none)" <<< "$INPUT"; then
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
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
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
    .to_owned()
}

/// Mock `tmux` script for conformance tests that enables tmux-mode execution
/// without a real tmux binary. Handles the tmux commands that `TmuxBackend`
/// issues: `has-session`, `new-session`, `new-window`, `set-option`,
/// `list-windows`, and `kill-window`.
///
/// The mock creates the output and exit files that `wait_for_exit_with_activity`
/// polls for, by extracting the shell command from `new-window` and running the
/// backend script inline.
pub fn mock_tmux_script() -> String {
    r###"#!/bin/sh
# Mock tmux for conformance tests.
# Handles: has-session, new-session, new-window, set-option, list-windows, kill-window
set -eu

case "$1" in
  has-session)
    # Session always exists (or will be created)
    exit 0
    ;;
  new-session)
    exit 0
    ;;
  new-window)
    # Extract the shell command (last argument) and run it.
    # new-window is called as: tmux new-window -t <session> -n <label> -P -F '#{window_id}' <shell_cmd>
    # The shell command is the last positional argument.
    shift  # skip 'new-window'
    shell_cmd=""
    while [ $# -gt 0 ]; do
      shell_cmd="$1"
      shift
    done
    # Run the shell command in background via sh, then print a fake window id
    if [ -n "$shell_cmd" ]; then
      sh -c "$shell_cmd" &
    fi
    printf '1\n'
    exit 0
    ;;
  set-option)
    exit 0
    ;;
  list-windows)
    # Return the window id so has_window succeeds
    printf '1\n'
    exit 0
    ;;
  kill-window)
    exit 0
    ;;
  *)
    echo "mock tmux: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Mock script where the planner emits partial output then stalls, used to verify
/// that inactivity timeout fires after the stall while preserving partial output.
pub fn hanging_after_partial_planner_mock_script(pid_file: &Path) -> String {
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  echo $$ > "{pid_file}"
  printf 'partial-output-before-stall'
  sleep 30
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created a mock implementation artifact.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
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
"###,
        pid_file = pid_file.to_string_lossy()
    )
}

/// Mock `gh` script for daemon bounded continuous-mode concurrency tests.
///
/// Like `daemon_mock_gh_concurrency_script` but uses a counter file
/// (`MOCK_GH_ISSUE_LIST_COUNTER`) to return issues only on the first
/// `issue list` call (with `ralph:ready` label). Subsequent calls return `[]`.
/// This allows a multi-iteration daemon run where iteration 1 dispatches
/// children and iteration 2+ sees them for auto-rebase.
///
/// Also logs rebase-candidate discovery attempts to `MOCK_REBASE_ATTEMPT_LOG`
/// when `pr list --head` is called, enabling tests to verify the auto-rebase
/// code path was actually entered.
///
/// Environment variables:
/// - `MOCK_GH_ISSUES` — JSON array of issues for first `issue list` call
/// - `MOCK_GH_ISSUE_LIST_COUNTER` — file to track `issue list` call count
/// - `MOCK_GH_LABEL_LOG` — file to log label add/remove operations
/// - `MOCK_GH_ISSUE_LABELS` — JSON for `issue view --json labels`
/// - `MOCK_PRD_TICK_LOG` — file to count PRD tick invocations
/// - `MOCK_PR_VIEW_JSON` — JSON for `pr view --json` merge metadata
/// - `MOCK_REBASE_ATTEMPT_LOG` — file to log rebase candidate PR lookups
pub fn daemon_mock_gh_bounded_concurrency_script() -> String {
    r###"#!/bin/sh
# Mock gh for daemon bounded continuous-mode concurrency tests.
# Returns issues only on the first `issue list` call; empty thereafter.

case "$1" in
  issue)
    case "$2" in
      list)
        # Detect PRD tick: check if any arg is ralph:prd or ralph:prd-active
        has_prd=0
        has_ready=0
        for arg in "$@"; do
          case "$arg" in
            ralph:prd|ralph:prd-active) has_prd=1 ;;
            ralph:ready) has_ready=1 ;;
          esac
        done
        if [ "$has_prd" = "1" ] && [ -n "${MOCK_PRD_TICK_LOG:-}" ]; then
          echo "prd-tick" >> "$MOCK_PRD_TICK_LOG"
        fi

        # For ralph:ready queries, use counter to return issues only once
        if [ "$has_ready" = "1" ] && [ -n "${MOCK_GH_ISSUE_LIST_COUNTER:-}" ]; then
          count="$(cat "$MOCK_GH_ISSUE_LIST_COUNTER" 2>/dev/null || echo 0)"
          count=$((count + 1))
          echo "$count" > "$MOCK_GH_ISSUE_LIST_COUNTER"
          if [ "$count" -gt 1 ]; then
            # Subsequent calls: no new issues
            printf '[]'
            exit 0
          fi
        fi

        if [ -n "${MOCK_GH_ISSUES:-}" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        exit 0
        ;;
      view)
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            want_labels=1
          fi
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          issue_number="${3:-0}"
          printf '{"title":"Mock issue %s","body":"Mock body for issue %s"}' "$issue_number" "$issue_number"
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled issue subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        has_head=0
        for arg in "$@"; do
          if [ "$arg" = "--head" ]; then
            has_head=1
          fi
        done
        if [ "$has_head" = "1" ]; then
          # Log rebase candidate discovery attempt
          if [ -n "${MOCK_REBASE_ATTEMPT_LOG:-}" ]; then
            echo "rebase-pr-lookup" >> "$MOCK_REBASE_ATTEMPT_LOG"
          fi
          printf 'https://github.com/mock/repo/pull/1'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      create)
        printf 'https://github.com/mock/repo/pull/1\n'
        exit 0
        ;;
      view)
        has_json=0
        for arg in "$@"; do
          if [ "$arg" = "mergeable,state,baseRefName,headRefOid" ]; then
            has_json=1
          fi
        done
        if [ "$has_json" = "1" ]; then
          if [ -n "${MOCK_PR_VIEW_JSON:-}" ]; then
            printf '%s' "$MOCK_PR_VIEW_JSON"
          else
            printf '{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}'
          fi
          exit 0
        fi
        printf ''
        exit 0
        ;;
      edit) exit 0 ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled pr subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  api)
    if [ "$2" = "user" ]; then
      printf 'ralph-bot\n'
      exit 0
    fi
    echo "mock gh: unhandled api subcommand: $2" >&2
    exit 1
    ;;
  label)
    case "$2" in
      create) exit 0 ;;
      *)
        echo "mock gh: unhandled label subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        if [ -n "$target_dir" ]; then
          mkdir -p "$target_dir"
          git init "$target_dir" --quiet 2>/dev/null
          git -C "$target_dir" config user.email "mock@test"
          git -C "$target_dir" config user.name "MockClone"
          touch "$target_dir/.gitkeep"
          git -C "$target_dir" add .gitkeep
          git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        fi
        exit 0
        ;;
      view)
        printf 'acme/widgets\n'
        exit 0
        ;;
      *)
        echo "mock gh: unhandled repo subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "mock gh: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Mock `gh` script for daemon concurrency conformance tests.
///
/// Extends `daemon_mock_gh_script` with:
/// - **PRD tick logging**: When `issue list` is called with a `ralph:prd` label,
///   appends a `prd-tick` line to `MOCK_PRD_TICK_LOG` (if set). This enables
///   tests to count exactly how many PRD poll ticks occurred.
/// - **PR URL for rebase candidates**: `gh pr list --head <branch>` returns a
///   mock PR URL so `find_existing_pr` discovers candidates.
/// - **PR merge metadata**: `gh pr view --json mergeable,state,baseRefName,headRefOid`
///   returns configurable JSON (via `MOCK_PR_VIEW_JSON`).
///
/// Environment variables:
/// - `MOCK_GH_ISSUES` — JSON array of issues for `issue list`
/// - `MOCK_GH_LABEL_LOG` — file to log label add/remove operations
/// - `MOCK_GH_ISSUE_LABELS` — JSON for `issue view --json labels`
/// - `MOCK_PRD_TICK_LOG` — file to count PRD tick invocations
/// - `MOCK_PR_VIEW_JSON` — JSON for `pr view --json` merge metadata
pub fn daemon_mock_gh_concurrency_script() -> String {
    r###"#!/bin/sh
# Mock gh for daemon concurrency conformance tests.
# Env: MOCK_GH_ISSUES - JSON array of issues for `issue list`
# Env: MOCK_GH_LABEL_LOG - file to log label add/remove operations
# Env: MOCK_GH_ISSUE_LABELS - JSON for `issue view --json labels`
# Env: MOCK_PRD_TICK_LOG - file to log PRD tick invocations
# Env: MOCK_PR_VIEW_JSON - JSON for `pr view --json` merge metadata

case "$1" in
  issue)
    case "$2" in
      list)
        # Detect PRD tick: check if any arg is ralph:prd or ralph:prd-active
        has_prd=0
        for arg in "$@"; do
          case "$arg" in
            ralph:prd|ralph:prd-active) has_prd=1 ;;
          esac
        done
        if [ "$has_prd" = "1" ] && [ -n "${MOCK_PRD_TICK_LOG:-}" ]; then
          echo "prd-tick" >> "$MOCK_PRD_TICK_LOG"
        fi

        if [ "${MOCK_GH_OVERFLOW:-}" = "true" ]; then
          printf '['
          i=1
          while [ $i -le 100 ]; do
            if [ $i -gt 1 ]; then printf ','; fi
            printf '{"number":%d,"title":"issue %d","labels":[]}' $i $i
            i=$((i + 1))
          done
          printf ']'
          exit 0
        fi
        if [ -n "${MOCK_GH_ISSUES:-}" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        exit 0
        ;;
      view)
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            want_labels=1
          fi
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          issue_number="${3:-0}"
          printf '{"title":"Mock issue %s","body":"Mock body for issue %s"}' "$issue_number" "$issue_number"
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled issue subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        # Check for --head flag to support find_existing_pr
        has_head=0
        for arg in "$@"; do
          if [ "$arg" = "--head" ]; then
            has_head=1
          fi
        done
        if [ "$has_head" = "1" ]; then
          # Return a mock PR URL for rebase candidate discovery
          printf 'https://github.com/mock/repo/pull/1'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      create)
        printf 'https://github.com/mock/repo/pull/1\n'
        exit 0
        ;;
      view)
        # Check for merge-info JSON query
        has_json=0
        for arg in "$@"; do
          if [ "$arg" = "mergeable,state,baseRefName,headRefOid" ]; then
            has_json=1
          fi
        done
        if [ "$has_json" = "1" ]; then
          if [ -n "${MOCK_PR_VIEW_JSON:-}" ]; then
            printf '%s' "$MOCK_PR_VIEW_JSON"
          else
            printf '{"mergeable":"MERGEABLE","state":"OPEN","baseRefName":"master","headRefOid":"abc123"}'
          fi
          exit 0
        fi
        printf ''
        exit 0
        ;;
      edit) exit 0 ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled pr subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  api)
    if [ "$2" = "user" ]; then
      printf 'ralph-bot\n'
      exit 0
    fi
    echo "mock gh: unhandled api subcommand: $2" >&2
    exit 1
    ;;
  label)
    case "$2" in
      create) exit 0 ;;
      *)
        echo "mock gh: unhandled label subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        if [ -n "$target_dir" ]; then
          mkdir -p "$target_dir"
          git init "$target_dir" --quiet 2>/dev/null
          git -C "$target_dir" config user.email "mock@test"
          git -C "$target_dir" config user.name "MockClone"
          touch "$target_dir/.gitkeep"
          git -C "$target_dir" add .gitkeep
          git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        fi
        exit 0
        ;;
      view)
        printf 'acme/widgets\n'
        exit 0
        ;;
      *)
        echo "mock gh: unhandled repo subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "mock gh: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Mock `gh` script for dispatch failure conformance tests.
///
/// Extends `daemon_mock_gh_concurrency_script` to also log when
/// dispatch-failure label swaps occur (transitions from
/// `ralph:in-progress` to `ralph:failed`), writing an explicit
/// `dispatch-failure:<issue_number>` marker to `MOCK_DISPATCH_FAILURE_LOG`.
///
/// Environment variables:
/// - All variables from `daemon_mock_gh_concurrency_script`
/// - `MOCK_DISPATCH_FAILURE_LOG` — file to log dispatch-failure markers
pub fn daemon_mock_gh_dispatch_failure_script() -> String {
    r###"#!/bin/sh
# Mock gh for dispatch-failure conformance tests.
# Logs dispatch-failure markers when in-progress -> failed transitions occur.

case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        for arg in "$@"; do
          case "$arg" in
            ralph:prd|ralph:prd-active) has_prd=1 ;;
          esac
        done
        if [ "$has_prd" = "1" ] && [ -n "${MOCK_PRD_TICK_LOG:-}" ]; then
          echo "prd-tick" >> "$MOCK_PRD_TICK_LOG"
        fi
        if [ -n "${MOCK_GH_ISSUES:-}" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        # Detect failure-label addition: gh issue edit <num> --add-label ralph:failed
        # swap_lifecycle_label calls remove_label then add_label in separate
        # gh invocations, so we detect the add-label call alone.
        has_add_failed=0
        issue_num=""
        prev=""
        for arg in "$@"; do
          if [ "$prev" = "--add-label" ] && [ "$arg" = "ralph:failed" ]; then
            has_add_failed=1
          fi
          # Extract issue number from positional arg (number only)
          case "$arg" in
            [0-9]*) issue_num="$arg" ;;
          esac
          prev="$arg"
        done
        if [ "$has_add_failed" = "1" ] && [ -n "${MOCK_DISPATCH_FAILURE_LOG:-}" ]; then
          echo "dispatch-failure:${issue_num}" >> "$MOCK_DISPATCH_FAILURE_LOG"
        fi
        exit 0
        ;;
      view)
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            want_labels=1
          fi
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          issue_number="${3:-0}"
          printf '{"title":"Mock issue %s","body":"Mock body for issue %s"}' "$issue_number" "$issue_number"
          exit 0
        fi
        # Comment body query
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled issue subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        has_head=0
        for arg in "$@"; do
          if [ "$arg" = "--head" ]; then
            has_head=1
          fi
        done
        if [ "$has_head" = "1" ]; then
          printf 'https://github.com/mock/repo/pull/1'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      create)
        printf 'https://github.com/mock/repo/pull/1\n'
        exit 0
        ;;
      view) printf '' ; exit 0 ;;
      edit) exit 0 ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled pr subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  api)
    if [ "$2" = "user" ]; then
      printf 'ralph-bot\n'
      exit 0
    fi
    echo "mock gh: unhandled api subcommand: $2" >&2
    exit 1
    ;;
  label)
    case "$2" in
      create) exit 0 ;;
      *)
        echo "mock gh: unhandled label subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        if [ -n "$target_dir" ]; then
          mkdir -p "$target_dir"
          git init "$target_dir" --quiet 2>/dev/null
          git -C "$target_dir" config user.email "mock@test"
          git -C "$target_dir" config user.name "MockClone"
          touch "$target_dir/.gitkeep"
          git -C "$target_dir" add .gitkeep
          git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        fi
        exit 0
        ;;
      view)
        printf 'acme/widgets\n'
        exit 0
        ;;
      *)
        echo "mock gh: unhandled repo subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "mock gh: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

// ---------------------------------------------------------------------------
// Quick-Dev mock scripts
// ---------------------------------------------------------------------------

/// Quick-dev implementer mock script (happy path).
///
/// Responds to quick-dev prompts that the implementer handles:
/// - `plan-and-implement phase` → implementation notes output + creates `mock_file.txt`
/// - `apply-fixes phase` → implementation response
/// - `final reviewer auditing` → `# Final Review: NO AMENDMENTS`
///
/// Environment variable `QUICK_DEV_FINAL_REVIEW_RESULT` controls the final-review
/// response: "NO_AMENDMENTS" (default) or "AMENDMENTS".
pub fn quick_dev_implementer_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "quick-dev plan-and-implement phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created quick-dev mock implementation.

## Spec Deviations
- None

## Testing
- Mock script only
EOF
  echo "quick-dev-implemented" > mock_file.txt
  git add mock_file.txt
elif grep -q "quick-dev apply-fixes phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Applied reviewer-requested fixes.

## Could Not Address
- None
EOF
  echo "quick-dev-fixed" >> mock_file.txt
  git add mock_file.txt
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  result="${QUICK_DEV_FINAL_REVIEW_RESULT:-NO_AMENDMENTS}"
  if [ "$result" = "AMENDMENTS" ]; then
    cat <<'EOF'
# Final Review: AMENDMENTS

## Issues
- Mock issue found by implementer final review.
EOF
  else
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
All requirements met per implementer review.
EOF
  fi
else
  echo "quick-dev-implementer: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Quick-dev reviewer mock script (happy path — review satisfied on first call).
///
/// Responds to quick-dev prompts that the reviewer handles:
/// - `quick-dev reviewer` → `# Review: SATISFIED`
/// - `final reviewer auditing` → `# Final Review: NO AMENDMENTS`
///
/// Environment variables:
/// - `QUICK_DEV_REVIEW_RESULT`: "SATISFIED" (default) or "CHANGES REQUESTED"
/// - `QUICK_DEV_FINAL_REVIEW_RESULT`: "NO_AMENDMENTS" (default) or "AMENDMENTS"
pub fn quick_dev_reviewer_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "quick-dev reviewer" <<< "$INPUT"; then
  review="${QUICK_DEV_REVIEW_RESULT:-SATISFIED}"
  if [ "$review" = "CHANGES REQUESTED" ]; then
    cat <<'EOF'
# Review: CHANGES REQUESTED

## Required Changes
- Fix mock issue in implementation.
EOF
  else
    cat <<'EOF'
# Review: SATISFIED

## Summary
Implementation looks good, no changes needed.
EOF
  fi
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  result="${QUICK_DEV_FINAL_REVIEW_RESULT:-NO_AMENDMENTS}"
  if [ "$result" = "AMENDMENTS" ]; then
    cat <<'EOF'
# Final Review: AMENDMENTS

## Issues
- Mock issue found by reviewer final review.
EOF
  else
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
All requirements met per reviewer review.
EOF
  fi
else
  echo "quick-dev-reviewer: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Quick-dev reviewer mock that always returns CHANGES REQUESTED.
/// Used to test the max-review-iterations guard.
pub fn quick_dev_reviewer_always_reject_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "quick-dev reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Review: CHANGES REQUESTED

## Required Changes
- Always-reject mock: changes requested every time.
EOF
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
Force-approved after iteration guard.
EOF
else
  echo "quick-dev-always-reject-reviewer: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Quick-dev reviewer that returns CHANGES REQUESTED on the first call,
/// then SATISFIED on subsequent calls. Uses a state file to track invocations.
///
/// Set `QUICK_DEV_REVIEW_STATE_FILE` to a path for the state file.
pub fn quick_dev_reviewer_reject_once_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"
STATE="${QUICK_DEV_REVIEW_STATE_FILE:-/tmp/quick-dev-review-state}"

if grep -q "quick-dev reviewer" <<< "$INPUT"; then
  if [ -f "$STATE" ]; then
    cat <<'EOF'
# Review: SATISFIED

## Summary
Second review pass: implementation looks good.
EOF
  else
    echo "rejected" > "$STATE"
    cat <<'EOF'
# Review: CHANGES REQUESTED

## Required Changes
- Fix the initial implementation issue.
EOF
  fi
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
All requirements met.
EOF
else
  echo "quick-dev-reject-once-reviewer: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Quick-dev final-review mock that returns AMENDMENTS on the first call,
/// then NO AMENDMENTS on subsequent calls. Used to test the final-review reloop.
///
/// Set `QUICK_DEV_FR_STATE_FILE` to a path for the state file.
pub fn quick_dev_final_review_issues_once_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"
STATE="${QUICK_DEV_FR_STATE_FILE:-/tmp/quick-dev-fr-state}"

if grep -q "quick-dev plan-and-implement phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Reloop implementation after final review issues.

## Spec Deviations
- None

## Testing
- Mock only
EOF
  echo "quick-dev-reloop" >> mock_file.txt
  git add mock_file.txt
elif grep -q "quick-dev apply-fixes phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Applied fixes.

## Could Not Address
- None
EOF
elif grep -q "quick-dev reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Review: SATISFIED

## Summary
Implementation satisfactory.
EOF
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  count=0
  if [ -f "$STATE" ]; then
    count="$(cat "$STATE")"
  fi
  count=$((count + 1))
  echo "$count" > "$STATE"
  if [ "$count" -le 2 ]; then
    cat <<'EOF'
# Final Review: AMENDMENTS

## Issues
- Mock issue requiring re-implementation.
EOF
  else
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
All requirements met after reloop.
EOF
  fi
else
  echo "quick-dev-fr-issues-once: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Quick-dev mock where final review always finds issues.
/// Used to test max-final-review-retries force-complete guard.
pub fn quick_dev_final_review_always_issues_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "quick-dev plan-and-implement phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Implementation attempt.

## Spec Deviations
- None

## Testing
- Mock only
EOF
  echo "quick-dev-force" >> mock_file.txt
  git add mock_file.txt
elif grep -q "quick-dev apply-fixes phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Applied fixes.

## Could Not Address
- None
EOF
elif grep -q "quick-dev reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Review: SATISFIED

## Summary
OK.
EOF
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: AMENDMENTS

## Issues
- Always-issues mock: perpetual issues found.
EOF
else
  echo "quick-dev-always-issues: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

/// Mock GH script for mixed-outcome dispatch isolation tests.
///
/// Behaves like `daemon_mock_gh_script()` but makes the label claim for a
/// specific issue number (`MOCK_GH_CLAIM_FAIL_ISSUE`) fail with exit 1.
/// All other issues are claimed normally.
///
/// Environment variables:
/// - `MOCK_GH_ISSUES` — JSON array of issues for `issue list`
/// - `MOCK_GH_LABEL_LOG` — file to log label operations
/// - `MOCK_GH_CLAIM_FAIL_ISSUE` — issue number whose claim should fail
pub fn daemon_mock_gh_mixed_outcome_script() -> String {
    r###"#!/bin/sh
# Mock gh for mixed-outcome dispatch isolation tests.
# MOCK_GH_CLAIM_FAIL_ISSUE: issue number whose claim should fail.

case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        for arg in "$@"; do
          case "$arg" in
            ralph:prd|ralph:prd-active) has_prd=1 ;;
          esac
        done
        if [ "$has_prd" = "1" ] && [ -n "${MOCK_PRD_TICK_LOG:-}" ]; then
          echo "prd-tick" >> "$MOCK_PRD_TICK_LOG"
        fi
        if [ -n "${MOCK_GH_ISSUES:-}" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        if [ -n "${MOCK_GH_LABEL_LOG:-}" ]; then
          echo "$@" >> "$MOCK_GH_LABEL_LOG"
        fi
        # Fail the claim for the specific issue
        if [ -n "${MOCK_GH_CLAIM_FAIL_ISSUE:-}" ]; then
          has_add_inprogress=0
          issue_num=""
          prev=""
          for arg in "$@"; do
            if [ "$prev" = "--add-label" ] && [ "$arg" = "ralph:in-progress" ]; then
              has_add_inprogress=1
            fi
            case "$arg" in
              [0-9]*) issue_num="$arg" ;;
            esac
            prev="$arg"
          done
          if [ "$has_add_inprogress" = "1" ] && [ "$issue_num" = "$MOCK_GH_CLAIM_FAIL_ISSUE" ]; then
            echo "mock gh: simulated claim failure for issue $issue_num" >&2
            exit 1
          fi
        fi
        exit 0
        ;;
      view)
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            want_labels=1
          fi
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
        if [ "$want_labels" = "1" ]; then
          if [ -n "${MOCK_GH_ISSUE_LABELS:-}" ]; then
            printf '%s' "$MOCK_GH_ISSUE_LABELS"
          else
            printf '{"labels":[]}'
          fi
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          issue_number="${3:-0}"
          printf '{"title":"Mock issue %s","body":"Mock body for issue %s"}' "$issue_number" "$issue_number"
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled issue subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/repo/pull/1\n' ; exit 0 ;;
      view) printf '' ; exit 0 ;;
      edit) exit 0 ;;
      comment) exit 0 ;;
      *)
        echo "mock gh: unhandled pr subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  api)
    if [ "$2" = "user" ]; then
      printf 'ralph-bot\n'
      exit 0
    fi
    echo "mock gh: unhandled api subcommand: $2" >&2
    exit 1
    ;;
  label)
    case "$2" in
      create) exit 0 ;;
      *)
        echo "mock gh: unhandled label subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  repo)
    case "$2" in
      view) exit 0 ;;
      *)
        echo "mock gh: unhandled repo subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "mock gh: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Quick-dev implementer mock that also creates stray impl-notes and
/// impl-response files in the worktree root (simulating the behaviour that
/// caused the infinite loop in issue #146).
pub fn quick_dev_implementer_with_stray_files_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "quick-dev plan-and-implement phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created quick-dev mock implementation.

## Spec Deviations
- None

## Testing
- Mock script only
EOF
  echo "quick-dev-implemented" > mock_file.txt
  git add mock_file.txt
  # Create stray impl artifacts at worktree root (the bug this tests)
  echo "stray notes" > 20260304120000-impl-notes.md
  echo "stray response" > 20260304120000-impl-response-001.md
elif grep -q "quick-dev apply-fixes phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Applied reviewer-requested fixes.

## Could Not Address
- None
EOF
  echo "quick-dev-fixed" >> mock_file.txt
  git add mock_file.txt
  # Create more stray files on second iteration
  echo "stray notes 2" > 20260304130000-impl-notes.md
  echo "stray response 2" > 20260304130000-impl-response-002.md
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  result="${QUICK_DEV_FINAL_REVIEW_RESULT:-NO_AMENDMENTS}"
  if [ "$result" = "AMENDMENTS" ]; then
    cat <<'EOF'
# Final Review: AMENDMENTS

## Issues
- Mock issue found by implementer final review.
EOF
  else
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
All requirements met per implementer review.
EOF
  fi
else
  echo "quick-dev-implementer: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}
