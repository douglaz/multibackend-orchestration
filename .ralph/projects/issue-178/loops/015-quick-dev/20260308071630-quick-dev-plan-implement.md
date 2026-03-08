---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T07:16:30Z
---

# Implementation Complete: Soft Rollback Default with `--hard` Gate

All acceptance criteria are already implemented and committed on the `ralph/issue-178` branch. Here's a verification summary of each requirement:

## Acceptance Criteria Verification

| Criterion | Status | Evidence |
|---|---|---|
| `rollback` without `--hard` → soft rollback | Done | `rollback.rs:120` — `let hard_ref = if args.hard { ... } else { None }` |
| `rollback --hard` → full destructive rollback | Done | `rollback.rs:120-196` — git reset + force push when `args.hard` is true |
| `hard_ref` only computed when `args.hard` is true | Done | `rollback.rs:120` — guarded by `if args.hard` |
| Soft rollback writes `.rollback-ceiling` marker | Done | `rollback.rs:277` — `fs::write(&ceiling_path, args.loop_number.to_string())` |
| Staleness check prevents stale markers from capping | Done | `lifecycle.rs:292` — `if checkpoint_loop > ceiling && checkpoint_loop > max_artifact_loop` |
| Hard rollback deletes marker only on successful push | Done | `rollback.rs:252-254` — `if push_outcome == PushOutcome::Succeeded { let _ = fs::remove_file(...) }` |
| `rollback 0` → `current_loop=1, current_phase=planning` | Done | `rollback.rs:235-237` + `lifecycle.rs:301-304` |
| Push failures don't prevent cleanup/invalidation | Done | `rollback.rs:174-182` — push error captured as warning via `PushOutcome::Failed` |
| Existing tests updated to use `--hard` flag | Done | `tests_commands.rs` — `rollback_with_completion_attempts`, `rollback_force_push`, `rollback_hard` all use `--hard` |
| `session_invalidation_on_rollback` passes as-is | Done | `tests_sessions.rs:266` — uses `rollback 0` without `--hard` |
| Dry-run distinguishes soft vs. hard | Done | `rollback.rs:54-106` — separate branches for `args.hard` true/false |

## Files Changed

| File | Changes |
|---|---|
| `src/cli/rollback.rs` | Gated `hard_ref` on `args.hard`; `PushOutcome` enum for push tracking; `.rollback-ceiling` marker management; distinct soft/hard dry-run + output messages; branch recovery from remote for hard rollback |
| `src/project/lifecycle.rs` | `read_rollback_ceiling` helper; ceiling enforcement with staleness check in `reconstruct_project_state_internal`; 5 unit tests for ceiling behavior |
| `src/validate/tests_commands.rs` | Updated existing tests to use `--hard` where git-destructive; new `rollback_push_failure_continues` test; updated `rollback_dry_run` to verify both soft and hard modes |
| `src/git/branch.rs` | Added `remote_branch_exists_on_remote` for branch recovery during hard rollback |

## New Tests

- **Unit tests** (`lifecycle.rs`): `reconstruct_respects_rollback_ceiling`, `reconstruct_ignores_absent_ceiling`, `reconstruct_stale_ceiling_ignored`, `reconstruct_ceiling_zero`, `reconstruct_ceiling_enforced_with_artifacts_past_ceiling`
- **Validate tests** (`tests_commands.rs`): `rollback_push_failure_continues`, updated `rollback_dry_run` with soft/hard assertions

No further code changes are needed — the implementation is complete.
