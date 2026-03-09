---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 3
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-07T20:40:38Z
---

# Applied Reviewer-Requested Fixes

## Fix 1 (High): Prevent `reset_hard` on wrong branch when project branch is missing

**File:** `src/cli/rollback.rs:101-116`

Changed the checkout logic from a conditional skip to a hard requirement. When the project branch doesn't exist locally:
1. First tries to recreate it from `origin/<branch>`
2. If origin ref also doesn't exist, returns a `Validation` error immediately — `reset_hard` is never reached

Before (buggy):
```rust
if branch_exists(repo_root, &branch)? {
    checkout_branch(repo_root, &branch)?;
}
reset_hard(repo_root, reference)?;  // runs on wrong branch!
```

After (fixed):
```rust
if !branch_exists(repo_root, &branch)? {
    let remote_ref = format!("origin/{branch}");
    if remote_ref_exists(repo_root, &remote_ref)? {
        create_branch(repo_root, &branch, &remote_ref)?;
    } else {
        return Err(RalphError::Validation(format!(
            "cannot hard-rollback: project branch '{}' does not exist locally or on origin",
            branch
        )));
    }
}
checkout_branch(repo_root, &branch)?;
reset_hard(repo_root, reference)?;  // only runs on correct branch
```

## Fix 2 (Medium): Conformance test for branch-missing hard-rollback path

**File:** `src/validate/tests_commands.rs` — new test `rollback_hard_missing_branch`

The test:
1. Sets up a project, runs 2 loops
2. Detaches HEAD, deletes the local project branch, and removes the remote tracking ref
3. Runs `rollback --hard 1`
4. Asserts the command fails (non-zero exit)
5. Asserts stderr mentions the missing branch
6. Asserts HEAD was not mutated (detached commit unchanged)
