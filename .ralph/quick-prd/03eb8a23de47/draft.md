Now I have a complete understanding of the codebase. Let me write the spec.

---

## Summary

Modify the daemon to conditionally preserve worktrees and git branches when tasks fail, enabling retry from partial progress instead of restarting from scratch. Currently, `complete_task()` in `src/daemon/runtime.rs:910` unconditionally calls `cleanup_worktree()` for all terminal states, destroying the worktree directory, loop artifacts, project state, and uncommitted changes. Similarly, `reconcile_worktrees()` at startup removes worktrees for all terminal tasks (including `Failed`), preventing retry from preserving any state.

The fix gates worktree cleanup on the terminal state: `Completed` tasks get cleaned up (work is on the remote via PR), `Failed` tasks are preserved for resumption, and `Aborted` tasks are cleaned up. On retry (daemon restart), a `Failed` task is reconciled to `Pending`, and `dispatch_task()` reuses the existing worktree directory (already handled by `create_worktree()` line 25-27) and routes through `spawn_ralph_run()` with the persisted `project_id` to resume from the last completed loop.

## Acceptance Criteria

- `complete_task()` skips `cleanup_worktree()` when `terminal_state` is `TaskState::Failed`
- `complete_task()` calls `cleanup_worktree()` when `terminal_state` is `TaskState::Completed` or `TaskState::Aborted`
- `reconcile_worktrees()` preserves worktrees for `Failed` tasks (includes their IDs in the active set)
- `persist_project_id_before_cleanup()` still runs for `Failed` tasks (so `project_id` is available on retry)
- `dispatch_task()` uses `spawn_ralph_run(project_id)` when retrying a failed task with a persisted `project_id`
- `dispatch_task()` uses `spawn_ralph_auto(idea)` for genuinely new tasks
- Failed task worktrees remain on disk at `.ralph/daemon/worktrees/<task_id>/`
- Retrying a failed task that has a `project_id` resumes from the last completed loop iteration (via `ralph run --project <id> --until-complete`)
- Test: `runtime_failed_worktree_preserved_and_reused_on_retry` passes — task fails, worktree is preserved, retry reuses worktree and sentinel file, task completes

## Technical Approach

### Change 1: Gate cleanup in `complete_task()` (runtime.rs:907-910)

Current code (lines 907-910):
```rust
persist_project_id_before_cleanup(store, &workspace_root, task_id).await;
cleanup_worktree(store, config, task_id).await;
```

Change to:
```rust
persist_project_id_before_cleanup(store, &workspace_root, task_id).await;
if terminal_state != TaskState::Failed {
    cleanup_worktree(store, config, task_id).await;
}
```

This is the core change. `persist_project_id_before_cleanup()` must still run unconditionally for `Failed` tasks so the `project_id` is captured in `tasks.json` before the daemon exits or restarts. The worktree is left in place, preserving all commits, project state, and loop artifacts.

### Change 2: Gate cleanup in the early-exit CAS path (runtime.rs:815-819)

The early-exit path (task already terminal) also unconditionally cleans up. This path fires when e.g. the child exits but the task was already `Aborted` externally. We need to read the existing task state and only skip cleanup if the stored state is `Failed`:

Current code (lines 815-819):
```rust
Ok(None) => {
    persist_project_id_before_cleanup(store, &workspace_root, task_id).await;
    cleanup_worktree(store, config, task_id).await;
    return;
}
```

Change to: load the task's current state, and skip cleanup if it's `Failed`. This requires reading the task from the store to check its state before deciding on cleanup.

### Change 3: Preserve Failed worktrees during startup reconciliation (runtime.rs:222-236)

Currently, `reconcile_worktrees()` builds the active set from only non-terminal tasks. After `reconcile_tasks()` resets `Failed` → `Pending`, the task is no longer terminal, so by execution order this already works correctly — `reconcile_tasks()` runs first (line 140), converting `Failed` to `Pending`, then `reconcile_worktrees()` runs (line 144), which sees those tasks as `Pending` (non-terminal) and preserves their worktrees.

**Verify** this ordering is correct and no race exists. If `reconcile_worktrees` were called before `reconcile_tasks`, Failed worktrees would be deleted. The current call order in `run()` (lines 140-144) is:
```rust
reconcile_tasks(store)?;        // line 140
reconcile_worktrees(store, config)?;  // line 144
```

This ordering is correct — no change needed here. Failed tasks become Pending before worktree reconciliation runs.

### Change 4: Existing worktree reuse in `create_worktree()` (worktree.rs:25-27)

Already handled:
```rust
if wt_path.exists() {
    return Ok(wt_path);
}
```

No change needed. When `dispatch_task()` calls `create_worktree()` for a retried task whose worktree was preserved, it returns the existing path immediately.

### Change 5: Project ID routing in `dispatch_task()` (runtime.rs:417-502)

Already handled. The dispatch logic (lines 417-436) already checks `task.project_id` (or discovers it) and routes to `spawn_ralph_run()` when a project_id exists, or `spawn_ralph_auto()` when it doesn't. Since `persist_project_id_before_cleanup()` runs for Failed tasks (Change 1), the project_id will be available on retry. No change needed.

### Summary of actual code changes required

Only **`complete_task()`** needs modification — two sites within the same function. Everything else (worktree reuse, project_id discovery, startup reconciliation order, dispatch routing) already works correctly.

## Files & Modules

| File | Change | Lines |
|------|--------|-------|
| `src/daemon/runtime.rs` | Gate `cleanup_worktree()` on `terminal_state != Failed` in `complete_task()` main path | ~910 |
| `src/daemon/runtime.rs` | Gate `cleanup_worktree()` on stored state != `Failed` in `complete_task()` early-exit CAS path | ~815-819 |
| `src/validate/tests_daemon.rs` | Verify `runtime_failed_worktree_preserved_and_reused_on_retry` test passes | ~703-777 |

No new files. No schema changes (the `project_id` field already exists on `DaemonTask`). No changes to `worktree.rs`, `process.rs`, `github.rs`, or `mod.rs`.

## Testing Strategy

1. **Existing test: `runtime_failed_worktree_preserved_and_reused_on_retry`** (tests_daemon.rs:703-777)
   - Seeds a `failed` task with a pre-existing worktree containing a `sentinel.txt` file
   - Runs daemon in `--single-iteration` mode
   - Mock ralph script checks for `sentinel.txt` — exits 0 if found (proving worktree reuse), exits 31 if missing
   - Asserts the reuse log says "reused" and the task reaches `completed`
   - This test should pass after the changes

2. **Regression test: Completed tasks still get cleaned up**
   - Seed a task that succeeds (mock ralph exits 0)
   - Assert worktree directory is removed after completion
   - Ensures the gate doesn't accidentally skip cleanup for successful tasks

3. **Regression test: Aborted tasks still get cleaned up**
   - Seed an aborted task with a worktree
   - Run daemon startup reconciliation
   - Assert worktree is removed

4. **Unit test: project_id persists for failed tasks**
   - Seed a task without `project_id`, mock ralph writes an `active-project` marker then exits non-zero
   - Assert `tasks.json` has `project_id` populated after failure
   - Assert worktree still exists

5. **Manual verification**: Run daemon against a real repo, trigger a failure (e.g., kill the child process), verify:
   - Worktree directory remains at `.ralph/daemon/worktrees/<task_id>/`
   - `tasks.json` shows `failed` state with `project_id` populated
   - Restarting daemon retries with `ralph run --project <id> --until-complete`

## Out of Scope

- **Configurable retry limits**: No max-retry counter or backoff. Failed tasks are retried once per daemon restart, indefinitely.
- **Partial loop state inspection**: The daemon does not inspect which loop iteration failed. Resume granularity is delegated to `ralph run --project <id> --until-complete`, which internally reads project state.
- **Branch deletion on cleanup**: Failed task branches remain in the local git repo even if the worktree is eventually cleaned up. Branch lifecycle management is a separate concern.
- **User-facing retry command**: No `ralph daemon retry <task_id>` CLI command. Retry happens automatically on daemon restart via reconciliation.
- **Worktree disk usage monitoring or garbage collection**: No age-based or size-based eviction of preserved worktrees.
- **Remote push of failed branches**: Failed tasks do not push their branches to the remote. Only `Completed` tasks go through the PR flow.
- **Auto-rebase of preserved worktrees**: If the base branch advances while a failed worktree sits on disk, no automatic rebase occurs (that's issue #14).