Now I have a thorough understanding of the codebase. Let me produce the revised specification addressing all five review issues.

---

## Summary

Add a `--verbose` flag to the `ralph daemon start` CLI command that enables detailed `eprintln!`-based diagnostic logging of poll cycles, child process status checks, and task state transitions. The flag is plumbed from `DaemonStartArgs` through `DaemonRuntimeConfig` into the runtime loop functions. When disabled (default), all new log statements are skipped via simple `if config.verbose` guards, preserving existing behavior and adding zero overhead. The flag is scoped exclusively to the `start` subcommand; `status` and `abort` reject it via clap's standard argument validation (clap `Subcommand` enum already enforces per-variant arg sets). Conformance tests are added in `src/validate/tests_daemon.rs` following the project's existing `ConformanceTest` registration pattern.

## Acceptance Criteria

- `ralph daemon start --verbose` is accepted by the CLI parser; `ralph daemon start` (without `--verbose`) continues to work identically to today.
- `ralph daemon status --verbose` and `ralph daemon abort --verbose` are rejected by the CLI parser (clap enforces this structurally since `--verbose` is only defined on `DaemonStartArgs`). A conformance test verifies this by asserting non-zero exit codes.
- Each poll cycle iteration logs: iteration number, active child count, available slots, and sleep duration.
- Each child process status check logs per-child terminal events (exited with status, or check error). Children that are still running are **not** individually logged; instead, a single summary line at the end of each `collect_children` call reports the count of still-running children (e.g. `verbose: collect: 3 children still running`). This avoids excessive noise during the 50ms drain loops.
- Each task state transition logs: task ID, previous state, new state, and trigger context. The authoritative transition log is emitted at the point where state is actually mutated in the store — specifically in the `dispatch_task` CAS block (pending → in_progress) and the `complete_task` CAS block (in_progress → terminal). The dispatch CAS race path (task already terminal when CAS executes) emits its own verbose line noting the abort-race condition. `reconcile_tasks` and `adopt_pending_tasks` log their per-task actions as operational context rather than state transitions.
- No new log output appears when `--verbose` is omitted.
- All new log lines are prefixed with `verbose:` to distinguish them from existing `warning:`/`reconcile:`/`dispatch:` prefixes.
- Code changes span `src/cli/daemon.rs`, `src/daemon/runtime.rs`, and `src/validate/tests_daemon.rs`. The test file change is required by project policy for new CLI behavior.

## Technical Approach

**1. CLI layer (`src/cli/daemon.rs`)**

Add a `verbose: bool` field to `DaemonStartArgs` with `#[arg(long)]`, defaulting to `false`. Because `--verbose` is defined only on `DaemonStartArgs` and not on `DaemonAbortArgs` or the unit `Status` variant, clap structurally rejects `ralph daemon status --verbose` and `ralph daemon abort --verbose` with its standard "unexpected argument" error. No additional validation code is needed.

In `execute_start`, plumb `args.verbose` into the `DaemonRuntimeConfig` constructor by setting `verbose: args.verbose`.

**2. Runtime config (`src/daemon/runtime.rs`)**

Add `pub verbose: bool` to `DaemonRuntimeConfig`. This is the single flag all runtime functions consult.

**3. Runtime loop instrumentation (`src/daemon/runtime.rs`)**

Insert guarded `eprintln!` calls at the points below — all gated behind `if config.verbose`. The design principle for child-status logging is to **log events, not non-events**: individual "still running" lines are suppressed in favor of a single summary count, preventing log flooding during the 50ms drain poll loops.

| Location | What is logged | Rationale |
|---|---|---|
| Top of `loop` body in `run` | `verbose: poll cycle {N}, active={count}, slots={slots}` | Trace iteration cadence |
| `collect_children`: after `try_wait` returns `Ok(Some(status))` | `verbose: child {task_id} (pid={pid}) exited status={status}` | Log terminal event per child |
| `collect_children`: after `try_wait` returns `Err(err)` | (existing `warning:` line already covers this) | No new line needed |
| `collect_children`: after the per-child loop, if any children remain | `verbose: collect: {N} child(ren) still running` | Single summary line replaces per-child "still running" logs |
| `dispatch_task` CAS success (line ~417) | `verbose: task {task_id} state pending -> in_progress (pid={pid})` | Authoritative state transition for dispatch |
| `dispatch_task` CAS race — task already terminal (line ~407) | `verbose: task {task_id} dispatch abort-race: already terminal ({state}), killing spawned child (pid={pid})` | Explicit abort-race event the original spec missed |
| `complete_task` CAS success (line ~629) | `verbose: task {task_id} state {old} -> {terminal_state}` where `{old}` is read from the task before mutation | Authoritative state transition for completion |
| `complete_task` CAS race — already terminal (line ~616) | `verbose: task {task_id} child exited but already terminal ({state}), preserving` | Abort-race in completion path |
| `reconcile_tasks` per-task reset | `verbose: reconcile: task {task_id} in_progress -> pending` | Operational context (startup recovery) |
| `adopt_pending_tasks` per-task dispatch | `verbose: re-adopting pending task {task_id}` | Operational context |
| Sleep before next iteration in `run` | `verbose: sleeping {poll_seconds}s until next poll` | Trace sleep duration |

**4. Passing verbose to sub-functions**

`collect_children`, `drain_all_children`, and `complete_task` already receive `config: &DaemonRuntimeConfig`, so they read `config.verbose` directly. `reconcile_tasks` currently takes only `&TaskStore`. Rather than cloning the full config (wasteful for a `spawn_blocking` closure), pass `verbose: bool` as a separate parameter: `fn reconcile_tasks(store: &TaskStore, verbose: bool) -> Result<()>`. The call site in `run` passes `config.verbose`.

**5. No new dependencies**

The project uses `eprintln!` for all daemon logging today. No `tracing`, `log`, or `env_logger` crate is introduced. The `verbose:` prefix is consistent with the existing `warning:`, `reconcile:`, and `dispatch:` conventions.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/daemon.rs` | Add `verbose` field to `DaemonStartArgs`; pass it into `DaemonRuntimeConfig` |
| `src/daemon/runtime.rs` | Add `verbose` field to `DaemonRuntimeConfig`; add `verbose: bool` parameter to `reconcile_tasks`; add guarded `eprintln!` calls in `run`, `reconcile_tasks`, `adopt_pending_tasks`, `dispatch_task`, `collect_children`, `complete_task` |
| `src/validate/tests_daemon.rs` | Add new conformance tests (see Testing Strategy); register them in the existing `tests()` function |

Three files total. No new files created.

## Testing Strategy

All new tests follow the project's conformance test conventions: they are added to `src/validate/tests_daemon.rs` as `ConformanceTest` entries registered in the existing `pub fn tests()` vector, using the `RalphHarness` API and `run_case(|| { ... })` pattern.

**New conformance tests:**

1. **`daemon::verbose_flag_accepted_by_start`** — Invoke `ralph daemon start --verbose --single-iteration --repo acme/widgets` with mock `gh`. Assert exit code 0, confirming the parser accepts `--verbose` on `start`.

2. **`daemon::verbose_flag_rejected_by_status_and_abort`** — Invoke `ralph daemon status --verbose` and `ralph daemon abort --verbose dummy-id`. Assert both exit with non-zero status and stderr contains "unexpected argument" (clap's standard rejection message). This directly validates the scope constraint from the requirements.

3. **`daemon::verbose_output_present_when_enabled`** — Invoke `ralph daemon start --verbose --single-iteration --repo acme/widgets` with mock `gh` and mock `ralph` binary. Capture stderr and assert at least one line matching `^verbose:` is present. Specifically look for `verbose: poll cycle` and `verbose: sleeping` lines, which always appear regardless of whether tasks exist.

4. **`daemon::verbose_output_absent_when_disabled`** — Invoke `ralph daemon start --single-iteration --repo acme/widgets` with identical mocks. Capture stderr and assert zero lines matching `^verbose:`. This validates the no-op guarantee.

**Existing tests:**

- All existing conformance tests in `tests_daemon.rs` must pass unchanged (regression gate). The existing `daemon::cli_parse_start_status_abort` test already exercises `start` without `--verbose` and validates it succeeds, confirming backward compatibility.

## Out of Scope

- Structured logging framework (`tracing`, `log` crate, JSON log format) — the codebase uses raw `eprintln!` and this feature stays consistent with that choice.
- Log-level granularity (debug, trace, info) — this is a single on/off toggle, not a leveled system.
- Verbose mode for other subcommands (`daemon status`, `daemon abort`) — scoped to `daemon start` only. Clap structurally enforces this; tests verify the rejection.
- Log output to file or configurable log sinks — verbose output goes to stderr like all existing daemon logs.
- Configuration file or environment variable toggle for verbose mode — CLI flag only.
- Performance benchmarking of verbose vs. non-verbose — the overhead of skipped `if` branches is negligible.
- Throttling/debouncing of `drain_all_children` verbose output beyond the summary-line approach — the single "N children still running" line per `collect_children` call is sufficient. If the 50ms drain loop runs for extended periods, this produces at most 20 summary lines/second, which is within the "traceable without overwhelming" threshold.