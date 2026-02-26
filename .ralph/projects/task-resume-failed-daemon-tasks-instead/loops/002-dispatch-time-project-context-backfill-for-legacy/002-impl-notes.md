# Implementation Notes

## Decisions Made
- Placed `discover_project_ids()` as a private function in `runtime.rs` rather than in `worktree.rs`, since it is dispatch-specific logic (scanning for project context) not general worktree management. It is only called from `dispatch_task()`.
- Used `effective_project_id` local variable to carry the backfilled project_id through the rest of `dispatch_task()`, avoiding mutation of the borrowed `&DaemonTask`. The spawn decision switches on `effective_project_id.as_deref()` instead of `task.project_id.as_deref()`.
- Discovery runs after worktree creation and cleaning but before raw idea resolution, ensuring the `.ralph/projects/` directory reflects the state of the preserved worktree (post-clean, which excludes `.ralph/`).
- Persist failure aborts dispatch via `return Err(err)`, which causes the caller (`poll_and_claim`) to mark the task as Failed. This satisfies AC4 (task not moved to running state).
- Structured log events use the pattern `event=<name>` consistent with existing retrigger logs (e.g., `event=claim_failure`, `event=cas_conflict`).

## Spec Deviations
- None. All acceptance criteria are implemented as specified.

## Testing
- `discover_project_id_ignores_dirs_without_state_json`: Sets up a worktree with one valid project (has `state.json`) and one stray directory (no `state.json`). Verifies only the valid project is discovered and dispatched via `ralph run --project`. Also verifies `project_id` is persisted to the task store.
- `runtime_dispatch_backfills_legacy_failed_task_project_id`: Sets up a legacy failed task (no `project_id`) retriggered to pending, with a preserved worktree containing one valid project and prior loop artifacts. Verifies: (1) `project_backfill` structured log emitted, (2) spawn uses `ralph run --project`, (3) `project_id` persisted to store, (4) prior artifacts preserved.
- To run: `cargo test` or the project's validate runner targeting `daemon::discover_project_id_ignores_dirs_without_state_json` and `daemon::runtime_dispatch_backfills_legacy_failed_task_project_id`.
