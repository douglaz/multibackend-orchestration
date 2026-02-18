# Implementation Response (Iteration 1)

## Changes Made
1. **Non-tmux idle timeout now terminates promptly** (Required Change 1): Added `pre_exec(|| { libc::setsid(); Ok(()) })` to the spawned child process command in `src/backend/mod.rs:276-283`. This places the child in its own process group (session leader), so the watchdog's `libc::kill(-(pid as i32), SIGKILL)` reliably terminates the child and all its descendants. Previously, without `setsid()`, the negative-PID kill targeted a process group that didn't match the child's actual group, causing the kill to miss. The stall-timeout test now completes in ~0.2s instead of 30s.

2. **Timeout tests now validate kill-at-timeout behavior** (Required Change 2): Added elapsed-time assertions to three tests:
   - `src/backend/mod.rs` — `cli_backend_timeout_kills_and_reaps_child_and_writes_footer`: asserts `elapsed < 5s` (timeout configured at 150ms, script sleeps 30s).
   - `src/backend/mod.rs` — `cli_backend_stall_after_partial_output_times_out_idle`: asserts `elapsed < 5s` (timeout configured at 200ms, script sleeps 30s).
   - `src/validate/tests_streaming.rs` — `hanging_stall_timeout`: asserts `elapsed < 20s` (timeout configured at 1s with up to 3 retry attempts, script sleeps 30s).

   These bounds are generous enough for CI scheduling jitter while proving the timeout fires near the configured threshold, not after the full process sleep.

3. **Propagate measured idle duration from tmux wait path** (Recommended Improvement): Changed `src/backend/tmux_backend.rs:238` to capture `idle_seconds: measured_idle` from the `BackendTimeout` error returned by `wait_for_exit_with_activity` and forward it as `idle_seconds: measured_idle` in the re-raised error, instead of using `self.inner.timeout().as_secs()` (the configured constant). This ensures diagnostic logging reports the actual measured idle duration.

## Could Not Address
- None

## Pending Changes (Pre-Commit)
- `src/backend/mod.rs`: Added `pre_exec(setsid)` to child process spawn; added elapsed-time assertions to two timeout tests.
- `src/backend/tmux_backend.rs`: Changed `idle_seconds` in timeout error from configured constant to measured value from `wait_for_exit_with_activity`.
- `src/validate/tests_streaming.rs`: Added elapsed-time assertion to `hanging_stall_timeout` conformance test.
- All 610 tests pass (408 lib + 202 integration/other), 0 failures.
