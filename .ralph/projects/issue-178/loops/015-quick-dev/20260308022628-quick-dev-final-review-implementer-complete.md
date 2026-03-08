---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T02:26:28Z
---

Everything checks out. Let me do a final comprehensive check — is there any place where `.rollback-ceiling` should be cleaned up on a normal `run` that advances past the ceiling, or is the staleness check sufficient?

The spec explicitly says: "Automatic cleanup of stale `.rollback-ceiling` by the orchestrator after a successful run advances past the ceiling" is out of scope. The staleness check makes markers inert. This is fine.

One last thing — let me verify there are no issues with `Command::new("git")` usage in tests (vs the harness's `ralph` helper). The tests use `Command::new("git")` directly for git setup operations, which is the correct pattern.

# Final Review: NO AMENDMENTS

## Summary

The implementation correctly gates destructive git operations (`git reset --hard`, `git push --force`) behind the `--hard` flag, introduces a `.rollback-ceiling` marker file for soft rollback persistence, and handles push failures gracefully.

**`src/cli/rollback.rs`** — Verified:
- `hard_ref` computation (lines 54–71) is properly gated on `args.hard`, making it `None` for soft rollbacks. The downstream `if let Some(reference) = hard_ref.as_deref()` block (line 96) is correctly unreachable for soft rollbacks.
- Push failure handling (lines 134–148) uses `if let Err(e)` instead of `?`, allowing artifact cleanup and session invalidation to proceed unconditionally.
- The `PushOutcome` enum correctly tracks three states (Succeeded, Failed, Skipped) and drives ceiling marker management.
- Ceiling marker is written for soft rollback (line 230) and for hard rollback with push failure (line 221); it is deleted only on successful hard rollback (line 207).
- Branch recovery logic (lines 104–115) correctly tries `origin/<branch>` before erroring, and the error message is clear.
- The in-memory state mutations (lines 165–200) are intentionally ephemeral — state is re-derived from disk artifacts via `reconstruct_project_state` on each access.

**`src/project/lifecycle.rs`** — Verified:
- `read_rollback_ceiling` (lines 402–407) safely parses the marker, returning `None` on any failure.
- Staleness check (line 292) correctly requires both `checkpoint_loop > ceiling` AND `max_artifact_loop <= ceiling` for enforcement. All four combinations (checkpoint above/below ceiling × artifacts above/below ceiling) produce correct behavior.
- Ceiling=0 case correctly filters out all checkpoint commits and defaults to `(1, Planning)` (lines 301–304).
- `commit_by_loop` and `loop_dirs` are properly filtered when ceiling is enforced (lines 307–309).

**`src/validate/tests_commands.rs`** — Verified:
- `rollback_with_completion_attempts` and `rollback_force_push` correctly updated to use `--hard` flag since they assert git-destructive behavior.
- `rollback_dry_run` updated with soft rollback assertions (no "git reset --hard" in output, "soft rollback" present).
- `rollback_removes_loops` and `rollback_resets_phase` correctly left unchanged — they test soft rollback behavior (no git assertions).
- New `rollback_hard_missing_branch` test properly validates error path when branch is absent both locally and on origin.
- New `rollback_ceiling_inert_after_forward_progress` test verifies staleness check works after forward progress.
- New `rollback_push_failure_continues` test correctly removes origin remote to force push failure, then verifies artifacts are cleaned, ceiling marker retained, and exit code is 0.

No bugs, race conditions, resource leaks, security issues, dead code, or stray changes found.
