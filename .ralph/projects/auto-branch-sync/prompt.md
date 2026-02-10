# Auto-Branch Sync on Run

## Overview

Fix a race condition where `ralph run` checks out a project branch that's behind the base branch (master), causing "No such file or directory" errors for files committed after the branch was created.

## Background

The sequence that triggers the bug:

1. `ralph project new myproject --prompt PLAN.md` — writes `.ralph/projects/myproject/` files to disk and creates branch `ralph/myproject` at current master HEAD. The `.ralph/` files are **not committed** by this command.
2. User commits the `.ralph/` project files to master (either manually or via another tool).
3. `ralph run --project myproject` — checks out `ralph/myproject`, which points at the **old** master HEAD (before the `.ralph/` commit). The project state files don't exist on this branch, causing errors.

## Design

### Fix Location

In `src/workflow/orchestrator.rs`, in the `run()` method, right after the auto_branch checkout (around line 188), merge the base branch into the project branch to bring it up to date.

### Implementation

After `checkout_branch(repo_root, &branch)?;`, add a merge of the base branch:

```rust
if self.workspace.config.git.auto_branch {
    if let Some(repo_root) = self.workspace.root.parent() {
        if is_git_repo(repo_root) {
            let branch =
                resolve_branch_name(&self.workspace.config.git.branch_format, &project_id);
            if branch_exists(repo_root, &branch)? {
                checkout_branch(repo_root, &branch)?;
                // Sync project branch with base branch to pick up any
                // commits made after the branch was created (e.g. project
                // state files committed to master).
                let base = &self.workspace.config.git.base_branch;
                merge_base_branch(repo_root, base)?;
            }
        }
    }
}
```

### New Git Helper

Add `merge_base_branch()` to `src/git/branch.rs`:

```rust
/// Merges the base branch into the current branch if it has diverged.
/// This is a fast-forward merge when possible, otherwise a real merge.
/// If the base branch has no new commits, this is a no-op.
pub fn merge_base_branch(workdir: &Path, base_ref: &str) -> Result<()> {
    ensure_git_repo(workdir)?;
    // Check if there are any commits on base_ref not on HEAD
    let output = run_git(workdir, &["rev-list", "--count", &format!("HEAD..{base_ref}")])?;
    let count: u64 = output.trim().parse().unwrap_or(0);
    if count == 0 {
        return Ok(()); // Already up to date
    }
    run_git(workdir, &["merge", base_ref, "--no-edit", "-m",
        &format!("Merge {} into project branch", base_ref)])?;
    Ok(())
}
```

### What NOT to Change

- Do NOT change `ralph project new` — the branch creation stays as-is
- Do NOT change `commit_feature_loop()` or any commit logic
- Do NOT change `create_project()` in `src/project/lifecycle.rs`
- Do NOT add new config options — this is automatic behavior
- Do NOT change the branch creation logic in `maybe_create_project_branch()`

## Files to Modify

| File | Change |
|------|--------|
| `src/git/branch.rs` | Add `merge_base_branch()` function |
| `src/workflow/orchestrator.rs` | Call `merge_base_branch()` after `checkout_branch()` in the auto_branch block of `run()` |
| `tests/orchestrator.rs` | Add test: create project, commit state to master, run orchestrator — verify project branch gets synced and state files are accessible |

## Unit Test

In `src/git/branch.rs` `#[cfg(test)]` module (or `tests/orchestrator.rs`):

```rust
#[test]
fn merge_base_branch_syncs_new_commits() {
    // Setup: create a git repo, make initial commit, create a branch,
    // add a commit to master, then call merge_base_branch on the branch.
    // Verify the branch now has the new commit.
}

#[test]
fn merge_base_branch_noop_when_up_to_date() {
    // Setup: create a git repo, create a branch at HEAD.
    // Call merge_base_branch — verify no merge commit is created.
}
```

## Verification

`nix build` passes. After the fix:

1. `ralph project new foo --prompt PLAN.md` creates branch at master
2. Commit `.ralph/` files to master
3. `ralph run --project foo` checks out `ralph/foo`, merges master → branch now has `.ralph/` files
4. Orchestration proceeds without "No such file or directory"

## Scope Boundaries

- Do NOT add new CLI flags or config options
- Do NOT change the project creation flow
- Do NOT modify how commits work in the orchestrator loop
- Keep the merge silent (no user-facing output unless it fails)
- If merge fails (conflicts), let the error propagate naturally via `RalphError`
