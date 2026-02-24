---
artifact: prompt-review
project: summary-eliminate-all-unsafe-blocks-from
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-14T20:08:19Z
---

# Prompt Review

## Issues Found
- The provided text is a revision summary, not a standalone implementation prompt, so scope and deliverables are still ambiguous.
- Runtime semantics for `kill`/`killpg` error handling are not fully explicit (`EPERM`, `ESRCH`, other errors), which can cause incorrect behavior.
- Platform constraints are not stated (`#[cfg(unix)]` expectations), risking cross-platform build/test failures.
- The end-to-end process-group test intent is good, but anti-flake details (timeouts, cleanup, deterministic assertions) are missing.
- There is no explicit requirement to preserve existing function signatures and external daemon behavior.
- Validation steps are incomplete as an execution checklist for downstream loops.

## Refined Prompt
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
