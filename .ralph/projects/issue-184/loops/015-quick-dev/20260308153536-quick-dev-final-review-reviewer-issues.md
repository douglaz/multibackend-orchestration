---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T15:35:36Z
---

# Final Review: AMENDMENTS

## Amendment: [P0] Cancellation Returns Before Backend Subprocesses Are Reaped

### Problem
Cancellation currently drops backend execution futures and relies on a detached thread in `Drop` to finish killing subprocess groups later.  
This creates a race where the orchestration task can finish and be terminalized before backend descendants are actually dead.

Evidence:
- Detached reaper thread in [`src/backend/mod.rs:59`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:59) to [`src/backend/mod.rs:123`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:123)
- Cancellation immediately returns from backend call sites in [`src/workflow/orchestrator.rs:6097`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:6097) and [`src/workflow/quick_dev_orchestrator.rs:1440`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs:1440)
- Runtime treats task as done and proceeds with terminal handling/cleanup in [`src/daemon/runtime.rs:1807`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:1807) and [`src/daemon/runtime.rs:2259`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:2259)

This is a correctness/safety issue: a cancelled task may be marked terminal while backend processes are still running.

### Proposed Change
Make backend cancellation synchronous from the task’s perspective:
1. Add a cancellation-aware backend execution path that performs awaited `kill_and_reap_child()` before returning `Cancelled`.
2. Remove detached-thread process-group killing from `KillOnDrop` (or reduce it to emergency SIGKILL-only fallback, not primary control flow).
3. Ensure orchestrator cancellation waits for backend cleanup completion before task completion is reported.

### Affected Files
- `src/backend/mod.rs` - replace `KillOnDrop` detached reaper logic with awaited cancellation cleanup path.
- `src/workflow/orchestrator.rs` - call cancellation-aware backend execution.
- `src/workflow/quick_dev_orchestrator.rs` - call cancellation-aware backend execution.
- `src/daemon/runtime.rs` - keep terminalization flow dependent on real task completion after cleanup.

## Amendment: [P2] Daemon Concurrency Conformance Tests No Longer Prove Claimed Behavior

### Problem
`concurrent_dispatch_evidence` claims to prove concurrent execution, but assertions only check that both tasks were dispatched and completed. Sequential execution would also pass.

Evidence:
- Claim in comments in [`src/validate/tests_daemon_concurrency.rs:606`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:606)
- Assertions in [`src/validate/tests_daemon_concurrency.rs:642`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:642) to [`src/validate/tests_daemon_concurrency.rs:662`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:662) do not verify overlap/order constraints.

### Proposed Change
Restore deterministic concurrency evidence:
1. Add explicit start/end timing markers (or barrier-based signals) per task.
2. Assert overlap or ordering that cannot pass under sequential dispatch.
3. Keep current in-process assertions as secondary checks, not primary proof.

### Affected Files
- `src/validate/tests_daemon_concurrency.rs` - strengthen `concurrent_dispatch_evidence` assertions to validate actual concurrency.

## Amendment: [P2] Dispatch-Failure Isolation Coverage Was Weakened

### Problem
Tests named/positioned for dispatch-failure isolation now assert both issues fail due single-iteration cancellation, so they no longer exercise “one dispatch fails, sibling succeeds” behavior.

Evidence:
- `partial_dispatch_rollback` now expects both issues to reach `ralph:failed` in [`src/validate/tests_daemon_concurrency.rs:256`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:256)
- `dispatch_failure_explicit_markers` expects failure markers for both issues in [`src/validate/tests_daemon_concurrency.rs:587`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:587)

This can pass even if per-issue rollback isolation is broken.

### Proposed Change
Add a dedicated in-process test that forces dispatch failure for exactly one issue while another dispatch succeeds, then assert:
1. only the failing issue gets rollback/failure transition from dispatch path,
2. the successful sibling is not mislabeled/rolled back.

### Affected Files
- `src/validate/tests_daemon_concurrency.rs` - add a true mixed-outcome dispatch test and restore isolation assertions.
