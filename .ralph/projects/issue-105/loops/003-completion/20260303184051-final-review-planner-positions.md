---
artifact: final-review-planner-positions
loop: 3
project: issue-105
backend: claude(opus)
role: planner
created_at: 2026-03-03T18:40:51Z
---

Good. Now I have a complete picture. Let me verify one more detail — the JoinSet's `Err(err)` on line 1223: when a Tokio JoinSet task panics, the `JoinError` does **not** carry the return value (the tuple `(u32, Result<...>)`). So in the panic case, the `issue_number` is indeed lost. The code at line 1223 only logs and cannot identify which issue panicked.

Similarly for the completion path at line 1695-1698 — but there the child is already removed from `children` at line 1660, so a panic in `complete_task` (spawned in the JoinSet at line 1690) means the issue_number is lost for recovery because the `complete_set` closure captures `issue_number` but on panic, it's not returned.

Let me verify the `Err` case in `JoinSet::join_next()` — Tokio's `JoinError` has a method `into_panic()` but does NOT provide the task's return value on panic. The `(u32, Result<ChildHandle, RalphError>)` return value is lost.

# Planner Positions

## Amendment: RVW-DAEMON-001

### Position
ACCEPT

### Rationale
Verified in `runtime.rs:1189-1227`. The dispatch JoinSet's type is `JoinSet<(u32, Result<ChildHandle, RalphError>)>`. When a spawned dispatch task panics (line 1195-1198), Tokio's `JoinError` does **not** carry the task's return value — the `(issue_number, result)` tuple is lost. The `Err(err)` arm at line 1223-1225 only logs `"warning: dispatch task panicked: {err}"` and does not perform any label rollback.

The problem is real: after claiming (`ralph:ready` → `ralph:in-progress`) at lines 1122-1141, if the dispatch future panics, the issue remains `ralph:in-progress` with no entry in `children` (since the handle was never inserted at line 1205). It will not be collected, will not be re-polled (it's not `ralph:ready`), and will not be cleaned up by `kill_aborted_children` (which only iterates existing `children` keys). The issue is permanently stuck until manual intervention.

While panics in `dispatch_task` are unlikely under normal operation (it's async and uses `spawn_blocking_op` wrappers), Rust does not prevent them — any `unwrap()`, `expect()`, or assertion failure inside the spawned future would trigger this path. The fix (extracting `issue_number` from the panic context or using `catch_unwind` within the spawned task) is straightforward and correct.

## Amendment: RVW-DAEMON-002

### Position
ACCEPT

### Rationale
Verified in `runtime.rs:1655-1699`. The completion flow works as follows:

1. **Line 1660**: `children.remove(&issue_number)` — child is removed from the map
2. **Lines 1663-1678**: Watcher/PR handles are cancelled and joined; child metadata is collected into `completion_tasks`
3. **Lines 1686-1699**: A `JoinSet<()>` spawns `complete_task` for each finished child

Inside `complete_task` (lines 1885-1919), the lifecycle label swap happens at line 1960-1968 (`ralph:in-progress` → terminal label). If `complete_task` panics before reaching line 1960, the label swap never executes.

The panic handler at line 1695-1698 only logs `"warning: complete_task panicked: {err}"`. Since the JoinSet type is `JoinSet<()>`, even the `issue_number` is not recoverable from the `JoinError`. The child has already been removed from `children` at line 1660, so:
- The issue won't be collected again (no entry in `children`)
- The issue won't be re-polled (it's `ralph:in-progress`, not `ralph:ready`)
- `kill_aborted_children` won't find it (iterates `children` keys only)

The issue is stuck in `ralph:in-progress` permanently. This is the same class of bug as RVW-DAEMON-001 but on the completion path rather than the dispatch path. The fix is equally straightforward — propagate structured data (at minimum `issue_number` and `terminal_label`) from the JoinSet task so that the panic handler can execute a fallback label transition.

## Amendment: RVW-DAEMON-003

### Position
ACCEPT

### Rationale
Verified all three claims against the test source in `tests_daemon_concurrency.rs`:

**Claim 1 — `concurrent_dispatch_two_issues` doesn't prove concurrency (line 107):**
Correct. The test asserts that both issues 200 and 201 are "dispatched" (lines 150-158) by checking output strings. This assertion would pass identically if dispatch were sequential. There is no timing barrier, concurrency marker, or ordering evidence. The test name and docstring (line 107-109) claim "both claimed and dispatched concurrently in a single poll cycle" but the assertions only prove "both dispatched," not "concurrently."

**Claim 2 — `partial_dispatch_rollback` tests child failure, not dispatch failure (line 201):**
Correct and this is a significant mischaracterization in the test. The mock ralph at line 201-211 is a shell script that `exit 1` for issue 301. However, `dispatch_task` (line 1300) spawns the ralph process and returns a `ChildHandle` — the process exiting non-zero is detected later during `collect_children`, not during dispatch. Looking at `dispatch_task`'s code: `spawn_ralph_auto` (line 1512) returns `Ok(ChildHandle)` as soon as the process is spawned. The `exit 1` happens after dispatch succeeds. So the test exercises the `collect_children` → `complete_task` terminal path with `ralph:failed`, not the `dispatch_task` `Err` → rollback path at line 1207-1221. The test name `partial_dispatch_rollback` and docstring (lines 177-184: "when dispatch_task fails for one issue") are misleading.

**Claim 3 — Weak assertion at line 244:**
Correct. The assertion `combined.contains("failed to dispatch issue #301") || combined.contains("301")` (lines 245-246) will pass if the string "301" appears *anywhere* in the output — including in issue listing, label operations, or any other context. The `contains("301")` fallback makes this assertion effectively useless as a correctness check for dispatch failure logging.
