## Summary

Modify the daemon to conditionally preserve worktrees and git branches when tasks fail, enabling retry from partial progress instead of restarting from scratch. Currently, `complete_task()` in `src/daemon/runtime.rs:907-910` unconditionally calls `cleanup_worktree()` for all terminal states, destroying the worktree directory, loop artifacts, project state, and uncommitted changes. Additionally, the `dispatch_task()` CAS failure path at `src/daemon/runtime.rs:546-576` unconditionally removes worktrees when a task is already terminal at activation time. Both paths must become state-aware.

The fix gates worktree cleanup on the terminal state across all three cleanup sites:

1. **`complete_task()` main path** (line 910): Skip cleanup when `terminal_state` is `Failed`.
2. **`complete_task()` early-exit CAS path** (lines 815-819): Read the stored task state and skip cleanup when it is `Failed`.
3. **`dispatch_task()` CAS failure path** (lines 546-576): Read the stored task state and skip cleanup when it is `Failed`.

`Completed` tasks are always cleaned up (work is on the remote via PR). `Aborted` tasks are always cleaned up — the requirements note this is optional, but we choose unconditional cleanup because aborted tasks represent user-initiated cancellation with no expectation of resumption. This is a deliberate product decision, not an oversight.

On retry (daemon restart), `reconcile_tasks()` resets `Failed` → `Pending` before `reconcile_worktrees()` runs, so the preserved worktree is seen as active. `dispatch_task()` reuses the existing worktree (handled by `create_worktree()` line 25-27) and routes through `spawn_ralph_run()` when `task.project_id` is set, or through `spawn_ralph_auto()` for genuinely new tasks. The dispatch routing decision is gated solely on `task.project_id` (not on discovered project IDs) to prevent fresh tasks from incorrectly taking the resume path.

## Acceptance Criteria

- `complete_task()` skips `cleanup_worktree()` when `terminal_state` is `TaskState::Failed` (main path, line ~910)
- `complete_task()` skips `cleanup_worktree()` when the stored task state is `TaskState::Failed` (early-exit CAS path, lines ~815-819)
- `dispatch_task()` skips worktree removal when the stored task state is `TaskState::Failed` (CAS failure path, lines ~546-576)
- `complete_task()` calls `cleanup_worktree()` when `terminal_state` is `TaskState::Completed` or `TaskState::Aborted`
- `persist_project_id_before_cleanup()` still runs unconditionally for all terminal states (so `project_id` is available on retry)
- `reconcile_worktrees()` preserves worktrees for tasks that were `Failed` (these become `Pending` via `reconcile_tasks()` before worktree reconciliation runs — verified by execution order at lines 140-144)
- `dispatch_task()` gates the `spawn_ralph_run` vs `spawn_ralph_auto` decision solely on `task.project_id.is_some()`, not on `effective_project_id` from discovery
- `dispatch_task()` still discovers project IDs for persistence (so retries have `project_id` set), but discovery does not influence routing
- Failed task worktrees remain on disk at `.ralph/daemon/worktrees/<task_id>/`
- Retrying a failed task that has a `project_id` resumes from the last completed loop iteration (via `ralph run --project <id> --until-complete`)
- **New tests** (all listed in Testing Strategy) pass
- All existing conformance tests continue to pass

## Technical Approach

### Change 1: Gate cleanup in `complete_task()` main path (runtime.rs:907-910)

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

`persist_project_id_before_cleanup()` runs unconditionally so the `project_id` is captured before the daemon exits or restarts.

### Change 2: Gate cleanup in `complete_task()` early-exit CAS path (runtime.rs:815-819)

The early-exit path fires when the task was already moved to a terminal state externally (e.g. aborted via `ralph daemon abort`). The current code unconditionally cleans up:

```rust
Ok(None) => {
    persist_project_id_before_cleanup(store, &workspace_root, task_id).await;
    cleanup_worktree(store, config, task_id).await;
    return;
}
```

Change to: load the task's current stored state and skip cleanup if it is `Failed`:

```rust
Ok(None) => {
    persist_project_id_before_cleanup(store, &workspace_root, task_id).await;
    // Read stored state to decide cleanup — skip for Failed tasks
    let should_cleanup = {
        let store = store.clone();
        let tid = task_id.to_owned();
        spawn_blocking_op(move || {
            let tasks = store.load()?;
            let state = tasks.iter().find(|t| t.task_id == tid).map(|t| t.state.clone());
            Ok(state != Some(TaskState::Failed))
        }).await.unwrap_or(true)
    };
    if should_cleanup {
        cleanup_worktree(store, config, task_id).await;
    }
    return;
}
```

This path can fire for any terminal state (Aborted, Completed, or Failed set by a concurrent process). When the stored state is `Failed`, the worktree is preserved for retry. For `Aborted` and `Completed`, cleanup proceeds normally.

### Change 3: Gate cleanup in `dispatch_task()` CAS failure path (runtime.rs:546-576)

The `dispatch_task()` CAS failure path fires when the task was moved to a terminal state between spawn and CAS check. The current code at lines 562-576 unconditionally removes the worktree. This path can fire when a task is concurrently aborted or (in edge cases) failed by another mechanism.

Change: read the stored task state and skip cleanup if it is `Failed`:

```rust
if !activated {
    // Task was already terminal — kill the just-spawned child
    let mut child = spawned.child;
    // ... kill/wait logic unchanged ...

    // Read stored state to decide cleanup
    let should_cleanup = {
        let store = store.clone();
        let tid = task.task_id.clone();
        spawn_blocking_op(move || {
            let tasks = store.load()?;
            let state = tasks.iter().find(|t| t.task_id == tid).map(|t| t.state.clone());
            Ok(state != Some(TaskState::Failed))
        }).await.unwrap_or(true)
    };
    if should_cleanup {
        let repo_root = config.repo_root.clone();
        let tid = task.task_id.clone();
        if let Err(err) = spawn_blocking_op(move || {
            worktree::remove_worktree(&repo_root, &workspace_root, &tid);
            Ok(())
        }).await {
            eprintln!("warning: failed to cleanup worktree for terminal-race task {}: {err}", task.task_id);
        }
    }
    return Ok(());
}
```

### Change 4: Startup reconciliation ordering (runtime.rs:140-144)

No code change required. Verified that `reconcile_tasks()` (line 140) runs before `reconcile_worktrees()` (line 144). This means `Failed` tasks are reset to `Pending` before worktree reconciliation, so their worktrees are seen as active and preserved. This ordering is critical and must not be reordered.

### Change 5: Existing worktree reuse in `create_worktree()` (worktree.rs:25-27)

No code change required. The existing early-return handles preserved worktrees:
```rust
if wt_path.exists() {
    return Ok(wt_path);
}
```

### Change 6: Fix dispatch routing gate (runtime.rs:433)

Current code gates the resume/fresh dispatch decision on `effective_project_id` (which includes discovered projects):
```rust
let spawned = if let Some(project_id) = effective_project_id.as_deref() {
```

Change to gate solely on `task.project_id`:
```rust
let spawned = if let Some(project_id) = task.project_id.as_deref() {
```

The `effective_project_id` computation (lines 417-424) and its persistence to the task store (lines 536-538) remain unchanged — discovery is still valuable for persisting the project_id so that it is available on future retries. But the routing decision must only use `task.project_id` to ensure genuinely new tasks always take the `spawn_ralph_auto` path, even when an unrelated project exists in the workspace.

### Summary of actual code changes required

| Site | File | Description |
|------|------|-------------|
| `complete_task()` main path | `runtime.rs:~910` | Gate `cleanup_worktree()` on `terminal_state != Failed` |
| `complete_task()` CAS early-exit | `runtime.rs:~815-819` | Read stored state, skip cleanup if `Failed` |
| `dispatch_task()` CAS failure | `runtime.rs:~546-576` | Read stored state, skip cleanup if `Failed` |
| `dispatch_task()` routing | `runtime.rs:~433` | Gate on `task.project_id`, not `effective_project_id` |

Everything else (worktree reuse, project_id discovery for persistence, startup reconciliation order) already works correctly.

## Files & Modules

| File | Change | Lines |
|------|--------|-------|
| `src/daemon/runtime.rs` | Gate `cleanup_worktree()` on `terminal_state != Failed` in `complete_task()` main path | ~910 |
| `src/daemon/runtime.rs` | Read stored state and gate `cleanup_worktree()` in `complete_task()` early-exit CAS path | ~815-819 |
| `src/daemon/runtime.rs` | Read stored state and gate worktree removal in `dispatch_task()` CAS failure path | ~546-576 |
| `src/daemon/runtime.rs` | Gate dispatch routing on `task.project_id` instead of `effective_project_id` | ~433 |
| `src/validate/tests_daemon.rs` | Add `runtime_task_fails_worktree_preserved` — validates that a task transitioning to `Failed` during runtime has its worktree preserved | new |
| `src/validate/tests_daemon.rs` | Add `runtime_completed_task_worktree_cleaned` — validates cleanup still happens for successful tasks | new |
| `src/validate/tests_daemon.rs` | Add `runtime_fresh_dispatch_ignores_discovered_project` — validates fresh tasks use `ralph auto --idea` even when a workspace project exists | new |

No new files. No schema changes (the `project_id` field already exists on `DaemonTask`). No changes to `worktree.rs`, `process.rs`, `github.rs`, or `mod.rs`.

## Testing Strategy

### Required new conformance tests

All new tests are added to `src/validate/tests_daemon.rs` and registered in the `tests()` function.

1. **`runtime_task_fails_worktree_preserved`** (new, required for merge)
   - Seeds a pending task with `raw_idea` set
   - Mock ralph script exits non-zero (simulating failure)
   - Runs daemon in `--single-iteration` mode
   - Asserts:
     - Task reaches `failed` state in `tasks.json`
     - Worktree directory at `.ralph/daemon/worktrees/<task_id>/` still exists on disk
   - **This directly tests the core behavior**: a task that transitions from `InProgress` → `Failed` during runtime preserves its worktree. The existing `runtime_failed_worktree_preserved_and_reused_on_retry` test starts from an already-failed task and only tests the retry path — it does not validate that `complete_task()` skips cleanup on failure.

2. **`runtime_completed_task_worktree_cleaned`** (new, required for merge)
   - Seeds a pending task
   - Mock ralph script exits 0 (success) and creates at least one commit
   - Runs daemon in `--single-iteration` mode
   - Asserts:
     - Task reaches `completed` state
     - Worktree directory has been removed
   - **Regression guard**: ensures the cleanup gate doesn't accidentally skip cleanup for successful tasks.

3. **`runtime_fresh_dispatch_ignores_discovered_project`** (new, required for merge)
   - Seeds a pending task with `project_id: null`
   - Creates exactly one valid project under `.ralph/projects/` in the workspace
   - Mock ralph logs its argv to a file and exits 0
   - Runs daemon in `--single-iteration` mode
   - Asserts:
     - Spawned command uses `auto --idea` args (not `run --project`)
     - After completion, `task.project_id` is populated in `tasks.json` (discovery-for-persistence worked)
   - **Guards against regression**: ensures fresh tasks are never routed through the resume path by discovered project IDs.

### Existing tests that must continue to pass

4. **`runtime_failed_worktree_preserved_and_reused_on_retry`** (existing, tests_daemon.rs:703-777)
   - Seeds a `failed` task with a pre-existing worktree containing `sentinel.txt`
   - Runs daemon in `--single-iteration` mode (which reconciles Failed → Pending, then dispatches)
   - Mock ralph checks for `sentinel.txt` — exits 0 if found (proving worktree reuse)
   - Asserts the task reaches `completed` state
   - **Tests the retry path end-to-end**, but does not test the initial failure-to-preserved transition.

5. **`runtime_resume_dispatch_uses_ralph_run_args`** (existing)
   - Validates that a task with `project_id` set dispatches via `ralph run --project <id> --until-complete`.

6. **`runtime_reconciliation_failed_to_pending`** (existing)
   - Validates that `Failed` tasks are reset to `Pending` during startup reconciliation.

7. **`runtime_worktree_reconcile_preserves_retryable_tasks`** (existing)
   - Validates that worktrees for retryable (Pending after reconciliation) tasks survive startup.

### Out of scope for testing

- **Loop-level resume conformance** ("fail at loop 3, retry continues from loop 3"): This behavior is a property of `ralph run --project <id> --until-complete`, which internally reads project state to determine the next loop. The daemon's responsibility is limited to: (a) preserving the worktree on failure, (b) persisting the project_id, and (c) dispatching via `spawn_ralph_run` on retry. All three are tested by the tests above. A full loop-resumption integration test would require a multi-iteration mock ralph with project state management, which is disproportionate to the daemon-level scope of this change. If desired, it belongs in `ralph run`'s own test suite.

### Verification commands

```sh
cargo check
cargo test
nix build -L
ralph validate --filter daemon
```

## Out of Scope

- **Configurable retry limits**: No max-retry counter or backoff. Failed tasks are retried once per daemon restart, indefinitely.
- **Partial loop state inspection**: The daemon does not inspect which loop iteration failed. Resume granularity is delegated to `ralph run --project <id> --until-complete`, which internally reads project state.
- **Branch deletion on cleanup**: Failed task branches remain in the local git repo even if the worktree is eventually cleaned up. Branch lifecycle management is a separate concern.
- **User-facing retry command**: No `ralph daemon retry <task_id>` CLI command. Retry happens automatically on daemon restart via reconciliation.
- **Worktree disk usage monitoring or garbage collection**: No age-based or size-based eviction of preserved worktrees.
- **Remote push of failed branches**: Failed tasks do not push their branches to the remote. Only `Completed` tasks go through the PR flow.
- **Auto-rebase of preserved worktrees**: If the base branch advances while a failed worktree sits on disk, no automatic rebase occurs (that's issue #14).
- **Configurable aborted-task cleanup policy**: Aborted tasks are always cleaned up. The requirements note this behavior is optional, but we choose unconditional cleanup: aborted tasks represent deliberate user cancellation with no expectation of later resumption. If a policy switch is needed in the future, it can be added behind a config flag.
- **Loop-level resume integration test**: Testing that `ralph run --project <id>` resumes at the correct loop iteration is the responsibility of `ralph run`'s test suite, not the daemon's. The daemon tests validate worktree preservation, project_id persistence, and correct dispatch routing, which are its contractual obligations.
