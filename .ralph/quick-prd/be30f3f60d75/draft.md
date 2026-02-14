Now I have a thorough understanding of the codebase. Let me produce the engineering specification.

---

## Summary

Add a `--verbose` flag to the `ralph daemon start` CLI command that enables detailed `eprintln!`-based diagnostic logging of poll cycles, child process status checks, and task state transitions. The flag is plumbed from `DaemonStartArgs` through `DaemonRuntimeConfig` into the runtime loop functions. When disabled (default), all new log statements are skipped via simple `if config.verbose` guards, preserving existing behavior and adding zero overhead.

## Acceptance Criteria

- `ralph daemon start --verbose` is accepted by the CLI parser; `ralph daemon start` (without `--verbose`) continues to work identically to today.
- Each poll cycle iteration logs: iteration number, active child count, available slots, and sleep duration.
- Each child process status check logs per-child `try_wait` results (still running, exited with status, error).
- Each task state transition logs: task ID, previous state, new state, and trigger (dispatch, collect, abort-race, reconciliation).
- No new log output appears when `--verbose` is omitted.
- All new log lines are prefixed with `verbose:` to distinguish them from existing `warning:`/`reconcile:`/`dispatch:` prefixes.

## Technical Approach

**1. CLI layer (`src/cli/daemon.rs`)**

Add a `verbose: bool` field to `DaemonStartArgs` with `#[arg(long)]`, defaulting to `false`. In `execute_start`, plumb `args.verbose` into the `DaemonRuntimeConfig` constructor.

**2. Runtime config (`src/daemon/runtime.rs`)**

Add `pub verbose: bool` to `DaemonRuntimeConfig`. This is the single flag all runtime functions consult.

**3. Runtime loop instrumentation (`src/daemon/runtime.rs`)**

Insert guarded `eprintln!` calls at these points—all gated behind `if config.verbose`:

| Location | What is logged |
|---|---|
| Top of `loop` body (~L73) | `verbose: poll cycle N, active={}, slots={}` |
| `collect_children` per-child `try_wait` (~L519-L538) | `verbose: child {task_id} (pid={pid}): still running` / `exited status={status}` / `check error={err}` |
| `collect_children` state transition (~L541-L546) | `verbose: task {task_id} transitioning to {terminal_state}` |
| `dispatch_task` CAS update (~L417) | `verbose: task {task_id} state pending -> in_progress (pid={pid})` |
| `complete_task` CAS update (~L629) | `verbose: task {task_id} state {old} -> {new}` |
| `reconcile_tasks` per-task reset (~L109-L115) | `verbose: reconcile task {task_id}: in_progress -> pending` |
| `adopt_pending_tasks` per-task (~L167-L190) | `verbose: re-adopting pending task {task_id}` |
| Sleep before next iteration (~L97) | `verbose: sleeping {}s until next poll` |

**4. Passing verbose to sub-functions**

`collect_children`, `drain_all_children`, and `complete_task` already receive `config: &DaemonRuntimeConfig`, so they can read `config.verbose` directly. `reconcile_tasks` currently only takes `&TaskStore`—add an optional `verbose: bool` parameter (or pass the full config). The most minimal change: pass `config.verbose` as a separate `bool` argument to `reconcile_tasks` since it runs in `spawn_blocking` and cloning the full config just for a bool is wasteful.

**5. No new dependencies**

The project uses `eprintln!` for all daemon logging today. No `tracing`, `log`, or `env_logger` crate is introduced. The `verbose:` prefix is consistent with the existing `warning:`, `reconcile:`, and `dispatch:` conventions.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/daemon.rs` | Add `verbose` field to `DaemonStartArgs`; pass it into `DaemonRuntimeConfig` |
| `src/daemon/runtime.rs` | Add `verbose` field to `DaemonRuntimeConfig`; add guarded `eprintln!` calls in `run`, `reconcile_tasks`, `adopt_pending_tasks`, `poll_and_claim`, `dispatch_task`, `collect_children`, `drain_all_children`, `complete_task` |

Two files total. No new files created.

## Testing Strategy

**Unit/compile-time:**
- Existing `cargo test` must pass unchanged (regression gate).
- Add a unit test in `src/daemon/runtime.rs` (or `src/daemon/mod.rs`) that constructs a `DaemonRuntimeConfig` with `verbose: true` and `verbose: false` to verify the field exists and defaults correctly.

**CLI parsing:**
- Add a test in `tests/validate_cli.rs` (or a new `tests/daemon_cli.rs`) that invokes `ralph daemon start --help` and asserts `--verbose` appears in the output.
- Verify `ralph daemon start --verbose --single-iteration` is accepted by the argument parser (dry-run via `clap::Command::try_get_matches_from`).

**Integration (manual / CI with `--single-iteration`):**
- Run `ralph daemon start --verbose --single-iteration --repo owner/repo` against a test fixture and capture stderr. Verify `verbose:` prefixed lines appear for poll cycle, child check, and state transition events.
- Run the same command without `--verbose` and verify no `verbose:` lines appear on stderr.

**No-op when disabled:**
- Grep `stderr` output of a non-verbose run for `^verbose:` — expect zero matches.

## Out of Scope

- Structured logging framework (`tracing`, `log` crate, JSON log format) — the codebase uses raw `eprintln!` and this feature stays consistent with that choice.
- Log-level granularity (debug, trace, info) — this is a single on/off toggle, not a leveled system.
- Verbose mode for other subcommands (`daemon status`, `daemon abort`) — scoped to `daemon start` only.
- Log output to file or configurable log sinks — verbose output goes to stderr like all existing daemon logs.
- Configuration file or environment variable toggle for verbose mode — CLI flag only.
- Performance benchmarking of verbose vs. non-verbose — the overhead of skipped `if` branches is negligible.