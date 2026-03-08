---
artifact: quick-dev-codex-review
loop: 15
project: issue-181
backend: codex
role: reviewer
created_at: 2026-03-07T22:18:48Z
---

# Review: CHANGES REQUESTED

1. High: branch correction is over-broad and rewrites existing issue branches.
- In [src/daemon/worktree.rs#L270](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs#L270), `verify_worktree_branch` now always runs `git checkout --force -B <expected_branch>`.
- `-B` resets `<expected_branch>` to the current (mismatched) HEAD even when `<expected_branch>` already exists. That is broader than the migration need (missing branch only), and can preserve wrong commits on fallback paths.
- Concrete fix:
  1. In `verify_worktree_branch`, first check `refs/heads/<expected_branch>` existence.
  2. If it exists: run `git checkout --force <expected_branch>` (no `-B`).
  3. If it does not exist: run `git checkout --force -B <expected_branch>` (migration create/reset path).
  4. Keep current error propagation/logging.

2. Medium: regression test does not guard against unintended branch pointer rewrite.
- [src/validate/tests_daemon.rs#L2323](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/validate/tests_daemon.rs#L2323) only checks branch name after reuse, not commit SHA.
- Concrete fix:
  1. In `runtime_reuse_worktree_corrects_branch_mismatch`, capture SHA of `expected_branch` before switching to mismatch branch.
  2. After `create_worktree` reuse, assert `rev-parse expected_branch` is unchanged.
  3. Keep existing migration test ([same file around `runtime_reuse_worktree_migrates_old_daemon_branch`]) for missing-branch creation behavior.

Everything else in the spec wiring looks correct: dispatch now resolves/plumbs one branch name through `create_worktree`, `find_existing_pr`, draft watcher, and `ChildHandle.branch`.
