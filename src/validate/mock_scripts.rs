pub fn standard_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
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
elif echo "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if echo "$INPUT" | grep -q "## Review Feedback" && ! echo "$INPUT" | grep -q "(none)"; then
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
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif echo "$INPUT" | grep -q "You are a QA engineer"; then
  cat <<'EOF'
# QA: PASS

## Tests Run
- mock check: passed

## Verification Summary
All acceptance criteria verified by mock QA.
EOF
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
if echo "$INPUT" | grep -q "You are a senior software engineer writing a focused engineering specification."; then
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
elif echo "$INPUT" | grep -q "You are a senior engineer reviewing an engineering specification"; then
  cat <<'EOF'
```json
{"approved": true, "issues": []}
```
EOF
elif echo "$INPUT" | grep -q "You are a senior software engineer revising an engineering specification"; then
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
elif echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
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
elif echo "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if echo "$INPUT" | grep -q "## Review Feedback" && ! echo "$INPUT" | grep -q "(none)"; then
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
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
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
elif echo "$INPUT" | grep -q "You are a QA engineer validating"; then
  cat <<'EOF'
# QA: PASS

## Tests Run
- cargo check: ok
- cargo test: 10 passed, 0 failed

## Verification Summary
All acceptance criteria from the spec have been verified.
EOF
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
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

/// Mock script whose prompt reviewer response includes nested `##` headings
/// inside the `## Refined Prompt` section, exercising the extract-to-EOF parser
/// semantics. All other roles respond identically to `standard_mock_script()`.
pub fn nested_heading_prompt_review_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
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
elif echo "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if echo "$INPUT" | grep -q "## Review Feedback" && ! echo "$INPUT" | grep -q "(none)"; then
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
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
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
elif echo "$INPUT" | grep -q "You are a QA engineer"; then
  cat <<'EOF'
# QA: PASS

## Tests Run
- mock check: passed

## Verification Summary
All acceptance criteria verified by mock QA.
EOF
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
/// - `gh issue edit ...` — no-op success
/// - `gh issue view ...` — returns empty comments
/// - `gh issue comment ...` — no-op success
/// - `gh pr list ...` — returns empty
/// - `gh pr create ...` — returns a fake PR URL
/// - `gh repo view ...` — returns the configured owner/repo
///
/// Set `MOCK_GH_ISSUES` env var to a JSON array of issues for poll responses.
/// Set `MOCK_GH_OVERFLOW` to "true" to return exactly 100 issues.
pub fn daemon_mock_gh_script() -> String {
    r###"#!/bin/sh
# Mock gh for daemon runtime tests.
# Env: MOCK_GH_ISSUES - JSON array of issues for `issue list`
# Env: MOCK_GH_OVERFLOW - if "true", return 100 identical issues

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
        # Claiming / label update — always succeed
        exit 0
        ;;
      view)
        # Return empty comments body
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
      *)
        echo "mock gh: unhandled pr subcommand: $2" >&2
        exit 1
        ;;
    esac
    ;;
  repo)
    case "$2" in
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

/// Mock `ralph` script for daemon tests that simulates `ralph auto` execution.
/// It simply exits successfully immediately.
pub fn daemon_mock_ralph_script() -> String {
    r###"#!/bin/sh
# Mock ralph for daemon child process tests.
# When called as `ralph auto <idea>`, just succeed immediately.
case "$1" in
  auto)
    exit 0
    ;;
  *)
    # Pass through other commands to real ralph
    echo "mock ralph: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Mock `ralph` script that creates a commit in the worktree before exiting,
/// so `has_diff` detects divergence from the base branch.
pub fn daemon_mock_ralph_with_commit_script() -> String {
    r###"#!/bin/sh
case "$1" in
  auto)
    # Create a file and commit it so the branch diverges from base
    echo "mock change" > ralph_daemon_change.txt
    git add ralph_daemon_change.txt
    git -c user.email="daemon@test" -c user.name="Daemon" commit -m "daemon: mock change" --quiet 2>/dev/null
    exit 0
    ;;
  *)
    echo "mock ralph: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

/// Mock `ralph` script that exits with non-zero for testing failure paths.
pub fn daemon_mock_ralph_fail_script() -> String {
    r###"#!/bin/sh
case "$1" in
  auto)
    exit 1
    ;;
  *)
    echo "mock ralph: unhandled command: $1" >&2
    exit 1
    ;;
esac
"###
    .to_owned()
}

pub fn always_reject_review_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
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
elif echo "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if echo "$INPUT" | grep -q "## Review Feedback" && ! echo "$INPUT" | grep -q "(none)"; then
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
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif echo "$INPUT" | grep -q "You are a QA engineer"; then
  cat <<'EOF'
# QA: PASS

## Tests Run
- mock check: passed

## Verification Summary
All acceptance criteria verified by mock QA.
EOF
elif echo "$INPUT" | grep -q "You are a code reviewer ensuring implementations match specifications."; then
  cat <<'EOF'
# Review: SUGGESTIONS

## Required Changes
1. Tighten mock validation behavior.
EOF
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
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
