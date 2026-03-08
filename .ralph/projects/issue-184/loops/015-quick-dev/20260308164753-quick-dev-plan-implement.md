---
artifact: quick-dev-plan-implement
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T16:47:53Z
---

Confirmed — this is a pre-existing flaky test that tries to spawn a shell script and hits `ExecutableFileBusy` (race condition with file creation). Completely unrelated to our changes.

# Implementation Summary

All 5 amendments from the final review have been addressed:

**1. [P1] TMUX-ENV-EXPORT-BROKEN** (`src/backend/tmux_backend.rs`)
- Changed `export` to emit unquoted keys (validated as POSIX shell identifiers) instead of single-quoted keys
- Added `is_valid_shell_identifier()` function that rejects invalid env key names
- Invalid keys are silently skipped (defensive)
- Updated existing tests to match new `export KEY='value'` format
- Added new tests: `valid_shell_identifiers`, `invalid_shell_identifiers`, `build_shell_command_skips_invalid_env_key`

**2. [P2] DRAIN-SHUTDOWN-WATCHER-RACE** (`src/daemon/runtime.rs`)
- `drain_all_children_with_deadline` now cancels `watcher_cancel` and `draft_pr_cancel` tokens alongside `cancel_token` in the initial cancellation loop
- Prevents draft-PR creation and artifact posting from racing with task teardown during drain

**3. [P2] Default `execute_with_cancel` warning** (`src/backend/mod.rs`)
- Added `WARNING` documentation to the default `execute_with_cancel` trait method explaining that backends spawning subprocesses MUST override it for proper cleanup

**4. [P2] `complete_task` inside `if let Some` guard** (`src/daemon/runtime.rs`)
- Moved the `complete_task` invocation (with panic isolation) inside the `if let Some(mut handle) = children.remove(&issue_number)` block
- Prevents unnecessary GitHub API calls and tokio task spawns when the handle was already collected

**5. [P3] Strengthen `dispatch_fresh_issue_passes_project_id`** (`src/validate/tests_daemon.rs`)
- Added assertion that stderr contains `--project-id issue-500` confirming the project ID was passed through
- Added conditional assertion for project state directory `issue-500/state.json` in the worktree

All tests pass (968/969 pass; 1 pre-existing flaky `ExecutableFileBusy` failure unrelated to changes).
