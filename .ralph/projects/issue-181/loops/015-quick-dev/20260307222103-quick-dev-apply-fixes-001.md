---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-181
backend: claude
role: implementer
created_at: 2026-03-07T22:21:03Z
---

# Apply Fixes: Branch Correction Scope & SHA Preservation Guard

## Changes Made

### 1. `src/daemon/worktree.rs` — Narrowed branch correction scope (High)

In `verify_worktree_branch`, the checkout now checks whether `refs/heads/<expected_branch>` exists before choosing the checkout strategy:

- **Branch exists** → `git checkout --force <expected_branch>` — switches to the branch without rewriting its commit pointer
- **Branch does not exist** → `git checkout --force -B <expected_branch>` — creates the branch at current HEAD (migration path for old `ralph/daemon/` worktrees)

This prevents the over-broad `-B` from resetting an existing branch's commit pointer to the (possibly wrong) HEAD of a mismatched branch.

### 2. `src/validate/tests_daemon.rs` — SHA preservation assertion (Medium)

In `runtime_reuse_worktree_corrects_branch_mismatch`, the test now:
1. Captures `rev-parse <expected_branch>` SHA **before** switching to the mismatch branch
2. After `create_worktree` reuse corrects the branch, asserts the SHA is **unchanged**

This guards against the exact regression where `-B` would silently rewrite the branch pointer. The existing `runtime_reuse_worktree_migrates_old_daemon_branch` test continues to cover the missing-branch creation behavior.
