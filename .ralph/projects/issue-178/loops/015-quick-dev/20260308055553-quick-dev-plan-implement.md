---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T05:55:53Z
---

# Implementation: Address Review Amendments RVW-001 and RVW-002

## Changes Made

### RVW-001 [P2]: Align dry-run hard ref resolution with execution semantics (`src/cli/rollback.rs`)

**Problem:** The dry-run hard path resolved the reset ref *before* any branch recovery, while the real execution path recovers/creates the branch first (from remote-tracking ref or fetch), then resolves. This could produce an incorrect ref preview when checkpoint commits are only visible after recovery.

**Fix:** The dry-run hard path now mirrors the execution path's branch-visibility check:
1. Resolves the project branch name
2. Checks if the branch exists locally via `branch_exists()`
3. If the branch exists: resolves the ref normally and prints it
4. If the branch needs recovery: prints a placeholder message (`<ref> (branch '...' requires recovery; exact ref unavailable in dry-run)`) instead of attempting resolution that could return a wrong value
5. If no git repo: prints a distinct message

This ensures dry-run never emits a concrete reset target unless it is derived from the same visible refs that execution would use — no side effects, no wrong refs.

### RVW-002 [P3]: Strengthen `rollback_push_failure_continues` test (`src/validate/tests_commands.rs`)

**Problem:** The test verified artifacts/sessions/marker but never asserted that `git reset --hard` actually happened. A regression skipping `reset_hard` could still pass.

**Fix:**
1. Capture the rollback target from `rollback --hard --dry-run 1` *before* removing the remote (branch exists locally, so dry-run resolves correctly per the RVW-001 fix)
2. Capture `head_before` before the rollback
3. After `rollback --hard 1` with push failure, assert `head_after != head_before` (HEAD moved) and `head_after == target_commit` (reset to correct target)
4. Assert `session_store` shape strictly — require it to be an object (`assert!(session_store.is_object(), ...)`) instead of conditional pass-through
