## Summary

When a daemon task fails and the GitHub issue is re-labeled `ralph:ready`, the daemon currently skips it because the task already exists in the store (`existing_ids.contains(&task_id)` at `runtime.rs:306`). The failed task is never re-dispatched. There is no mechanism to transition a failed task back to `Pending` and re-dispatch it — either fresh or as a resume.

This feature adds three capabilities:
1. **Re-trigger**: When the daemon detects a `ralph:ready` label on an issue whose task is in `Failed` state, transition it back to `Pending` and re-dispatch it.
2. **Project discovery**: After a child process completes (success or failure), discover the `project_id` from the worktree's `.ralph/projects/` directory and persist it to the task store. This ensures subsequent retries use `ralph run --project <id>` (the resume path) instead of `ralph auto --idea` (the fresh path).
3. **Dispatch-time backfill**: Before dispatching a failed task that has `project_id: None`, attempt to discover the project from the existing worktree. This covers legacy failed tasks where `project_id` was never persisted, avoiding branch collisions on the first re-trigger.

The dispatch path (`runtime.rs:523`) already branches on `task.project_id`: if set, it calls `spawn_ralph_run`; if `None`, it calls `spawn_ralph_auto`. The worktree module (`worktree.rs:26-28`) already reuses existing worktree directories, and `clean_worktree` (`worktree.rs:88-128`) already strips dirty state outside `.ralph/`. The primary gaps are that `project_id` is never backfilled into the task store after the child runs, failed tasks are never re-enqueued, and stale git worktree metadata can prevent worktree recreation.

## Acceptance Criteria

- [ ] When a failed task's issue has `ralph:ready` (and `ralph:failed` removed), the daemon transitions the task from `Failed` to `Pending` and re-dispatches it in the same poll cycle.
- [ ] Failed tasks with an existing `project_id` resume via `ralph run --project <id>` (verified by mock ralph receiving `["run", "--project", "<id>"]`).
- [ ] Failed tasks without a `project_id` (project creation itself failed) re-dispatch fresh via `ralph auto --idea`.
- [ ] After any child exit (success or failure), the daemon discovers the `project_id` from `.ralph/projects/` inside the worktree and persists it to the task store.
- [ ] Legacy failed tasks with `project_id: None` but an existing worktree containing `.ralph/projects/<id>/state.json` get their `project_id` backfilled at dispatch time, before the child is spawned.
- [ ] Existing project branches are checked out and reused, not recreated (no branch collision errors) — already handled by `worktree::create_worktree`, validated by test.
- [ ] Stale uncommitted files in worktree are cleaned before resuming — already handled by `worktree::clean_worktree` in `dispatch_task`, validated by test.
- [ ] Fresh dispatch via `ralph auto` still works for issues with no prior runs and no existing task in the store.
- [ ] The `ralph:in-progress` label is re-added when a failed task is re-triggered.
- [ ] No duplicate tasks: re-triggering updates the existing task record, not creates a new one.
- [ ] If the GitHub claim fails during re-trigger, the task remains in `Failed` state (not stranded in `Pending`).
- [ ] Stale git worktree metadata (`.git/worktrees/<name>` pointing to a removed directory) is pruned before worktree creation, preventing `git worktree add` failures.
- [ ] Prior planning and implementation artifacts (`.ralph/projects/<id>/loops/`, `state.json`) are preserved across retries and continued by `ralph run`.

## Technical Approach

### Change 1: Add `discover_project_id` helper to `worktree.rs`

**Where**: `daemon/worktree.rs`

**What**: Add a function `discover_project_id(workspace_root: &Path, task_id: &str) -> Option<String>` that scans `.ralph/projects/` inside the task's worktree. A project directory is only considered valid if it contains a `state.json` file. This keeps filesystem discovery logic in the worktree module alongside `create_worktree` and `clean_worktree`.

```
pub fn discover_project_id(workspace_root: &Path, task_id: &str) -> Option<String> {
    let wt_path = task_worktree_path(workspace_root, task_id);
    let projects_dir = wt_path.join(".ralph").join("projects");
    let entries = fs::read_dir(&projects_dir).ok()?;
    let project_dirs: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && e.path().join("state.json").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    match project_dirs.len() {
        1 => Some(project_dirs.into_iter().next().unwrap()),
        0 => None,
        n => {
            eprintln!(
                "warning: {} project directories in worktree for {task_id}; \
                 skipping discovery (ambiguous)",
                n
            );
            None
        }
    }
}
```

**Why validate with `state.json`**: Bare directories without `state.json` may be leftover artifacts from interrupted runs or non-project tooling. Requiring `state.json` eliminates false positives from stray directories and ensures the discovered project is actually resumable by `ralph run --project`.

**Edge case — multiple valid projects**: If multiple project directories with `state.json` exist (shouldn't happen in normal daemon use, but possible if a manual `ralph auto` was run), log a warning and return `None`. The task re-dispatches fresh via `ralph auto --idea`. This is safe because `ralph auto` creates a new project on a new branch — it does not collide with existing project branches since the daemon branch (`ralph/daemon/<task_id>`) is distinct from project branches (`ralph/<project_id>`).

### Change 2: Discover and persist `project_id` after child exit

**Where**: `runtime.rs`, in `complete_task()` (after line ~872, inside the successful CAS branch, before GitHub label updates).

**What**: After transitioning a task to its terminal state, call `discover_project_id` if the task's `project_id` is `None`. If discovery succeeds, persist it to the store. If the task already has a `project_id`, skip discovery (it was set on a previous run or manually).

```
// Pseudocode in complete_task, after CAS transition succeeds:
if task.project_id.is_none() {
    let workspace_root = derive_workspace_root(store);
    if let Some(discovered) = worktree::discover_project_id(&workspace_root, task_id) {
        eprintln!("task {task_id}: discovered project_id={discovered}");
        let store_clone = store.clone();
        let tid = task_id.to_owned();
        let _ = spawn_blocking_op(move || {
            store_clone.update_task(&tid, |t| {
                t.project_id = Some(discovered.clone());
                Ok(())
            })
        }).await;
        // Update local copy for downstream logic
        task.project_id = Some(discovered);
    }
}
```

**Why scan `.ralph/projects/`**: The `ralph auto` command creates project state at `.ralph/projects/{project_id}/state.json` inside the worktree. The worktree's `.ralph/` directory is preserved by `clean_worktree` (which uses `git clean --exclude=.ralph`). Reading the active project file is not reliable because it depends on `git rev-parse --git-dir` resolution within worktrees. Scanning the projects directory is simpler and more robust.

### Change 3: Dispatch-time backfill for legacy failed tasks

**Where**: `runtime.rs`, in `dispatch_task()` (after worktree creation and cleaning at ~line 412, before the child spawn at ~line 519).

**What**: If `task.project_id` is `None` and the worktree exists, call `discover_project_id` and persist the result to the store before spawning the child. This covers failed tasks from before this feature was deployed — they already have a worktree with `.ralph/projects/<id>/` but the `project_id` was never persisted to the task store.

```
// In dispatch_task, after clean_worktree and before spawning the child:
let effective_project_id = match task.project_id.as_deref() {
    Some(pid) => Some(pid.to_owned()),
    None => {
        // Backfill: check worktree for an existing project
        let ws = workspace_root.clone();
        let tid = task.task_id.clone();
        let discovered = spawn_blocking_op(move || {
            Ok(worktree::discover_project_id(&ws, &tid))
        }).await?;
        if let Some(ref pid) = discovered {
            eprintln!(
                "dispatch: task {} backfill project_id={pid} from worktree",
                task.task_id
            );
            let store_clone = store.clone();
            let tid = task.task_id.clone();
            let pid_clone = pid.clone();
            let _ = spawn_blocking_op(move || {
                store_clone.update_task(&tid, |t| {
                    t.project_id = Some(pid_clone.clone());
                    Ok(())
                })
            }).await;
        }
        discovered
    }
};

// Use effective_project_id for spawn decision:
match effective_project_id.as_deref() {
    Some(project_id) => process::spawn_ralph_run(..., project_id, ...),
    None => process::spawn_ralph_auto(..., &idea, ...),
}
```

**Why backfill at dispatch time rather than startup**: Startup backfill would require scanning all failed-task worktrees at daemon boot, adding latency and complexity. Dispatch-time backfill is lazy — it only runs for tasks that are actually being re-triggered — and naturally integrates into the existing dispatch flow. It also handles the case where a worktree was manually populated between daemon restarts.

### Change 4: Re-trigger failed tasks on `ralph:ready`

**Where**: `runtime.rs`, in `poll_and_claim()` (around lines 305-308).

**What**: Currently, `existing_ids` contains all task IDs regardless of state, and any issue with an existing task is skipped. Change this to check whether the existing task is in `Failed` state. If so: (1) claim the issue on GitHub first, (2) only then transition the task to `Pending`, (3) dispatch it. If the claim fails, the task stays `Failed` — no stranded state.

```
// In poll_and_claim, replace the simple skip with:
if existing_ids.contains(&task_id) {
    // Load the task's state to check if it's a failed task eligible for re-trigger.
    let task_state = {
        let store = store.clone();
        let tid = task_id.clone();
        spawn_blocking_op(move || {
            let tasks = store.load()?;
            Ok(tasks.iter().find(|t| t.task_id == tid).map(|t| t.state.clone()))
        }).await
    };

    let is_failed = matches!(task_state, Ok(Some(TaskState::Failed)));
    if !is_failed {
        continue; // Not failed — genuinely skip
    }

    // Step 1: Claim on GitHub FIRST (add ralph:in-progress, remove ralph:ready).
    // If this fails, the task stays Failed — no state mutation occurred.
    {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let issue_number = issue.number;
        if let Err(err) = spawn_blocking_op(move || {
            github::claim_issue(&owner, &repo, issue_number)
        }).await {
            eprintln!(
                "warning: failed to re-claim issue #{} for re-trigger: {err}",
                issue.number
            );
            continue; // Task stays Failed, safe to retry next cycle
        }
    }

    // Step 2: Transition Failed → Pending (claim succeeded, safe to proceed).
    let retrigger_task = {
        let store = store.clone();
        let tid = task_id.clone();
        spawn_blocking_op(move || {
            store.with_exclusive_tasks(|tasks| {
                let t = tasks.iter_mut().find(|t| t.task_id == tid)
                    .ok_or_else(|| RalphError::Validation(...))?;
                if t.state != TaskState::Failed {
                    // Race: state changed between read and CAS. Skip.
                    return Ok(None);
                }
                t.state = TaskState::Pending;
                t.updated_at = now_iso8601();
                Ok(Some(t.clone()))
            })
        }).await
    };

    match retrigger_task {
        Ok(Some(task)) => {
            if let Err(err) = dispatch_task(store, config, children, &task).await {
                eprintln!("warning: failed to dispatch re-triggered task {}: {err}", task_id);
                complete_task(store, config, &task_id, TaskState::Failed).await;
            }
            claimed += 1;
        }
        Ok(None) => {
            // State changed concurrently; claim was already made but task
            // is no longer Failed. This is benign — the ralph:in-progress
            // label will be resolved by the next terminal-label update.
            eprintln!("warning: task {task_id} state changed during re-trigger; skipping");
        }
        Err(err) => {
            eprintln!("warning: failed to re-trigger task {task_id}: {err}");
        }
    }
    continue;
}
```

**Why claim-first ordering**: The previous spec transitioned `Failed → Pending` before calling `claim_issue`. If the claim failed, the task would be stranded in `Pending` with no `ralph:in-progress` label — and since `adopt_pending` only runs at startup, the task would remain stuck until the next daemon restart. By claiming first, we ensure the GitHub label state is consistent before mutating the task store. If the claim fails, the task remains `Failed` and can be retried on the next poll cycle.

**Why this works**: The `filter_claimable` function already ensures the issue has `ralph:ready` and does NOT have `ralph:failed`, `ralph:in-progress`, etc. So a failed task's issue only appears in the claimable list after a human removes `ralph:failed` and ensures `ralph:ready` is present. This is the expected manual re-trigger flow.

**Label handling**: The `claim_issue` call adds `ralph:in-progress`, matching the normal claim flow. The `ralph:ready` label is left in place (consistent with fresh claims); it is harmless because `filter_claimable` checks for blocking labels, not the absence of trigger labels.

### Change 5: Harden `create_worktree` against stale git metadata

**Where**: `daemon/worktree.rs`, in `create_worktree()` (before `git worktree add`).

**What**: When the worktree directory does not exist but `git worktree add` fails, it may be because git's internal `.git/worktrees/<name>` metadata is stale (pointing to a previously-removed directory). Add `git worktree prune` as a recovery step, and verify the checked-out branch matches the expected branch.

```
pub fn create_worktree(repo_root: &Path, workspace_root: &Path, task_id: &str) -> Result<PathBuf> {
    let wt_path = task_worktree_path(workspace_root, task_id);

    if wt_path.exists() {
        // Verify the worktree is on the expected branch
        let expected_branch = format!("ralph/daemon/{task_id}");
        verify_worktree_branch(&wt_path, &expected_branch)?;
        return Ok(wt_path);
    }

    // Prune stale worktree metadata before attempting to add.
    // This handles the case where the worktree directory was removed
    // (e.g., by an external cleanup or filesystem issue) but git still
    // has a stale entry in .git/worktrees/.
    let _ = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo_root)
        .output();

    // ... rest of existing logic (branch check, worktree add) ...
}

/// Verify a worktree is on the expected branch. If not, force-checkout
/// the expected branch. This handles the case where a previous run
/// switched branches (e.g., ralph auto creating a project branch).
fn verify_worktree_branch(wt_path: &Path, expected_branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(wt_path)
        .output()
        .map_err(|err| RalphError::Orchestration(
            format!("failed to check branch in worktree {}: {err}", wt_path.display())
        ))?;

    if !output.status.success() {
        // Worktree may be in a detached HEAD or broken state.
        // Log and proceed — clean_worktree + ralph will handle it.
        eprintln!(
            "warning: could not determine branch in worktree {}; proceeding anyway",
            wt_path.display()
        );
        return Ok(());
    }

    let actual = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if actual != expected_branch {
        eprintln!(
            "worktree {}: branch is {actual}, expected {expected_branch}; resetting",
            wt_path.display()
        );
        let checkout = Command::new("git")
            .args(["checkout", "--force", expected_branch])
            .current_dir(wt_path)
            .output()
            .map_err(|err| RalphError::Orchestration(
                format!("failed to checkout {expected_branch}: {err}")
            ))?;
        if !checkout.status.success() {
            eprintln!(
                "warning: failed to reset worktree to {expected_branch}: {}",
                String::from_utf8_lossy(&checkout.stderr).trim()
            );
            // Non-fatal: dispatch will proceed with the current branch.
            // ralph auto/run will operate on whatever branch is checked out.
        }
    }

    Ok(())
}
```

**Why prune before add**: When a failed task's worktree was cleaned up externally (by `reconcile_worktrees` on a previous daemon run, manual deletion, or filesystem issues), git may still have a stale entry in `.git/worktrees/`. A subsequent `git worktree add` to the same path fails with "already registered". Pruning first removes these stale entries. This is safe because `git worktree prune` only removes entries whose working directory no longer exists.

**Why verify the branch**: The orchestrator (`ralph auto`) may switch the worktree to a different branch (e.g., `ralph/<project_id>`) during execution. On retry, `create_worktree` returns early because the path exists, but the checked-out branch may not be `ralph/daemon/<task_id>`. This is actually benign for `ralph run --project` (which operates on project state, not the daemon branch), but verifying and logging the mismatch makes debugging easier. The force-checkout is best-effort — if it fails, dispatch continues with the current branch.

## Files & Modules

| File | Change | Lines |
|------|--------|-------|
| `src/daemon/worktree.rs` | Add `discover_project_id()` helper | New function, ~20 lines |
| `src/daemon/worktree.rs` | Add `verify_worktree_branch()` helper, prune before `worktree add` | New function ~30 lines + ~5 lines in `create_worktree` |
| `src/daemon/runtime.rs` | Re-trigger failed tasks in `poll_and_claim()` with claim-first ordering | ~305-370 |
| `src/daemon/runtime.rs` | Call `discover_project_id` in `complete_task()` and persist result | ~872-896 |
| `src/daemon/runtime.rs` | Dispatch-time backfill of `project_id` for legacy tasks in `dispatch_task()` | ~412-520 |
| `src/validate/tests_daemon.rs` | Add tests for re-trigger, discovery, backfill, claim failure, and artifact continuity | New test functions |

## Testing Strategy

### Unit tests (in `worktree.rs`)

1. **`discover_project_id_single_project`**: Create a fake worktree with `.ralph/projects/foo-bar/state.json`, assert `discover_project_id` returns `Some("foo-bar")`.
2. **`discover_project_id_no_projects`**: Empty `.ralph/projects/` directory, assert returns `None`.
3. **`discover_project_id_multiple_projects`**: Two project directories each with `state.json`, assert returns `None` (ambiguous).
4. **`discover_project_id_no_worktree`**: Worktree path doesn't exist, assert returns `None`.
5. **`discover_project_id_ignores_dirs_without_state_json`**: Create `.ralph/projects/stale-dir/` without `state.json` alongside `.ralph/projects/valid-proj/state.json`. Assert returns `Some("valid-proj")`.

### Integration tests (in `tests_daemon.rs`, conformance test style)

6. **`runtime_failed_task_retrigger_with_project_id`**: Seed a failed task with `project_id` set. Simulate re-label by presenting the issue as claimable (`ralph:ready`, no `ralph:failed`). Run single-iteration daemon. Assert: task transitions `Failed → Pending → InProgress → Failed/Completed`, mock ralph receives `["run", "--project", "<id>"]`, `ralph:in-progress` label is applied.

7. **`runtime_failed_task_retrigger_without_project_id`**: Seed a failed task with `project_id: None` and no worktree. Run single-iteration. Assert: dispatches via `ralph auto --idea`, not `ralph run --project`.

8. **`runtime_project_id_discovered_after_child_exit`**: Seed a pending task with `project_id: None`. Mock ralph creates `.ralph/projects/test-proj/state.json` in worktree and exits. Assert: after child collection, `task.project_id == Some("test-proj")` in the store.

9. **`runtime_project_id_not_overwritten_if_already_set`**: Seed a task with `project_id: Some("original")`. Mock ralph creates a different project directory with `state.json`. Assert: `task.project_id` remains `"original"`.

10. **`runtime_completed_task_not_retriggered`**: Seed a completed task. Present issue as claimable. Assert: task is NOT re-dispatched (only `Failed` triggers re-dispatch).

11. **`runtime_retrigger_claim_failure_preserves_failed_state`**: Seed a failed task. Configure mock gh to fail on `issue edit` (claim). Run single-iteration. Assert: task remains in `Failed` state (not stranded in `Pending`), no child process is spawned, and the next poll cycle can retry.

12. **`runtime_retrigger_backfills_project_id_from_worktree`**: Seed a failed task with `project_id: None`. Pre-populate the worktree with `.ralph/projects/legacy-proj/state.json`. Run single-iteration. Assert: `task.project_id` is backfilled to `Some("legacy-proj")` before spawning, mock ralph receives `["run", "--project", "legacy-proj"]`.

13. **`runtime_retrigger_preserves_project_artifacts`**: Seed a failed task with `project_id` set. Pre-populate the worktree with `.ralph/projects/<id>/loops/loop-1/plan.md` and `.ralph/projects/<id>/state.json`. Run single-iteration. Assert: after dispatch, `.ralph/projects/<id>/loops/loop-1/plan.md` still exists in the worktree (artifacts preserved by `clean_worktree --exclude=.ralph`), and mock ralph receives `["run", "--project", "<id>"]`.

14. **`runtime_create_worktree_handles_stale_metadata`**: Create a worktree, then manually remove its directory (simulating external cleanup) without running `git worktree prune`. Assert: `create_worktree` succeeds on second call (prune-then-add recovery path works).

### Existing tests (must continue passing)

- `runtime_failed_worktree_preserved_and_reused_on_retry` — validates the existing resume path when `project_id` is pre-populated
- `runtime_fresh_dispatch_ignores_discovered_project` — validates that fresh tasks (`project_id: None`) use `ralph auto`
- `runtime_activation_failed_task_preserved` — validates CAS race handling

## Out of Scope

- **Automatic label management**: The daemon does not automatically remove `ralph:failed` and add `ralph:ready`. Re-triggering requires manual label changes by a human operator.
- **Retry limits / backoff**: No cap on how many times a failed task can be re-triggered. This can be added later as a counter on `DaemonTask`.
- **Partial project state repair**: If `.ralph/projects/<id>/state.json` is corrupt or missing fields, we do not attempt repair — `ralph run --project` will fail and the task will re-enter `Failed` state.
- **Multi-project disambiguation**: If multiple valid projects exist in a worktree, `discover_project_id` returns `None` and the task re-dispatches fresh via `ralph auto --idea`. This is safe because `ralph auto` creates a new project with its own branch, not colliding with existing project branches.
- **CLI `ralph daemon retry` command**: A manual CLI command to re-trigger without label changes. Useful but not part of this scope.
- **Notification on re-trigger**: No GitHub comment is posted when a task is re-triggered (only on completion/failure).
- **Startup-wide backfill scan**: We do not scan all failed-task worktrees at daemon startup to backfill `project_id`. Dispatch-time backfill (Change 3) covers the same gap lazily with less startup cost.
- **Worktree branch convergence**: If the worktree is on a project branch (e.g., `ralph/<project_id>`) instead of the daemon branch (`ralph/daemon/<task_id>`), `verify_worktree_branch` logs the mismatch and attempts a best-effort checkout, but does not fail dispatch. The `ralph run --project` command operates on project state regardless of the checked-out branch.
