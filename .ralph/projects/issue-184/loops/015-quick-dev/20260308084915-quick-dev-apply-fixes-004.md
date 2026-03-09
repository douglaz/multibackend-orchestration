---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 4
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T08:49:15Z
---

# Apply-Fixes: Reviewer-Requested Changes

## P1: Daemon dispatch tmux override

**Issue:** `daemon/tasks.rs:166` and `daemon/tasks.rs:194` set `tmux: Some(false)`, forcing tmux off regardless of workspace config. The old subprocess path preserved config defaults.

**Fix:** Changed both `RunOptions` instances (lines 166, 194) from `tmux: Some(false)` to `tmux: None`. With `None`, `resolve_tmux_settings` falls back to `workspace.tmux` config value, preserving the user's configured tmux behavior. Existing test `resolve_tmux_settings_falls_back_to_config` (orchestrator.rs:6524) already validates this path.

## P1: KillOnDrop blocking and early-return disarm

**Issue (blocking):** `KillOnDrop::drop()` in `backend/mod.rs:44` had a 5-second `SIGTERM` → poll → `SIGKILL` loop with `std::thread::sleep`, which blocks async executor threads when the future is cancelled.

**Fix:** Replaced the blocking drop with an immediate `SIGKILL` + non-blocking `waitpid(WNOHANG)`. The Drop guard is a last-resort safety net; graceful shutdown (SIGTERM + wait) is handled by the async timeout/completion paths that already exist in `execute_streaming`. Added a `disarm()` method for clarity.

**Issue (early return):** At line 736, `watchdog_cancel_error` caused an early `return Err(...)` before the disarm at line 745, leaving the guard armed against an already-exited process (potential PID reuse kill).

**Fix:** Moved `kill_guard.disarm()` to immediately after the `execution_outcome` block, before the `watchdog_cancel_error` early-return check. Now the guard is always disarmed when execution has completed, regardless of which return path is taken.

## P1: Missing conformance tests

**Issue:** Validate migration weakened coverage — several acceptance criteria lacked explicit tests.

**Added tests:**

1. **Backend env sanitization** (`backend/mod.rs`): `cli_backend_strips_claudecode_from_subprocess_env` — sets `CLAUDECODE` in process env, runs a shell script backend, asserts the var is not inherited by the subprocess.

2. **Per-task log isolation** (`daemon/tasks.rs`): `spawn_inprocess_task_log_isolation_no_cross_contamination` — spawns two concurrent tasks with distinct markers, verifies each log file contains only its own task's events.

3. **Cancellation behavior** (`daemon/tasks.rs`): `spawn_inprocess_task_returns_cancelled_on_token_cancel` — spawns a task awaiting cancellation, cancels token, verifies `Err(Cancelled)` is returned.

4. **`collect_children` result mapping** (`daemon/runtime.rs`): Extracted `derive_terminal_label()` pure function from inline logic. Four tests cover all cases:
   - `derive_terminal_label_ok_result_is_completed` — `Ok(Ok(_))` → `"ralph:completed"`
   - `derive_terminal_label_cancelled_is_failed` — `Ok(Err(Cancelled))` → `"ralph:failed"`
   - `derive_terminal_label_error_is_failed` — `Ok(Err(_))` → `"ralph:failed"`
   - `derive_terminal_label_panic_join_error_is_failed` — `Err(JoinError)` → `"ralph:failed"`

5. **`drain_all_children` timeout-abort** (`daemon/runtime.rs`): `drain_all_children_aborts_stuck_tasks_after_timeout` — spawns a task blocking until cancelled, calls `drain_all_children`, verifies the task is cancelled and the children map is emptied.
