---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-07T19:58:37Z
---

# Summary

All changes implemented and passing (386/386 conformance tests):

## Files Changed

### `src/cli/rollback.rs`
- **Gated `hard_ref` on `args.hard`**: The `resolve_hard_reset_ref` call and all git operations (reset, force-push) now only execute when `--hard` is passed
- **Push failure as warning**: `run_git(["push", "--force", ...])` error is captured via `match`, printed as `eprintln!("warning: ...")`, and tracked via `push_failed` flag — no early return, so artifact cleanup and session invalidation always execute
- **`.rollback-ceiling` marker lifecycle**:
  - Soft rollback: writes `{loop_number}` to `.rollback-ceiling`
  - Hard rollback + push success: deletes `.rollback-ceiling`
  - Hard rollback + push failure: writes/retains `.rollback-ceiling` to guard against checkpoint resurrection
- **Dry-run messages**: Prefix distinguishes `(hard rollback)` vs `(soft rollback)`, with reset ref only shown for hard
- **Output messages**: `soft-rolled back ... (no git reset)` vs existing hard rollback format, plus push-failure warning when applicable

### `src/project/lifecycle.rs`
- **`read_rollback_ceiling()`**: Reads and parses `.rollback-ceiling` from project dir
- **Ceiling enforcement in `reconstruct_project_state_internal`**: After checkpoint resolution and loop dir collection:
  1. Staleness check: marker is inert when `checkpoint_loop <= ceiling` AND `max_artifact_loop <= ceiling`
  2. When active (`checkpoint_loop > ceiling`): filters checkpoint commits to `<= ceiling`, re-derives position from filtered list (defaults to `(1, Planning)` when empty), and filters `commit_by_loop` and `loop_dirs`

### `src/validate/tests_commands.rs`
- `rollback_with_completion_attempts`: dry-run and rollback calls updated to use `--hard`
- `rollback_force_push`: dry-run call updated to use `--hard`
- `rollback_dry_run`: Added assertions that soft dry-run does NOT contain `git reset --hard` and DOES contain `soft rollback`
