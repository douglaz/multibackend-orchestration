# Real-Time Streaming of Backend Output to Log Files

## Summary

Replace `CliBackend::execute()`'s buffered `child.wait_with_output()` call with async chunk-based streaming of stdout/stderr, writing data to a log file as it arrives while accumulating the full output in memory for the existing return value. The streaming uses byte-level reads (not line-buffered) to preserve exact output encoding including partial lines, carriage returns, and progress indicators.

The log file path is communicated to `CliBackend` via the existing `SharedTmuxContext` pattern — extended into a general-purpose `SharedExecutionContext` — so the orchestrator can set it per-invocation without trait changes. When no log path is set (tests, non-daemon contexts), no file I/O occurs and behavior is identical to today.

Log paths are derived deterministically from `loop_number` and `role` alone — not from `loop_slug` or backend name — so they are available at all invocation points (including pre-slug Planning and prompt-review phases) and remain stable across retries, reformatter fallbacks, and backend switching within `execute_with_parse_retries`.

## Acceptance Criteria

- [ ] Backend output appears in log file incrementally as the child process produces it (verifiable via `tail -f`)
- [ ] Log file header (backend name, role, timestamp) is written immediately after process spawn, before any output arrives
- [ ] Exit status line is appended to log file after process completes
- [ ] `CliBackend::execute()` still returns the full aggregated `String` to callers — no change to `Backend` trait or return type
- [ ] Returned stdout `String` is byte-identical to what `wait_with_output()` would have produced (no newline normalization, no CR stripping, no trailing newline injection)
- [ ] When no `log_path` is configured in the execution context, behavior is identical to current (no file I/O side-effects)
- [ ] `TmuxBackend` continues to work unchanged (it wraps `CliBackend` but uses its own `tee`-based streaming; it never calls `CliBackend::execute()`)
- [ ] `MockBackend` is unaffected
- [ ] Stderr content is logged to the file (prefixed with `[stderr] `) but is NOT mixed into the returned stdout `String`
- [ ] Non-zero exit still returns `Err(BackendCommandFailed)` with stderr content, same as today
- [ ] On timeout, the child process is explicitly killed (`child.kill()`) and reaped (`child.wait()`), and the log file receives a `Timed out` footer before closing
- [ ] Log write failures are best-effort: logged via `tracing::warn!` but do NOT cause `execute()` to return an error. Backend execution continues regardless of log I/O problems
- [ ] Backend names containing parentheses or special characters are sanitized in log filenames (only `[a-zA-Z0-9_-]` retained)
- [ ] Retry attempts within `execute_with_parse_retries` append to the same log file with attempt separators, not to separate files
- [ ] Partial output prior to timeout is preserved in the log file

## Technical Approach

### 1. Extend `SharedTmuxContext` into `SharedExecutionContext`

Rename `TmuxExecutionContext` → `ExecutionContext` in `src/backend/tmux_backend.rs` and add `log_path`:

```rust
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    pub loop_number: Option<u32>,
    pub role: Option<String>,
    pub log_path: Option<PathBuf>,  // NEW
}
```

Rename `SharedTmuxContext` → `SharedExecutionContext` in `src/backend/mod.rs`. Rename `set_tmux_context()` → `set_execution_context()` on `BackendRegistry`. Update all call sites in `orchestrator.rs` — this is a mechanical rename with no logic changes.

### 2. Log path derivation strategy

Log paths use only `loop_number` and `role` — both of which are available at every invocation point — and do **not** include `loop_slug` or backend name in the filename. This avoids the issues where:

- The planner phase runs *before* `loop_slug` exists (the slug is derived from the planner's output)
- The prompt-review phase has no `loop_number` at all
- `execute_with_parse_retries` may switch backends (reformatter path), which would change the filename mid-retry

**Path formula for in-loop phases:**
```
{project_dir}/loops/{loop_number:03}/agent-output-{role}.log
```

The loop directory uses only the zero-padded loop number, not the slug. This is a new intermediate directory that exists before the slug-named directory (e.g. `001-user-auth/`) is created by `write_artifact()`. Log files exist at `loops/001/agent-output-planner.log` while spec artifacts exist at `loops/001-user-auth/20260215-spec.md`.

**Path formula for prompt-review (no loop number):**
```
{project_dir}/agent-output-prompt-reviewer.log
```

**Role values** map directly to the role strings already passed to `set_execution_context()`: `planner`, `impl`, `qa`, `reviewer`, `completer`, `prompt_reviewer`.

**Retry and reformatter handling:** Since the path is keyed on `(loop_number, role)` and the orchestrator sets context once before calling `execute_with_parse_retries`, all retry attempts (timeout retries, empty-output retries, reformatter fallbacks, format-reminder retries) write to the **same** log file. Each new `execute()` call within a retry sequence appends an attempt separator to the existing file:

```
--- Attempt 2 (reformatter: codex(o3)) ---
Started: 2026-02-16T14:32:00Z

[output from reformatter attempt]

---
Exit status: 0
Completed: 2026-02-16T14:33:15Z
```

This is achieved by having `CliBackend::execute()` open the log file in **append mode** (`OpenOptions::new().create(true).append(true)`). The first call writes the initial header; subsequent calls within the same retry sequence append attempt separators. Each `execute()` call reads the file size before writing — if non-zero, it writes an attempt separator instead of the initial header.

### 3. Add `shared_context` field to `CliBackend`

`CliBackend` gains a `shared_context: SharedExecutionContext` field. The `BackendRegistry::new()` passes its `shared_ctx` clone to all `CliBackend` instances during construction. `CliBackend::new()` gains a `shared_context` parameter. `claude::backend_from_config()` and `codex::backend_from_config()` gain a corresponding parameter and pass it through.

When `CliBackend` is wrapped by `TmuxBackend`, both hold a reference to the same `SharedExecutionContext`. However, `TmuxBackend::execute()` never calls `CliBackend::execute()` — it only reads `inner.command()`, `inner.args()`, `inner.env()` to construct its own shell command with `tee`. So there is no double-logging.

### 4. Replace buffered I/O in `CliBackend::execute()` (lines 150-197)

Replace `child.wait_with_output()` with manual async chunk-based reading:

1. After `child.spawn()` and writing the prompt to stdin, **take** `child.stdout` and `child.stderr` handles.
2. Read `log_path` from `self.shared_context.get().await`. If `Some`, open the log file in append mode. If the file is empty (new), write the initial header. If non-empty (retry), write an attempt separator with the current backend name and timestamp. If `None`, skip all file I/O.
3. Create parent directories via `tokio::fs::create_dir_all()` before opening the file.
4. Wrap each handle in `tokio::io::BufReader`. Use a `tokio::select!` loop calling `read_buf()` on both stdout and stderr concurrently, reading into byte buffers:

```rust
let mut stdout_buf = Vec::new();
let mut stderr_buf = Vec::new();
let mut stdout_reader = BufReader::new(child.stdout.take().unwrap());
let mut stderr_reader = BufReader::new(child.stderr.take().unwrap());
let mut stdout_chunk = BytesMut::with_capacity(8192);
let mut stderr_chunk = BytesMut::with_capacity(8192);
let mut stdout_done = false;
let mut stderr_done = false;

while !stdout_done || !stderr_done {
    tokio::select! {
        result = stdout_reader.read_buf(&mut stdout_chunk), if !stdout_done => {
            match result {
                Ok(0) => stdout_done = true,
                Ok(_) => {
                    stdout_buf.extend_from_slice(&stdout_chunk);
                    if let Some(f) = &mut log_file {
                        let _ = f.write_all(&stdout_chunk).await;
                        let _ = f.flush().await;
                    }
                    stdout_chunk.clear();
                }
                Err(_) => stdout_done = true,
            }
        }
        result = stderr_reader.read_buf(&mut stderr_chunk), if !stderr_done => {
            match result {
                Ok(0) => stderr_done = true,
                Ok(n) => {
                    stderr_buf.extend_from_slice(&stderr_chunk);
                    if let Some(f) = &mut log_file {
                        // Write [stderr] prefix for each line in the chunk
                        let _ = write_stderr_prefixed(f, &stderr_chunk).await;
                        let _ = f.flush().await;
                    }
                    stderr_chunk.clear();
                }
                Err(_) => stderr_done = true,
            }
        }
    }
}
```

**Why chunk-based, not line-based:** Using `read_buf()` instead of `read_line()` preserves the exact byte stream. `read_line()` would:
- Block on partial lines (no visibility until `\n` arrives — breaks progress indicators)
- Normalize line endings (the returned `String` must be byte-identical to `wait_with_output()`)
- Fail on non-UTF-8 sequences

Stdout chunks are written directly to the log file as raw bytes. Stderr chunks get `[stderr] ` prefixed per-line via a helper `write_stderr_prefixed()` that scans for newlines within the chunk and inserts prefixes after each one.

The in-memory accumulation uses `Vec<u8>` (not `String`) to avoid lossy conversion mid-stream. `String::from_utf8_lossy()` is applied only at the end, matching the current behavior at line 196.

5. After both streams reach EOF, call `child.wait()` for the exit status.
6. Write the footer to the log file (exit status + completion timestamp), flush, and drop.
7. Return `Ok(String::from_utf8_lossy(&stdout_buf).to_string())` on success or `Err(BackendCommandFailed { details: String::from_utf8_lossy(&stderr_buf).trim().to_owned() })` on non-zero exit — identical to current behavior.

### 5. Timeout and process cleanup

The entire streaming loop (step 4-5 above) is wrapped in `tokio::time::timeout(self.timeout, ...)`. On timeout:

```rust
match timeout(self.timeout, streaming_loop(&mut child, ...)).await {
    Ok(Ok((stdout_buf, stderr_buf, status))) => {
        // Normal completion — write footer with exit status
        finalize_log(&mut log_file, status).await;
        // Return Ok/Err based on exit status
    }
    Ok(Err(io_err)) => {
        // I/O error during streaming
        let _ = child.kill().await;
        let _ = child.wait().await;
        finalize_log_error(&mut log_file, &io_err).await;
        Err(BackendCommandFailed { ... })
    }
    Err(_timeout) => {
        // Timeout: explicitly kill and reap the child
        let _ = child.kill().await;
        let _ = child.wait().await;  // Reap to avoid zombie
        // Write timeout footer to log file
        if let Some(f) = &mut log_file {
            let _ = write!(f, "\n---\nTimed out after {}s\nAt: {}\n",
                self.timeout.as_secs(), Utc::now().to_rfc3339());
            let _ = f.flush().await;
        }
        Err(BackendTimeout { backend: self.name.clone() })
    }
}
```

The `child` handle must remain accessible outside the timeout future. The streaming loop borrows `child.stdout` and `child.stderr` (via `take()`), but `child` itself stays in the outer scope so `kill()` and `wait()` can be called on timeout. This means the streaming loop function takes the stdout/stderr handles as parameters, not `&mut child`.

### 6. File path safety

Backend names like `claude(opus)` or `codex(o3-xhigh)` contain parentheses which are problematic in filenames. Add a `sanitize_for_filename()` utility in `src/util/slug.rs`:

```rust
pub fn sanitize_for_filename(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            result.push(ch);
        } else if ch == '(' || ch == ')' || ch == ' ' || ch == '/' {
            result.push('-');
        }
    }
    // Collapse consecutive dashes, trim boundary dashes
    // (reuse logic from slugify_feature_name or extract shared helper)
    collapse_dashes(&result)
}
```

This is applied only to the backend name when writing the attempt separator line inside the log file (for human readability), **not** to the log filename itself. Log filenames use only the `role` string, which is always a safe ASCII identifier (`planner`, `impl`, `qa`, `reviewer`, `completer`, `prompt_reviewer`).

### 7. Log I/O failure policy

All log file operations (open, write, flush, close) are **best-effort**. Failures are:
- Logged via `tracing::warn!("failed to write to agent output log: {err}")`
- NOT propagated as errors from `execute()`
- The `log_file` handle is set to `None` on any I/O error to suppress further write attempts for that invocation

This ensures a filesystem issue (full disk, permissions) does not break backend execution. The in-memory output accumulation and return value are completely independent of log file I/O.

### 8. Flush strategy

Use `tokio::fs::File` directly (no `BufWriter` wrapper). Call `flush()` after every write to ensure each chunk is visible to `tail -f` immediately. Since the writes are already chunk-sized (typically 4-8KB), there is no meaningful syscall overhead from skipping `BufWriter`. The priority is real-time visibility over I/O efficiency.

### 9. Orchestrator sets `log_path` before each `execute()` call

In `src/workflow/orchestrator.rs`, each `set_execution_context()` call (9 call sites) adds a `log_path`:

**In-loop phases** (planner, impl, qa, reviewer, completer — 8 call sites):
```rust
registry
    .set_execution_context(ExecutionContext {
        loop_number: Some(loop_number),
        role: Some("planner".to_owned()),
        log_path: Some(
            project_dir
                .join("loops")
                .join(format!("{loop_number:03}"))
                .join("agent-output-planner.log"),
        ),
    })
    .await;
```

**Prompt reviewer** (1 call site, no loop number):
```rust
registry
    .set_execution_context(ExecutionContext {
        loop_number: None,
        role: Some("prompt_reviewer".to_owned()),
        log_path: Some(
            project_dir.join("agent-output-prompt-reviewer.log"),
        ),
    })
    .await;
```

### 10. Log file format

**Initial header (first execute() call for this log file):**
```
=== Backend Output Log ===
Backend: claude(opus)
Role: implementer
Started: 2026-02-16T14:30:00Z
---

```

**Attempt separator (retry calls appending to same file):**
```

--- Attempt 2 (backend: codex(o3)) ---
Started: 2026-02-16T14:32:00Z

```

**Normal completion footer:**
```

---
Exit status: 0
Completed: 2026-02-16T14:35:22Z
```

**Timeout footer:**
```

---
Timed out after 300s
At: 2026-02-16T14:35:00Z
```

**Non-zero exit footer:**
```

---
Exit status: 1
Completed: 2026-02-16T14:35:22Z
```

## Files & Modules

| File | Change |
|------|--------|
| `src/backend/tmux_backend.rs` | Rename `TmuxExecutionContext` → `ExecutionContext`, add `log_path: Option<PathBuf>` field. Update `build_label()` and all internal references. |
| `src/backend/mod.rs` | Rename `SharedTmuxContext` → `SharedExecutionContext`. Add `shared_context: SharedExecutionContext` field to `CliBackend`. Rewrite `CliBackend::execute()` to use chunk-based async streaming with `read_buf()` + `select!` loop, append-mode log file writing, explicit `child.kill()`/`child.wait()` on timeout. Rename `set_tmux_context()` → `set_execution_context()`. Update `BackendRegistry::new()` to pass shared context to `CliBackend`. Add helper `write_stderr_prefixed()` for stderr chunk logging. Add `use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader}` and `use bytes::BytesMut`. |
| `src/backend/claude.rs` | Add `SharedExecutionContext` parameter to `backend_from_config()`, pass to `CliBackend::new()`. |
| `src/backend/codex.rs` | Same as `claude.rs`. |
| `src/util/slug.rs` | Add `sanitize_for_filename()` function for safe backend-name display in log headers/separators. |
| `src/workflow/orchestrator.rs` | Rename all `set_tmux_context(TmuxExecutionContext { .. })` → `set_execution_context(ExecutionContext { .. , log_path: Some(...) })`. Derive `log_path` from `project_dir`, `loop_number`, and `role` at each call site (9 sites). No `loop_slug` or backend name in path. |
| `src/backend/mock.rs` | No changes needed — `MockBackend` doesn't use execution context. |
| `tests/backend.rs` | Update `CliBackend::new()` calls to pass a default `SharedExecutionContext`. Update renamed types. |
| `tests/backend_tmux.rs` / `tests/backend_tmux_backend.rs` | Update for renamed types (`TmuxExecutionContext` → `ExecutionContext`, `SharedTmuxContext` → `SharedExecutionContext`). |
| `Cargo.toml` | Add `bytes` crate dependency (for `BytesMut`). |

## Testing Strategy

### Unit tests (in `tests/backend.rs` or `src/backend/mod.rs`)

1. **`cli_backend_streaming_returns_full_output`** — Set `log_path` in the execution context pointing to a temp directory. Spawn `CliBackend` running `printf "line1\nline2\nline3\n"`. Assert returned string equals `"line1\nline2\nline3\n"` exactly (byte comparison). Assert log file exists, contains the header, all three lines, exit status 0, and footer.

2. **`cli_backend_no_log_path_no_file_created`** — Leave `log_path` as `None`. Execute a simple command. Assert output returned correctly and no log files were created in the temp directory.

3. **`cli_backend_stderr_logged_not_in_return_value`** — Use `sh -c 'echo out; echo err >&2'`. Assert returned `String` contains only `"out\n"`. Assert log file contains `"out\n"` and a line with `[stderr] err`.

4. **`cli_backend_nonzero_exit_writes_status_to_log`** — Use `sh -c 'echo partial; echo fail >&2; exit 1'`. Assert `BackendCommandFailed` error with `"fail"` in details. Assert log file contains `"partial"`, `"[stderr] fail"`, and `Exit status: 1`.

5. **`cli_backend_timeout_kills_child_and_writes_footer`** — Use `sh -c 'echo started; sleep 999'` with a 1-second timeout. Assert `BackendTimeout` error. Assert log file contains `"started"` and `"Timed out"`. Assert the child process is no longer running (query PID via `/proc` or `kill -0` check).

6. **`cli_backend_preserves_partial_lines_and_cr`** — Use `printf "progress: 50%%\rprogress: 100%%\rdone\n"`. Assert returned string contains exact bytes including `\r` characters. Assert log file contains the same bytes.

7. **`cli_backend_retry_appends_to_same_log`** — Set `log_path` in context. Call `execute()` twice on the same `CliBackend` (simulating retries). Assert log file contains two headers/separators, with the second marked as a subsequent attempt. Assert no second log file was created.

8. **`cli_backend_log_write_failure_does_not_fail_execute`** — Set `log_path` to a path under a read-only directory. Execute a command. Assert `execute()` returns `Ok(output)` successfully. Verify via tracing test subscriber that a warning was emitted.

9. **`sanitize_for_filename_handles_special_chars`** — Unit test for `sanitize_for_filename()`. Assert `"claude(opus)"` → `"claude-opus-"` or similar safe form. Assert `"codex(o3/test)"` produces no `/` in output. Assert empty input returns something deterministic.

### Conformance tests (in `src/validate/tests_tail.rs` or new `src/validate/tests_streaming.rs`)

10. **`streaming::log_file_appears_during_execution`** — Using the conformance harness with a mock backend script that sleeps between output lines, assert that the log file exists and contains partial output while the backend is still running (not just after completion). This validates true mid-execution streaming visibility, not just end-state file content.

11. **`streaming::log_file_contains_header_and_footer`** — Run a single loop via `ralph run --loops 1` with mock backends. Assert `agent-output-planner.log` exists under the loop directory with the expected header format (`=== Backend Output Log ===`, `Backend:`, `Role:`, `Started:`) and footer (`Exit status:`, `Completed:`).

12. **`streaming::prompt_reviewer_log_at_project_root`** — Enable prompt review, run a minimal flow. Assert `agent-output-prompt-reviewer.log` exists directly under the project directory (not in a `loops/` subdirectory).

13. **`streaming::timeout_log_contains_timed_out_marker`** — Use a mock backend that hangs, with a short timeout configured. Assert the log file contains the `Timed out` footer and partial output.

### Integration test (in `tests/orchestrator.rs`)

14. **`orchestrator_creates_agent_output_logs`** — Run a minimal orchestrator flow using mock backends. Assert `agent-output-*.log` files appear in the appropriate loop artifact directories with correct headers containing backend name and role.

### Regression verification

15. Run full `cargo test` to confirm no regressions in `TmuxBackend` tests, `MockBackend` behavior, orchestrator parsing, and all existing backend tests. Specifically verify that renamed types compile and tests using `SharedTmuxContext`/`TmuxExecutionContext` still pass under the new names.

## Out of Scope

- **Changing the `Backend` trait signature** — The trait remains `async fn execute(&self, prompt: &str) -> Result<String>`. Log path is communicated via the `SharedExecutionContext` side-channel.
- **Streaming for `TmuxBackend`** — Already handled via `tee` in the tmux shell command. `TmuxBackend::execute()` never calls `CliBackend::execute()`, so no changes needed.
- **WebSocket/event-based streaming to UI** — This spec covers file-based streaming only.
- **Log rotation or cleanup** — Log files accumulate alongside artifacts; lifecycle management is handled by existing project cleanup.
- **Structured/JSON log format** — Plain text with `[stderr]` prefixes is sufficient for `tail -f`.
- **The `persist_cli_output()` function referenced in the PRD** — This function does not exist in the current codebase. The PRD references were inaccurate.
- **Concurrent write safety for multiple readers** — The log file is append-only from a single writer. `tail -f` handles this natively on POSIX systems.
- **Backpressure or rate limiting** — Backend processes produce output at manageable rates. No throttling needed.
- **Using `loop_slug` in log file paths** — Intentionally excluded. The slug is derived from planner output and is unavailable during the planning phase itself. Using only `loop_number` and `role` keeps paths deterministic and available at all call sites.
