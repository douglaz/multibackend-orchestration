## Summary

Improve backend execution observability and resilience across four coordinated changes: (1) default Claude to `--output-format stream-json` for real-time observability in both non-tmux and tmux execution paths, (2) replace wall-clock timeouts with inactivity-based heartbeat timeouts in `execute_streaming` and `TmuxBackend`, (3) extend the output normalizer to parse Claude's streaming NDJSON wire format, and (4) enrich `RalphError::BackendTimeout` with idle-duration context for better diagnostics. The output normalizer gains a new `normalize_claude_stream_json` path that accumulates text deltas across `content_block_delta` events (concatenation, not last-wins), extracts `session_id` using a defined field-precedence rule from `message_start`, and pulls usage metrics from `message_delta` events. Existing single-object JSON and raw-text fallback paths remain unchanged.

## Acceptance Criteria

- [ ] Fresh Claude calls (no `session_id`) emit exactly one `--output-format stream-json` pair regardless of user-supplied args — in **both** `CliBackend` (via `ensure_json_output_args`) and `TmuxBackend` (via `build_shell_command`) paths
- [ ] Resumed Claude calls (`effective_args_claude`) strip any prior `--output-format` value and append `--output-format stream-json`
- [ ] Arg rewrite is idempotent — duplicate `--output-format` flags in user args produce exactly one pair in final args
- [ ] `normalize_claude_stream_json` accumulates all `content_block_delta` text deltas into a joined string (not last-wins)
- [ ] `normalize_claude_stream_json` extracts `session_id` from `message_start` events using the defined field-precedence rule (see §2)
- [ ] `normalize_claude_stream_json` extracts `usage` from `message_delta` events
- [ ] Detection heuristic correctly routes stream-json NDJSON vs single-object JSON for Claude output, using a known-event-type allowlist (not a generic `type`-field check)
- [ ] Existing single-object JSON and raw-text normalizer paths pass all existing tests unchanged
- [ ] Non-tmux path: a hanging backend (no stdout/stderr output) times out after `timeout_seconds` of inactivity
- [ ] Non-tmux path: an actively streaming backend does NOT time out while heartbeats continue
- [ ] Tmux path: same inactivity semantics via capture-file size growth detection
- [ ] `RalphError::BackendTimeout` carries `idle_seconds: u64` and `timeout_kind: TimeoutKind`
- [ ] Canonical warn-level log on timeout is emitted once from `execute_with_timeout_retries` and includes: backend name, role, attempt number, idle duration, and total elapsed time
- [ ] At least one Codex-backed conformance test explicitly configures `planner_backend=codex` to verify the shared inactivity-timeout path
- [ ] Tmux conformance tests cover: stdout growth resets timer, stderr-only growth resets timer, no-growth triggers timeout, and stderr capture is persisted in artifacts
- [ ] `max_walltime` is NOT added (out of scope)
- [ ] No config-file migration required; `timeout_seconds` config key semantics unchanged

## Technical Approach

### 1. Claude defaults to `stream-json` (both execution paths)

**`ensure_json_output_args`** (`src/backend/mod.rs:314`): Change from "only add if not present" to "strip-then-append". Iterate `self.args`, skipping any `--output-format` flag and its value (both `--output-format <val>` and `--output-format=<val>` forms). After the filter pass, unconditionally append `--output-format stream-json`. This makes the function idempotent regardless of what the user passes in their backend config.

**`effective_args_claude`** (`src/backend/mod.rs:342`): The existing strip loop at lines 371–381 already removes `--output-format` and its value. Change the unconditional append at lines 390–392 from `"json"` to `"stream-json"`. No structural change needed.

**`TmuxBackend::build_shell_command`** (`src/backend/tmux_backend.rs:87`): The fresh-invocation branch (line 115–116) currently uses `self.inner.args().to_vec()` directly, bypassing `ensure_json_output_args` entirely. Fix: when `ctx.session_id` is `None`, construct a `BackendInvocationContext` with `json_output_required: true` and call `self.inner.effective_args(&ctx)`, which dispatches to `ensure_json_output_args` for the no-session-id case. This mirrors how `CliBackend::execute_streaming` already handles the fresh case (lines 493–511). The fallback-to-base-args on error pattern is preserved. Updated code:

```rust
let args = if let Some(ref session_id) = ctx.session_id {
    let invocation_ctx = super::BackendInvocationContext {
        loop_dir: ctx.loop_dir.clone().unwrap_or_default(),
        role: ctx.role.clone().unwrap_or_default(),
        session_id: Some(session_id.clone()),
        json_output_required: true,
    };
    match self.inner.effective_args(&invocation_ctx) {
        Ok(rewritten) => rewritten,
        Err(e) => {
            debug!(..., "effective_args rewrite failed, falling back to default args");
            self.inner.args().to_vec()
        }
    }
} else {
    // Fresh invocation: still need JSON output flags (e.g., --output-format stream-json).
    let invocation_ctx = super::BackendInvocationContext {
        loop_dir: ctx.loop_dir.clone().unwrap_or_default(),
        role: ctx.role.clone().unwrap_or_default(),
        session_id: None,
        json_output_required: true,
    };
    match self.inner.effective_args(&invocation_ctx) {
        Ok(rewritten) => rewritten,
        Err(e) => {
            debug!(..., "ensure_json_output_args failed in tmux, falling back to default args");
            self.inner.args().to_vec()
        }
    }
};
```

All three paths (non-tmux fresh, non-tmux resumed, tmux fresh, tmux resumed) now produce exactly one `--output-format stream-json` pair for Claude backends.

### 2. Output normalizer: Claude stream-json support

**Detection heuristic** in `normalize_output` (`src/backend/output_normalizer.rs:40–43`): The current Claude branch checks `trimmed.starts_with('{')` and calls `normalize_claude_json`. Replace with:

```rust
if trimmed.starts_with('{') {
    if is_claude_stream_json(trimmed) {
        return normalize_claude_stream_json(raw_stdout);
    }
    return normalize_claude_json(trimmed, raw_stdout);
}
```

**`is_claude_stream_json`** detection function — uses a **known-event-type allowlist** rather than a generic `type`-field check, to avoid misrouting single-object JSON that happens to contain a `type` field:

```rust
const CLAUDE_STREAM_EVENT_TYPES: &[&str] = &[
    "message_start",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
    "message_delta",
    "message_stop",
    "ping",
];

fn is_claude_stream_json(trimmed: &str) -> bool {
    // Check the FIRST non-empty line only (deterministic, not any-line).
    let first_line = trimmed.lines().find(|l| !l.trim().is_empty());
    if let Some(line) = first_line {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if let Some(type_str) = value.get("type").and_then(|v| v.as_str()) {
                return CLAUDE_STREAM_EVENT_TYPES.contains(&type_str);
            }
        }
    }
    false
}
```

This is deterministic (first-line only, not any-line), and the allowlist prevents false positives from single-object JSON responses that might incidentally contain a `"type"` key with an unrelated value.

**`normalize_claude_stream_json`** function:

- Iterate lines, skip empty, parse each as `serde_json::Value`, skip malformed.
- Track `saw_json_event: bool` (set true when at least one line parses as JSON with a recognized `type`).
- Dispatch on `value["type"]` string:
  - `"message_start"`: extract `session_id` using the following **field-precedence rule** (first match wins):
    1. `value["message"]["session_id"]` — the Claude CLI session resume identifier, if present
    2. `value["session_id"]` — top-level session_id, if present
    3. `value["message"]["id"]` — the message id, as last resort (usable for `--resume` in Claude CLI)
  - `"content_block_delta"`: extract `value["delta"]["text"]` and push to `Vec<String>` accumulator.
  - `"message_delta"`: extract `value["usage"]` fields (`input_tokens`, `output_tokens`, `cache_read_input_tokens`) via the existing `extract_usage_fields` helper (adapted to accept `&serde_json::Map`). Usage from the final `message_delta` event takes precedence (overwrite, not accumulate).
  - `"content_block_start"`, `"content_block_stop"`, `"message_stop"`, `"ping"`: skip gracefully.
  - Unknown types: skip (forward-compatible, no error).
- After iteration:
  - If `!saw_json_event`: return raw text fallback (same as existing non-JSON behavior).
  - If text accumulator is non-empty: join all deltas with `""` (no separator), build `NormalizedOutput`.
  - If text accumulator is empty but `saw_json_event` is true: return `Err(ParseError("claude stream-json response has no assistant text"))`.
- Return type: `Result<NormalizedOutput>`.

**Error contract clarification**: `normalize_claude_stream_json` (and `normalize_claude_json`) return `Err(ParseError)` when structured data was successfully parsed but no assistant text was found. The caller `normalize_output` propagates this error directly — it does **not** silently fall back to raw text. The orchestrator-level `normalize_backend_output` in `src/workflow/orchestrator.rs` catches `ParseError` and triggers parse-retry logic, which is the intended behavior. The "must return valid NormalizedOutput" guarantee applies only to the non-JSON fallback path (no JSON parsed at all), not to the error path.

Existing `normalize_claude_json` and `extract_claude_text` remain untouched.

### 3. Heartbeat-based timeout (non-tmux `execute_streaming`)

Replace the single `tokio::time::sleep(timeout)` watchdog in `execute_streaming` (`src/backend/mod.rs:595–617`):

- Add `last_activity: Arc<AtomicU64>` initialized to current monotonic millis (`Instant::now()` epoch stored as u64 via a helper).
- In the stdout read loop (line 621–639): after each successful `read_buf` that returns >0 bytes, store current monotonic millis to `last_activity`.
- In the stderr reader task (line 553–593): after each successful read with >0 bytes, store current monotonic millis to `last_activity`.
- Watchdog task becomes a loop instead of a single sleep:
  ```
  loop {
      sleep(Duration::from_secs(1)).await;  // poll_interval
      let idle_ms = now_millis() - last_activity.load(Ordering::Relaxed);
      if idle_ms >= timeout.as_millis() as u64 {
          // kill process group, set timed_out flag
          break;
      }
  }
  ```
  The watchdog still uses `tokio::select!` with the `oneshot` cancel channel so it exits immediately when the command completes.
- On timeout, record `idle_seconds = idle_ms / 1000`. The `BackendTimeout` error is **constructed here** with `idle_seconds` and `timeout_kind: TimeoutKind::Idle`, but the **canonical warn log is NOT emitted here** — it is emitted by the orchestrator's `execute_with_timeout_retries` which has access to `attempt`, `role`, and `total_elapsed` context (see §5).

### 4. Heartbeat-based timeout (tmux `wait_for_exit`)

Refactor `wait_for_exit` in `src/backend/tmux.rs:114`:

- Add parameters: `stdout_capture: &Path` and `stderr_capture: &Path`.
- Track `last_activity: Instant` initialized to `Instant::now()`.
- Track previous sizes: `prev_stdout_size: u64` and `prev_stderr_size: u64`, both initialized to 0.
- On each poll iteration, `stat` both capture files (best-effort, ignore errors). If either file's size has grown since the last check, reset `last_activity = Instant::now()` and update the tracked sizes.
- Timeout condition changes from `started.elapsed() >= timeout` to `last_activity.elapsed() >= timeout`.
- On timeout, compute `idle_seconds` from `last_activity.elapsed().as_secs()` and populate `BackendTimeout` with `idle_seconds` and `timeout_kind: TimeoutKind::Idle`.

**Stderr capture in tmux shell command** (`src/backend/tmux_backend.rs:130–141`): Modify `build_shell_command` to accept a `stderr_file: &Path` parameter and redirect stderr to a separate capture file:

```
cat prompt | command args 2>stderr_file | tee stdout_file; echo ${PIPESTATUS[1]} > exit_file
```

Update `TmuxBackend::execute` to:
- Create a `{prefix}-stderr.txt` path alongside the existing `output_file` and `exit_file`.
- Add it to `TempFileGuard`.
- Pass it to `build_shell_command` and to `wait_for_exit`.
- After successful exit, read stderr file and pass bytes to `persist_cli_output` alongside stdout.

### 5. Enriched `BackendTimeout` error and canonical logging

Extend the `BackendTimeout` variant in `src/error.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Idle,
    Walltime,  // Reserved for future use; not emitted by this change.
}

#[error("backend timeout ({timeout_kind:?}): {backend} idle for {idle_seconds}s")]
BackendTimeout {
    backend: String,
    idle_seconds: u64,
    timeout_kind: TimeoutKind,
},
```

All existing `BackendTimeout` construction sites (3 total: `execute_streaming` watchdog, `wait_for_exit`, and the tmux external-disappearance path in `TmuxBackend::execute`) must be updated to include the new fields. The tmux external-disappearance path (window vanished without timeout) should use `idle_seconds: 0, timeout_kind: TimeoutKind::Idle` since it's not a genuine idle timeout.

**Canonical logging point**: The single authoritative warn log that includes all five required fields (backend name, role, attempt, idle duration, total elapsed) is emitted in `execute_with_timeout_retries` (`src/workflow/orchestrator.rs:4078`). This is the only place where `attempt` and `role` are naturally in scope. Low-level kill sites (`execute_streaming` watchdog, `wait_for_exit`) do **not** emit warn logs with attempt/role context — they only construct and return the enriched `BackendTimeout` error. This prevents partial or duplicated logging.

Update `execute_with_timeout_retries`:
- Add `let started = Instant::now();` at the top of the function (before the retry loop) for total-elapsed computation.
- Destructure new `BackendTimeout` fields in the match arm:

```rust
Err(RalphError::BackendTimeout { backend: backend_name, idle_seconds, timeout_kind }) => {
    if attempt == 3 {
        warn!(
            role = role,
            backend = %backend_name,
            attempt = attempt,
            idle_seconds = idle_seconds,
            timeout_kind = ?timeout_kind,
            total_elapsed_secs = started.elapsed().as_secs(),
            "backend timeout, retries exhausted"
        );
        return Err(RalphError::BackendTimeoutExhausted {
            backend: backend_name,
            phase: phase.to_owned(),
            role: role.to_owned(),
            timeout_secs,
            attempts: attempt,
        });
    }
    let backoff = 2_u64.pow((attempt - 1) as u32);
    warn!(
        role = role,
        backend = %backend_name,
        attempt = attempt,
        idle_seconds = idle_seconds,
        timeout_kind = ?timeout_kind,
        total_elapsed_secs = started.elapsed().as_secs(),
        backoff_secs = backoff,
        "backend timeout, retrying..."
    );
    sleep(Duration::from_secs(backoff)).await;
}
```

### 6. Codex parity

Codex already flows through `execute_streaming` (non-tmux) and `TmuxBackend::execute` (tmux). No separate Codex timeout implementation is needed. The heartbeat mechanism is backend-agnostic — it triggers on stdout/stderr byte flow regardless of which CLI is running.

Add one conformance test that **explicitly configures a Codex backend** to verify the shared inactivity-timeout path (see Testing Strategy).

## Files & Modules

| File | Changes |
|---|---|
| `src/error.rs` | Add `TimeoutKind` enum. Extend `BackendTimeout` with `idle_seconds: u64` and `timeout_kind: TimeoutKind`. Update `Display` format string. |
| `src/backend/mod.rs` | **`ensure_json_output_args`**: strip-then-append `stream-json` instead of guard-then-append `json`. **`effective_args_claude`**: change appended value from `"json"` to `"stream-json"`. **`execute_streaming`**: replace wall-clock watchdog with heartbeat loop using `Arc<AtomicU64>` for `last_activity`; update stderr reader to bump `last_activity`; construct enriched `BackendTimeout` on timeout (no warn log here). |
| `src/backend/output_normalizer.rs` | Add `CLAUDE_STREAM_EVENT_TYPES` allowlist constant. Add `is_claude_stream_json` detection helper using allowlist on first line's `type` field. Add `normalize_claude_stream_json` function with delta accumulation, `session_id` extraction (3-level field precedence), and usage extraction from `message_delta`. Update `normalize_output` Claude branch to route between stream-json and single-object paths. |
| `src/backend/tmux.rs` | **`wait_for_exit`**: add `stdout_capture` and `stderr_capture` path parameters; track file-size growth as heartbeat; timeout on inactivity instead of wall-clock; construct enriched `BackendTimeout` with `idle_seconds`. |
| `src/backend/tmux_backend.rs` | **`build_shell_command`**: accept `stderr_file` parameter; add `2>stderr_file` redirect; for fresh invocations (no `session_id`), call `effective_args` with `json_output_required: true` instead of using raw args. **`execute`**: create `{prefix}-stderr.txt` capture path, add to `TempFileGuard`, pass to `build_shell_command` and `wait_for_exit`, read and include stderr in `persist_cli_output`. |
| `src/workflow/orchestrator.rs` | **`execute_with_timeout_retries`**: add `started = Instant::now()` before retry loop; destructure new `BackendTimeout` fields (`idle_seconds`, `timeout_kind`) in match arm; emit canonical warn log with all five required fields (backend, role, attempt, idle duration, total elapsed). |
| `src/validate/tests_streaming.rs` | Add new conformance tests (see Testing Strategy). Update existing `timeout_cleanup` test if `BackendTimeout` field changes affect assertions. |

## Testing Strategy

### Unit tests (in-module `#[cfg(test)]`)

**`src/backend/output_normalizer.rs`**:

1. **`claude_stream_json_accumulates_text_deltas`**: Multi-line NDJSON with 3+ `content_block_delta` events. Assert text is concatenation of all deltas, not last-wins.
2. **`claude_stream_json_extracts_session_id_precedence`**: Test the 3-level field-precedence rule: (a) `message.session_id` takes priority, (b) top-level `session_id` is used when `message.session_id` absent, (c) `message.id` is used as fallback. Three sub-cases.
3. **`claude_stream_json_extracts_usage`**: `message_delta` with `usage` object. Assert `tokens_in`, `tokens_out`, `cached_in` populated correctly.
4. **`claude_stream_json_no_text_returns_err`**: Events exist (e.g., `message_start` + `message_stop`) but no `content_block_delta`. Assert `Err(ParseError)`.
5. **`claude_stream_json_unknown_types_skipped`**: Events with unrecognized `type` values mixed with valid deltas. Assert unrecognized types are silently ignored and text is still extracted.
6. **`claude_stream_json_malformed_lines_skipped`**: Mix of valid NDJSON and garbage lines. Assert valid events are processed, garbage is ignored.
7. **`claude_stream_json_fallback_to_raw_on_no_json`**: Non-JSON input passed through the Claude path. Assert raw text fallback.
8. **`claude_detection_heuristic_allowlist`**: (a) Single-object JSON with `"type": "text"` (not in allowlist) routes to `normalize_claude_json`, not stream-json. (b) NDJSON with first line `"type": "message_start"` routes to `normalize_claude_stream_json`. (c) Single-object JSON with `"type": "message_start"` but also containing `content` array routes to stream-json (correct — the allowlist check is on the first JSON line only, and this is a valid stream-json first event).
9. **`claude_single_object_json_still_works`**: Existing-format single-object JSON (with `content` array, `session_id`, `usage`) still parses correctly. Regression guard.

**`src/backend/mod.rs`**:

10. **`ensure_json_output_args_claude_stream_json`**: No prior `--output-format` in args. Assert output contains `["--output-format", "stream-json"]`.
11. **`ensure_json_output_args_strips_existing_format`**: User args contain `--output-format json`. Assert replaced with `--output-format stream-json`, exactly one pair.
12. **`ensure_json_output_args_strips_equals_form`**: User args contain `--output-format=text`. Assert replaced with `--output-format stream-json`, exactly one pair.
13. **`ensure_json_output_args_idempotent`**: Multiple `--output-format` entries. Assert exactly one `stream-json` pair.
14. **`effective_args_claude_emits_stream_json`**: Resumed call. Assert args contain `--output-format stream-json` (not `json`).

**`src/backend/tmux.rs`** (using temp files in a `#[tokio::test]`):

15. **`wait_for_exit_heartbeat_resets_on_stdout_growth`**: Background task writes to a mock stdout capture file at intervals shorter than the timeout. Exit file is written after total duration exceeds wall-clock timeout. Assert `wait_for_exit` returns Ok (timeout was reset by file growth).
16. **`wait_for_exit_heartbeat_resets_on_stderr_only_growth`**: Same as above but only the stderr capture file grows. Assert timeout is still reset.
17. **`wait_for_exit_times_out_on_idle`**: No file growth after initial creation. Assert `BackendTimeout` fires and contains `idle_seconds > 0` and `timeout_kind: Idle`.
18. **Update existing `wait_for_exit_times_out` test** to pass new capture-path parameters and assert new `BackendTimeout` fields.

**`src/error.rs`**:

19. **`backend_timeout_display_includes_fields`**: Construct `BackendTimeout` with known `idle_seconds` and `timeout_kind`. Assert Display output includes both values.

### Conformance tests (`src/validate/tests_streaming.rs`)

20. **`streaming::heartbeat_prevents_timeout`**: Use a slow-streaming mock script that emits one line every 500ms for 4 seconds total, with `timeout_seconds=2`. The mock's total wall-clock runtime (4s) exceeds the timeout threshold (2s), but each heartbeat arrives within the 2s window so the process should complete successfully. This validates heartbeat semantics: continuous output (even slow) prevents timeout.

21. **`streaming::codex_inactivity_timeout`**: Configure a **Codex** backend explicitly (e.g., backend name starts with `codex`, using `workflow.planner_backend=codex(...)` in the test config) with `timeout_seconds=1`. Use a hanging mock script (adapt `timeout_hanging_planner_mock_script` pattern). Verify: partial output preserved in log, timeout footer present, hanging process killed. This explicitly validates that Codex flows through the same inactivity-timeout path, not just Claude.

22. **`streaming::tmux_heartbeat_prevents_timeout`** (if tmux available, skip otherwise): Same slow-streaming mock as #20 but executed through `TmuxBackend`. Verify process completes despite wall-clock exceeding timeout.

23. **`streaming::tmux_stderr_resets_heartbeat`** (if tmux available, skip otherwise): Mock script that writes only to stderr at intervals shorter than timeout (no stdout after initial line). Verify process completes (stderr growth alone resets the heartbeat timer).

24. **`streaming::tmux_stderr_captured_in_artifacts`** (if tmux available, skip otherwise): Mock script that writes known content to stderr. Verify the stderr content appears in persisted artifacts.

25. **Existing `streaming::timeout_cleanup` test**: Must continue to pass with new heartbeat semantics. A hanging process (no output after partial write) still times out on inactivity. The test currently checks log content (partial output, timeout footer) and process death, none of which depend on `BackendTimeout` error fields directly. Should pass unchanged.

## Out of Scope

- No `max_walltime` config key is introduced by this change
- No changes to backend spec syntax (e.g., `backend(model)` template format)
- No template or prompt changes required from users
- No removal of existing parse-retry orchestration logic
- No config-file migration; `timeout_seconds` key semantics unchanged (now means idle-timeout instead of wall-clock, but the config surface is identical)
- No changes to Codex arg rewriting (`--json` flag) or Codex output normalization
- The `TimeoutKind::Walltime` variant is defined but not emitted; it is reserved for a future `max_walltime` feature
- The `BackendTimeoutExhausted` variant structure is unchanged
