---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-07T20:13:33Z
---

# Apply-fixes: reviewer-requested changes

Both reviewer issues have been addressed:

## 1. High: Fixed dead staleness logic in `.rollback-ceiling` enforcement

**File:** `src/project/lifecycle.rs:291-296`

The old code had a `stale` variable defined as `checkpoint_loop <= ceiling && max_artifact_loop <= ceiling`, but then gated enforcement on `!stale && checkpoint_loop > ceiling`. Since enforcement only runs when `checkpoint_loop > ceiling`, the first condition of `stale` (`checkpoint_loop <= ceiling`) was always false in that branch, making `!stale` always true — the ceiling would permanently cap progress.

**Fix:** Replaced the two-step `stale` + compound guard with a single condition: `checkpoint_loop > ceiling && max_artifact_loop <= ceiling`. This correctly enforces capping only when checkpoint commits have advanced past the ceiling (stale commits from before rollback) AND no artifact directories exist beyond the ceiling (no successful forward progress). When artifacts have advanced past the ceiling, the marker is inert and reconstruction sees the full uncapped state.

## 2. Medium: Added two new validate tests

**File:** `src/validate/tests_commands.rs`

- **`rollback_ceiling_inert_after_forward_progress`**: Soft-rollbacks to loop 1, runs again to create artifacts past the ceiling, verifies reconstruction ignores the stale marker and returns the full state.
- **`rollback_push_failure_continues`**: Removes the `origin` remote to force push failure, runs `rollback --hard 1`, verifies: exit code 0, stderr contains push-failure warning, loop-2 artifacts removed, loop-1 artifacts retained, `.rollback-ceiling` marker retained.

All 10 rollback validate tests pass (8 existing + 2 new).
