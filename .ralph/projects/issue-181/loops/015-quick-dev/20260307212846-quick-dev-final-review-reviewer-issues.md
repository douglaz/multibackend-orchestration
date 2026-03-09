---
artifact: quick-dev-final-review
loop: 15
project: issue-181
backend: codex
role: reviewer
created_at: 2026-03-07T21:28:46Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Avoid Destructive Branch Reset in Worktree Reuse

### Problem
In branch-mismatch recovery, [`src/daemon/worktree.rs:270`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs:270) now runs `git checkout --force -B <expected_branch>`.  
`-B` always resets `<expected_branch>` to the current `HEAD`, even when `<expected_branch>` already exists.

That means if a reused worktree is temporarily on a different branch, this path can silently rewrite `ralph/issue-*` to the wrong commit before sync/dispatch completes (data-loss risk on failure paths and for local-only commits).

Current tests don’t catch this ref-overwrite case:
- [`src/daemon/worktree.rs:697`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs:697) validates only missing-branch migration.
- [`src/validate/tests_daemon.rs:2321`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/validate/tests_daemon.rs:2321) checks branch name after correction, but not whether expected branch tip was preserved.

### Proposed Change
Make correction two-phase and non-destructive by default:
1. Try `git checkout --force <expected_branch>` first.
2. Only if the branch truly does not exist, fallback to `git checkout --force -B <expected_branch>` (migration path).
3. If checkout fails for other reasons, return an error instead of resetting refs.

Add regression coverage where:
- expected issue branch already exists with unique commit,
- worktree is on mismatched branch,
- reuse/correction keeps expected branch commit unchanged.

### Affected Files
- [`src/daemon/worktree.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs) - make branch correction non-destructive and fallback-only.
- [`src/daemon/worktree.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs) - add/adjust unit test to assert branch tip preservation.
- [`src/validate/tests_daemon.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/validate/tests_daemon.rs) - add conformance test asserting existing issue-branch ref is not rewritten during mismatch correction.
