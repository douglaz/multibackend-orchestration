You are implementing a daemon process-safety update in `src/daemon/process.rs`.

### Goal
Harden PID/PGID handling and switch process probing/termination to `nix` signal APIs, without changing external behavior.

### Scope
1. Update PID existence probing.
2. Update process-group termination.
3. Add targeted tests for new behavior, including a real spawned process-group kill path.
4. Preserve existing interfaces and caller behavior.

### Required Code Changes
1. In `pid_exists`, add a guard returning `false` for `pid <= 1`.
2. Replace probing logic with `nix::sys::signal::kill(Pid::from_raw(pid), None)`.
3. Map probe outcomes explicitly:
- `Ok(_)` => process exists (`true`)
- `Err(EPERM)` => process exists (`true`)
- `Err(ESRCH)` => process missing (`false`)
- Any other error => conservative `false` (and log if this module already logs)
4. In `terminate_process_group`, add a guard that no-ops for `pgid <= 1`.
5. Use `nix::sys::signal::killpg(Pid::from_raw(pgid), Signal::SIGTERM)`.
6. Treat `Err(ESRCH)` from `killpg` as success (already dead); keep existing error propagation style for other errors.
7. Keep current function signatures/return types unless minimal compile-fix changes are required.

### Testing Strategy
Add or update these unit tests in `src/daemon/process.rs`:
- `test_pid_exists_self`
- `test_pid_exists_bogus`
- `test_pid_exists_rejects_low_pids`
- `test_terminate_process_group_noop_for_low_pgid`
- `test_terminate_process_group_dead_pgid`
- `test_terminate_spawned_process_group`

For `test_terminate_spawned_process_group`:
- Spawn a real child with `.process_group(0)`.
- Call `terminate_process_group` on its PGID.
- Assert child exits due to `SIGTERM`.
- Use bounded timeout polling and cleanup to prevent flakes/orphans.

### Platform and Safety Constraints
- Gate Unix-specific logic/tests with `#[cfg(unix)]` where needed.
- Never signal PID/PGID 0 or 1.
- Do not refactor unrelated modules.

### Acceptance Criteria
- All six tests above exist and pass.
- Existing tests pass unchanged.
- `nix develop -c cargo check` passes.
- `nix develop -c cargo test` passes.
- No behavioral regression in daemon lifecycle handling.
- No benchmark required. Rationale: these are thin wrappers over the same lifecycle syscalls; overhead is negligible relative to child runtime.

### Out of Scope
- Performance benchmarking.
- CLI/config/workflow changes.
- Unrelated daemon refactors.