Now I have a thorough understanding of the codebase. Let me write the engineering specification.

## Summary

Two bugs cause daemon-dispatched tasks to branch from stale local `master`, producing PRs with guaranteed merge conflicts:

1. **`sync_project_branch()` never updates the local base branch.** After `git fetch origin`, the local `master` (or configured base branch) still points at whatever commit it had at clone time. Subsequent code that references the local base branch uses a stale ref.

2. **`create_project_branch()` branches from the local `{base_branch}` ref, not `origin/{base_branch}`.** In `maybe_create_project_branch()` (lifecycle.rs:353-359), when no parent project exists, new project branches are created from the local `base_branch` ref. Since the daemon never updates this local ref, every new project branch starts from stale code.

Both bugs are invisible in interactive (non-daemon) workflows where the user keeps their local `master` up to date.

## Acceptance Criteria

- [ ] `sync_project_branch()` in `src/git/branch.rs` runs `git branch -f {base_branch} origin/{base_branch}` after fetching, using the configured base branch name from workspace config (not hardcoded)
- [ ] `create_project_branch()` in `src/project/lifecycle.rs` uses `origin/{base_branch}` as the `from_ref` when creating new project branches with no parent project
- [ ] Base branch name is always read from `workspace.config.git.base_branch`, never hardcoded as `"master"` or `"main"`
- [ ] Existing parent-project branching logic (`resolve_branch_name` path at lifecycle.rs:354) remains unchanged
- [ ] `sync_project_branch()` signature changes to accept a `base_branch: &str` parameter
- [ ] `dispatch_task()` in runtime.rs passes the configured base branch to `sync_project_branch()`
- [ ] New unit tests confirm the local base branch is force-updated after sync
- [ ] New unit tests confirm `create_project_branch` uses `origin/{base_branch}` for parentless projects
- [ ] Existing tests in `branch.rs` and `tests_daemon.rs` continue to pass

## Technical Approach

### Change 1: `sync_project_branch()` — force-update local base branch after fetch

**File:** `src/git/branch.rs:73-118`

After the `git fetch origin` at line 80-85, insert a force-update of the local base branch:

```rust
pub fn sync_project_branch(repo_root: &Path, issue_number: u32, base_branch: &str) -> Result<()> {
    // ... existing fetch code (lines 80-85) ...

    // NEW: force-update local base branch to match remote
    run_git(repo_root, &["branch", "-f", base_branch, &format!("origin/{base_branch}")]).map_err(|err| {
        RalphError::Orchestration(format!(
            "sync_project_branch: git branch -f {base_branch} origin/{base_branch} failed \
             for issue {issue_number}: {err}"
        ))
    })?;

    // ... rest of existing code unchanged ...
}
```

The `base_branch` parameter is added to the signature. This is a defense-in-depth measure: even though Change 2 makes `create_project_branch` use the remote ref, keeping the local base branch up-to-date prevents other code paths (e.g., `merge_base_branch` in the orchestrator at `orchestrator.rs:239`) from using stale refs.

The force-update (`-f`) is safe here because the daemon never commits to the base branch itself — `master` is always a tracking mirror of `origin/master`.

### Change 2: `create_project_branch()` — use remote tracking ref

**File:** `src/project/lifecycle.rs:353-359`

Change the `else` arm of the `from_ref` assignment to use `origin/{base_branch}` instead of the local ref:

```rust
let from_ref = if let Some(parent_id) = parent_project {
    resolve_branch_name(&workspace.config.git.branch_format, parent_id)
} else {
    format!("origin/{}", workspace.config.git.base_branch)
};
```

This is the primary fix. By branching from the remote-tracking ref, we guarantee the new project branch starts from the latest fetched remote state, regardless of whether the local base branch was updated.

### Change 3: Thread `base_branch` through dispatch

**File:** `src/daemon/runtime.rs:566-584`

Update the `sync_project_branch` call site to pass the base branch from workspace config:

```rust
let base_branch = /* resolve from workspace config */;
spawn_blocking_op(move || crate::git::branch::sync_project_branch(&wt, issue_number, &base_branch))
```

The `DaemonRuntimeConfig` does not currently carry `base_branch`, but it does carry `workspace_root`. The base branch can be resolved from the workspace config loaded at dispatch time, or added as a field to `DaemonRuntimeConfig`. The simpler approach is to load it from the workspace config at the call site since `config.workspace_root` is already available and `Workspace::load` is cheap.

Alternatively, if loading workspace config at dispatch time is undesirable (it's already done during bootstrap), add a `base_branch: String` field to `DaemonRuntimeConfig` and populate it during daemon startup.

**Recommended:** Add `base_branch: String` to `DaemonRuntimeConfig` — it's a single field addition with no extra I/O at dispatch time.

## Files & Modules

| File | Function | Change |
|------|----------|--------|
| `src/git/branch.rs:73-118` | `sync_project_branch()` | Add `base_branch: &str` param; insert `git branch -f {base_branch} origin/{base_branch}` after fetch |
| `src/project/lifecycle.rs:353-359` | `maybe_create_project_branch()` | Change `from_ref` else-arm from `workspace.config.git.base_branch.clone()` to `format!("origin/{}", workspace.config.git.base_branch)` |
| `src/daemon/runtime.rs:566-584` | `dispatch_task()` | Pass configured `base_branch` to `sync_project_branch()` |
| `src/daemon/runtime.rs:17-48` | `DaemonRuntimeConfig` | Add `pub base_branch: String` field |
| `src/daemon/runtime.rs` (callers) | `run()` / daemon startup | Populate `base_branch` from workspace config when constructing `DaemonRuntimeConfig` |
| `src/git/branch.rs` (tests) | Existing + new tests | Update existing `sync_project_branch` test calls to pass base_branch; add test for local base branch force-update |

### Files NOT modified

- `src/workflow/orchestrator.rs` — Uses `merge_base_branch` with the local ref after the daemon has already synced it. Will benefit from Change 1 automatically.
- `src/daemon/worktree.rs` — Already uses `origin/HEAD` correctly for worktree creation. No change needed.
- `src/cli/rollback.rs` — Interactive CLI path, not affected by daemon stale-branch bug.

## Testing Strategy

### Unit tests (in `src/git/branch.rs`)

1. **`sync_project_branch_updates_local_base_branch`**: Set up a bare remote + clone. Advance the remote's master by committing directly to the bare repo. Call `sync_project_branch` on the clone. Assert that the local `master` ref now matches `origin/master` (i.e., includes the new commit).

2. **`sync_project_branch_updates_custom_base_branch`**: Same as above but with a non-`master` base branch name (e.g., `"main"`) to verify the config-driven base branch is used correctly.

3. **Update existing tests**: All four existing `sync_project_branch` tests need their call sites updated to pass `"master"` (or the default branch name from the test fixture) as the `base_branch` parameter.

### Unit tests (in `src/project/lifecycle.rs` or integration)

4. **`create_project_branch_uses_remote_ref`**: Verify that `maybe_create_project_branch` with no parent project calls `create_branch` with `origin/{base_branch}` as the `from_ref`. This can be tested by setting up a test workspace with a git repo that has diverged local vs remote base branches, creating a project, and verifying the new branch points at the remote ref.

### Integration tests (in `src/validate/tests_daemon.rs`)

5. **Update existing conformance tests**: Ensure the existing daemon conformance tests that call `sync_project_branch` pass the base branch parameter.

6. **`stale_master_does_not_affect_new_project_branch`**: End-to-end test that:
   - Creates a bare remote + clone
   - Advances the remote's master past the clone's local master
   - Runs the dispatch flow (or directly calls `sync_project_branch` + `create_project_branch`)
   - Asserts the project branch starts from the remote HEAD, not the stale local master

### Manual verification

- Deploy to a test daemon instance, let it pick up an issue after `master` has advanced on the remote, confirm the resulting PR has no merge conflicts from stale base.

## Out of Scope

- **`origin/HEAD` symref resolution**: The existing `sync_project_branch` falls back to `origin/HEAD` when the remote project branch doesn't exist. This is correct behavior and not changed by this spec.
- **Non-daemon (interactive) workflows**: The interactive `ralph auto` / `ralph run` paths are not affected since the user is expected to keep their local branches current. No changes to `src/workflow/orchestrator.rs` merge logic.
- **Worktree creation base ref**: `src/daemon/worktree.rs` already correctly uses `origin/HEAD` for worktree creation. No changes needed.
- **Auto-rebase flow**: `src/daemon/runtime.rs` rebase logic already uses `origin/{merge_info.base_branch}` from the GitHub PR API. No changes needed.
- **`detect_base_branch()` in `daemon/github.rs`**: This function is used for PR operations and already handles remote detection. Not related to the project branch creation bug.
- **Config schema changes**: No new config keys are introduced. The existing `git.base_branch` config is simply threaded through to new call sites.
- **Migration of existing stale branches**: Existing worktrees/branches from before this fix are not automatically repaired. Re-triggering the task will pick up the fix.