---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T06:23:21Z
---

# Review Amendments Implementation

## Changes Made

### [P1] Rollback ceiling staleness condition fix

**`src/project/lifecycle.rs:292`** — Changed the staleness check from:
```rust
if checkpoint_loop > ceiling && max_artifact_loop <= ceiling {
```
to:
```rust
if checkpoint_loop > ceiling && checkpoint_loop > max_artifact_loop {
```

**Problem**: The old condition disabled ceiling enforcement as soon as *any* artifact existed above the ceiling (e.g., from a failed run after rollback), even when the checkpoint was still stale. With ceiling=1, stale checkpoint at loop 3, and artifacts at loop 2, the old condition was `3 > 1 && 2 <= 1` = false, allowing the stale checkpoint to resurrect loop 3.

**Fix**: The new condition enforces the ceiling when the checkpoint exceeds both the ceiling AND the artifact level. With the same scenario: `3 > 1 && 3 > 2` = true, so the stale checkpoint is blocked. Once a genuine new checkpoint matches the artifact level, the marker becomes inert naturally.

**Regression test added**: `reconstruct_ceiling_enforced_with_artifacts_past_ceiling` — ceiling=1, stale checkpoint at loop 3, artifacts at loops 1-2. Verifies `current_loop` is capped to 1 and only loop-1 artifacts survive.

### [P2] Hard dry-run branch validation fix

**`src/cli/rollback.rs:79-92`** — When the local branch is missing in hard dry-run mode, now checks `remote_ref_exists` and `remote_branch_exists_on_remote` before printing the placeholder message. If the branch doesn't exist anywhere, returns the same `Validation` error as the real execution path.

**Test coverage**: Extended `rollback_hard_missing_branch` to assert that `rollback --hard --dry-run 1` also fails with exit code != 0 and "does not exist" in stderr when the branch is truly gone (both local and remote).

### Test Results
- 5/5 unit tests pass (`lifecycle::tests::reconstruct_*`)
- 11/11 validate tests pass (all rollback-related tests)
