---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T09:06:17Z
---

Now I have a comprehensive picture. Let me write the final review.

# Final Review: AMENDMENTS

## Summary

The migration from forked subprocesses to in-process tokio tasks is substantially complete and well-structured. Key architectural changes — `TaskHandle` replacing `ChildHandle`, `CancellationToken`-based cooperative cancellation, per-task tracing subscribers via `WithSubscriber`, `KillOnDrop` guard for cancellation safety, `SANITIZED_ENV_VARS` at the backend layer — are all correctly implemented. The `derive_terminal_label` function has unit tests covering all four paths including panics. The `drain_all_children` test verifies timeout-abort behavior. Log isolation and cancellation tests exist in `src/daemon/tasks.rs`.

The issues below are real bugs or gaps that should be addressed.

## Amendment: A1

### Problem
**[P1]** In `src/backend/mod.rs:66`, the `KillOnDrop::drop` implementation calls `libc::waitpid(raw, ...)` with a positive `raw` value (the child PID). While this correctly reaps the group leader process we spawned, the comment on line 65 says "Non-blocking reap to avoid zombies when possible." Because `setsid()` was called (line 545), the child's PID equals its PGID. If we want to reap the leader specifically, using the positive PID is correct. However, if the child has already exited before `KillOnDrop` fires (e.g., process completed normally but guard wasn't disarmed due to an unexpected code path), we'd be calling `kill(-(raw), SIGKILL)` on a potentially recycled PGID. The guard is disarmed at line 732 in all normal paths, and the guard only fires on future cancellation (drop), so the window for PID recycling is small but nonzero.

More importantly: `waitpid(raw, ..., WNOHANG)` with a positive PID only waits for that exact PID. But since this child was spawned by `tokio::process::Command`, tokio's `Child` struct also monitors that PID via its internal signal handler. Calling `waitpid` on a PID that tokio is also tracking can race with tokio's reaper, causing one of them to get `ECHILD`. In the Drop path this is harmless (result is ignored), but it's worth a comment noting this.

**Severity**: Low — the guard only fires on unexpected future drops, and the `waitpid` result is ignored. No functional impact, but the code would benefit from a comment.

### Proposed Change
Add a clarifying comment at `src/backend/mod.rs:66` noting that the `waitpid` call may race with tokio's child reaper and the result is intentionally ignored. No code change needed.

### Affected Files
- `src/backend/mod.rs` - comment improvement at line 65-66

---

## Amendment: A2

### Problem
**[P2]** In `src/workflow/orchestrator.rs:6132` and `src/workflow/quick_dev_orchestrator.rs:1471,1489`, the retry backoff `sleep` in `execute_with_timeout_retries` is not wrapped in a `tokio::select!` with the cancellation token. If the token is cancelled during a backoff period, the task will block for the full backoff duration (up to 8 seconds for attempt 4, exponential) before noticing cancellation on the next loop iteration.

This delays cancellation responsiveness during retry backoff. The spec says "check it between phases" and "short-circuit on cancellation via `tokio::select!`" — the select is applied to the backend execution call but not to the backoff sleep.

### Proposed Change
Wrap the backoff sleep in `tokio::select!` with `cancel.cancelled()`:

In `src/workflow/orchestrator.rs` around line 6132:
```rust
tokio::select! {
    _ = sleep(Duration::from_secs(backoff)) => {},
    _ = cancel.cancelled() => return Err(RalphError::Cancelled),
}
```

Same pattern in `src/workflow/quick_dev_orchestrator.rs` at lines 1471 and 1489.

### Affected Files
- `src/workflow/orchestrator.rs` - line 6132
- `src/workflow/quick_dev_orchestrator.rs` - lines 1471, 1489

---

## Amendment: A3

### Problem
**[P2]** `run_in()` in `src/prd/quick.rs` was not renamed to `run()` as specified. The spec says: "Change `run()` to require an explicit `working_dir: PathBuf` parameter (removing the zero-arg `run()`, making `run_in()` the public API, renamed to `run()`)." The zero-arg `run()` was removed (confirmed: no `pub async fn run` match), but the method is still named `run_in()` at line 301. All callers have been updated to call `run_in()`, so there is no correctness bug, but the naming diverges from the spec.

**Severity**: Low — purely naming consistency. All call sites are correct.

### Proposed Change
Rename `run_in` to `run` in `src/prd/quick.rs:301` and update the 4 call sites:
- `src/cli/auto.rs:237`
- `src/cli/quick_dev_auto.rs:212` (approximately)
- `src/cli/quick_prd.rs:116`
- `src/daemon/tasks.rs` (2 call sites)

### Affected Files
- `src/prd/quick.rs` - rename method
- `src/cli/auto.rs`, `src/cli/quick_dev_auto.rs`, `src/cli/quick_prd.rs`, `src/daemon/tasks.rs` - update call sites

---

## Amendment: A4

### Problem
**[P2]** The `max_backend_retries` helper function is duplicated identically in both `src/workflow/orchestrator.rs:6143-6151` and `src/workflow/quick_dev_orchestrator.rs:1501-1509`. Both have identical logic (default 3, max 10, treat 0 as None). This is copy-paste duplication that could diverge over time.

### Proposed Change
Extract `max_backend_retries()` to a shared location (e.g., a small helper in `src/workflow/mod.rs` or `src/backend/mod.rs`) and import from both orchestrators.

### Affected Files
- `src/workflow/orchestrator.rs` - remove local function, import shared one
- `src/workflow/quick_dev_orchestrator.rs` - remove local function, import shared one
- `src/workflow/mod.rs` (or similar) - add shared function

---

## Amendment: A5

### Problem
**[P3]** Several test assertions in `src/validate/tests_daemon_concurrency.rs` use overly loose `OR` patterns that could pass for the wrong reason. For example, at lines 656-662:
```rust
assert!(
    combined.contains("acme-widgets-600 completed")
        || combined.contains("acme-widgets-600"),
    "issue 600 should complete: {combined}"
);
```
The second clause matches any mention of the issue number, even if the task never completed. Similarly, in the same file at lines ~151-159 and ~223-231, assertions accept either `"dispatched task"` or `"dispatch: task"` patterns.

These tests would pass if the task simply appeared in any log line, rather than actually completing or being dispatched.

### Proposed Change
Tighten the assertions to require the specific string that proves the behavior being tested. Remove the overly generic fallback clauses. For example:
```rust
assert!(
    combined.contains("acme-widgets-600 completed"),
    "issue 600 should complete: {combined}"
);
```

### Affected Files
- `src/validate/tests_daemon_concurrency.rs` - tighten assertions at ~10 locations

---
