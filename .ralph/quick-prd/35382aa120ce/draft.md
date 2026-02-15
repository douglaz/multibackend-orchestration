## Summary

Eliminate all `unsafe` blocks from the project's own source code by replacing direct `libc` FFI calls with safe abstractions: the `nix` crate for signal operations (`kill`, `killpg`) and the stable `tokio::process::Command::process_group(0)` API for child process group isolation. All six `unsafe` blocks reside in a single file (`src/daemon/process.rs`) and map cleanly to safe, well-maintained alternatives with no behavioral regressions.

## Acceptance Criteria

- No `unsafe` keyword appears in any project source file (excluding third-party dependencies under `target/`)
- All existing tests pass (`cargo test`)
- The validation test suite passes (including `runtime_pid_pgid_persistence` and related daemon tests)
- New unit tests for `terminate_process_group` and `pid_exists` pass (see Testing Strategy)
- No new compiler warnings introduced
- The `libc` dependency is removed from `Cargo.toml` (fully replaced by `nix`)
- Child process group cleanup (SIGTERM then SIGKILL escalation) continues to function identically
- Process spawning correctly isolates children into their own process group
- No performance regression: all replaced operations are process-lifecycle syscalls (`setsid`/`setpgid`, `kill`/`killpg`) executed at most once per task dispatch or abort — not on hot paths. The `nix` wrappers add only a thin conversion layer over the same underlying syscalls. No benchmark is needed because these calls are I/O-bound at OS-scheduling timescales (microseconds), and the daemon's throughput is gated by child process runtime (seconds to minutes), not by signal delivery overhead.

## Technical Approach

### Dependency Changes

**Add** to `Cargo.toml`:
```toml
nix = { version = "0.31", features = ["process", "signal"] }
```

**Remove** from `Cargo.toml`:
```toml
libc = "0.2"
```

(Verify `libc` is not used elsewhere in `src/` before removing.)

### Refactoring `src/daemon/process.rs`

**1. Replace `pre_exec` + `libc::setsid()` with `process_group(0)`**

The current code uses `unsafe { cmd.pre_exec(|| { libc::setsid() ... }) }` to create a new session. `CommandExt::pre_exec` is inherently `unsafe` because it runs between `fork` and `exec`. Replace this with the safe, stable `tokio::process::Command::process_group(0)`, which calls `setpgid` in the child to create a new process group. This is semantically sufficient: the daemon only needs process group isolation for signal-based cleanup, not full session leadership. The invariant `pgid == pid` is preserved because `process_group(0)` sets the child's PGID to its own PID.

Before:
```rust
unsafe {
    cmd.pre_exec(|| {
        if libc::setsid() == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    });
}
```

After:
```rust
cmd.process_group(0);
```

Update the doc comment on `spawn_ralph_auto` to reflect the change from `setsid` to `process_group`.

**2. Replace `libc::kill(pid, 0)` in `pid_exists` with `nix::sys::signal::kill`**

Before:
```rust
unsafe { libc::kill(pid as i32, 0) == 0 }
```

After:
```rust
nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
```

**3. Replace all `libc::kill(neg_pgid, ...)` calls in `terminate_process_group` with `nix::sys::signal::killpg`**

The function currently constructs a negative PGID and passes it to `libc::kill`. Replace with the dedicated `killpg` function which takes a positive PGID directly — this is both safer and more readable.

- `libc::kill(neg_pgid, 0)` → `killpg(Pid::from_raw(pgid as i32), None).is_ok()`
- `libc::kill(neg_pgid, libc::SIGTERM)` → `let _ = killpg(Pid::from_raw(pgid as i32), Signal::SIGTERM);`
- `libc::kill(neg_pgid, libc::SIGKILL)` → `let _ = killpg(Pid::from_raw(pgid as i32), Signal::SIGKILL);`

The `let _ =` discard pattern matches the current behavior where signal-send return values are ignored.

**4. Remove the `neg_pgid` local variable**

With `killpg`, the negative-PID trick is no longer needed. The function takes a positive PGID directly.

**5. Add a guard for `pgid <= 1` in `terminate_process_group`**

The current code guards only `pgid == 0`. Extend this to reject `pgid <= 1`, because `killpg(1, ...)` would signal every process in process group 1 (the init group), which could be catastrophic. While a PGID of 1 should never appear in practice (the daemon sets `pgid = pid` and PIDs > 1 for non-init processes), corrupted or stale state files could produce unexpected values. This is a defense-in-depth invariant.

Before:
```rust
if pgid == 0 {
    return;
}
```

After:
```rust
if pgid <= 1 {
    return;
}
```

Apply the same `pid <= 1` guard in `pid_exists` for consistency — probing PID 0 or 1 is never the intended behavior for this daemon.

**6. Update imports**

Remove:
```rust
// No explicit libc import exists at the top, but remove any if present
```

Add:
```rust
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
```

### Semantic Difference: `setsid` vs `process_group(0)`

`setsid()` creates a new session *and* a new process group; the child becomes a session leader. `process_group(0)` only creates a new process group. The difference matters for terminal control (session leaders can acquire controlling terminals), but this daemon spawns headless worker processes with stdin/stdout/stderr redirected to files — terminal control is irrelevant. All downstream code uses the PGID for signal delivery via `killpg`, which works identically with either approach.

## Files & Modules

| File | Change |
|---|---|
| `Cargo.toml` | Add `nix = { version = "0.31", features = ["process", "signal"] }`, remove `libc = "0.2"` (after confirming no other usage) |
| `src/daemon/process.rs` | Replace all 6 `unsafe` blocks with safe `nix` and `process_group(0)` calls; add `pgid <= 1` guard; add new unit tests; update imports and doc comments |

No other files contain `unsafe` blocks or direct `libc` usage that needs changing.

## Testing Strategy

1. **Existing unit tests**: Run `cargo test` — the `spawn_command_uses_long_idea_flag` test in `process.rs` validates command construction and must continue to pass.

2. **Compile-time verification**: Run `cargo build` and confirm zero warnings. Grep the `src/` directory for `unsafe` to confirm complete removal.

3. **New unit tests in `src/daemon/process.rs`**: Add the following tests to the existing `#[cfg(test)] mod tests` block:

   **a. `test_pid_exists_self`** — Verify `pid_exists(std::process::id())` returns `true` (the current process always exists). This confirms the `nix::sys::signal::kill` replacement works for basic process probing.

   **b. `test_pid_exists_bogus`** — Verify `pid_exists(u32::MAX)` returns `false` (no process with that PID). Confirms the replacement correctly maps `Err` from `kill` to `false`.

   **c. `test_pid_exists_rejects_low_pids`** — Verify `pid_exists(0)` and `pid_exists(1)` return `false` (the guard rejects them before making any syscall).

   **d. `test_terminate_process_group_noop_for_low_pgid`** — Call `terminate_process_group(0, Duration::from_millis(50)).await` and `terminate_process_group(1, Duration::from_millis(50)).await` inside a `#[tokio::test]` and assert they return promptly without error. This validates the `pgid <= 1` guard.

   **e. `test_terminate_process_group_dead_pgid`** — Call `terminate_process_group(u32::MAX - 1, Duration::from_millis(50)).await` and assert it returns promptly (the non-existent group triggers the early-exit "group does not exist" path). This exercises the `killpg(..., None).is_ok()` replacement.

   **f. `test_terminate_spawned_process_group`** — Spawn a real child process (`sleep 60`) using `tokio::process::Command` with `.process_group(0)`, extract its PID (== PGID), call `terminate_process_group(pgid, Duration::from_secs(2))`, then confirm the child has exited. This is the critical integration-level unit test that validates SIGTERM delivery through the new `killpg` path end-to-end.

4. **Validation suite**: The project includes integration/validation tests in `src/validate/tests_daemon.rs` (e.g., `runtime_pid_pgid_persistence`, `abort_stale_pid_and_terminal_state_handling`) that exercise the daemon's process spawning, PGID tracking, and abort lifecycle. These must continue to pass.

5. **Regression check on `libc` removal**: Run `cargo build` after removing the `libc` dependency to confirm no other code paths depend on it. If the build fails, keep `libc` in `Cargo.toml` but still remove all `unsafe` from source.

## Out of Scope

- Removing `unsafe` from third-party dependencies (e.g., `unsafe-libyaml` in the dependency tree)
- Migrating from `tokio::process::Command` to `std::process::Command`
- Adding the nightly-only `CommandExt::setsid()` API
- Performance benchmarking (the replaced operations are process-lifecycle syscalls, not hot paths — see Acceptance Criteria for rationale)
- Refactoring the `terminate_process_group` function's control flow or error handling beyond what is needed for the `unsafe` removal and the `pgid <= 1` guard