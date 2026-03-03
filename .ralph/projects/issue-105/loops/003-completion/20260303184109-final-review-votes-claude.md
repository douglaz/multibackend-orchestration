---
artifact: final-review-votes
loop: 3
project: issue-105
backend: claude
role: final_reviewer
created_at: 2026-03-03T18:41:09Z
---

# Vote Results

## Amendment: RVW-DAEMON-001

### Vote
ACCEPT

### Rationale
The planner's analysis is thorough and correct. The dispatch JoinSet panic path at `runtime.rs:1223` is indeed a real gap: when a spawned dispatch task panics, Tokio's `JoinError` does not carry the `(issue_number, Result<ChildHandle, RalphError>)` return value, so the code cannot identify which issue panicked. Since the issue was already claimed (`ralph:ready` → `ralph:in-progress`) at line 1122 but the `ChildHandle` was never inserted into `children`, the issue becomes permanently stuck — it won't be collected, re-polled, or cleaned up by `kill_aborted_children`. The fix (using `catch_unwind` within the spawned task or propagating `issue_number` outside the panic-losing return path) is straightforward and preserves the lifecycle invariant that every claimed issue must reach a terminal state.

## Amendment: RVW-DAEMON-002

### Vote
ACCEPT

### Rationale
This is the same class of bug as RVW-DAEMON-001 but on the completion path, and the planner's verification is accurate. The child is removed from `children` at line 1660 *before* `complete_task` is spawned into the JoinSet at line 1690. If `complete_task` panics before performing the lifecycle label swap at line 1960-1968, the issue is stuck in `ralph:in-progress` with no recovery path — it's not in `children`, it's not `ralph:ready`, and the completion JoinSet type is `JoinSet<()>` so the panic handler at line 1695 cannot even recover the `issue_number`. The fix should propagate structured completion data (at minimum `issue_number` and the intended terminal label) so the panic handler can execute a fallback transition to `ralph:failed`.

## Amendment: RVW-DAEMON-003

### Vote
ACCEPT

### Rationale
The planner's verification of all three claims is convincing:

1. **Concurrency not proven**: The test only asserts both issues were dispatched, which sequential dispatch would also satisfy. The test name and docstring claim concurrency but the assertions don't demonstrate it.

2. **Wrong failure path exercised**: The mock script's `exit 1` is detected during `collect_children`/`complete_task`, not during `dispatch_task`. The test claims to exercise dispatch failure rollback but actually exercises child terminal-state failure — a fundamentally different code path.

3. **Overly weak assertion**: `contains("301")` as a fallback is essentially a no-op check that would pass on any output mentioning the issue number in any context.

These test weaknesses mean the concurrency feature's correctness claims are not adequately backed by evidence. Strengthening them is warranted, especially given that RVW-DAEMON-001 and RVW-DAEMON-002 identify real bugs in the paths these tests purport to cover.
