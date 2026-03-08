---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T05:40:44Z
---

All changes are consistent and correctly updated to use `--hard`.

# Final Review: NO AMENDMENTS

## Summary

The implementation correctly introduces soft rollback (default) and gates destructive git operations behind `--hard`. All four changed files are within scope and properly implement the specified behavior.

**`src/cli/rollback.rs`** — Verified that `hard_ref` computation is gated on `args.hard` in both the dry-run and live paths. The `PushOutcome` enum correctly tracks push state, and push failures are caught with `if let Err(e)` rather than `?`, allowing artifact cleanup and session invalidation to proceed unconditionally. The `.rollback-ceiling` marker is written on soft rollback and on hard rollback when push fails/skipped; it is deleted only when hard rollback push succeeds. Dry-run messages correctly distinguish soft vs. hard. The branch recovery logic (lines 107-123) correctly handles missing local branch by trying the remote-tracking ref first, then `ls-remote` to the actual remote.

**`src/project/lifecycle.rs`** — The ceiling enforcement logic at lines 285-311 is correct: it reads the marker, computes `max_artifact_loop` from `loop_dirs`, and only enforces capping when `checkpoint_loop > ceiling && max_artifact_loop <= ceiling`. This correctly makes stale markers inert. The `capped.first()` returns the most recent commit at or below the ceiling (git log order is newest-first), and the `(1, Planning)` default when `capped` is empty correctly handles ceiling=0. Both `commit_by_loop` and `loop_dirs` are filtered to the ceiling, ensuring no artifacts above the ceiling are reconstructed. The `read_rollback_ceiling` helper safely returns `None` on file-not-found or parse failure.

**`src/git/branch.rs`** — `remote_branch_exists_on_remote` correctly uses `git ls-remote --exit-code` to query the actual remote rather than the local tracking ref.

**`src/validate/tests_commands.rs`** — All existing git-destructive tests (`rollback_with_completion_attempts`, `rollback_force_push`) are updated to use `--hard`. Soft rollback tests (`rollback_removes_loops`, `rollback_resets_phase`) are unchanged and correctly test the new default soft behavior. New tests (`rollback_hard_missing_branch`, `rollback_ceiling_inert_after_forward_progress`, `rollback_push_failure_continues`) cover branch recovery, staleness, and push failure scenarios with appropriate assertions. The `rollback_dry_run` test now verifies both soft (no `git reset --hard` in output) and hard (contains `git reset --hard`) dry-run output.

Unit tests in `lifecycle.rs` (`reconstruct_respects_rollback_ceiling`, `reconstruct_ignores_absent_ceiling`, `reconstruct_stale_ceiling_ignored`, `reconstruct_ceiling_zero`) thoroughly verify the ceiling enforcement logic including the staleness check, absent-marker case, and the ceiling=0 edge case that defaults to `(1, Planning, Pending)`.

No race conditions (project lock is held), no resource leaks, no dead code, and no stray changes outside scope.
