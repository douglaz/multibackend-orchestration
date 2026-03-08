---
artifact: quick-dev-codex-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T08:59:33Z
---

# Review: CHANGES REQUESTED
1. [P1] Cancellation cleanup does not implement the required graceful-then-hard fallback behavior.  
   Evidence: [src/backend/mod.rs:54](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:54), [src/backend/mod.rs:64](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:64), [src/workflow/orchestrator.rs:6088](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:6088).  
   Problem: cancellation drops `execute_with_log` and `KillOnDrop` immediately sends `SIGKILL`; there is no 5s graceful termination window before hard kill.  
   Fix: implement cancellation cleanup as `SIGTERM` + bounded wait (5s) + `SIGKILL` fallback (or equivalent helper reused by cancellation path), then reap.

2. [P2] `RALPH_MAX_BACKEND_RETRIES` is still read from env in CLI entrypoints, and daemon in-process tasks always pass `None`, creating inconsistent behavior across code paths and violating the “moved to options field” requirement.  
   Evidence: [src/cli/run.rs:12](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/run.rs:12), [src/cli/auto.rs:17](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/auto.rs:17), [src/cli/quick_dev_auto.rs:18](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/quick_dev_auto.rs:18), [src/cli/quick_dev_run.rs:13](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/quick_dev_run.rs:13), [src/daemon/tasks.rs:172](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:172).  
   Fix: remove env reads from CLI shims and pass explicit option values consistently from callers (or add an explicit CLI/config field and thread it through both CLI and daemon task params).

3. [P3] The new “timeout-abort” drain test does not exercise the timeout/abort branch it claims to validate.  
   Evidence: [src/daemon/runtime.rs:3836](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:3836), [src/daemon/runtime.rs:3878](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:3878), [src/daemon/runtime.rs:1954](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:1954).  
   Problem: the test task exits on cancellation immediately, so `drain_all_children` never reaches the forced-abort path behind the 7200s deadline.  
   Fix: make drain timeout injectable in tests and use a task that ignores cancellation to assert `join_handle.abort()` path and terminal failure handling.
