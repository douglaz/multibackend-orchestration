---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T18:40:34Z
---

Both agents have completed. The second agent confirms all struct updates are consistent, no missing `set_cwd` calls, no remaining `ChildHandle`/`RALPH_DAEMON_BIN` references in source code, and validates the test changes as appropriate. It confirms the same findings I had.

Integrating all findings, here is my final review:

# Final Review: AMENDMENTS

## Summary

The implementation successfully moves daemon orchestration from forked subprocesses to in-process tokio tasks across 32 changed files. The core architecture is correct and well-tested. Key components verified:

- **`src/daemon/tasks.rs`** (new, 722 lines) — Four library entry points (`run_auto_task`, `run_run_task`, `run_quick_dev_auto_task`, `run_quick_dev_run_task`) with explicit `Workspace::load()`, per-task tracing subscriber via `WithSubscriber`, and `spawn_inprocess_task` helper. Log isolation test at line 617 validates no cross-contamination.
- **`src/daemon/runtime.rs`** — `dispatch_task` (line 1534) spawns tokio tasks instead of subprocesses. `collect_children` (line 1750) uses `JoinHandle::is_finished()` + await pattern. `kill_aborted_children` (line 1919) cancels tokens instead of SIGTERM. `drain_all_children_with_deadline` (line 2013) does cancel-all → poll → force-abort with bounded await.
- **`src/daemon/mod.rs`** — `TaskHandle` replaces `ChildHandle` with `JoinHandle`, `CancellationToken`, and `aborted_externally: Arc<AtomicBool>`.
- **`src/backend/mod.rs`** — `KillOnDrop` guard (line 47) SIGKILL-s process group on drop. `SANITIZED_ENV_VARS` (line 37) strips `CLAUDECODE` from backend subprocess environments. `execute_with_cancel` trait method (line 106) with `CliBackend` override (line 978). Two-stage SIGTERM→5s→SIGKILL in `kill_and_reap_child` (line 853).
- **`src/backend/tmux_backend.rs`** — `TmuxWindowGuard` (line 391) kills tmux window on drop via background thread. Env var sanitization with `is_valid_shell_identifier` guard (line 452).
- **`src/workflow/orchestrator.rs`** — `cancel` + `max_backend_retries` on `RunOptions`. `set_cwd` + `preload_bare_default_backends` after registry creation (line 242-243). Cancellation checks between phases (line 533) and in `execute_with_timeout_retries` (line 6086-6098).
- **`src/workflow/quick_dev_orchestrator.rs`** — Same cancel/retry threading. `execute_backend` (line 1427) refactored with retry loop, cancel-aware backoff sleep.
- **`src/workflow/mod.rs`** — Pure `max_backend_retries(configured: Option<u8>) -> u8` function replacing env var lookup.
- **`src/prd/quick.rs`** — `run()` takes explicit `working_dir: PathBuf`, removing `std::env::current_dir()` dependency.
- **`src/error.rs`** — `Cancelled` variant, non-transient, exit code 15.
- **All 4 CLI entry points** (`auto.rs`, `run.rs`, `quick_dev_auto.rs`, `quick_dev_run.rs`) refactored to call library entry points with `CancellationToken::new()` and `WithSubscriber`.
- **`src/daemon/process.rs`** — Reduced to `run_command_with_timeout`, `pid_exists`, `terminate_process_group`.
- **`src/cli/daemon.rs`** — Removed `RALPH_DAEMON_BIN` resolution, `ralph_bin` field, added `max_backend_retries`.
- **No remaining** `std::env::current_dir()` in library code (only CLI boundary), no `ChildHandle`/`RALPH_DAEMON_BIN` in `src/daemon/` or `src/cli/`.
- **Test coverage** includes: log isolation, cancellation, terminal label derivation, drain cooperative + force-abort, env sanitization, two-stage SIGTERM→SIGKILL, descendant cleanup, stored PGID fallback, shell identifier validation.

---

## Amendment: A1

### Problem
`[P2]` **Test semantic drift in concurrency tests — `partial_dispatch_rollback` and `dispatch_failure_explicit_markers` no longer test their named invariants.**

In `src/validate/tests_daemon_concurrency.rs:195`, `partial_dispatch_rollback` now expects **both** issues (300 and 301) to reach `ralph:failed` via `drain_all_children` cancellation. The original test verified that when one task fails, the sibling is *not* rolled back (demonstrating independent lifecycle management with different exit outcomes). The new test only proves both tasks can independently reach terminal state, which is a weaker invariant — it cannot distinguish "both cancelled" from "one succeeded, one failed."

Similarly, `dispatch_failure_explicit_markers` at line 544 now expects both issues to produce failure markers (both cancelled during drain), whereas the original tested that only the intentionally-failing task produced a failure marker while the healthy sibling did not.

### Proposed Change
Add a test that demonstrates **mixed outcomes**: configure a mock backend that completes instantly for one task (returning `Ok(OrchestrationResult)`) while the other blocks until cancelled. After single-iteration drain, assert the first task reaches `ralph:completed` and the second reaches `ralph:failed`. This would restore the original sibling-isolation invariant. Rename the existing tests to reflect their actual semantics (e.g., `drain_cancels_all_tasks_independently`).

### Affected Files
- `src/validate/tests_daemon_concurrency.rs` - `partial_dispatch_rollback` (line 195), `dispatch_failure_explicit_markers` (line 544)

---

## Amendment: A2

### Problem
`[P2]` **`concurrent_dispatch_evidence` uses weak ordering assertion that doesn't prove concurrency.**

In `src/validate/tests_daemon_concurrency.rs:621`, the test proves concurrency by asserting both "dispatched" messages appear before any "collect" messages in stderr. However, the daemon loop works by dispatching all claimed tasks in `poll_and_claim` *before* calling `collect_children`, so sequential execution naturally produces this ordering. The old test used overlapping START/END wall-clock timestamps from concurrent mock processes, which was definitively stronger proof of concurrent execution.

### Proposed Change
Inject timing instrumentation: have each in-process task record entry and exit wall-clock times via a shared `Arc<Mutex<Vec<(Instant, Instant)>>>`. After drain, assert the execution windows overlap (task B's start time precedes task A's end time). Alternatively, use a barrier synchronization: both tasks signal arrival at a shared `tokio::sync::Barrier(2)` — if they don't both arrive within a timeout, they weren't concurrent.

### Affected Files
- `src/validate/tests_daemon_concurrency.rs` - `concurrent_dispatch_evidence` (line 621)

---

## Amendment: A3

### Problem
`[P3]` **Quick-prd backend processes receive SIGKILL instead of cooperative SIGTERM on task cancellation.**

In `src/daemon/tasks.rs:178`, the quick-prd phase races against `params.cancel.cancelled()` via `tokio::select!`. When cancellation wins, the `quick_prd.run()` future is dropped. Inside the pipeline (`src/prd/quick.rs:229,369,433`), backends are called via `backend.execute()` which routes to `CliBackend::execute_streaming(prompt, None, &CancellationToken::new())` — an **uncancellable** token. The `KillOnDrop` guard correctly SIGKILL-s the process group on future drop, but the cooperative SIGTERM→5s grace→SIGKILL path in `kill_and_reap_child` is bypassed entirely. If a backend is writing intermediate files when hard-killed, they may be corrupted.

This is **safe** (no orphaned processes) but **not graceful**. The spec states "Active backend `execute_streaming` calls short-circuit on cancellation via `tokio::select!`", which holds for orchestrator backends but not quick-prd backends.

### Proposed Change
Thread the task's `CancellationToken` through to `QuickPrdPipeline` and call `execute_with_cancel` instead of `execute`. This enables the cooperative SIGTERM-first shutdown path. Low priority because quick-prd is short-lived and `KillOnDrop` prevents leaks.

### Affected Files
- `src/prd/quick.rs` - backend execute calls at lines 229, 369, 433
- `src/daemon/tasks.rs` - `run_auto_task` (line 178) and `run_quick_dev_auto_task` (line 381)

---

## Amendment: A4

### Problem
`[P3]` **`preload_bare_default_backends` hard-codes backend names, creating a silent failure for custom backends.**

In `src/workflow/orchestrator.rs:2955-2960`, the function hard-codes `["claude", "codex", "openrouter"]` to repopulate the registry cache after `set_cwd` clears it. This fixes a real bug where the reformatter lookup for the opposite backend fails after cache invalidation. However, custom backend configurations (e.g., `"gemini"`, `"claude-3.5"`) won't be preloaded, causing the reformatter to silently fall back to the original backend and exhaust parse retries.

The `QuickDevOrchestrator` (`src/workflow/quick_dev_orchestrator.rs:126`) calls `set_cwd` without calling `preload_bare_default_backends`, creating an asymmetry (acceptable since quick-dev doesn't use the reformatter path, but undocumented).

### Proposed Change
Make `set_cwd` invalidate only the CWD-dependent state in cached backends (e.g., update the `current_dir` on existing backends) rather than clearing the entire cache. Alternatively, document the limitation and preload based on the workspace's configured backends rather than a hard-coded list.

### Affected Files
- `src/workflow/orchestrator.rs` - `preload_bare_default_backends` at line 2955

---
