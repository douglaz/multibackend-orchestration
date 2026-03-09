---
artifact: quick-dev-plan-implement
loop: 15
project: issue-181
backend: claude
role: implementer
created_at: 2026-03-07T20:42:24Z
---

## Summary of changes

### Problem
The daemon used `ralph/daemon/{task_id}` as the worktree branch but `sync_project_branch` immediately switched to `ralph/issue-{N}`, causing the draft PR watcher to push a stale branch and `gh pr create` to fail.

### Changes made

**Production code (4 files):**

1. **`src/daemon/worktree.rs`** — Added `branch_name: &str` parameter to `create_worktree`, removing the hardcoded `format!("ralph/daemon/{task_id}")`. Changed `verify_worktree_branch` to use `git checkout --force -B` (creates-or-resets) instead of `git checkout --force` for migration of pre-existing worktrees on old daemon branches.

2. **`src/daemon/runtime.rs`** — Computed `branch_name` early in `dispatch_task` via `resolve_branch_name(&config.global_config.git.branch_format, &project_id)`, passed it to `create_worktree`, and removed the stale `let branch_name = format!("ralph/daemon/{task_id}")` line. All downstream consumers (`find_existing_pr`, `draft_pr_watcher`, `ChildHandle.branch`) now automatically use `ralph/issue-{N}`.

3. **`src/daemon/github.rs`** — Updated `current_branch` doc comment to reflect the unified branch naming.

4. **`src/git/branch.rs`** — Updated test setup to use `ralph/issue-1` instead of `ralph/daemon/test-task`.

**Test code (3 files):**

5. **`src/daemon/worktree.rs` (tests)** — Updated all 3 unit tests for new signature; changed the "missing branch" test to verify the `-B` migration fallback creates the branch successfully.

6. **`src/validate/tests_daemon.rs`** — Updated all 11 `create_worktree` call sites for new signature, updated branch assertions from `ralph/daemon/` to `ralph/issue-`, added new `runtime_reuse_worktree_migrates_old_daemon_branch` regression test.

7. **`src/validate/tests_pr_runtime.rs`** — Updated `expected_branch` from `ralph/daemon/{task_id}` to `ralph/issue-{issue_number}`.

8. **`src/validate/mock_scripts.rs`** — Updated doc comment.
