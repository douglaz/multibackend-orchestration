---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T11:25:35Z
---

Now I have enough information. Let me compile the final review.

# Final Review: AMENDMENTS

## Amendment: TMX-CANCEL-01

### Problem
`TmuxBackend::execute_raw()` in `src/backend/tmux_backend.rs` has no drop guard for cancellation safety. When the orchestrator cancels a task via `tokio::select!`, the `execute_raw()` future is dropped mid-await (during `wait_for_exit_with_activity()` at ~line 235). The cleanup path at lines 302-313 (`kill_window_best_effort()`) runs **after** the await and is therefore skipped on drop. This leaves orphaned tmux windows and backend processes running indefinitely.

In contrast, `CliBackend::execute_streaming()` correctly uses `KillOnDrop` (line ~614 of `src/backend/mod.rs`) to kill child process groups on future drop. The tmux path lacks an equivalent guard.

### Proposed Change
Add a `TmuxWindowGuard` drop guard (analogous to `KillOnDrop`) that captures the tmux session/window target and calls `kill_window_best_effort()` on drop. Disarm it on normal completion. This ensures cancelled futures still clean up tmux windows.

```rust
struct TmuxWindowGuard {
    runner: Arc<dyn TmuxRunner>,
    target: Option<String>, // e.g. "session:window"
}
impl Drop for TmuxWindowGuard {
    fn drop(&mut self) {
        if let Some(target) = self.target.take() {
            // Best-effort kill; non-blocking
            let runner = self.runner.clone();
            std::thread::spawn(move || {
                let _ = /* kill-window shell command */;
            });
        }
    }
}
```

### Affected Files
- `src/backend/tmux_backend.rs` - Add `TmuxWindowGuard`, create after `create_window_with_retry()`, disarm in normal cleanup path

---

## Amendment: ORCH-CANCEL-02

### Problem
`run_final_review_phase()` in `src/workflow/orchestrator.rs` (line 3741) accepts `cancel: &CancellationToken` but does not check `cancel.is_cancelled()` before beginning work. While the main loop at line 534 checks before dispatching to a phase, there is a window where cancellation could be signaled after the main-loop check but before the phase starts expensive reviewer/arbiter backend calls. All other major phase handlers or their inner functions check early; this one does not.

### Proposed Change
Add `if cancel.is_cancelled() { return Err(RalphError::Cancelled); }` near line 3757 (after the `info!` log, before constructing reviewer specs).

### Affected Files
- `src/workflow/orchestrator.rs` - Add early cancellation check in `run_final_review_phase()`

---

## Amendment: ORCH-CANCEL-03

### Problem
The completer loop in `src/workflow/orchestrator.rs` (line 2189, `for completer_backend_name in &effective_completers`) lacks a cancellation check at the top of each iteration. While the inner `execute_with_parse_retries()` call will eventually detect cancellation, the work preceding it (backend creation, prompt construction, session resolution, tmux context setup at lines 2190-2224) executes unnecessarily if already cancelled.

### Proposed Change
Add `if self.cancel.is_cancelled() { return Err(RalphError::Cancelled); }` at the start of the loop body (after line 2189).

### Affected Files
- `src/workflow/orchestrator.rs` - Add cancellation check at top of completer loop body

---

## Amendment: BACKEND-PID-04

### Problem
`kill_and_reap_child()` in `src/backend/mod.rs` (lines 861, 869, 881) casts `pid as i32` without overflow checking:
```rust
libc::kill(-(pid as i32), libc::SIGTERM);
```
This is inconsistent with the newly-added `KillOnDrop::drop()` (line 61) which correctly uses `i32::try_from(pgid)`. If `pid > i32::MAX` (theoretically possible), the `as` cast silently wraps, potentially sending signals to the wrong process group.

### Proposed Change
Replace `pid as i32` with `i32::try_from(pid)` and return early (or log a warning) on conversion failure, matching the pattern used in `KillOnDrop`.

### Affected Files
- `src/backend/mod.rs` - Change lines 861, 869, 881 to use `i32::try_from()`

---

## Summary

The implementation is solid overall. Code compiles cleanly, all 18 test targets build, and the core architecture — in-process dispatch via `tokio::spawn`, per-task tracing via `WithSubscriber`, cooperative cancellation via `CancellationToken`, environment sanitization in `CliBackend` and `TmuxBackend` — is correctly implemented. The four amendments above address:

| ID | Severity | Category |
|----|----------|----------|
| TMX-CANCEL-01 | **[P1]** | Resource leak: orphaned tmux windows on cancellation |
| ORCH-CANCEL-02 | **[P2]** | Missing early cancellation check in final-review phase |
| ORCH-CANCEL-03 | **[P2]** | Missing per-iteration cancellation check in completer loop |
| BACKEND-PID-04 | **[P3]** | Inconsistent overflow handling in signal delivery |
