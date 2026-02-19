## Summary

Real-time streaming of stdout to `agent-output-*.log` files during non-tmux (`CliBackend`) execution is **already implemented** as of PR #31 (`532f17c`). The read loop in `execute_streaming` (`src/backend/mod.rs:619-639`) writes each chunk to `log_writer` as it arrives and simultaneously accumulates bytes into `captured_stdout` for post-exit normalization. There is no post-exit duplicate write in the `CliBackend` path. The original feature request's streaming requirements are satisfied.

This spec addresses a **separate, related gap** discovered during streaming verification: the **timeout model**. The current implementation uses a fixed-duration global watchdog (`tokio::time::sleep(timeout)` spawned once at execution start, `src/backend/mod.rs:595-617`). A backend that produces output slowly but continuously — common for AI agents emitting reasoning tokens — is killed after the global timeout expires, even though it is clearly making progress. This defeats the purpose of real-time streaming for long-running backends.

This spec adds an **activity-aware idle timeout** alongside the existing global timeout, creating a dual-limit model: the process is killed if *either* it produces no output for `timeout_seconds` (idle timeout) *or* it exceeds a hard maximum duration of `10 * timeout_seconds` (absolute cap). This preserves existing safety guarantees while allowing active backends to run longer.

## Acceptance Criteria

- [ ] `agent-output-*.log` files grow in real time during non-tmux backend execution (already passes — preserved as regression gate)
- [ ] `tail -f` on `.log` files shows live progress during execution (already passes — preserved as regression gate)
- [ ] Idle timeout resets on each stdout or stderr chunk — a backend producing output every N seconds is never killed by the idle timer as long as chunks arrive within the `timeout_seconds` window
- [ ] A backend that stops producing output for longer than `timeout_seconds` **is** killed (same SIGKILL-to-process-group behavior as today)
- [ ] A chatty backend that exceeds `10 * timeout_seconds` total wall-clock time is killed regardless of activity (absolute cap prevents indefinite execution)
- [ ] Post-exit output normalization still works correctly (`captured_stdout` returned as `String::from_utf8_lossy`)
- [ ] No duplicate content appears in log files
- [ ] Existing unit tests (`cli_backend_streaming_preserves_exact_bytes_in_log`, `cli_backend_timeout_kills_and_reaps_child_and_writes_footer`) pass unchanged
- [ ] Existing conformance tests (`streaming::retry_append_behavior`, `streaming::prompt_reviewer_path`, `streaming::mid_execution_visibility`, `streaming::timeout_cleanup`) pass unchanged
- [ ] New unit test verifies that a slow-but-active backend survives past the nominal idle timeout
- [ ] New unit test verifies that stderr-only activity resets the idle timer
- [ ] New conformance test (`streaming::idle_timeout_active_backend`) verifies end-to-end idle-timeout behavior

## Technical Approach

### 1. Dual-limit watchdog: idle timeout + absolute cap

**Current** (`src/backend/mod.rs:595-617`): A single `tokio::spawn` sleeps for `self.timeout`. If the sleep completes before `timeout_cancel_rx` fires, the process group is killed via SIGKILL.

**Proposed**: Replace the single `sleep(timeout)` with a `Notify`-driven loop that implements two limits:

1. **Idle timeout** (`timeout_seconds`): Resets on each stdout/stderr chunk. If no output arrives within this window, kill the process.
2. **Absolute cap** (`10 * timeout_seconds`): Hard wall-clock limit. A chatty process that continuously produces output is still killed after this duration. This prevents indefinite execution and preserves the safety property that `timeout_seconds` provides an upper bound on total runtime (albeit a looser one).

**Compatibility note**: The semantics of `timeout_seconds` change from "total runtime cap" to "inactivity window." The absolute cap at `10x` ensures no backend can run more than 10x longer than it could under the old model. For the default 7200s (2h) timeout, this means a 20h absolute cap — long enough for legitimate large-context AI agent runs, short enough to catch runaway processes. Existing configurations where backends complete well within `timeout_seconds` are unaffected. Configurations that depend on `timeout_seconds` as a hard runtime cap should reduce the value or (in a future follow-up) use a dedicated `max_duration_seconds` config field.

**Implementation using `tokio::sync::Notify`**: The watchdog uses `Notify` to receive activity signals from the read loops. On each notification, the idle timer restarts. A separate `tokio::time::sleep` tracks the absolute deadline.

```rust
let activity_notify = Arc::new(tokio::sync::Notify::new());
let activity_notify_watchdog = activity_notify.clone();

// Cancel signal: replace oneshot with a second Notify for cancellation.
// This avoids adding tokio-util as a dependency for CancellationToken.
let cancel_notify = Arc::new(tokio::sync::Notify::new());
let cancel_notify_watchdog = cancel_notify.clone();

let absolute_deadline = timeout.saturating_mul(10);

let watchdog = tokio::spawn(async move {
    let abs_sleep = tokio::time::sleep(absolute_deadline);
    tokio::pin!(abs_sleep);

    loop {
        tokio::select! {
            biased;
            // Cancellation (process exited naturally) — highest priority
            _ = cancel_notify_watchdog.notified() => return,
            // Absolute cap exceeded
            _ = &mut abs_sleep => {
                timed_out_watchdog.store(true, Ordering::SeqCst);
                if let Some(pid) = child_pid {
                    #[cfg(unix)]
                    {
                        let _ = unsafe {
                            libc::kill(-(pid as libc::pid_t), libc::SIGKILL)
                        };
                    }
                    #[cfg(not(unix))]
                    let _ = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
                }
                return;
            }
            // Activity received — restart idle window
            _ = activity_notify_watchdog.notified() => continue,
            // Idle timeout expired — kill
            _ = tokio::time::sleep(timeout) => {
                timed_out_watchdog.store(true, Ordering::SeqCst);
                if let Some(pid) = child_pid {
                    #[cfg(unix)]
                    {
                        let _ = unsafe {
                            libc::kill(-(pid as libc::pid_t), libc::SIGKILL)
                        };
                    }
                    #[cfg(not(unix))]
                    let _ = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
                }
                return;
            }
        }
    }
});
```

After the read loop exits, call `cancel_notify.cancel()` → `cancel_notify.notify_one()` instead of `timeout_cancel_tx.send(())`. Remove the `oneshot` channel.

**Race condition handling**: The `biased;` directive ensures deterministic priority: cancellation > absolute cap > activity > idle sleep. This prevents a false timeout when activity arrives simultaneously with the idle sleep expiring. Specifically:

- If a notification is pending when `sleep(timeout)` completes in the same poll cycle, `biased;` ensures `activity_notify.notified()` is checked first, and the idle window restarts.
- If the process exits (cancel fires) at the same instant as a timeout, cancellation wins and no SIGKILL is sent to an already-exited process group (harmless either way since SIGKILL to a dead PID returns `ESRCH`).
- The `timed_out` flag is only set in the kill branches, so the post-loop check at line 649 correctly distinguishes timeout from natural exit.

### 2. Signal activity on each stdout and stderr chunk

**Stdout read loop** (`src/backend/mod.rs:619-639`): After the existing `captured_stdout.extend_from_slice(bytes)` and `writer.write_bytes(bytes)` calls, add:

```rust
activity_notify.notify_one();
```

**Stderr capture task** (`src/backend/mod.rs:569-593`): Clone `activity_notify` into the stderr task and call `activity_notify.notify_one()` on each `Ok(n)` branch. This ensures stderr-only output (e.g., progress bars, download indicators printed to stderr) also resets the idle timer.

### 3. No changes to log writing or captured_stdout

The existing incremental `log_writer.write_bytes(bytes)` inside the read loop (line 637) and `captured_stdout.extend_from_slice(bytes)` (line 635) are already correct. No post-exit `write_bytes(&captured_stdout)` exists in `CliBackend`, so there is no duplicate-write to guard against.

### 4. Cancellation mechanism — Notify instead of CancellationToken

The original spec proposed `tokio_util::sync::CancellationToken`, but `tokio-util` is **not** in `Cargo.toml` and adding a dependency for one type is excessive. Instead, use a second `Arc<tokio::sync::Notify>` for cancellation:

- `cancel_notify.notify_one()` replaces `timeout_cancel_tx.send(())`
- `cancel_notify.notified()` replaces `timeout_cancel_rx` in the select
- Unlike `oneshot::Receiver`, `Notify::notified()` can be polled repeatedly without being consumed, which fits naturally in the watchdog loop

No new dependencies are required.

## Files & Modules

| File | Change |
|------|--------|
| `src/backend/mod.rs:595-617` | Replace fixed-sleep watchdog with `Notify`-driven idle-timeout + absolute-cap loop |
| `src/backend/mod.rs:619-639` | Add `activity_notify.notify_one()` after each stdout chunk write |
| `src/backend/mod.rs:569-593` | Clone `activity_notify` into stderr task; call `notify_one()` on each `Ok(n)` chunk |
| `src/backend/mod.rs:646` | Replace `timeout_cancel_tx.send(())` with `cancel_notify.notify_one()` |
| `src/backend/mod.rs:598` | Remove `oneshot::channel` import/usage; replace with second `Arc<Notify>` |
| `src/backend/mod.rs` (tests) | Add 3 new unit tests (idle-timeout-resets, idle-timeout-fires, stderr-only-activity) |
| `src/validate/tests_streaming.rs` | Add conformance test `streaming::idle_timeout_active_backend` |
| `src/validate/mock_scripts.rs` | Add `idle_timeout_active_mock_script()` helper for the new conformance test |

## Testing Strategy

### Existing tests (must continue to pass unchanged)

- **`cli_backend_streaming_preserves_exact_bytes_in_log`** (`src/backend/mod.rs:1140`) — exact byte preservation in log; write path is unchanged
- **`cli_backend_timeout_kills_and_reaps_child_and_writes_footer`** (`src/backend/mod.rs:1171`) — timeout kills stalled process; the script (`sleep 30` after one `printf`) produces no further output after the initial chunk, so the idle timeout fires at the same point as the old global timeout. The 150ms timeout is well under the `1500ms` absolute cap
- **`streaming::mid_execution_visibility`** (`src/validate/tests_streaming.rs:190`) — log grows while process runs; unchanged
- **`streaming::timeout_cleanup`** (`src/validate/tests_streaming.rs:261`) — partial output preserved + footer on timeout; the hanging script emits one line then sleeps, so idle timeout fires within the 1s configured `timeout_seconds`
- **`streaming::retry_append_behavior`** (`src/validate/tests_streaming.rs:39`) — multi-attempt log appending; unrelated to timeout
- **`streaming::prompt_reviewer_path`** (`src/validate/tests_streaming.rs:141`) — log path placement; unrelated to timeout

### New unit tests (`src/backend/mod.rs`)

**`cli_backend_idle_timeout_resets_on_activity`**: Script emits 5 chunks at 200ms intervals (total ~1s). Backend timeout set to 400ms. Under the old fixed timeout, the process would be killed at 400ms. Under idle timeout, each chunk arrives within the 400ms window, so the process completes successfully.

```rust
#[tokio::test]
async fn cli_backend_idle_timeout_resets_on_activity() {
    let temp = tempdir().expect("tempdir");
    let script_path = write_executable_script(
        temp.path(),
        "slow-active.sh",
        r#"#!/bin/sh
for i in 1 2 3 4 5; do
    printf "chunk-$i\n"
    sleep 0.2
done
"#,
    );
    let backend = CliBackend::new(
        "idle-test",
        script_path.to_string_lossy().to_string(),
        vec![],
        Duration::from_millis(400),
        BTreeMap::new(),
    );
    let mut writer = LogWriter::open(temp.path(), Some(1), None, "planner");
    let result = Backend::execute_with_log(&backend, "ignored", Some(&mut writer)).await;
    assert!(result.is_ok(), "slow-but-active backend should not be killed: {result:?}");
    let output = result.unwrap();
    assert!(output.contains("chunk-5"), "all chunks should be captured");
}
```

**`cli_backend_idle_timeout_fires_on_stall`**: Script emits one chunk then sleeps forever. Timeout set to 200ms. Process should be killed after 200ms of inactivity.

```rust
#[tokio::test]
async fn cli_backend_idle_timeout_fires_on_stall() {
    let temp = tempdir().expect("tempdir");
    let script_path = write_executable_script(
        temp.path(),
        "stall-after-one.sh",
        r#"#!/bin/sh
printf "initial-chunk\n"
sleep 30
"#,
    );
    let backend = CliBackend::new(
        "stall-test",
        script_path.to_string_lossy().to_string(),
        vec![],
        Duration::from_millis(200),
        BTreeMap::new(),
    );
    let mut writer = LogWriter::open(temp.path(), Some(1), None, "planner");
    let result = Backend::execute_with_log(&backend, "ignored", Some(&mut writer)).await;
    match result {
        Err(RalphError::BackendTimeout { backend }) => assert_eq!(backend, "stall-test"),
        other => panic!("expected BackendTimeout, got: {other:?}"),
    }
    let log_content = fs::read_to_string(writer.path()).expect("read log");
    assert!(log_content.contains("initial-chunk"));
    assert!(log_content.contains("--- timeout ts="));
}
```

**`cli_backend_stderr_activity_resets_idle_timeout`**: Script emits progress to stderr at 200ms intervals (no stdout). Timeout set to 400ms. The process should survive because stderr chunks reset the idle timer.

```rust
#[tokio::test]
async fn cli_backend_stderr_activity_resets_idle_timeout() {
    let temp = tempdir().expect("tempdir");
    let script_path = write_executable_script(
        temp.path(),
        "stderr-active.sh",
        r#"#!/bin/sh
for i in 1 2 3 4 5; do
    printf "progress-%s\n" "$i" >&2
    sleep 0.2
done
printf "final-stdout\n"
"#,
    );
    let backend = CliBackend::new(
        "stderr-idle-test",
        script_path.to_string_lossy().to_string(),
        vec![],
        Duration::from_millis(400),
        BTreeMap::new(),
    );
    let mut writer = LogWriter::open(temp.path(), Some(1), None, "planner");
    let result = Backend::execute_with_log(&backend, "ignored", Some(&mut writer)).await;
    assert!(result.is_ok(), "stderr-active backend should not be killed: {result:?}");
    let output = result.unwrap();
    assert!(output.contains("final-stdout"), "stdout should be captured after stderr activity");
}
```

### New conformance test (`src/validate/tests_streaming.rs`)

**`streaming::idle_timeout_active_backend`**: End-to-end test using `ralph run` with a mock backend that emits planner output slowly (5 chunks at 200ms intervals). `timeout_seconds` is set to `1` (idle window). The backend completes in ~1s and should succeed because each chunk arrives within the 1s idle window. Under the old fixed timeout, the backend would be killed at 1s.

```rust
fn idle_timeout_active_backend(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "streaming-idle-timeout";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "idle-timeout-active.sh",
                &idle_timeout_active_mock_script(),
            )
            .expect("failed to write idle-timeout mock script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");
        // 1-second idle timeout; script total runtime ~1.2s
        h.ralph_ok(["config", "set", "backends.claude.timeout_seconds", "1"])
            .expect("set claude timeout");
        h.ralph_ok(["config", "set", "backends.codex.timeout_seconds", "1"])
            .expect("set codex timeout");
        h.create_project(
            project_id,
            "Idle Timeout Active Backend",
            "Idle timeout test prompt",
        )
        .expect("create_project failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        // Verify planner log contains all chunks
        let loop_dir = h
            .loop_dir(project_id, 1)
            .expect("loop_dir should succeed")
            .expect("loop directory should exist");
        let planner_log = loop_dir.join("agent-output-planner.log");
        let content = fs::read_to_string(&planner_log).expect("read planner log");
        assert!(
            !content.contains("--- timeout ts="),
            "planner log should NOT contain timeout footer (backend completed normally), got:\n{content}"
        );
    })
}
```

The mock script (`idle_timeout_active_mock_script()` in `src/validate/mock_scripts.rs`) will emit valid planner output with artificial inter-chunk delays (sleep 0.2 between sections), producing a total runtime of ~1.2s that exceeds the 1s idle window but keeps each inter-chunk gap under 1s.

Register the test in the `tests()` function:

```rust
ConformanceTest {
    name: "streaming::idle_timeout_active_backend",
    func: idle_timeout_active_backend,
},
```

## Out of Scope

- **`ralph tail` reading `.log` files**: `ralph tail` (`src/cli/tail.rs`) monitors structured `.md` artifacts and `state.json` state transitions by design. It does not follow `agent-output-*.log` files. Adding raw log streaming to `ralph tail` is a separate feature. Users can use `tail -f` directly on `.log` files for raw output. The acceptance criteria have been updated to reference `tail -f` only (not `ralph tail`).
- **TmuxBackend streaming**: `TmuxBackend` captures output via `tee` to a temp file and writes to `LogWriter` post-exit. Making tmux output stream incrementally to `.log` files would require a fundamentally different approach (e.g., polling the output file) and is not addressed here.
- **Per-chunk progress callbacks or event emission**: No new progress event system; the existing `LogWriter.write_bytes` + flush is sufficient for `tail -f` consumers.
- **Configurable `max_duration_seconds`**: The absolute cap is hardcoded at `10 * timeout_seconds`. A dedicated `max_duration_seconds` config field that allows users to independently tune the absolute cap is a natural follow-up but out of scope for this change. If user demand arises, it can be added to `BackendConfig` and `RoleTimeouts` without further architectural changes.
- **Migration tooling**: No automated migration for existing `timeout_seconds` values. The semantic change (global cap → idle window with 10x absolute cap) is documented in release notes. Existing default of 7200s yields 2h idle / 20h absolute, which is safe for all known use cases.
