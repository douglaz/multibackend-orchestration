---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T16:36:12Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] TMUX-ENV-EXPORT-BROKEN

### Problem
Configured backend env vars are not actually exported in tmux mode.  
In [`src/backend/tmux_backend.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/tmux_backend.rs):136, the command is built as `export 'KEY'='value';` (quoted key), which is a shell no-op for setting variables.  
Tests in [`src/backend/tmux_backend.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/tmux_backend.rs):652 and :671 only assert string shape, so they pass even though runtime behavior is wrong.

### Proposed Change
Emit exports as `export KEY='value';` (unquoted validated identifier for key, escaped value).  
Reject or skip invalid env keys explicitly.  
Update tests to execute the built shell snippet and assert env visibility/sanitization behavior at runtime.

### Affected Files
- [`src/backend/tmux_backend.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/tmux_backend.rs) - fix export generation and strengthen tests.

## Amendment: [P2] DRAIN-SHUTDOWN-WATCHER-RACE

### Problem
Shutdown drain cancels only orchestration tasks, not watcher tasks initially.  
[`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs):2019-2022 cancels `cancel_token` only.  
Meanwhile, draft/artifact watchers continue running and can still perform side effects (including PR creation at :265-297) until later teardown paths.

### Proposed Change
In the initial drain cancellation loop, also cancel `watcher_cancel` and `draft_pr_cancel` for every active handle, not only `cancel_token`.  
Add a regression test ensuring drain cancellation prevents draft-PR side effects.

### Affected Files
- [`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs) - cancel watcher tokens at drain start and add coverage.

## Amendment: [P3] DAEMON-CONFORMANCE-TEST-MISSES-ITS-CLAIMED-ASSERTION

### Problem
`dispatch_fresh_issue_passes_project_id` does not actually verify project-id pass-through.  
In [`src/validate/tests_daemon.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs):2582-2629, it only checks dispatch occurred and worktree exists; it never asserts that `issue-500` was passed/used as project ID.

### Proposed Change
Assert the dispatch path includes `--project-id issue-500` (or equivalent quick-dev fresh path), and assert resulting project artifacts/state are under `.ralph/projects/issue-500` in the worktree.

### Affected Files
- [`src/validate/tests_daemon.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs) - strengthen test assertions to match test intent.
