---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T07:46:16Z
---

# Implementation Complete — Soft/Hard Rollback

All changes are already committed. The implementation satisfies every acceptance criterion:

## What's Implemented

### `src/cli/rollback.rs`
- **`hard_ref` gated on `args.hard`** — the entire branch recovery, checkout, reset, and push block is inside `if args.hard { ... } else { None }` (line 120)
- **Push failure safety** — `PushOutcome` enum tracks success/failure/skip; push errors print a warning via `eprintln!` and do NOT early-return; artifact cleanup and session invalidation always execute afterward (lines 198-233)
- **`.rollback-ceiling` marker management** — soft rollback writes the marker (line 277); hard rollback deletes it only on push success (line 254), retains it otherwise (line 268)
- **Dry-run output** — soft dry-run prints `"dry-run (soft rollback): ..."` without `git reset --hard` (line 101); hard dry-run prints `"dry-run (hard rollback): ... git reset --hard {ref}"` (line 76)

### `src/project/lifecycle.rs`
- **`read_rollback_ceiling()`** helper reads and parses the marker file (line 402)
- **Ceiling enforcement in `reconstruct_project_state_internal`** — after checkpoint resolution, reads ceiling, applies staleness check (`checkpoint_loop > ceiling && checkpoint_loop > max_artifact_loop`), and caps position + filters `commit_by_loop` and `loop_dirs` when the marker is active (lines 282-311)
- **5 unit tests**: `reconstruct_respects_rollback_ceiling`, `reconstruct_ignores_absent_ceiling`, `reconstruct_stale_ceiling_ignored`, `reconstruct_ceiling_zero`, `reconstruct_ceiling_enforced_with_artifacts_past_ceiling`

### `src/validate/tests_commands.rs`
- `rollback_with_completion_attempts` and `rollback_force_push` use `--hard` flag for git-destructive assertions
- `rollback_dry_run` tests both soft (no `--hard`) and hard (`--hard`) dry-run output
- `rollback_hard` tests the `--hard` flag with git HEAD movement verification
- `rollback_push_failure_continues` tests that push failure doesn't block cleanup, retains ceiling marker

### `src/validate/tests_sessions.rs`
- `session_invalidation_on_rollback` uses `rollback 0` without `--hard` — correctly tests soft rollback session invalidation

No source changes are pending; all code is committed.
