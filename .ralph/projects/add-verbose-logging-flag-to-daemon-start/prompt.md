### Objective
Add a `--verbose` flag to `ralph daemon start` that enables detailed daemon runtime diagnostics on `stderr` using `eprintln!`.

### Scope
- In scope: `ralph daemon start` only.
- Out of scope: `ralph daemon status`, `ralph daemon abort`, logging frameworks, config/env toggles, file logging, log levels.

### Required CLI Behavior
1. `ralph daemon start --verbose` is accepted.
2. `ralph daemon start` without `--verbose` behaves as it does today.
3. `ralph daemon status --verbose` and `ralph daemon abort --verbose` are rejected by clap because `--verbose` is defined only on `DaemonStartArgs`.
4. Rejection tests must assert non-zero exit code and that stderr references invalid/unexpected use of `--verbose` (do not hardcode one exact clap sentence).

### Runtime Behavior
1. Add `verbose: bool` to `DaemonRuntimeConfig`.
2. Plumb `args.verbose` from `DaemonStartArgs` into `DaemonRuntimeConfig` in `execute_start`.
3. All new logs must be guarded by `if config.verbose`.
4. When `--verbose` is not set, no line with prefix `verbose:` is emitted.
5. All new verbose logs must go to `stderr` and start with `verbose:`.

### Verbose Logging Contract
Use these event points and fields. Exact punctuation can vary, but fields must be present and testable.

1. Poll loop cycle:
- iteration number
- active child count
- available slot count
- planned sleep duration

2. Child collection:
- log each terminal child event (task id, pid, exit status) on `try_wait -> Ok(Some(status))`
- do not log per-child “still running”
- log one summary line with count of still-running children after each collection pass when count > 0

3. Task transitions (authoritative transition logs only where state is mutated):
- `dispatch_task` CAS success: `pending -> in_progress` with task id and pid
- `complete_task` CAS success: `in_progress -> terminal` (or prior -> terminal if applicable) with task id
- dispatch CAS race where task is already terminal: log abort-race context and spawned pid
- completion CAS race where task already terminal: log preservation message

4. Operational context logs (non-authoritative transitions):
- `reconcile_tasks`: per-task reset action
- `adopt_pending_tasks`: per-task re-adoption action

### Implementation Requirements
1. Update `src/cli/daemon.rs`:
- add `verbose: bool` to `DaemonStartArgs` with `#[arg(long)]`
- pass to `DaemonRuntimeConfig` in `execute_start`

2. Update `src/daemon/runtime.rs`:
- add `pub verbose: bool` to `DaemonRuntimeConfig`
- instrument runtime functions with guarded `eprintln!` per contract above
- update `reconcile_tasks` signature to accept `verbose: bool` (or config reference) and plumb call sites

3. Update `src/validate/tests_daemon.rs`:
- add new conformance tests listed below
- register tests in `tests()` vector
- if module wiring is missing, register `tests_daemon` in `src/validate/mod.rs`

4. Do not add new dependencies.

### Required Conformance Tests
Add the following tests using existing `ConformanceTest` + `RalphHarness` patterns.

1. `daemon::verbose_flag_accepted_by_start`
- run `ralph daemon start --verbose --single-iteration --repo acme/widgets`
- assert exit code `0`

2. `daemon::verbose_flag_rejected_by_status_and_abort`
- run `ralph daemon status --verbose`
- run `ralph daemon abort --verbose dummy-id`
- assert non-zero exit for both
- assert stderr indicates invalid/unexpected `--verbose` usage

3. `daemon::verbose_output_present_when_enabled`
- run `ralph daemon start --verbose --single-iteration --repo acme/widgets`
- assert stderr contains at least one `^verbose:` line
- assert at least one poll-cycle verbose line exists

4. `daemon::verbose_output_absent_when_disabled`
- run `ralph daemon start --single-iteration --repo acme/widgets`
- assert stderr contains zero `^verbose:` lines

### Acceptance Criteria
1. CLI behavior matches the scope rules exactly.
2. Default behavior is unchanged except for added optional flag support.
3. Verbose logs appear only when enabled and always use `verbose:` prefix.
4. Poll-cycle, child terminal events, child-running summary, and task transition/race logs are implemented per contract.
5. Conformance tests for new behavior pass.
6. Existing daemon and broader validate tests continue to pass.

### Validation Commands
Run and pass:
- `nix develop -c cargo check`
- `nix develop -c cargo test`
- `nix build -L`
- `./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon`