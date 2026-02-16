I now have complete understanding of the codebase. Let me write the engineering specification.

## Summary

Replace `CliBackend::execute()`'s buffered `child.wait_with_output()` call with async line-by-line streaming of stdout/stderr, writing each line to a log file as it arrives while accumulating the full stdout in memory for the existing return value. The log file path is communicated via the existing `SharedTmuxContext` mechanism — renamed to `SharedExecutionContext` — so the orchestrator sets it per-invocation without changing the `Backend` trait. When no log path is set (tests, non-daemon contexts), no file I/O occurs and behavior is identical to today.

## Acceptance Criteria

- Backend output appears in log file incrementally as the child process produces it (verifiable via `tail -f`)
- Log file header (backend name, role, timestamp) is written immediately after process spawn, before any output arrives
- Exit status line is appended to log file after process completes
- `CliBackend::execute()` returns the full aggregated stdout `String` to callers — no change to `Backend` trait or return type
- When no `log_path` is configured in the execution context, behavior is identical to current (no file I/O side-effects)
- `TmuxBackend` continues to work unchanged (it doesn't call `CliBackend::execute()`; it constructs its own shell command with `tee`)
- `MockBackend` is unaffected
- Stderr content is logged to the file (prefixed with `[stderr]`) but is NOT included in the returned stdout `String`
- Non-zero exit still returns `Err(BackendCommandFailed)` with stderr content, same as today
- Timeout handling via `tokio::time::timeout` wraps the streaming loop and works identically to today

## Technical Approach

### 1. Rename `SharedTmuxContext` to `SharedExecutionContext` and extend the context struct

In `src/backend/tmux_backend.rs`, rename `TmuxExecutionContext` to `ExecutionContext` and add `log_path`:

```rust
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    pub loop_number: Option<u32>,
    pub role: Option<String>,
    pub log_path: Option<PathBuf>,  // NEW
}
```

In `src/backend/mod.rs`, rename `SharedTmuxContext` to `SharedExecutionContext`. Update `BackendRegistry` field names, `set_tmux_context()` to `set_execution_context()`, and all call sites in `orchestrator.rs`. This is a mechanical rename — no logic changes.

### 2. Add `shared_context` field to `CliBackend`

`CliBackend` gains a `shared_context: SharedExecutionContext` field so it can read the `log_path` at execution time. The field is set during `BackendRegistry::new()` when constructing backends. `CliBackend::new()` gains a `shared_context` parameter. `claude::backend_from_config()` and `codex::backend_from_config()` gain a corresponding parameter and pass it through.

When `CliBackend` is wrapped by `TmuxBackend`, the context is shared but `TmuxBackend` never calls `CliBackend::execute()` — it only reads `inner.command()`, `inner.args()`, `inner.env()`, etc. to build a shell command. So there is no double-logging concern.

### 3. Replace buffered I/O in `CliBackend::execute()` (lines 150-197)

Replace `child.wait_with_output()` with manual async reading:

1. After `child.spawn()` and writing the prompt to stdin, take `child.stdout` and `child.stderr` handles.
2. Read `log_path` from `self.shared_context.get().await`. If `Some`, create the log file and write the header immediately. If `None`, skip all file I/O.
3. Wrap a `tokio::io::BufReader` around each handle. Use a `tokio::select!` loop reading lines from both stdout and stderr concurrently:
   - stdout lines: append to `stdout_buf: String` AND write to log file (if open)
   - stderr lines: append to `stderr_buf: String` AND write to log file with `[stderr] ` prefix
4. After both streams reach EOF, call `child.wait()` for the exit status.
5. Append exit status and completion timestamp to log file, flush and close.
6. Return `Ok(stdout_buf)` on success or `Err(BackendCommandFailed { details: stderr_buf })` on non-zero exit — identical to current behavior.

The entire streaming loop (step 3-4) is wrapped in `tokio::time::timeout(self.timeout, ...)` to preserve existing timeout semantics.

**Log file header format:**
```
=== Backend Output Log ===
Backend: claude(opus)
Role: implementer
Started: 2026-02-16T14:30:00Z
---

```

**Log file footer format:**
```

---
Exit status: 0
Completed: 2026-02-16T14:35:22Z
```

### 4. Orchestrator sets `log_path` before each `execute()` call

In `src/workflow/orchestrator.rs`, each `set_execution_context()` call (currently ~10 call sites) adds a `log_path` derived from existing variables already in scope:

```rust
let log_path = project_dir
    .join("loops")
    .join(format!("{loop_number:03}-{loop_slug}"))
    .join(format!("agent-output-{backend_name}-{role}.log"));
```

The `prompt_reviewer` call site (which has no loop number) uses the project dir directly:

```rust
project_dir.join("agent-output-prompt-reviewer.log")
```

### 5. Handle `log_path` directory creation

`CliBackend::execute()` creates the parent directory of the log path via `tokio::fs::create_dir_all()` before opening the file. This mirrors how `write_artifact()` calls `fs::create_dir_all()` for the loop directory.

### 6. Flush strategy

Use `tokio::io::BufWriter` around the log file handle with explicit `flush()` after every line write. This ensures each line is visible to `tail -f` immediately. The BufWriter avoids syscall overhead for the in-memory buffer accumulation while the explicit flush after each write-to-file ensures real-time visibility.

## Files & Modules

| File | Change |
|------|--------|
| `src/backend/tmux_backend.rs` | Rename `TmuxExecutionContext` → `ExecutionContext`, add `log_path: Option<PathBuf>` field. Update all internal references. |
| `src/backend/mod.rs` | Rename `SharedTmuxContext` → `SharedExecutionContext`. Add `shared_context: SharedExecutionContext` field to `CliBackend`. Rewrite `CliBackend::execute()` to stream stdout/stderr via `BufReader` + `select!` loop, writing lines to log file when `log_path` is set. Rename `set_tmux_context()` → `set_execution_context()`. Update `BackendRegistry::new()` to pass shared context to `CliBackend`. Add `use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter}`. |
| `src/backend/claude.rs` | Add `SharedExecutionContext` parameter to `backend_from_config()`, pass to `CliBackend::new()`. |
| `src/backend/codex.rs` | Same as `claude.rs`. |
| `src/workflow/orchestrator.rs` | Rename all `set_tmux_context(TmuxExecutionContext { .. })` → `set_execution_context(ExecutionContext { .. , log_path: Some(...) })`. Derive `log_path` from `project_dir`, `loop_number`, `loop_slug`, backend name, and role at each call site (~10 sites). |
| `src/backend/mock.rs` | No changes needed — `MockBackend` doesn't use execution context. |
| `tests/backend.rs` | Update `CliBackend::new()` calls to pass a default `SharedExecutionContext`. Update renamed types. |
| `tests/backend_tmux.rs` / `tests/backend_tmux_backend.rs` | Update for renamed types (`TmuxExecutionContext` → `ExecutionContext`, `SharedTmuxContext` → `SharedExecutionContext`). |

## Testing Strategy

### Unit tests (in `src/backend/mod.rs` or `tests/backend.rs`)

1. **`cli_backend_streaming_returns_full_output`** — Set `log_path` in the execution context. Spawn `CliBackend` running `echo "line1\nline2\nline3"`. Assert returned string equals `"line1\nline2\nline3\n"`. Assert log file exists, contains the header, all three lines, exit status 0, and footer.

2. **`cli_backend_no_log_path_no_file_created`** — Leave `log_path` as `None`. Execute a simple command. Assert output returned correctly and no log file was created in the temp directory.

3. **`cli_backend_stderr_logged_not_in_return_value`** — Use a shell command that writes to both stdout and stderr (e.g., `sh -c 'echo out; echo err >&2'`). Assert returned `String` contains only `"out\n"`. Assert log file contains `"out\n"` and `"[stderr] err\n"`.

4. **`cli_backend_nonzero_exit_writes_status_to_log`** — Use `sh -c 'echo partial; exit 1'`. Assert `BackendCommandFailed` error. Assert log file contains `"partial"`, exit status 1, and the completion footer.

5. **`cli_backend_timeout_writes_partial_log`** — Use a command that hangs after producing some output (e.g., `sh -c 'echo started; sleep 999'`) with a short timeout. Assert `BackendTimeout` error. Assert log file contains `"started"` (partial output up to timeout).

### Integration test (in `tests/orchestrator.rs`)

6. **`orchestrator_creates_agent_output_logs`** — Run a minimal orchestrator flow using mock backends. Assert `agent-output-*.log` files appear in the loop artifact directory with correct headers containing backend name and role.

### Regression verification

7. Run full `cargo test` to confirm no regressions in `TmuxBackend` tests, `MockBackend` behavior, orchestrator parsing, and all existing backend tests.

## Out of Scope

- **Changing the `Backend` trait signature** — The trait remains `async fn execute(&self, prompt: &str) -> Result<String>`. Log path is communicated via the `SharedExecutionContext` side-channel.
- **Streaming for `TmuxBackend`** — Already handled via `tee` in the tmux shell command. No changes needed.
- **WebSocket/event-based streaming to UI** — This spec covers file-based streaming only.
- **Log rotation or cleanup** — Log files accumulate alongside artifacts; lifecycle management is handled by existing project cleanup.
- **Structured/JSON log format** — Plain text with `[stderr]` prefixes is sufficient for `tail -f`.
- **The `persist_cli_output()` function referenced in the PRD** — This function does not exist in the current codebase. The PRD line references were inaccurate; no refactoring or removal is needed.
- **Concurrent write safety for multiple readers** — The log file is append-only from a single writer. `tail -f` handles this natively on POSIX systems.