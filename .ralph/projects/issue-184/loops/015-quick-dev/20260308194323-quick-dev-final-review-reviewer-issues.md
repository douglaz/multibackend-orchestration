---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T19:43:23Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] AMD-001 Quick-PRD Cancellation Uses Future Drop Instead Of Cooperative Shutdown

### Problem
Both auto entrypoints race quick-prd with an outer cancellation branch:

- [`src/daemon/tasks.rs:178`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:178)
- [`src/daemon/tasks.rs:386`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:386)

When cancellation wins, `quick_prd.run(...)` is dropped mid-flight. For CLI backends, drop activates the emergency `KillOnDrop` path:

- [`src/backend/mod.rs:49`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:49)
- [`src/backend/mod.rs:61`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:61)
- [`src/backend/mod.rs:601`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:601)

That bypasses the normal cooperative SIGTERM→grace→SIGKILL cleanup path and can leave less deterministic teardown behavior during daemon cancellation.

### Proposed Change
Remove the outer `tokio::select!` cancellation race around `quick_prd.run(...)` in both task entrypoints.  
Let `QuickPrdPipeline` handle cancellation internally via its shared `CancellationToken` and cancel-aware backend calls. Keep only lightweight pre/post `is_cancelled()` checks if needed for fast fail before/after the run.

### Affected Files
- [`src/daemon/tasks.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs) - remove outer cancel `select!` around quick-prd execution and rely on cooperative cancellation inside pipeline/backend.

## Amendment: [P2] AMD-002 Concurrency Conformance Test Does Not Actually Prove Concurrency

### Problem
`concurrent_dispatch_evidence` infers concurrency from “both dispatch logs appear before terminal logs”:

- [`src/validate/tests_daemon_concurrency.rs:686`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:686)

But daemon control flow always runs `poll_and_claim(...)` before the next `collect_children(...)` call:

- [`src/daemon/runtime.rs:866`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:866)
- [`src/daemon/runtime.rs:873`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:873)

So the asserted ordering can pass even with fully sequential task execution.

### Proposed Change
Change the test to assert real overlap (or equivalent parallel evidence), e.g.:
1. Use a deterministic backend/mock that records per-issue START/END timestamps.
2. Assert interval overlap across issues.
3. Keep log-order checks only as secondary sanity assertions.

### Affected Files
- [`src/validate/tests_daemon_concurrency.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs) - replace ordering-based “concurrency evidence” assertion with overlap-based proof.
- [`src/validate/mock_scripts.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/mock_scripts.rs) - add/restore deterministic overlap instrumentation helper if needed.

## Amendment: [P3] AMD-003 Two-Issue Dispatch Test Can Pass Without Verifying Success Path

### Problem
`concurrent_dispatch_two_issues` no longer asserts successful daemon exit and only validates label operations if the log file happens to exist:

- [`src/validate/tests_daemon_concurrency.rs:136`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:136)
- [`src/validate/tests_daemon_concurrency.rs:170`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:170)

This can hide regressions where daemon execution fails or label transitions are never written.

### Proposed Change
Reinstate strict assertions:
1. `assert_exit_code(&output, 0)`.
2. Assert label log exists.
3. Then assert both issues were claimed/transitioned.

### Affected Files
- [`src/validate/tests_daemon_concurrency.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs) - strengthen pass/fail criteria for the two-issue dispatch test.
