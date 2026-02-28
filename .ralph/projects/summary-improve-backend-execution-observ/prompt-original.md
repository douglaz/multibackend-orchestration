Now I have a thorough understanding of the codebase. Let me write the spec.

## Summary

Improve backend execution observability and resilience across four coordinated changes: (1) default Claude to `--output-format stream-json` for real-time observability, (2) replace wall-clock timeouts with inactivity-based heartbeat timeouts in both `execute_streaming` and `TmuxBackend` paths, (3) extend the output normalizer to parse Claude's streaming NDJSON wire format, and (4) enrich `RalphError::BackendTimeout` with idle-duration context for better diagnostics.

## Acceptance Criteria

- Fresh Claude calls (no `session_id`) emit exactly one `--output-format stream-json` pair via `ensure_json_output_args`, regardless of user-supplied args
- Resumed Claude calls emit `--output-format stream-json` via `effective_args_claude` (replacing the current hardcoded `json`)
- Arg rewriting is idempotent — duplicate `--output-format` flags are stripped before appending
- `normalize_claude_stream_json` accumulates all `content_block_delta` text deltas (concatenation, not last-wins)
- `normalize_claude_stream_json` extracts `session_id` from `message_start` events
- `normalize_claude_stream_json` extracts `tokens_in`, `tokens_out`, `cached_in` from `message_delta`/summary events
- Detection heuristic correctly routes stream-json (lines starting with `{` containing a `type` field) vs. single-object JSON
- Existing single-object JSON and raw-text fallback paths continue to pass all current tests
- Non-tmux path: a hanging backend (zero stdout/stderr output) times out after `timeout_seconds` of inactivity
- Non-tmux path: an actively streaming backend does NOT time out while heartbeats continue
- Tmux path: same inactivity semantics via capture-file `stat` growth detection
- `RalphError::BackendTimeout` carries `idle_seconds: u64` and `timeout_kind: TimeoutKind` (`Idle` | `Walltime`)
- Warn log on kill includes: backend name, role, attempt number, idle duration, total elapsed time
- At least one Codex conformance test verifies the shared inactivity-timeout code path
- No `max_walltime` config key is added
- No config-file migration required; the `timeout_seconds` key is semantically unchanged

## Technical Approach

### 1. Claude `stream-json` default (src/backend/mod.rs)

**`ensure_json_output_args` (line 314):** Before appending, iterate args and remove any `--output-format` flag plus its value (handles both `--output-format <val>` and `--output-format=<val>` forms). Then append `["--output-format", "stream-json"]`. This replaces the current conditional-append of `json`.

**`effective_args_claude` (line 342):** The existing strip logic at lines 371–381 already removes `--output-format` variants. Change the unconditional append at lines 390–392 from `"json"` to `"stream-json"`.

Both paths produce exactly one `--output-format stream-json` pair regardless of input args.

### 2. Stream-JSON normalizer (src/backend/output_normalizer.rs)

**Detection heuristic in `normalize_output` (line 34):** The existing Claude branch checks if trimmed output starts with `{`. Add a sub-check: parse the first valid JSON line — if it contains a `"type"` key, route to `normalize_claude_stream_json`. Otherwise, fall through to the existing `normalize_claude_json`.

**New function `normalize_claude_stream_json(raw: &str) -> Result<NormalizedOutput>`:**
- Iterate lines; for each line that parses as `serde_json::Value`, dispatch on `value["type"]`:
  - `"message_start"`: extract `value["message"]["id"]` as `session_id`
  - `"content_block_delta"`: push `value["delta"]["text"]` onto a `Vec<String>`
  - `"message_delta"`: extract `usage` from `value["usage"]` via the existing `extract_usage_fields` helper (after adapting it to accept `&Value` or `&Map`)
  - `"content_block_start"`, `"content_block_stop"`, `"message_stop"`, `"ping"`: skip gracefully
  - Unknown types: skip (forward-compatible)
- After iteration: join text deltas with empty separator. If no text was accumulated but at least one JSON event was parsed, return `Err(RalphError::ParseError(...))`. If no JSON events parsed at all, return raw text as fallback.

### 3. Heartbeat timeout — non-tmux (src/backend/mod.rs `execute_streaming`)

Replace the single `tokio::time::sleep(timeout)` watchdog (lines 600–617) with a heartbeat loop:

```
let last_activity = Arc::new(AtomicU64::new(now_millis()));
```

**Stdout read loop** (existing, ~line 620): after each `read_buf` returning >0 bytes, store `now_millis()` into `last_activity`.

**Stderr read loop** (existing): same heartbeat update on >0 bytes.

**Watchdog task:** Replace the single sleep with a loop:
```
loop {
    sleep(Duration::from_secs(1)).await;
    let idle_ms = now_millis() - last_activity.load(Ordering::Relaxed);
    if idle_ms >= timeout.as_millis() as u64 {
        // set timed_out flag, kill process group, break
    }
}
```

The `timed_out` flag and `timeout_cancel_rx` cancellation channel remain unchanged. On timeout, record `idle_ms / 1000` as `idle_seconds` for the enriched error.

### 4. Heartbeat timeout — tmux (src/backend/tmux.rs, tmux_backend.rs)

**`wait_for_exit` signature change:**
```rust
pub async fn wait_for_exit(
    exit_file_path: &Path,
    stdout_capture_path: &Path,
    stderr_capture_path: &Path,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<i32>
```

On each poll iteration, `stat` both `stdout_capture_path` and `stderr_capture_path`. If either file's `len()` grew since last check, reset `last_activity = Instant::now()`. Timeout triggers when `last_activity.elapsed() >= timeout` instead of `started.elapsed() >= timeout`.

**Stderr capture (tmux_backend.rs):** Add a `{prefix}-stderr.txt` temp file. Update the tmux shell command from:
```
cat prompt | command args 2>&1 | tee output; echo ${PIPESTATUS[1]} > exit
```
to:
```
cat prompt | command args 2>stderr | tee output; echo ${PIPESTATUS[1]} > exit
```

Pass the stderr capture path to `wait_for_exit`. After successful execution, read stderr file and persist it alongside stdout in artifacts.

### 5. Enriched BackendTimeout (src/error.rs)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Idle,
    Walltime,
}

#[error("backend timeout: {backend} ({timeout_kind:?}, idle {idle_seconds}s)")]
BackendTimeout {
    backend: String,
    idle_seconds: u64,
    timeout_kind: TimeoutKind,
},
```

All `BackendTimeout` construction sites (execute_streaming watchdog, wait_for_exit) populate `idle_seconds` from the measured idle duration and set `timeout_kind: TimeoutKind::Idle`.

**Orchestrator update (src/workflow/orchestrator.rs):** Update the `BackendTimeout` match arm in `execute_with_timeout_retries` (line 4078) to destructure the new fields. Enhance the warn log:
```rust
warn!(
    role = role,
    backend = %backend_name,
    attempt = attempt,
    idle_seconds = idle_seconds,
    total_elapsed_secs = started.elapsed().as_secs(),
    "backend timeout ({timeout_kind:?}), retrying..."
);
```

The `BackendTimeoutExhausted` variant remains unchanged (it already has `timeout_secs`).

### 6. Codex parity

No separate Codex implementation needed — Codex executes through the same `execute_streaming` and `TmuxBackend::execute` paths that now use heartbeat timeouts. One new conformance test validates this.

## Files & Modules

| File | Changes |
|---|---|
| `src/backend/mod.rs` | `ensure_json_output_args`: strip-then-append `stream-json` for Claude. `effective_args_claude`: change appended value from `json` to `stream-json`. `execute_streaming`: replace wall-clock watchdog with heartbeat loop using `Arc<AtomicU64>` last-activity tracker, update on stdout/stderr reads, populate enriched `BackendTimeout`. |
| `src/backend/output_normalizer.rs` | Add `normalize_claude_stream_json` function. Update dispatch in `normalize_output` with stream-json detection heuristic (first JSON line has `type` field). Reuse `extract_usage_fields` for `message_delta` events. |
| `src/backend/tmux.rs` | Expand `wait_for_exit` signature to accept stdout/stderr capture paths. Replace `started.elapsed()` wall-clock check with `last_activity.elapsed()` heartbeat check driven by file-size growth. Populate enriched `BackendTimeout` with `idle_seconds`. |
| `src/backend/tmux_backend.rs` | Create `{prefix}-stderr.txt` temp file. Update shell command to redirect stderr to capture file (`2>stderr`). Pass stdout/stderr paths to `wait_for_exit`. Read and persist stderr artifacts on completion. |
| `src/error.rs` | Add `TimeoutKind` enum (`Idle`, `Walltime`). Extend `BackendTimeout` variant with `idle_seconds: u64` and `timeout_kind: TimeoutKind`. Update Display format string. |
| `src/workflow/orchestrator.rs` | Update `BackendTimeout` match destructuring in `execute_with_timeout_retries` to include `idle_seconds` and `timeout_kind`. Enhance warn logs with idle duration and total elapsed time. |
| `src/validate/tests_streaming.rs` | Add conformance tests (see Testing Strategy). |

## Testing Strategy

### Unit tests (src/backend/output_normalizer.rs, inline `#[cfg(test)]`)

1. **`claude_stream_json_accumulates_deltas`** — Multi-line NDJSON with 3 `content_block_delta` events. Assert text is concatenated (not last-wins).
2. **`claude_stream_json_extracts_session_id`** — NDJSON with `message_start` containing message id. Assert `session_id` is populated.
3. **`claude_stream_json_extracts_usage`** — NDJSON with `message_delta` containing usage object. Assert `tokens_in`, `tokens_out`, `cached_in` populated.
4. **`claude_stream_json_no_text_returns_err`** — NDJSON with only `message_start`/`message_stop`, no `content_block_delta`. Assert `ParseError`.
5. **`claude_stream_json_detection_heuristic`** — Verify `normalize_output("claude", ...)` routes to stream-json when first JSON line has `type` field, and routes to single-object JSON when it does not.
6. **`claude_single_object_json_still_works`** — Existing-format JSON still parses correctly (regression guard).
7. **`claude_stream_json_skips_unknown_types`** — Include unknown event types in NDJSON, verify they are silently skipped.
8. **`claude_stream_json_malformed_lines_skipped`** — Mix of valid JSON and garbage lines, verify valid events are still extracted.

### Unit tests (src/backend/mod.rs, inline `#[cfg(test)]`)

9. **`ensure_json_output_args_claude_stream_json`** — Assert output contains `["--output-format", "stream-json"]` and no other `--output-format`.
10. **`ensure_json_output_args_strips_existing`** — Input args with `--output-format json` already present; assert replaced with `stream-json`.
11. **`effective_args_claude_stream_json`** — Assert resumed args contain `stream-json` not `json`.
12. **`ensure_json_output_args_idempotent`** — Call twice on same backend; assert exactly one `--output-format` pair.

### Conformance tests (src/validate/tests_streaming.rs)

13. **`timeout_inactivity_not_walltime`** — Mock script that outputs one line, sleeps `timeout + 2s`, outputs another line. With heartbeat timeout set to `timeout + 1s`, verify process completes successfully (heartbeat was reset by first output; second output arrives before idle timeout from first output expires). This validates heartbeat semantics: active streaming prevents timeout.
14. **`timeout_hanging_codex_backend`** — Codex-backed variant of existing `timeout_cleanup` test. Mock script that hangs after partial output. Verify: partial output preserved, timeout footer written, process killed. Validates Codex shares the same inactivity-timeout path.
15. **Existing `timeout_cleanup` test** — Must continue to pass with the new heartbeat semantics (a hanging process still times out).

### Error type tests (src/error.rs or integration)

16. **`backend_timeout_carries_idle_seconds`** — Construct `BackendTimeout` with known `idle_seconds`, verify Display output includes the value.

## Out of Scope

- No changes to backend spec syntax (`backend(model)`) or user-facing configuration schema
- No `max_walltime` config key — this change only reinterprets the existing `timeout_seconds` as inactivity timeout
- No template or prompt changes required from users
- No removal or modification of existing parse-retry orchestration logic
- No changes to Codex arg handling (`--json` flag unchanged)
- No config-file migration — `timeout_seconds` key name and location are unchanged
- No changes to the `BackendTimeoutExhausted` variant structure