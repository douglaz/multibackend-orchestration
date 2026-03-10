---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T15:17:32Z
---

# Final Review: AMENDMENTS

## Amendment: PRR-001 [P1] Closed-PR Candidates Can Deadlock Ready Issues

### Problem
With `workspace.daemon_pr_review_whitelist` enabled, claim flow skips any ready issue that has a resume marker or staged PR-review amendments ([runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1136), [runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1142)).

In PR-review flow, if the associated PR is closed, the candidate is just skipped ([runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2640), [runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2666)) without clearing marker/staged state.  
This can leave the issue permanently unclaimable in normal polling.

### Proposed Change
When PR state is closed in `pr_review_phase`:
1. Clear resume-pending marker for that task.
2. Purge staged PR-review amendments for that task.
3. Log explicit cleanup reason (`PR closed; clearing PR-review resume state`).

Add a conformance test that sets `ralph:ready` + staged/marker + `PR_STATE=closed` and verifies the PR-review blocker state is cleared so normal claim path is no longer blocked on subsequent tick.

### Affected Files
- `src/daemon/runtime.rs` - clear PR-review blocker state on closed PR instead of silent `continue`.
- `src/validate/tests_pr_review.rs` - add regression test for closed-PR cleanup/unblock behavior.

## Amendment: PRR-002 [P2] PR-Review Dispatch Path Lacks Panic Isolation

### Problem
`poll_and_claim` isolates `dispatch_task` panics via nested `tokio::spawn` and `JoinError` handling ([runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1220)).  
`pr_review_phase` calls `dispatch_task` directly ([runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2855)).

If `dispatch_task` panics in PR-review resume flow, that panic can unwind the daemon task and bypass rollback/marker handling, leaving lifecycle state inconsistent.

### Proposed Change
Mirror claim-path robustness in `pr_review_phase`:
1. Wrap dispatch in panic-isolating spawn.
2. Convert panic (`JoinError`) into failure path.
3. Reuse existing rollback logic (`in-progress -> from_label`) and marker handling for both error and panic outcomes.

Add a regression test (or small injectable panic hook in test builds) proving panic does not crash the daemon loop and rollback/marker semantics still execute.

### Affected Files
- `src/daemon/runtime.rs` - panic-isolated dispatch in PR-review resume path.
- `src/validate/tests_pr_review.rs` - regression coverage for panic containment in PR-review dispatch.

---
