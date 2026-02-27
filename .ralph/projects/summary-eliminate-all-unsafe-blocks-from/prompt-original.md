The spec has been revised. Here's a summary of how each review issue was addressed:

**Issue 1 — Testing coverage**: Added six new unit tests in `src/daemon/process.rs` to the Testing Strategy (section 3). These directly cover the highest-risk behavioral changes:
- `test_pid_exists_self` and `test_pid_exists_bogus` validate the `nix::sys::signal::kill` replacement for process probing.
- `test_pid_exists_rejects_low_pids` validates the new low-PID guard.
- `test_terminate_process_group_noop_for_low_pgid` validates the `pgid <= 1` guard.
- `test_terminate_process_group_dead_pgid` exercises the `killpg` early-exit path.
- `test_terminate_spawned_process_group` is the critical end-to-end test: spawns a real child with `.process_group(0)`, calls `terminate_process_group`, and confirms the child exits via SIGTERM. This directly validates both PGID isolation and the SIGTERM delivery path through `killpg`.

**Issue 2 — Edge case guard for `pgid <= 1`**: Added Technical Approach step 5, which extends the existing `pgid == 0` guard to `pgid <= 1` (defense-in-depth against corrupted state signaling the init process group). Also applies a matching `pid <= 1` guard to `pid_exists`. Both guards are covered by the new unit tests.

**Issue 3 — Performance acceptance criterion**: Removed the contradiction between the acceptance criteria requiring "comparable performance" and the out-of-scope section declaring benchmarking unnecessary. The acceptance criteria now contains an inline rationale explaining *why* no benchmark is needed: all replaced operations are process-lifecycle syscalls (microsecond-scale, I/O-bound) executed at most once per task dispatch/abort, while the daemon's throughput is gated by child process runtime (seconds to minutes). The `nix` wrappers are thin conversion layers over the same underlying syscalls. The out-of-scope section cross-references this rationale.