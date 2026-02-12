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
