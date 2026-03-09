---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T03:59:09Z
---

The implementation is already fully complete. Every acceptance criterion from the spec is addressed:

# Implementation Summary — Soft Rollback with `.rollback-ceiling` Marker

All changes were already committed across the previous loop iterations. Here's what was implemented:

## `src/cli/rollback.rs`
- **`hard_ref` gated on `args.hard`** (line 54): `if args.hard { ... } else { None }` — soft rollback skips all git operations
- **`PushOutcome` enum** (lines 88-93): Tracks push success/failure/skipped for marker management
- **Push failure as warning** (lines 134-142): `run_git` errors captured with `eprintln!("warning: ...")` instead of `?` early return; artifact cleanup and session invalidation always execute
- **Missing branch handling** (lines 104-115): Tries `origin/<branch>` recreation before erroring
- **`.rollback-ceiling` marker management** (lines 202-235): Soft rollback writes marker; hard rollback deletes marker only on successful push; push failure retains marker
- **Dry-run messages** (lines 73-86): Distinguishes `"dry-run (soft rollback)"` vs `"dry-run (hard rollback)"`

## `src/project/lifecycle.rs`
- **`read_rollback_ceiling`** (line 402): Helper reads `.rollback-ceiling` from project dir
- **Ceiling enforcement** (lines 282-311): In `reconstruct_project_state_internal`, after checkpoint resolution:
  - Staleness check: marker is inert when `max_artifact_loop > ceiling`
  - When enforced: caps `checkpoint_loop`/`checkpoint_phase`, filters `commit_by_loop` and `loop_dirs`
  - Ceiling 0: defaults to `(1, Planning)`
- **Unit tests** (lines 1806+): `reconstruct_respects_rollback_ceiling`, `reconstruct_ignores_absent_ceiling`, `reconstruct_stale_ceiling_ignored`, `reconstruct_ceiling_zero`

## `src/validate/tests_commands.rs`
- **`rollback_with_completion_attempts`**: Updated to use `rollback --hard 1` / `rollback --hard --dry-run 1`
- **`rollback_force_push`**: Updated to use `rollback --hard --dry-run 1` / `rollback --hard 1`
- **`rollback_dry_run`**: Added soft dry-run assertions (no `git reset --hard`, contains `soft rollback`) and hard dry-run assertions
- **New `rollback_hard`**: Dedicated test for `--hard` flag with git HEAD movement
- **New `rollback_hard_missing_branch`**: Tests error when branch is missing locally and on origin
- **New `rollback_ceiling_inert_after_forward_progress`**: Verifies marker becomes inert after forward progress
- **New `rollback_push_failure_continues`**: Verifies exit 0, push-failure warning, artifact cleanup, marker retention

## `src/validate/tests_sessions.rs`
- **`session_invalidation_on_rollback`**: No changes needed — already tests soft rollback (`rollback 0` without `--hard`)
