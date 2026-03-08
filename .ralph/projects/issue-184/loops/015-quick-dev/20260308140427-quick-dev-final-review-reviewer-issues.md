---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T14:04:27Z
---

# Final Review: AMENDMENTS

## Amendment: [P0] Drain timeout can mark a task failed while it is still executing

### Problem
`drain_all_children_with_deadline()` force-removes tasks and calls `complete_task()` immediately after `join_handle.abort()` ([`src/daemon/runtime.rs:2030`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:2030)).  
`JoinHandle::abort()` is cooperative; it does not preempt a task stuck in synchronous blocking code. Orchestration tasks still execute blocking git/process work in async flows (for example [`src/git/mod.rs:11`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/git/mod.rs:11), [`src/workflow/orchestrator.rs:274`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:274), [`src/workflow/orchestrator.rs:4752`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:4752), [`src/workflow/quick_dev_orchestrator.rs:1276`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs:1276)).  
Result: the daemon can label a task terminally failed while the task can still mutate git state.

### Proposed Change
Make orchestration git/process calls cancellation-aware and timeout-bounded, and only apply terminal transition after the aborted join handle has actually resolved (bounded wait + explicit unresolved handling).

### Affected Files
- `src/daemon/runtime.rs` - force-abort sequencing and terminal transition.
- `src/git/mod.rs` - blocking git primitive.
- `src/workflow/orchestrator.rs` - blocking git/checkpoint call sites inside async run.
- `src/workflow/quick_dev_orchestrator.rs` - blocking checkpoint/git call sites inside async phase machine.

## Amendment: [P1] External abort does not immediately stop watcher side effects

### Problem
On external abort, `kill_aborted_children()` cancels only the task token ([`src/daemon/runtime.rs:1983`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:1983)).  
Watcher cancellation is deferred until task completion in `collect_children()` ([`src/daemon/runtime.rs:1829`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:1829)).  
If the task does not exit promptly, draft-PR and artifact-comment watchers can continue acting after abort ([`src/daemon/runtime.rs:265`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:265), [`src/daemon/runtime.rs:429`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:429)).

### Proposed Change
Cancel `watcher_cancel` and `draft_pr_cancel` immediately in `kill_aborted_children()` when marking `aborted_externally`, instead of waiting for join completion.

### Affected Files
- `src/daemon/runtime.rs` - external-abort path and watcher cancellation timing.

## Amendment: [P2] “Non-cooperative task” test is cooperative and can pass for the wrong reason

### Problem
`drain_all_children_force_aborts_non_cooperative_task` says it validates non-cooperative behavior, but the task body uses `tokio::time::sleep(...).await` in a loop ([`src/daemon/runtime.rs:4098`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:4098)), which is abort-cooperative.  
The test only asserts map removal ([`src/daemon/runtime.rs:4152`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:4152)), not that execution truly stopped.

### Proposed Change
Replace the body with a genuinely non-cooperative blocking section and assert no post-drain side effects occur after force-abort.

### Affected Files
- `src/daemon/runtime.rs` - drain timeout/force-abort test block.
