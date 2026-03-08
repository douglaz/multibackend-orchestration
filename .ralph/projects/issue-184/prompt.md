## Summary

Move daemon orchestration from forked subprocesses (`tokio::process::Command` with `setsid`) to in-process tokio tasks. The daemon currently spawns `ralph auto/run/quick-dev-auto/quick-dev-run` as child processes in their own process groups, redirecting stdout/stderr to log files. This introduces fragile binary path resolution, fork/exec overhead, CLI re-parsing per task, log-file-based observability, and signal-based cancellation cascades. Since all four dispatch paths converge on constructing a `Workspace` → constructing an `Orchestrator`/`QuickDevOrchestrator` → calling `.run()`, and the orchestrators are `&mut self` with no process-global singletons, the orchestration layer can run as tokio tasks in the daemon process. Backend subprocesses (claude, codex, goose) continue to spawn externally.

## Acceptance Criteria

1. **In-process dispatch**: `dispatch_task()` in `src/daemon/runtime.rs` spawns orchestration as `tokio::spawn` tasks instead of child processes. No `ralph` binary is exec'd for orchestration.
2. **CWD safety**: No orchestration code path calls `std::env::set_current_dir()` or `std::env::current_dir()` for functional purposes. All paths are explicit. The `CwdGuard` in `src/cli/auto.rs` is removed from the library dispatch path. All four dispatch variants (`auto`, `run`, `quick-dev-auto`, `quick-dev-run`) pass the worktree path explicitly to `BackendRegistry::set_cwd()` and `Workspace::load()` — including the `run` and `quick-dev-run` resumed-project paths that previously relied on subprocess `current_dir(worktree)`.
3. **Environment sanitization**: `CLAUDECODE` (and any future `SANITIZED_ENV_VARS`) is removed from the environment of every backend subprocess `Command`, not just the top-level ralph subprocess. This is enforced in `CliBackend` command construction so that in-process tasks cannot leak daemon environment variables to backends.
4. **Per-task logging**: Each in-process task writes its output to the same log file path as before (`.ralph/tmp/logs/<task_id>.log`). `println!`/`eprintln!` in the library entry points are replaced with `tracing` events routed to a per-task file subscriber via a `tracing::dispatcher::Dispatch` set on `tokio::task::Builder::new()` using `tracing::instrument::WithSubscriber`. No cross-task log contamination occurs.
5. **Cooperative cancellation**: Orchestrators accept a `CancellationToken` and check it between phases (plan, implement, review, QA, completion, final-review). Active backend `execute_streaming` calls short-circuit on cancellation via `tokio::select!`. On cancellation, the backend's child process group is killed via `kill_and_reap_child()`. A hard-abort fallback kills the process group with SIGKILL if the backend does not exit within 5 seconds of cancellation.
6. **Task completion detection**: `collect_children()` detects task completion via `JoinHandle::is_finished()` instead of `child.try_wait()`. Exit status is derived from the `Result<OrchestrationResult>` return value: `Ok(_)` → `ralph:completed`, `Err(Cancelled)` → `ralph:failed` (abort), `Err(_)` → `ralph:failed`.
7. **Abort support**: `kill_aborted_children()` cancels the `CancellationToken` instead of sending SIGTERM/SIGKILL to a process group.
8. **Drain and shutdown**: `drain_all_children()` cancels each task's `CancellationToken`, then awaits `JoinHandle` with a bounded timeout (matching the existing 7200s deadline). After the deadline, remaining tasks are aborted via `JoinHandle::abort()`. Single-iteration mode continues to call `drain_all_children()` and exits deterministically after all tasks reach a terminal state or timeout.
9. **Backward compatibility**: CLI commands (`ralph auto`, `ralph run`, etc.) continue to work unchanged for interactive use. The library entry points are additive. CLI callers pass `CancellationToken::new()` (never cancelled).
10. **`RALPH_MAX_BACKEND_RETRIES`**: Moved from `std::env::var` to a field on `RunOptions`/`QuickDevRunOptions` with a default of 3.
11. **No regression in daemon features**: Artifact watchers, draft-PR watchers, rebase agent, log tail on failure, and retrigger separator all continue to function.
12. **Test migration**: Existing daemon tests that assert `RALPH_DAEMON_BIN`/mock-child invocation behavior are migrated to assert in-process dispatch behavior. `DaemonRuntimeConfig::ralph_bin` is removed. New tests validate all four in-process dispatch variants, cancellation, env sanitization, and log isolation.

## Technical Approach

### Phase 1: Remove CWD dependencies from library code

**`src/prd/quick.rs`** — `QuickPrdPipeline::run()` (line 299) calls `std::env::current_dir()`. Callers already have the worktree path. Change `run()` to require an explicit `working_dir: PathBuf` parameter (removing the zero-arg `run()`, making `run_in()` the public API, renamed to `run()`).

**`src/cli/auto.rs`** (line 195) and **`src/cli/quick_dev_auto.rs`** (line 177) — `registry.set_cwd(Some(std::env::current_dir()?))`. Replace with the worktree path from `workspace.root.parent()` (the repo root above `.ralph/`). For CLI invocations without `--workspace-root`, this is equivalent to `current_dir()`.

**`src/cli/run.rs`** and **`src/cli/quick_dev_run.rs`** — These do not call `set_cwd()` or `current_dir()` explicitly, but when invoked as subprocesses the daemon sets `.current_dir(worktree_path)` on the `Command`, so `Workspace::discover()` implicitly uses the subprocess CWD. For in-process dispatch, the library entry points must pass `workspace_root` to `Workspace::load()` directly (not `Workspace::discover()`) and call `registry.set_cwd(Some(workspace_root.parent()))` to set the backend working directory. This ensures all four dispatch variants have explicit CWD wiring.

**`src/workflow/orchestrator.rs`** — The `debug_assert_eq!(cwd, root)` at line ~5995 and `current_dir()` usage at line ~5690 for `loop_dir_hint`: change to use `self.workspace.root.parent()` directly. The `max_backend_retries()` function (line 6064) reads `RALPH_MAX_BACKEND_RETRIES` from env: add a `max_backend_retries: Option<u8>` field to `RunOptions` and `QuickDevRunOptions`, defaulting to 3.

### Phase 2: Environment sanitization at the backend layer

Move the `SANITIZED_ENV_VARS` list and `sanitize_command_env()` function from `src/daemon/process.rs` to `src/backend/mod.rs`. Apply sanitization in `CliBackend::execute_streaming()` when constructing the `Command`:

```rust
// In CliBackend::execute_streaming(), after building the Command:
for var in SANITIZED_ENV_VARS {
    cmd.env_remove(var);
}
```

This ensures backend subprocesses never inherit `CLAUDECODE` or other problematic env vars regardless of whether the orchestration runs in-process or as a subprocess. The daemon's `process.rs` `sanitize_command_env()` calls in `build_ralph_*` functions become redundant once all four dispatch paths move in-process and are removed along with those functions.

Add a regression test that spawns an in-process task with `CLAUDECODE` set in the daemon environment and verifies the backend `Command` does not inherit it.

### Phase 3: Library entry points

Add a new module **`src/daemon/tasks.rs`** with four async functions that mirror the CLI `execute()` handlers but take explicit parameters and return `Result<OrchestrationResult>`:

```rust
// src/daemon/tasks.rs
pub struct AutoTaskParams {
    pub workspace_root: PathBuf,
    pub idea: String,
    pub project_id: Option<String>,
    pub pr_url: Option<String>,
    pub global_config: GlobalConfig,
    pub cancel: CancellationToken,
}

pub struct RunTaskParams {
    pub workspace_root: PathBuf,
    pub project_id: String,
    pub pr_url: Option<String>,
    pub cancel: CancellationToken,
}

pub struct QuickDevAutoTaskParams {
    pub workspace_root: PathBuf,
    pub idea: String,
    pub project_id: Option<String>,
    pub pr_url: Option<String>,
    pub global_config: GlobalConfig,
    pub cancel: CancellationToken,
}

pub struct QuickDevRunTaskParams {
    pub workspace_root: PathBuf,
    pub project_id: String,
    pub pr_url: Option<String>,
    pub cancel: CancellationToken,
}
```

Each function: loads `Workspace` from `workspace_root` via `Workspace::load()` (never `Workspace::discover()`), constructs `BackendRegistry` with `set_cwd(Some(workspace_root.parent()))` for the repo root, runs quick-prd if applicable (passing `workspace_root` to `QuickPrdPipeline::run()`), creates project if needed, then runs the orchestrator passing the `CancellationToken`. No `println!` — all output via `tracing::info!`.

The existing CLI `execute()` functions in `src/cli/auto.rs`, `src/cli/run.rs`, etc. are refactored to call these library entry points, wrapping the tracing output as `println!` for interactive use. CLI callers pass `CancellationToken::new()` (never cancelled).

### Phase 4: Per-task tracing subscriber

Each in-process task gets its own `tracing::Dispatch` backed by a `tracing_subscriber::fmt::Layer` writing to the task's log file. The dispatch is attached to the tokio task using the `WithSubscriber` trait from `tracing::instrument`:

```rust
fn spawn_inprocess_task<F, Fut>(
    task_fn: F,
    log_path: &Path,
) -> Result<(CancellationToken, JoinHandle<Result<OrchestrationResult>>)>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<OrchestrationResult>> + Send,
{
    let file = open_log_file_append(log_path)?;
    let subscriber = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(false)
        .finish();
    let dispatch = tracing::dispatcher::Dispatch::new(subscriber);

    let handle = tokio::spawn(task_fn().with_subscriber(dispatch));
    Ok(handle)
}
```

Using `WithSubscriber` (not `with_default()`) ensures the subscriber is propagated to all `.await` points within the task, including across thread migrations in the multi-threaded tokio runtime. This is the correct approach because `with_default()` sets a thread-local subscriber that does not follow the task when it migrates between executor threads, while `WithSubscriber` attaches the dispatch to the task's `Instrumented` future, guaranteeing log isolation regardless of thread scheduling.

The retrigger separator logic from `process::open_log_file_append()` (lines 185-254) is extracted to a shared helper `open_log_file_append()` in `src/daemon/tasks.rs` that both the tracing setup and any future code path can call.

### Phase 5: Replace subprocess dispatch with task dispatch

In `src/daemon/runtime.rs`, replace the `dispatch_task()` spawn block (lines 1535-1595) with:

```rust
let cancel_token = CancellationToken::new();
let (join_handle) = match (is_quick, resume_existing_project) {
    (true, true)  => spawn_inprocess_task(|| tasks::run_quick_dev_run_task(params), &log_path)?,
    (true, false) => spawn_inprocess_task(|| tasks::run_quick_dev_auto_task(params), &log_path)?,
    (false, true) => spawn_inprocess_task(|| tasks::run_run_task(params), &log_path)?,
    (false, false) => spawn_inprocess_task(|| tasks::run_auto_task(params), &log_path)?,
};
```

Replace `ChildHandle` with `TaskHandle`:

```rust
pub struct TaskHandle {
    pub join_handle: JoinHandle<Result<OrchestrationResult>>,
    pub cancel_token: CancellationToken,
    pub watcher_cancel: CancellationToken,
    pub watcher_handle: Option<JoinHandle<()>>,
    pub draft_pr_cancel: CancellationToken,
    pub draft_pr_handle: Option<JoinHandle<()>>,
    pub branch: String,
    pub log_file: PathBuf,
    pub last_rebase_at: Option<Instant>,
    pub last_rebase_failure_sha: Option<String>,
    pub pr_url: Option<String>,
}
```

Update `collect_children()` to poll `join_handle.is_finished()` instead of `child.try_wait()`. When finished, call `join_handle.await` to get the result. Derive terminal label from `Result`: `Ok(_)` → `ralph:completed`, `Err(RalphError::Cancelled)` → `ralph:failed`, `Err(_)` → `ralph:failed`. If the `JoinHandle` returns a `JoinError` (task panicked), log the panic and treat as `ralph:failed`.

Update `kill_aborted_children()` to call `cancel_token.cancel()` instead of `terminate_process_group()`. After cancelling, do not immediately remove the task — let `collect_children()` observe the `JoinHandle` completion on the next poll cycle, so that watcher teardown and label transitions happen through the normal path.

Update `drain_all_children()`: cancel each remaining task's `CancellationToken` at the start, then poll via `collect_children()` with the existing 7200s deadline. After the deadline, call `join_handle.abort()` on remaining tasks (replacing the current `child.kill().await`), cancel watchers, and mark as `ralph:failed`. This preserves the existing semantics: cooperative cancellation first, forced abort after timeout.

Remove `DaemonRuntimeConfig::ralph_bin` field since no binary path resolution is needed.

### Phase 6: Cancellation threading

Add `cancel: CancellationToken` parameter to:
- `Orchestrator::run()` in `src/workflow/orchestrator.rs`
- `QuickDevOrchestrator::run()` in `src/workflow/quick_dev_orchestrator.rs`

Check `cancel.is_cancelled()` at the top of each phase loop iteration (before planning, implementing, reviewing, QA). In `execute_with_timeout_retries()` (orchestrator.rs line 5981), wrap the backend `execute_streaming` call in `tokio::select!` with `cancel.cancelled()`:

```rust
tokio::select! {
    result = backend.execute_streaming(prompt, log_writer) => result,
    _ = cancel.cancelled() => Err(RalphError::Cancelled),
}
```

On cancellation, `execute_streaming` is dropped. However, dropping the future does not kill the backend child process — the `Child` handle is inside `execute_streaming` and dropping it only closes pipes, leaving the process running. To ensure child termination, add an `AbortOnDrop` guard inside `execute_streaming` that kills the child process group on drop:

```rust
// Inside CliBackend::execute_streaming, immediately after spawn:
struct KillOnDrop(Option<u32>); // holds pgid
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(pgid) = self.0 {
            unsafe { libc::kill(-(pgid as i32), libc::SIGKILL); }
        }
    }
}
let guard = KillOnDrop(child.id());
// ... execute streaming logic ...
// On successful completion, disarm:
guard.0 = None;
```

This ensures that when `tokio::select!` drops the `execute_streaming` future on cancellation, the backend process group is immediately killed. The existing `kill_and_reap_child()` path for timeouts remains unchanged.

For CLI callers that don't need cancellation, pass `CancellationToken::new()` (never cancelled). Add a new `RalphError::Cancelled` variant to `src/error.rs`.

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/mod.rs` | Replace `ChildHandle` with `TaskHandle`. Remove `pid`/`pgid`/`child` fields. Add `join_handle: JoinHandle<Result<OrchestrationResult>>` and `cancel_token: CancellationToken`. |
| `src/daemon/tasks.rs` | **New file.** Library entry points: `run_auto_task`, `run_run_task`, `run_quick_dev_auto_task`, `run_quick_dev_run_task`. Param structs with `CancellationToken`. `open_log_file_append()` helper (moved from `process.rs`). `spawn_inprocess_task()` helper for per-task subscriber setup. |
| `src/daemon/runtime.rs` | Rewrite `dispatch_task()` to spawn tokio tasks via `spawn_inprocess_task()`. Update `collect_children()` for `JoinHandle::is_finished()`. Update `kill_aborted_children()` to cancel token (not SIGTERM). Update `drain_all_children()` to cancel tokens then await with bounded timeout, then `abort()` remaining. Remove `ralph_bin` from `DaemonRuntimeConfig`. |
| `src/daemon/process.rs` | Remove `spawn_ralph_auto`, `spawn_ralph_run`, `spawn_ralph_quick_dev_auto`, `spawn_ralph_quick_dev_run`, `SpawnedChild`, `build_ralph_*` functions, and `sanitize_command_env`. Move `open_log_file_append` to `tasks.rs`. Keep `terminate_process_group`, `pid_exists`, `run_command_with_timeout` (used by git operations and backend subprocesses). |
| `src/backend/mod.rs` | Add `SANITIZED_ENV_VARS` and apply `cmd.env_remove()` in `execute_streaming()` command construction. Add `KillOnDrop` guard in `execute_streaming()` to kill child process group when future is dropped (for cancellation safety). |
| `src/prd/quick.rs` | Make `run_in()` the public API (rename to `run()`, remove zero-arg `run()`). |
| `src/cli/auto.rs` | Refactor `execute()` to call library entry point or pass explicit path instead of `current_dir()`. Pass `CancellationToken::new()`. |
| `src/cli/run.rs` | Refactor `execute()` to call library entry point. Pass explicit workspace root path and `CancellationToken::new()`. |
| `src/cli/quick_dev_auto.rs` | Same as `auto.rs`. |
| `src/cli/quick_dev_run.rs` | Same as `run.rs`. |
| `src/cli/daemon.rs` | Remove `RALPH_DAEMON_BIN` resolution and `ralph_bin` from `DaemonRuntimeConfig` construction. |
| `src/workflow/orchestrator.rs` | Add `cancel: CancellationToken` to `run()`. Add `max_backend_retries: Option<u8>` to `RunOptions`. Remove `current_dir()` calls. Check cancellation between phases and in `execute_with_timeout_retries` via `tokio::select!`. |
| `src/workflow/quick_dev_orchestrator.rs` | Add `cancel: CancellationToken` to `run()`. Add `max_backend_retries: Option<u8>` to `QuickDevRunOptions`. Check cancellation between phases. |
| `src/error.rs` | Add `RalphError::Cancelled` variant. |

## Testing Strategy

### Conformance migration for existing tests

Existing daemon tests in `src/validate/tests_daemon.rs` and `src/validate/tests_daemon_concurrency.rs` heavily rely on `RALPH_DAEMON_BIN` and mock shell scripts that simulate ralph subprocess behavior (exit 0, exit 1, create commits, capture args). These must be migrated:

1. **Remove `RALPH_DAEMON_BIN` from test setup**: All `.daemon_env()` calls that set `RALPH_DAEMON_BIN` are updated to omit it. The `DaemonRuntimeConfig` no longer has a `ralph_bin` field, so tests that construct it directly are updated.

2. **Replace mock scripts with mock backends**: Instead of shell scripts that fake `ralph auto` behavior, tests inject mock `Backend` implementations (or use the existing `MockBackend` if available) into the `BackendRegistry`. The library entry points in `src/daemon/tasks.rs` accept a `GlobalConfig` that controls which backends are constructed, so tests can configure backends that return canned responses.

3. **Assert on `JoinHandle` results instead of process exit codes**: Tests that previously checked `child.try_wait()` exit status now check `join_handle.await` results. `Ok(OrchestrationResult { .. })` replaces exit code 0; `Err(_)` replaces nonzero exit.

4. **Preserve behavioral assertions**: Tests that verify side effects (GitHub labels set, PRs created, log files written, git commits made) continue to assert the same outcomes — only the dispatch mechanism changes.

5. **Remove mock script helpers**: `daemon_mock_ralph_script()`, `daemon_mock_ralph_capturing_script()`, `daemon_mock_ralph_fail_script()`, etc. in `src/validate/mock_scripts.rs` are removed once all tests are migrated.

### New tests

6. **CWD isolation (all 4 variants)**: Spawn two concurrent in-process tasks with different worktree paths for each of the four dispatch variants (`auto`, `run`, `quick-dev-auto`, `quick-dev-run`). Verify that each task's `Workspace::load()` receives the correct worktree path and `BackendRegistry::set_cwd()` is called with the correct repo root. Verify neither task's file operations interfere with the other.

7. **Environment sanitization regression**: Set `CLAUDECODE=1` in the test process environment. Spawn an in-process task with a mock backend that captures the `Command` environment. Assert that `CLAUDECODE` is not present in the backend subprocess environment.

8. **Cancellation unit tests**: Create an orchestrator with a `CancellationToken`. Cancel the token after a short delay. Verify the orchestrator returns `RalphError::Cancelled` without completing subsequent phases. Verify backend subprocess groups are killed.

9. **Cancellation of `execute_streaming`**: Start a backend execution with a slow mock process. Cancel the token mid-execution. Verify the backend child process group is killed (via `KillOnDrop`) and the orchestrator returns `Cancelled`.

10. **Per-task log isolation**: Spawn two concurrent in-process tasks with different log file paths. Each task emits distinct tracing events. Verify that each log file contains only its own task's events and no cross-contamination.

11. **`collect_children` refactor**: Create `TaskHandle` instances with completed `JoinHandle`s (Ok and Err). Verify `collect_children` correctly derives `ralph:completed` vs `ralph:failed` labels. Also test the panic case (`JoinError`).

12. **`drain_all_children` shutdown**: Spawn a task that blocks until cancelled. Call `drain_all_children` with a short timeout override. Verify the task is cancelled, then aborted after timeout, and marked `ralph:failed`.

13. **Single-iteration mode**: Run the daemon in `single_iteration` mode with an in-process task. Verify it dispatches the task, waits for completion via `drain_all_children`, and exits.

14. **`max_backend_retries` from config**: Set `max_backend_retries` on `RunOptions`. Verify the orchestrator uses the configured value instead of reading `RALPH_MAX_BACKEND_RETRIES` from the environment.

15. **CLI backward compatibility**: Verify `ralph auto --idea "test" --workspace-root /tmp/foo` still works identically from the command line, passing `CancellationToken::new()` to the orchestrator.

## Out of Scope

- **Backend subprocess management**: Backend processes (claude, codex, goose) continue to spawn as external subprocesses via `tokio::process::Command`. Moving them in-process is a separate concern.
- **Shared `BackendRegistry`**: Each task creates its own `BackendRegistry`. Sharing a registry across concurrent tasks would require `Arc<RwLock<BackendRegistry>>` and is deferred.
- **Removing `process.rs` entirely**: Functions like `terminate_process_group`, `pid_exists`, and `run_command_with_timeout` are still needed for backend subprocess management and git operations.
- **Tmux integration changes**: The tmux backend spawns windows via `tmux new-window` — this is unaffected since it already operates via `CliBackend::execute_streaming` with per-Command `current_dir()`.
- **Interactive PRD CWD changes**: The interactive PRD pipeline (`src/daemon/interactive_prd.rs`) has its own CWD handling which was addressed in a prior PR. Not revisited here.
- **Process-level crash isolation**: If an in-process task panics, `tokio::spawn` catches it via `JoinHandle` (returns `JoinError`). If the panic is unrecoverable (e.g., stack overflow or abort), the entire daemon process exits. Process-level crash isolation (e.g., re-spawning the daemon) is out of scope; `JoinHandle` panic catching provides sufficient isolation for recoverable panics.
- **Shared backend registry benefits**: The "Benefits" section of the original feature idea mentioned shared backend registry across concurrent tasks. This is not implemented in this spec — each task creates its own registry. Sharing is deferred to a future change.