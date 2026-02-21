## Summary

Two bugs cause daemon-dispatched tasks to branch from stale local `master`, producing PRs with guaranteed merge conflicts:

1. **`sync_project_branch()` never updates the local base branch.** After `git fetch origin`, the local `master` (or configured base branch) still points at whatever commit it had at clone time. Downstream code that references the local base branch (e.g., `merge_base_branch` in the orchestrator at `orchestrator.rs:239`) uses a stale ref.

2. **`maybe_create_project_branch()` branches from the local `{base_branch}` ref, not `origin/{base_branch}`.** In `lifecycle.rs:353-359`, when no parent project exists, new project branches are created from the local `base_branch` ref. Since the daemon never updates this local ref, every new project branch starts from stale code.

Both bugs are invisible in interactive (non-daemon) workflows where the user keeps their local `master` up to date.

**Clarification on `maybe_create_project_branch` scope:** This function is only called from the interactive `create_project()` path (`lifecycle.rs:114`), not from daemon dispatch. The daemon creates branches via `sync_project_branch()` which already uses `origin/HEAD`. However, `maybe_create_project_branch` should still be hardened for the case where a user runs `ralph project new` in a daemon-managed worktree or any environment with a stale local base branch.

## Acceptance Criteria

- [ ] `sync_project_branch()` in `src/git/branch.rs` runs `git update-ref refs/heads/{base_branch} refs/remotes/origin/{base_branch}` after fetching, using the configured base branch name (not hardcoded)
- [ ] The ref update uses `git update-ref` (not `git branch -f`) to be safe when the base branch is checked out in any worktree
- [ ] `maybe_create_project_branch()` in `src/project/lifecycle.rs` uses `origin/{base_branch}` as the `from_ref` when `origin/{base_branch}` exists, falling back to the local `{base_branch}` when it does not (preserving local-only repo compatibility)
- [ ] Base branch name is always read from `workspace.config.git.base_branch` (via `global_config.git.base_branch` in daemon context), never hardcoded as `"master"` or `"main"`
- [ ] Existing parent-project branching logic (`resolve_branch_name` path at `lifecycle.rs:354`) remains unchanged
- [ ] `sync_project_branch()` signature changes to accept a `base_branch: &str` parameter
- [ ] `dispatch_task()` in `runtime.rs` passes `config.global_config.git.base_branch` to `sync_project_branch()`
- [ ] All existing call sites of `sync_project_branch` are updated (runtime.rs, branch.rs tests, commit.rs test, tests_daemon.rs)
- [ ] New tests confirm the local base branch is force-updated after sync using `update-ref`
- [ ] New tests confirm `maybe_create_project_branch` prefers `origin/{base_branch}` and falls back to local ref
- [ ] New test confirms ref update succeeds when base branch is checked out in another worktree
- [ ] Existing tests in `branch.rs`, `commit.rs`, and `tests_daemon.rs` continue to pass

## Technical Approach

### Change 1: `sync_project_branch()` — force-update local base branch after fetch (worktree-safe)

**File:** `src/git/branch.rs:73-118`

**Problem with original approach:** The original spec proposed `git branch -f {base_branch} origin/{base_branch}`. This command fails with `fatal: Cannot force update the branch 'master' which you are currently on` when the base branch is checked out in *any* worktree. In the daemon layout, the main repo is typically on the base branch (via `origin/HEAD`) while task worktrees exist alongside it, so this failure is expected in normal operation.

**Solution:** Use `git update-ref` which directly manipulates the ref store without checking worktree state:

```rust
pub fn sync_project_branch(repo_root: &Path, issue_number: u32, base_branch: &str) -> Result<()> {
    // ... existing fetch code (lines 80-85) ...

    // NEW: force-update local base branch to match remote (worktree-safe).
    // `git update-ref` bypasses the checked-out-branch guard that `git branch -f`
    // enforces, which is important because the main repo may have base_branch
    // checked out while this runs in a task worktree.
    run_git(repo_root, &[
        "update-ref",
        &format!("refs/heads/{base_branch}"),
        &format!("refs/remotes/origin/{base_branch}"),
    ]).map_err(|err| {
        RalphError::Orchestration(format!(
            "sync_project_branch: git update-ref refs/heads/{base_branch} \
             refs/remotes/origin/{base_branch} failed for issue {issue_number}: {err}"
        ))
    })?;

    // ... rest of existing code unchanged ...
}
```

The `base_branch` parameter is added to the signature. This is a defense-in-depth measure: even though `sync_project_branch` itself already creates branches from `origin/HEAD` or `origin/ralph/issue-<n>`, keeping the local base branch current prevents other code paths (e.g., `merge_base_branch` at `orchestrator.rs:239`) from using stale refs.

The `update-ref` approach is safe because:
- It works regardless of which branch is checked out in any worktree
- The daemon never commits to the base branch itself — it's always a tracking mirror of `origin/{base_branch}`
- It's atomic — the ref is updated in a single operation

**Error handling:** If `origin/{base_branch}` doesn't exist (e.g., misconfigured `base_branch` in config), the `update-ref` will fail and the error is propagated. This is correct: if the configured base branch doesn't exist on the remote, that's a configuration error that should surface early rather than silently proceeding with a stale ref. The subsequent `origin/HEAD` fallback for branch creation (existing lines 100-115) still works independently.

### Change 2: `maybe_create_project_branch()` — prefer remote tracking ref with local fallback

**File:** `src/project/lifecycle.rs:353-359`

**Problem with original approach:** The original spec unconditionally switched `from_ref` to `origin/{base_branch}`. However, `maybe_create_project_branch()` is called from the interactive `create_project()` path (`lifecycle.rs:114`), not just the daemon. Users working with local-only repos (no remote) or repos where `origin/{base_branch}` doesn't exist would see failures. The spec's claim that "interactive workflows are unaffected" would be violated.

**Solution:** Prefer `origin/{base_branch}` when it exists, fall back to the local ref otherwise:

```rust
let from_ref = if let Some(parent_id) = parent_project {
    resolve_branch_name(&workspace.config.git.branch_format, parent_id)
} else {
    let remote_ref = format!("origin/{}", workspace.config.git.base_branch);
    if remote_ref_exists(repo_root, &remote_ref)? {
        remote_ref
    } else {
        workspace.config.git.base_branch.clone()
    }
};
```

This requires importing `remote_ref_exists` into `lifecycle.rs` (it's already `pub` in `branch.rs`).

**Behavior matrix:**

| Scenario | `origin/{base_branch}` exists? | `from_ref` used | Effect |
|----------|-------------------------------|-----------------|--------|
| Daemon worktree | Yes | `origin/master` | Branches from latest remote (bug fix) |
| Interactive, normal clone | Yes | `origin/master` | Branches from latest remote (improvement) |
| Interactive, local-only repo | No | `master` | Unchanged from current behavior |
| Interactive, custom remote name | No (no `origin`) | `master` | Unchanged from current behavior |

### Change 3: Thread `base_branch` through dispatch via existing `global_config`

**File:** `src/daemon/runtime.rs:566-584`

**Problem with original approach:** The original spec proposed adding a `base_branch: String` field to `DaemonRuntimeConfig`. However, `DaemonRuntimeConfig` already carries `global_config: GlobalConfig` (line 37), which contains `git.base_branch` at `global_config.git.base_branch`. Adding a separate field creates redundancy and drift risk.

**Solution:** Use the existing `config.global_config.git.base_branch` at the call site:

```rust
// Remote-first project branch sync
{
    let wt = wt_path.clone();
    let base_branch = config.global_config.git.base_branch.clone();
    match spawn_blocking_op(move || {
        crate::git::branch::sync_project_branch(&wt, issue_number, &base_branch)
    })
    .await
    {
        // ... existing match arms unchanged ...
    }
}
```

No new fields on `DaemonRuntimeConfig`. No additional config loading. The `global_config` is already populated during daemon startup and available at dispatch time.

## Files & Modules

| File | Function/Struct | Change |
|------|----------------|--------|
| `src/git/branch.rs:73-118` | `sync_project_branch()` | Add `base_branch: &str` param; insert `git update-ref refs/heads/{base_branch} refs/remotes/origin/{base_branch}` after fetch |
| `src/project/lifecycle.rs:353-359` | `maybe_create_project_branch()` | Change `from_ref` else-arm to prefer `origin/{base_branch}` with fallback to local ref; add `use crate::git::branch::remote_ref_exists` |
| `src/daemon/runtime.rs:566-584` | `dispatch_task()` | Extract `config.global_config.git.base_branch` and pass to `sync_project_branch()` |
| `src/git/branch.rs:120+` | tests module | Update 0 existing `sync_project_branch` calls (none in this module — tests only cover `merge_base_branch`) |
| `src/git/commit.rs:536` | `sync_project_branch_discards_local_only_checkpoint_and_position_reverts` test | Update call to pass `"master"` as `base_branch` |
| `src/validate/tests_daemon.rs:2679,2698,2724,2768` | Four conformance tests | Update `sync_project_branch` calls to pass `"master"` as `base_branch` |

### Files NOT modified

- **`src/daemon/runtime.rs:17-48` (`DaemonRuntimeConfig`)** — No new fields. The existing `global_config: GlobalConfig` already carries `git.base_branch`.
- **`src/workflow/orchestrator.rs`** — Uses `merge_base_branch` with the local ref after the daemon has already synced it. Will benefit from Change 1 automatically. No code changes needed.
- **`src/daemon/worktree.rs`** — Already correctly uses `origin/HEAD` for worktree creation. No change needed.
- **`src/cli/rollback.rs`** — Interactive CLI path, not affected by daemon stale-branch bug.

## Testing Strategy

### Unit tests (in `src/git/branch.rs`)

1. **`sync_project_branch_updates_local_base_branch`**: Set up a bare remote + clone. Advance the remote's master by committing directly to the bare repo. Call `sync_project_branch` on the clone with `base_branch="master"`. Assert that the local `master` ref now matches `origin/master` (i.e., `git rev-parse master` equals `git rev-parse origin/master`).

2. **`sync_project_branch_updates_custom_base_branch`**: Same as above but with a non-`master` base branch name (e.g., `"main"`) to verify the config-driven base branch parameter works correctly.

3. **`sync_project_branch_update_ref_works_when_base_branch_checked_out`**: Set up a bare remote + clone where the clone has `master` checked out (the default after clone). Advance the remote's master. Call `sync_project_branch` from the clone directory (simulating a worktree that shares the ref store with a main repo where `master` is checked out). Assert that the local `master` ref is updated successfully — this test validates the `git update-ref` approach over `git branch -f`.

4. **`sync_project_branch_tolerates_missing_remote_base_branch`**: Set up a scenario where `origin/{base_branch}` doesn't exist (e.g., config says `"develop"` but remote has no such branch). Assert that `sync_project_branch` returns an error with a clear message mentioning the ref name and issue number.

### Unit tests (in `src/project/lifecycle.rs` or integration)

5. **`create_project_branch_prefers_remote_ref`**: Set up a test workspace with a git repo that has diverged local vs remote base branches (local `master` behind `origin/master`). Call `maybe_create_project_branch` with no parent. Assert the new branch points at the `origin/master` commit, not the stale local `master`.

6. **`create_project_branch_falls_back_to_local_ref`**: Set up a test workspace with a local-only git repo (no `origin` remote). Call `maybe_create_project_branch` with no parent. Assert it succeeds and the new branch points at the local `master` — confirming backward compatibility with repos without remotes.

7. **`create_project_branch_parent_resolution_unchanged`**: Set up a test workspace with a parent project branch. Call `maybe_create_project_branch` with a `parent_project` argument. Assert the new branch is created from the parent's branch (via `resolve_branch_name`), not from any base branch ref. This explicitly validates the acceptance criterion that parent-project logic is unchanged.

### Existing call site updates

8. **`src/git/commit.rs:536`**: Update the `sync_project_branch(&work, 42)` call to `sync_project_branch(&work, 42, "master")`. The test's `init_repo_with_remote` helper creates a `master`-default repo.

9. **`src/validate/tests_daemon.rs`**: Update all four conformance test calls:
   - Line 2679: `sync_project_branch(&clone, 42, "master")`
   - Line 2698: `sync_project_branch(&clone, 99, "master")`
   - Line 2724: `sync_project_branch(&clone, 7, "master")`
   - Line 2768: `sync_project_branch(&clone, 10, "master")`

### Integration / daemon worktree test

10. **`stale_master_does_not_affect_new_project_branch`** (in `tests_daemon.rs`): End-to-end test that:
    - Creates a bare remote + clone (simulating the daemon's repo)
    - Adds a git worktree from the clone (simulating a task worktree, with the main clone on `master`)
    - Advances the remote's master past the clone's local master
    - Runs `sync_project_branch` from inside the worktree
    - Asserts that the local `master` ref in the shared ref store is updated (even though `master` is checked out in the main clone)
    - Creates a project branch and asserts it starts from the current remote HEAD

### Manual verification

- Deploy to a test daemon instance, let it pick up an issue after `master` has advanced on the remote, confirm the resulting PR has no merge conflicts from stale base.

## Out of Scope

- **`origin/HEAD` symref resolution**: The existing `sync_project_branch` falls back to `origin/HEAD` when the remote project branch doesn't exist. This is correct behavior and not changed by this spec.
- **Non-daemon (interactive) workflows (orchestrator)**: The interactive `ralph auto` / `ralph run` paths use `merge_base_branch` with the local base ref at `orchestrator.rs:239`. These paths benefit passively from Change 1 (the local ref is now current) but receive no direct code changes. No changes to `src/workflow/orchestrator.rs`.
- **Worktree creation base ref**: `src/daemon/worktree.rs` already correctly uses `origin/HEAD` for worktree creation. No changes needed.
- **Auto-rebase flow**: `src/daemon/runtime.rs` rebase logic already uses `origin/{merge_info.base_branch}` from the GitHub PR API. No changes needed.
- **`detect_base_branch()` in `daemon/github.rs`**: This function is used for PR operations and already handles remote detection. Not related to the project branch creation bug.
- **Config schema changes**: No new config keys are introduced. The existing `git.base_branch` config is simply threaded through to the new `sync_project_branch` call site via the already-available `global_config`.
- **`DaemonRuntimeConfig` field additions**: No new fields are added. The existing `global_config: GlobalConfig` field already provides access to `git.base_branch`.
- **Migration of existing stale branches**: Existing worktrees/branches from before this fix are not automatically repaired. Re-triggering the task will pick up the fix.
