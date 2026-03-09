---
artifact: quick-dev-codex-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T10:48:32Z
---

# Review: CHANGES REQUESTED

1. **[P1] Cancellation hard-abort can leave backend descendants running** in [src/backend/mod.rs#L58](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs#L58) and [src/backend/mod.rs#L838](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs#L838).  
Both `KillOnDrop` and `kill_and_reap_child` gate escalation on the **leader PID** exiting (`waitpid`/`child.wait()`), not on the **process group** exiting. If the leader exits on `SIGTERM` but a descendant ignores `SIGTERM`, no `SIGKILL` escalation is sent, violating the “kill process group after 5s” requirement.

Fix: track process-group liveness (`kill(-pgid, 0)` / `ESRCH`) across the grace window, and send `SIGKILL` to the group if still alive at deadline; only use `waitpid`/`child.wait()` for best-effort reaping. Add a regression test where parent exits on `SIGTERM` but child ignores it.

2. **[P2] Spec phase-3 wiring is incomplete: CLI still bypasses new daemon task entrypoints** in [src/cli/auto.rs#L294](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/auto.rs#L294), [src/cli/run.rs#L32](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/run.rs#L32), [src/cli/quick_dev_auto.rs#L244](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/quick_dev_auto.rs#L244), [src/cli/quick_dev_run.rs#L57](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/quick_dev_run.rs#L57), while new shared entrypoints exist in [src/daemon/tasks.rs#L77](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs#L77).  
This leaves duplicated orchestration paths and increases divergence risk for CWD/cancel/env behavior.

Fix: refactor CLI `execute()` functions to call `run_auto_task`, `run_run_task`, `run_quick_dev_auto_task`, and `run_quick_dev_run_task` with `CancellationToken::new()`, keeping interactive `println!` UX in CLI wrappers only.
