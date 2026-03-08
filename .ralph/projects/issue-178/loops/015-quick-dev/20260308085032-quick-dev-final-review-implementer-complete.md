---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T08:50:32Z
---

The `rollback_resets_phase` test at line 339 calls `rollback 0` (soft), and at line 354 asserts `current_loop == 1` and `current_phase == "planning"`. This verifies the acceptance criterion through the `load_state` path (which calls `reconstruct_project_state`).

I've completed my review. Here's my assessment:

# Final Review: NO AMENDMENTS

## Summary

The implementation correctly introduces soft rollback (default) and gates destructive git operations behind `--hard`. All key behaviors are verified:

**`src/cli/rollback.rs`** — The `hard_ref` computation is properly gated on `args.hard` (line 143). Push failures are caught and surfaced as warnings without blocking artifact cleanup or session invalidation (lines 192-200). The `.rollback-ceiling` marker is written for soft rollbacks and retained on push failure for hard rollbacks (lines 267-300). The disk-scan union (lines 57-74) correctly ensures `rollback 0` removes all loop artifacts even when ceiling-capped reconstruction hides some from state. Dry-run output properly distinguishes soft vs hard (lines 79-129). Branch recovery from remote handles stale local tracking refs by using `remote_branch_exists_on_remote` which queries the actual remote via `git ls-remote` (lines 151-163).

**`src/git/branch.rs`** — The new `remote_branch_exists_on_remote` function (lines 79-90) correctly uses `git ls-remote --exit-code` to check branch existence on the actual remote server, avoiding false positives from stale local tracking refs.

**`src/project/lifecycle.rs`** — The ceiling enforcement logic at lines 282-310 is correct: it caps checkpoint-derived position only when the checkpoint is above both the ceiling AND all artifact loop directories (`checkpoint_loop > ceiling && checkpoint_loop > max_artifact_loop`). This ensures the marker becomes inert once a new run produces artifacts at or above the checkpoint level. The `read_rollback_ceiling` helper (lines 402-407) gracefully handles missing/malformed files. The `ceiling == 0` case correctly defaults to `(1, Planning)`. Five thorough unit tests verify: ceiling enforcement, absent ceiling, stale ceiling, ceiling zero, and ceiling with artifacts past ceiling.

**`src/validate/tests_commands.rs`** — Existing tests `rollback_with_completion_attempts` and `rollback_force_push` are correctly updated to use `--hard`. Four new validate tests cover: branch recovery from remote + truly-missing branch error (`rollback_hard_missing_branch`), ceiling becoming inert after forward progress (`rollback_ceiling_inert_after_forward_progress`), push failure continuation (`rollback_push_failure_continues`), stale tracking ref rejection (`rollback_hard_stale_tracking_ref`), and disk-scan union for ceiling-hidden loops (`rollback_removes_ceiling_hidden_loops`). Existing soft-rollback tests (`rollback_removes_loops`, `rollback_resets_phase`, `session_invalidation_on_rollback`) are unchanged and continue to verify soft rollback behavior correctly.

No dead code, no unused imports, no stray files, no race conditions, and no security issues identified. The change is exactly scoped to the 4 files listed in the spec.
