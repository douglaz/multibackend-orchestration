---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T03:23:10Z
---

# Final Review: NO AMENDMENTS

## Summary

The implementation correctly introduces soft rollback (default) and gates destructive git operations behind `--hard`. All changes are confined to the three expected files with no stray modifications.

**`src/cli/rollback.rs`** — Verified:
- `hard_ref` computation is properly gated on `args.hard` (line 54). The `if args.hard { ... } else { None }` guard eliminates all git operations for soft rollback since the downstream `if let Some(reference) = hard_ref.as_deref()` block (line 96) becomes unreachable.
- Push failure handling is correct: `run_git(["push", "--force", ...])` error is captured with `if let Err(e)` (line 134), sets `push_outcome = PushOutcome::Failed`, prints a warning, and does NOT early-return. Artifact cleanup (lines 151–163) and session invalidation (lines 171–186) execute unconditionally regardless of push outcome.
- `.rollback-ceiling` marker lifecycle is correct: written on soft rollback (line 230), deleted on successful hard push (line 207), retained on failed/skipped push (line 221). The `PushOutcome` enum cleanly tracks all three states.
- Missing branch handling is robust: checks `branch_exists`, falls back to `remote_ref_exists` to recreate from `origin/`, returns a clear validation error if neither exists (lines 104–114).
- Dry-run output correctly distinguishes soft vs. hard (lines 73–85), and neither path mutates any state.

**`src/project/lifecycle.rs`** — Verified:
- `read_rollback_ceiling` (line 402) is a clean read-only helper. `.ok()` + `.and_then(parse)` silently ignores absent/malformed markers.
- Staleness check (line 292): `checkpoint_loop > ceiling && max_artifact_loop <= ceiling` correctly identifies when the marker should be enforced (stale checkpoint commits present, no forward-progress artifacts beyond ceiling). When artifacts advance past ceiling, the marker is inert — no capping occurs.
- Position capping (lines 294–309): filters `checkpoint_commits` by `loop_number <= ceiling`, takes `.first()` (most recent, matching `derive_position` ordering from `git log`), and falls back to `(1, Planning)` when no commits remain (ceiling=0 case). `commit_by_loop` and `loop_dirs` are consistently filtered to `<= ceiling`.
- The `checkpoint_commits` vector itself is not mutated, only filtered via iterators. The later `!checkpoint_commits.is_empty()` check at line 364 correctly indicates "git context exists" (presence of any checkpoint commits regardless of ceiling), which is the intended semantics.

**`src/validate/tests_commands.rs`** — Verified:
- `rollback_with_completion_attempts` and `rollback_force_push`: correctly updated to use `--hard` flag since they assert git HEAD movement and force-push behavior.
- `rollback_removes_loops` and `rollback_resets_phase`: correctly do NOT use `--hard` — they only assert artifact/state changes, which soft rollback provides.
- `rollback_dry_run`: tests both soft dry-run (asserts no "git reset --hard" in output, asserts "soft rollback") and hard dry-run (asserts "git reset --hard" present, asserts "hard rollback"), with HEAD-unchanged verification for both.
- New `rollback_hard_missing_branch`: properly deletes both local branch and tracking ref, verifies hard rollback fails with descriptive error and HEAD is untouched.
- New `rollback_ceiling_inert_after_forward_progress`: validates soft rollback writes marker, reconstruction caps state, subsequent run advances past ceiling making marker inert.
- New `rollback_push_failure_continues`: removes origin remote to force push failure, verifies exit 0, stderr warning, artifact cleanup proceeds, and ceiling marker is retained.

**`src/validate/tests_sessions.rs`** — `session_invalidation_on_rollback` unchanged, correctly tests soft rollback (no `--hard` flag, asserts sessions and loops empty after `rollback 0`).

All imports (`create_branch`, `remote_ref_exists`) are used. No dead code introduced. The `PushOutcome` enum is locally scoped (function-level), which is idiomatic for single-use discriminants.
