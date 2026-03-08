---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T18:57:43Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] DRAIN-EXTERNAL-ABORT-FLAG-LOSS

### Problem
In forced-drain timeout handling, externally aborted tasks are completed as if they were normal failures.  
At [src/daemon/runtime.rs:2088](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:2088), `complete_task` is called with `externally_aborted = false` unconditionally. If the task had already been externally aborted, this can trigger incorrect `in-progress -> failed` swap attempts against labels that were already changed, causing noisy retries and missed intended abort semantics.

### Proposed Change
Propagate the per-task `aborted_externally` state in the forced-abort path:
1. Read `handle.aborted_externally.load(...)` before calling `complete_task`.
2. Pass that value instead of hardcoded `false`.
3. Keep terminalization logic aligned with the normal `collect_children` path.

### Affected Files
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs) - Preserve external-abort state during forced drain completion.

## Amendment: [P2] TEST-CONCURRENCY-ORDERING-CAN-BE-SKIPPED

### Problem
The concurrency proof test can pass without asserting ordering.  
In [src/validate/tests_daemon_concurrency.rs:703](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:703), ordering is checked only inside `if let (Some(...), Some(...))`. If positions are not found, the test silently skips the key assertion.

### Proposed Change
Make ordering mandatory:
1. Assert both `last_dispatch_pos` and `first_terminal_pos` are `Some(...)`.
2. Fail explicitly if either position is missing.
3. Include all accepted terminal markers in the position search so the check can’t be bypassed by alternate log wording.

### Affected Files
- [src/validate/tests_daemon_concurrency.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs) - Strengthen concurrency ordering assertion so it always executes.

## Amendment: [P3] TEST-FRESH-DISPATCH-PROJECT-CREATION-NOT-ACTUALLY-ASSERTED

### Problem
A key assertion is conditional and can never fail when the directory is missing.  
In [src/validate/tests_daemon.rs:2647](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:2647), `state.json` is only checked inside `if project_state_dir.exists()`. This allows the test to pass even if project creation regresses completely.

### Proposed Change
Require existence explicitly:
1. Assert `project_state_dir.exists()` unconditionally.
2. Then assert `state.json` exists.
3. If intermittent creation is acceptable, rename the test and narrow its claim to logging-only behavior.

### Affected Files
- [src/validate/tests_daemon.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs) - Make the project-creation assertion enforce the stated behavior.

---
