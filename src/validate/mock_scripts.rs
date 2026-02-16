use std::path::Path;

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

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

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
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

## Manual Testing
- ran binary with test args: ok
- verified CLI output matches spec

## Automated Tests
- cargo check: ok
- cargo test: 10 passed, 0 failed

## Acceptance Criteria Verification
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
  issue)
    case "${2:-}" in
      list) printf '[]'; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
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
      view) printf 'acme/widgets\n'; exit 0 ;;
    esac
    ;;
esac

echo "mock gh: unhandled command: $*" >&2
exit 1
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

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
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
/// - `gh issue view --json title,body ...` — returns title/body JSON
/// - `gh issue view ... -q .comments[].body` — returns empty comments
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
        # Title/body fetch used by pending-task hydration.
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
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
    # Set up a local bare remote so git push works from the worktree
    bare_dir="$(pwd)/../_bare_remote.git"
    if [ ! -d "$bare_dir" ]; then
      git init --bare "$bare_dir" --quiet 2>/dev/null
    fi
    git remote remove origin 2>/dev/null
    git remote add origin "$bare_dir"

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

/// Mock `ralph` script that switches the worktree to a different branch before
/// creating a commit. Simulates how the orchestrator switches from the daemon
/// branch (`ralph/daemon/{task_id}`) to a project branch during `ralph auto`.
pub fn daemon_mock_ralph_with_branch_switch_script() -> String {
    r###"#!/bin/sh
case "$1" in
  auto)
    # Set up a local bare remote so git push works from the worktree
    bare_dir="$(pwd)/../_bare_remote.git"
    if [ ! -d "$bare_dir" ]; then
      git init --bare "$bare_dir" --quiet 2>/dev/null
    fi
    git remote remove origin 2>/dev/null
    git remote add origin "$bare_dir"

    # Switch to a different branch (simulating orchestrator behavior)
    git checkout -b ralph/mock-project-branch 2>/dev/null

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
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
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

/// Mock `ralph` script that creates a commit but makes `git diff --stat`
/// fail against the base branch, exercising the diff-stat fallback path.
/// It achieves this by removing the origin remote so diff_stat cannot
/// find a base branch, but `has_diff` still returns true via the
/// merge-base-less fallback.
pub fn daemon_mock_ralph_with_commit_no_diffstat_script() -> String {
    r###"#!/bin/sh
case "$1" in
  auto)
    # Set up a local bare remote so git push works
    bare_dir="$(pwd)/../_bare_remote.git"
    if [ ! -d "$bare_dir" ]; then
      git init --bare "$bare_dir" --quiet 2>/dev/null
    fi
    git remote remove origin 2>/dev/null
    git remote add origin "$bare_dir"

    # Create a file and commit
    echo "mock change" > ralph_daemon_change.txt
    git add ralph_daemon_change.txt
    git -c user.email="daemon@test" -c user.name="Daemon" commit -m "daemon: mock change" --quiet 2>/dev/null

    # Remove symbolic-ref to break diff --stat base detection
    git symbolic-ref --delete refs/remotes/origin/HEAD 2>/dev/null
    # Remove origin/main and origin/master refs to prevent fallback detection
    git update-ref -d refs/remotes/origin/main 2>/dev/null
    git update-ref -d refs/remotes/origin/master 2>/dev/null

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

/// Mock `gh` script for daemon auto-rebase tests.
///
/// Environment variables:
/// - `MOCK_PR_VIEW_JSON` — JSON response for `gh pr view --json ...`
/// - `MOCK_PR_VIEW_EXIT` — exit code for `gh pr view` (default: 0)
/// - `MOCK_PR_COMMENT_LOG` — file path to log pr comment bodies
pub fn daemon_mock_gh_rebase_script() -> String {
    r###"#!/bin/sh
# Mock gh for daemon auto-rebase tests.
# Env: MOCK_GH_ISSUES - JSON array of issues for `issue list`
# Env: MOCK_PR_VIEW_JSON - JSON response for `pr view --json`
# Env: MOCK_PR_VIEW_EXIT - exit code for `pr view` (default 0)
# Env: MOCK_PR_COMMENT_LOG - file to log pr comment bodies

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
      edit) exit 0 ;;
      view)
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
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
      edit) exit 0 ;;
      view)
        want_title_body=0
        for arg in "$@"; do
          if [ "$arg" = "title,body" ]; then
            want_title_body=1
          fi
        done
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

if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
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
elif echo "$INPUT" | grep -q "You are a software developer implementing a feature specification."; then
  if echo "$INPUT" | grep -q "## Review Feedback" && ! echo "$INPUT" | grep -q "(none)"; then
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

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
elif echo "$INPUT" | grep -q "You are a code reviewer ensuring implementations match specifications."; then
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
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
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

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
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
