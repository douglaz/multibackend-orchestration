---
artifact: quick-dev-plan-implement
loop: 15
project: issue-181
backend: claude
role: implementer
created_at: 2026-03-07T20:59:40Z
---

All implementation is complete and verified. Here's a summary of what was done across the 7 modified files:

# Implementation Summary

**Problem:** The draft PR watcher used a hardcoded `ralph/daemon/{task_id}` branch, while `sync_project_branch` immediately switched to `ralph/issue-{N}`, causing `gh pr create` to fail.

**Changes applied:**

1. **`src/daemon/worktree.rs`** — `create_worktree` accepts `branch_name: &str` parameter instead of computing it internally. `verify_worktree_branch` uses `git checkout --force -B` to handle migration from old daemon branches. Added migration regression test.

2. **`src/daemon/runtime.rs`** — `dispatch_task` computes `branch_name` early via `resolve_branch_name()` and passes it to `create_worktree`. All downstream consumers (`find_existing_pr`, `draft_pr_watcher`, `ChildHandle.branch`) automatically use the correct `ralph/issue-{N}` value.

3. **`src/daemon/github.rs`** — Doc comment updated to reference project branch instead of old daemon branch.

4. **`src/git/branch.rs`** — Test setup uses `ralph/issue-1` branch name.

5. **`src/validate/tests_daemon.rs`** — All `create_worktree` call sites updated for new signature, branch assertions updated. New `runtime_reuse_worktree_migrates_old_daemon_branch` test verifies migration from `ralph/daemon/{task_id}` to `ralph/issue-{N}`.

6. **`src/validate/tests_pr_runtime.rs`** — `expected_branch` updated to `ralph/issue-{N}`.

7. **`src/validate/mock_scripts.rs`** — Doc comment updated.

**Verification:** Build, fmt, clippy, and all unit tests pass.
