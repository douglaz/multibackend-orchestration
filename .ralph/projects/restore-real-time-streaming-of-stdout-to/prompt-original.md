Now I have complete context. Let me write the engineering specification.

---

## Summary

Real-time streaming of stdout to `agent-output-*.log` files during non-tmux (`CliBackend`) execution is **already implemented** as of PR #31 (`532f17c`). The read loop in `execute_streaming` (`src/backend/mod.rs:619-639`) writes each chunk to `log_writer` as it arrives and simultaneously accumulates bytes into `captured_stdout` for post-exit normalization. There is no post-exit duplicate write in the `CliBackend` path.

The remaining gap is the **timeout model**: the current implementation uses a fixed-duration global watchdog (`tokio::time::sleep(timeout)` spawned once at execution start). There is no `last_activity` tracking or idle-timeout that resets on each stdout chunk. A backend that produces output slowly but continuously will still be killed after the global timeout expires, even though it is clearly making progress.

This spec adds an **activity-aware idle timeout** that resets its deadline on each stdout (or stderr) chunk, replacing the fixed global timeout. This ensures long-running but active backends are not prematurely killed while still killing truly stalled processes.

## Acceptance Criteria

- [ ] `agent-output-*.log` files grow in real time during non-tmux backend execution (already passes — preserved as regression gate)
- [ ] `ralph tail` and `tail -f` show live progress during execution (already passes for `tail -f`; `ralph tail` tracks `.md` artifacts by design and is out of scope here)
- [ ] Heartbeat idle-timeout resets on each stdout/stderr chunk — a backend producing output every N seconds is never killed as long as chunks arrive within the idle-timeout window
- [ ] A backend that stops producing output for longer than the configured timeout **is** killed (same SIGKILL-to-process-group behavior as today)
- [ ] Post-exit output normalization still works correctly (`captured_stdout` returned as `String::from_utf8_lossy`)
- [ ] No duplicate content appears in log files
- [ ] Existing unit tests (`cli_backend_streaming_preserves_exact_bytes_in_log`, `cli_backend_timeout_kills_and_reaps_child_and_writes_footer`) and conformance tests (`streaming::*`) pass
- [ ] New unit test verifies that a slow-but-active backend survives past the nominal timeout

## Technical Approach

### 1. Replace fixed watchdog with activity-aware idle timeout

**Current** (`src/backend/mod.rs:595-617`): A `tokio::spawn` sleeps for the full `self.timeout` duration. If that sleep completes before `timeout_cancel_rx` fires, the process group is killed.

**Proposed**: Replace the single `sleep(timeout)` with a loop that sleeps for `timeout` and checks whether `last_activity` has been updated. On each iteration:

1. Record `Instant::now()` as the check-start time.
2. Sleep for the remaining time until `last_activity + timeout`.
3. On wake, re-check `last_activity`. If it advanced, compute the new remaining time and loop. If it hasn't advanced (i.e., `last_activity.elapsed() >= timeout`), fire SIGKILL.

Use a `Arc<AtomicU64>` storing `Instant::now().elapsed().as_millis()` relative to a shared epoch (`Instant` captured before spawn), or more simply, use `Arc<Mutex<Instant>>` (the lock is uncontended — only the read loop writes, only the watchdog reads, and both are on different tasks).

**Simpler alternative using `tokio::sync::Notify`**: The watchdog loops calling `tokio::time::timeout(self.timeout, notify.notified())`. If the notify fires (activity), the loop restarts the timeout. If the timeout expires without notification, kill the process group. The read loop calls `notify.notify_one()` on each chunk.

Preferred: the `Notify`-based approach — it's idiomatic, avoids shared mutable state, and composes cleanly with `tokio::select!`.

```rust
// Watchdog task (replaces lines 600-617):
let activity_notify = Arc::new(tokio::sync::Notify::new());
let activity_notify_watchdog = activity_notify.clone();

let watchdog = tokio::spawn(async move {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(timeout) => {
                // No activity within timeout window — kill
                timed_out_watchdog.store(true, Ordering::SeqCst);
                if let Some(pid) = child_pid {
                    #[cfg(unix)]
                    {
                        let _ = unsafe {
                            libc::kill(-(pid as libc::pid_t), libc::SIGKILL)
                        };
                    }
                }
                return;
            }
            _ = timeout_cancel_rx => { return; }  // Note: must handle oneshot differently
            _ = activity_notify_watchdog.notified() => {
                // Activity detected — restart timeout
                continue;
            }
        }
    }
});
```

Since `timeout_cancel_rx` is a `oneshot::Receiver` (consumed on first poll), restructure to use a `CancellationToken` from `tokio_util` or a second `Notify`. Alternatively, wrap `timeout_cancel_rx` in a fused future or use `tokio::select!` with `biased;` and a flag.

**Simplest correct approach**: Replace `oneshot` with `tokio_util::sync::CancellationToken` (already in the `tokio-util` dependency tree via `tokio`). The watchdog becomes:

```rust
let cancel = CancellationToken::new();
let cancel_watchdog = cancel.clone();

let watchdog = tokio::spawn(async move {
    loop {
        tokio::select! {
            biased;
            _ = cancel_watchdog.cancelled() => return,
            _ = activity_notify_watchdog.notified() => continue,
            _ = tokio::time::sleep(timeout) => {
                timed_out_watchdog.store(true, Ordering::SeqCst);
                // ... SIGKILL ...
                return;
            }
        }
    }
});
```

After the read loop exits, call `cancel.cancel()` instead of `timeout_cancel_tx.send(())`.

### 2. Signal activity on each stdout and stderr chunk

**Stdout read loop** (`src/backend/mod.rs:619-639`): After the existing `captured_stdout.extend_from_slice(bytes)` and `writer.write_bytes(bytes)` calls, add:

```rust
activity_notify.notify_one();
```

**Stderr capture task** (`src/backend/mod.rs:569-593`): Clone `activity_notify` into the stderr task and call `activity_notify.notify_one()` on each `Ok(n)` branch. This ensures stderr-only output (e.g., progress bars printed to stderr) also resets the idle timer.

### 3. No changes to log writing or captured_stdout

The existing incremental `log_writer.write_bytes(bytes)` inside the read loop (line 637) and `captured_stdout.extend_from_slice(bytes)` (line 635) are already correct. No post-exit `write_bytes(&captured_stdout)` exists in `CliBackend`, so there is no duplicate-write to guard against.

### 4. Dependency check

Verify `tokio-util` with the `sync` feature is in `Cargo.toml`. If not, add it. If the project avoids `tokio-util`, use a bare `Arc<Notify>` + `AtomicBool` for cancellation instead of `CancellationToken`.

## Files & Modules

| File | Change |
|------|--------|
| `src/backend/mod.rs:595-617` | Replace fixed-sleep watchdog with `Notify`-driven idle-timeout loop |
| `src/backend/mod.rs:619-639` | Add `activity_notify.notify_one()` after each stdout chunk |
| `src/backend/mod.rs:569-593` | Clone `activity_notify` into stderr task; call `notify_one()` on each chunk |
| `src/backend/mod.rs:646` | Replace `timeout_cancel_tx.send(())` with `cancel.cancel()` (or equivalent) |
| `Cargo.toml` | Add `tokio-util = { version = "...", features = ["sync"] }` if `CancellationToken` is used and not already present |
| `src/backend/mod.rs` (tests) | Add new test: slow-but-active backend survives past nominal timeout |

## Testing Strategy

### Existing tests (must continue to pass)

- **`cli_backend_streaming_preserves_exact_bytes_in_log`** — exact byte preservation in log; unaffected since write path is unchanged
- **`cli_backend_timeout_kills_and_reaps_child_and_writes_footer`** — timeout kills stalled process; the script (`sleep 30` after one `printf`) produces no further output, so idle timeout fires identically to the old global timeout
- **`streaming::mid_execution_visibility`** — log grows while process runs; unchanged
- **`streaming::timeout_cleanup`** — partial output preserved + footer on timeout; unchanged

### New unit test

**`cli_backend_idle_timeout_resets_on_activity`**: Script emits a chunk every 200ms for 1 second total. Backend timeout set to 400ms. Under the old fixed timeout, the process would be killed at 400ms. Under idle timeout, it completes successfully because each chunk resets the 400ms window.

```rust
#[tokio::test]
async fn cli_backend_idle_timeout_resets_on_activity() {
    // Script: emit 5 chunks at 200ms intervals (total ~1s)
    // Timeout: 400ms (idle)
    // Expected: succeeds (each chunk arrives within 400ms window)
    let script = r#"#!/bin/sh
for i in 1 2 3 4 5; do
    printf "chunk-$i\n"
    sleep 0.2
done
"#;
    // ... setup CliBackend with 400ms timeout, assert Ok(...)
}
```

**`cli_backend_idle_timeout_fires_on_stall`**: Script emits one chunk then sleeps forever. Timeout set to 200ms. Process should be killed after 200ms of no activity.

```rust
#[tokio::test]
async fn cli_backend_idle_timeout_fires_on_stall() {
    // Script: emit one chunk, then sleep 30s
    // Timeout: 200ms
    // Expected: BackendTimeout after ~200ms
}
```

### Conformance tests

Existing `streaming::*` conformance tests in `src/validate/tests_streaming.rs` exercise the end-to-end path and will validate no regressions.

## Out of Scope

- **`ralph tail` reading `.log` files**: `ralph tail` is designed to monitor structured `.md` artifacts and `state.json` state transitions. Adding raw log file streaming to `ralph tail` is a separate feature. Users can use `tail -f` directly on `.log` files for raw output monitoring.
- **TmuxBackend streaming**: `TmuxBackend` captures output via `tee` to a temp file and writes to `LogWriter` post-exit. Making tmux output stream incrementally to `.log` files would require a fundamentally different approach (e.g., polling the output file) and is not addressed here.
- **Per-chunk progress callbacks or event emission**: No new progress event system; the existing `LogWriter.write_bytes` + flush is sufficient for `tail -f` consumers.
- **Configurable idle-timeout vs global-timeout mode**: The idle-timeout replaces the global timeout unconditionally. A configuration flag to choose between modes is not included in this scope.