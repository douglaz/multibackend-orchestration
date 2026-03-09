---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T08:35:12Z
---

The changes look correct. Let me provide the response.

# Fix: rollback cleanup now scans disk to find ceiling-hidden loop directories

## Changes

### 1. `src/cli/rollback.rs` — Disk-scan union for `to_remove`

**Problem:** `to_remove` was derived solely from reconstructed state (`state.loops` + `state.completion_attempts`). When reconstruction caps state via `.rollback-ceiling` (filtering `loop_dirs.retain(|(n,_,_)| *n <= ceiling)`), loop directories above the ceiling are invisible to state, so `to_remove` misses them. This means `rollback 0` can leave artifact directories on disk.

**Fix:** After collecting state-derived loop numbers into a `HashSet`, scan `{project_dir}/loops/` on disk and add any directory loop numbers `> args.loop_number` to the set. Convert to a sorted `Vec` for the existing deletion and session-invalidation loops. This ensures both artifact deletion and `session_store.remove_for_loop(...)` cover all loop numbers regardless of ceiling capping.

### 2. `src/validate/tests_commands.rs` — New conformance test `rollback_removes_ceiling_hidden_loops`

The test:
1. Runs 2 loops (checkpoint at loop 2, artifacts for loops 1 & 2)
2. Manually removes loop-2 artifacts (so `max_artifact_loop = 1`)
3. Writes `.rollback-ceiling = 0` (simulating a stale ceiling)
4. Reconstruction now enforces ceiling: `checkpoint_loop (2) > ceiling (0)` AND `checkpoint_loop (2) > max_artifact_loop (1)` → loop dirs filtered to ≤ 0, hiding loop-1 from state
5. Runs `rollback 0` and asserts all loop dirs are removed and state shows `current_loop=1, current_phase=planning`
