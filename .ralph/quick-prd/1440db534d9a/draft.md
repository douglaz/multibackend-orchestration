Now I have a thorough understanding of the codebase. Let me write the spec.

---

## Summary

The daemon's `create_worktree()` function unconditionally passes `-b <branch>` to `git worktree add`, which fails when the branch already exists from a previous failed run. After a daemon restart, `reconcile_tasks()` correctly resets `InProgress` tasks back to `Pending`, but if `reconcile_worktrees()` removed the worktree directory without deleting the git branch, the subsequent re-dispatch hits `fatal: a branch named 'ralph/daemon/<task_id>' already exists`. The fix requires branch-existence detection before worktree creation and branch cleanup during worktree removal.

## Acceptance Criteria

1. `create_worktree()` checks if branch `ralph/daemon/<task_id>` already exists before calling `git worktree add`
2. When the branch exists, worktree is created with `git worktree add <path> <branch>` (no `-b` flag)
3. When the branch does not exist, worktree is created with `git worktree add -b <branch> <path> HEAD` (current behavior)
4. A task that failed and was reset to `pending` can be successfully re-dispatched after daemon restart without manual `git branch -D`
5. `remove_worktree()` deletes the associated `ralph/daemon/<task_id>` branch after removing the worktree directory
6. `reconcile_worktrees()` cleans up orphaned `ralph/daemon/*` branches for stale/orphaned task IDs
7. New test: verify retry flow succeeds when a stale branch exists from a previous run

## Technical Approach

### 1. Branch existence check in `create_worktree()` (`src/daemon/worktree.rs:22-57`)

Add a helper function `branch_exists(repo_root: &Path, branch: &str) -> bool` that runs `git branch --list <branch>` and returns `true` if the output is non-empty. This is the most reliable cross-platform check.

Modify `create_worktree()` to branch on the result:

```
let branch_name = format!("ralph/daemon/{task_id}");
if branch_exists(repo_root, &branch_name) {
    // Reuse existing branch — no -b flag
    git worktree add <wt_path> <branch_name>
} else {
    // Create new branch from HEAD — current behavior
    git worktree add -b <branch_name> <wt_path> HEAD
}
```

The existing early-return at line 25-26 (if `wt_path.exists()`) is preserved — it handles the case where both the worktree directory and branch still exist (idempotent).

### 2. Branch deletion in `remove_worktree()` (`src/daemon/worktree.rs:60-92`)

After the worktree is removed (or the directory is force-deleted) and before `git worktree prune`, add:

```
git branch -D ralph/daemon/<task_id>
```

This is best-effort — failures are logged as warnings, consistent with the existing error-handling style in `remove_worktree()`. The branch may already be gone if git cleaned it up, and that's fine.

### 3. Orphaned branch cleanup in `reconcile_worktrees()` (`src/daemon/worktree.rs:102-122`)

After removing stale worktree directories, add a sweep for orphaned `ralph/daemon/*` branches:

1. Run `git branch --list "ralph/daemon/*"` to get all daemon-managed branches
2. For each branch, extract the task ID suffix
3. If the task ID is not in `active_task_ids`, run `git branch -D <branch>`

This catches branches left behind by crashes, interrupted removals, or bugs. It runs once at startup, keeping the cost negligible.

### 4. Helper function: `delete_branch()`

Add `fn delete_branch(repo_root: &Path, branch: &str)` as a best-effort helper (logs warnings, never fails) to share logic between `remove_worktree()` and `reconcile_worktrees()`.

## Files & Modules

| File | Changes |
|---|---|
| `src/daemon/worktree.rs` | Add `branch_exists()` helper; modify `create_worktree()` to conditionally use `-b`; add `delete_branch()` helper; call it from `remove_worktree()` after directory removal; add orphaned branch sweep to `reconcile_worktrees()` |
| `src/validate/tests_daemon.rs` | Add test `runtime_worktree_retry_with_stale_branch`: pre-create a `ralph/daemon/<task_id>` branch, populate a pending task, run daemon, assert dispatch succeeds and task reaches terminal state. Optionally add test for branch cleanup on removal. |

No other files need changes. The call sites in `runtime.rs` (lines 376, 542, 909) invoke `create_worktree` and `remove_worktree` with the same signatures — no API changes needed.

## Testing Strategy

**New integration test: `runtime_worktree_retry_with_stale_branch`**
1. Initialize workspace with `h.init_workspace()`
2. Pre-create branch `ralph/daemon/acme-widgets-99` via `git branch ralph/daemon/acme-widgets-99`
3. Write a pending task `acme-widgets-99` to `tasks.json`
4. Run daemon with `--single-iteration` and mock binaries
5. Assert: dispatch succeeds (stderr contains `"dispatched task acme-widgets-99"`)
6. Assert: task reaches terminal state (`completed` or `failed`)
7. Assert: worktree directory was created under `.ralph/daemon/worktrees/acme-widgets-99/`

**New integration test: `runtime_worktree_removal_cleans_branch`**
1. Initialize workspace, dispatch a task to completion (similar to existing `runtime_worktree_isolation`)
2. After task completes, verify branch `ralph/daemon/<task_id>` no longer exists via `git branch --list`

**Existing tests must continue passing:**
- `runtime_worktree_isolation` — new task with no pre-existing branch (the happy path is unchanged)
- `runtime_reconciliation_on_startup` — reconciliation still works
- `runtime_worktree_branch_switched` / `runtime_worktree_branch_unchanged` — branch detection unaffected

## Out of Scope

- **Remote branch cleanup**: Only local branches are managed. Pushed remote branches (`origin/ralph/daemon/*`) are not deleted — they may be referenced by open PRs
- **Worktree lock files**: Git worktree lock/unlock mechanisms are not addressed
- **Task state machine changes**: No changes to `TaskState` enum or state transitions
- **Concurrent daemon instances**: Branch conflicts between multiple daemon processes running simultaneously are not handled (single daemon assumption is unchanged)
- **Branch reset to HEAD**: When reusing an existing branch, the branch tip is left as-is (it may point to a commit from the previous run). The orchestrator (`ralph auto`) is responsible for handling its own git state within the worktree