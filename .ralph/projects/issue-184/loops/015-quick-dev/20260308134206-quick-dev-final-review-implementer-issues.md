---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T13:42:06Z
---

The `CwdGuard` is **only in the `#[cfg(test)]` module** (line 210 is inside tests), used for test isolation. This is acceptable - the spec said to remove it from the "library dispatch path", which has been done. The test-only usage is fine.

I have now completed my thorough review. Let me compile the findings.

# Final Review: AMENDMENTS

## Amendment: DOC-1

### Problem
The `KillOnDrop` docstring at `src/backend/mod.rs:43` states "On drop, sends SIGKILL to the process group" but the actual implementation sends **SIGTERM** first (line 67), waits up to 5 seconds for graceful exit, then escalates to SIGKILL only if the process group survives (line 111). The docstring is misleading about the two-stage termination behavior.

### Proposed Change
Update the docstring to accurately describe the two-stage SIGTERM → SIGKILL behavior.

### Affected Files
- `src/backend/mod.rs` line 43 - Change "sends SIGKILL" to "sends SIGTERM (then SIGKILL after a grace period if still alive)"

**Priority: [P3]** — Documentation accuracy, no runtime impact.

---

## Summary

The implementation is **correct and robust** across all critical dimensions. Specific areas verified:

**Correctness:**
- `dispatch_task()` (`src/daemon/runtime.rs`) correctly spawns all 4 dispatch variants as in-process tokio tasks via `spawn_inprocess_task()`, with per-task `CancellationToken` and `tracing::Dispatch` for log isolation.
- `collect_children()` correctly uses `JoinHandle::is_finished()` for non-blocking polling and `derive_terminal_label()` to map `Ok(Ok(_))` → `ralph:completed`, all error variants → `ralph:failed`.
- `kill_aborted_children()` cancels the `CancellationToken` (not SIGTERM/SIGKILL), sets `aborted_externally` via `AtomicBool::store(SeqCst)`, and defers removal to `collect_children()`.
- `drain_all_children()` implements correct cooperative-then-forced shutdown: cancel tokens → poll with 50ms intervals → `abort()` after 7200s deadline.

**CWD Safety:**
- No `std::env::current_dir()` in library dispatch paths (`src/daemon/tasks.rs`, `src/workflow/orchestrator.rs`, `src/workflow/quick_dev_orchestrator.rs`). Only remaining calls are in CLI fallback paths (`src/cli/auto.rs:129`, `src/cli/quick_dev_auto.rs:64`) for backward compatibility when `--workspace-root` is not provided.
- `CwdGuard` exists only in `#[cfg(test)]` module at `src/cli/auto.rs:210` — removed from library dispatch path.
- `load_workspace()` at `src/daemon/tasks.rs:585` uses `Workspace::load()` (never `discover()`).

**Environment Sanitization:**
- `SANITIZED_ENV_VARS` defined at `src/backend/mod.rs:37`, applied via `cmd.env_remove()` at lines 584-586 in `execute_streaming()` — enforced at the backend subprocess layer for all dispatch paths.

**Cancellation Safety:**
- `CancellationToken` threaded through both orchestrators (`orchestrator.rs:143`, `quick_dev_orchestrator.rs:56`).
- Checked between phases (`orchestrator.rs:534`, `quick_dev_orchestrator.rs:309`).
- `tokio::select!` in `execute_with_timeout_retries` (`orchestrator.rs:6097-6100`) and `execute_backend` (`quick_dev_orchestrator.rs:1443-1445`), plus backoff sleep cancellation.
- `KillOnDrop` guard in `execute_streaming` (`mod.rs:614-615`) ensures backend process groups are killed on future cancellation; disarmed only after confirmed child cleanup (lines 815, 845, 852, 867).

**Per-task Log Isolation:**
- `spawn_inprocess_task()` at `src/daemon/tasks.rs:483-501` creates per-task `fmt::Subscriber` with `Mutex<File>` writer, bound via `.with_subscriber(dispatch)` — correct `tracing::instrument::WithSubscriber` pattern that follows the task across thread migrations.

**Error Model:**
- `RalphError::Cancelled` at `src/error.rs:139-140`, marked non-transient (line 182), exit code 15 (line 216).

**Cleanup:**
- `DaemonRuntimeConfig::ralph_bin` removed. No `RALPH_DAEMON_BIN` reads in `src/daemon/` or `src/cli/`.
- Old subprocess spawn functions (`spawn_ralph_auto`, `spawn_ralph_run`, `SpawnedChild`, `sanitize_command_env`) removed from `src/daemon/process.rs`.
- `max_backend_retries` moved from env var to `RunOptions`/`QuickDevRunOptions` fields; resolver at `src/workflow/mod.rs:10-17` with correct default=3, clamp-to-10 semantics.
- `TaskHandle` at `src/daemon/mod.rs:26-50` fully replaces `ChildHandle`.

**Tests:**
- `derive_terminal_label` covered by 4 unit tests including panic/JoinError case (`src/daemon/runtime.rs:3914-3961`).
- `drain_all_children` timeout-abort test (`runtime.rs:3968`).
- All `RunOptions`/`QuickDevRunOptions` constructions in `tests/orchestrator.rs` and `tests/quick_dev_orchestrator.rs` include `cancel: CancellationToken::new()` and `max_backend_retries: None`.

The only finding is the minor docstring inaccuracy (P3 above). The project is otherwise complete and correct.
