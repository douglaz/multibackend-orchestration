## Summary

Add structured debug logging to `interactive_prd.rs` that persists every backend call and response to disk. Currently, when a backend produces malformed output (missing sections, unparseable JSON review feedback), the raw response is discarded with only an `eprintln!` trace. This makes production debugging of PRD failures effectively impossible. The feature will instrument all `run_backend_sync()` and `run_review_with_retry_sync()` call sites — including each individual retry attempt inside `run_review_with_retry()` in `prd/quick.rs` — to write per-call log entries to a dedicated directory, separate from the existing state JSON file.

## Acceptance Criteria

- All `run_backend_sync()` calls in `interactive_prd.rs` (question-gen A, question-gen B, synthesis, initial draft retries, revision loop writes, feedback revision writes) emit a debug log entry to disk.
- All `run_review_with_retry()` retry attempts (up to 3 per call, across the 2 call sites: draft review loop and feedback revision review loop) each emit an individual debug log entry capturing the raw backend response and parse result before any discard or retry.
- Each log entry contains: ISO-8601 timestamp, backend spec string (e.g. `claude(opus)`), call purpose label (e.g. `question-gen-a`, `draft-attempt-1`, `review-attempt-2-of-3`), full prompt text, raw output (or error message), and validation result (sections missing, parse success/failure with error detail, etc.).
- Log files are written to `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}/logs/` — a sibling directory adjacent to the existing `{issue_number}.json` state file.
- The existing state file path `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json` is unchanged. No files are moved or restructured.
- Malformed backend output (unparseable review JSON, drafts with missing sections) is captured in full in the log entry before the response is discarded or a retry is attempted.
- No changes to `ralph auto` / `ralph run` orchestration artifacts.
- Log writes are best-effort: failures are `eprintln!`-warned but never block the workflow.
- Log file names are collision-proof across daemon restarts and repeated ticks (achieved via timestamp prefix rather than in-memory sequence counter).

## Technical Approach

### 1. Log directory and file naming

Logs are written to a per-issue `logs/` directory. The existing state file remains at its current path — no state file is moved or restructured:

```
{data_dir}/{owner}/{repo}/.ralph/interactive-prd/
├── 103.json                                          # existing state file (UNCHANGED)
└── 103/
    └── logs/
        ├── 20260226T143012Z-question-gen-a.json
        ├── 20260226T143018Z-question-gen-b.json
        ├── 20260226T143025Z-synthesis.json
        ├── 20260226T143031Z-draft-attempt-0.json
        ├── 20260226T143042Z-review-attempt-1-of-3.json
        ├── 20260226T143048Z-review-attempt-2-of-3.json
        └── ...
```

File names use a compact UTC timestamp prefix (`YYYYMMDDTHHMMSSZ`) for chronological ordering and collision avoidance across daemon restarts. If two log entries share the same second, a monotonic suffix is appended (e.g. `20260226T143012Z-001-question-gen-a.json`). This eliminates the in-memory sequence counter reset problem entirely.

Implementation: the `write_debug_log()` helper generates the timestamp at call time, constructs the filename, and on `AlreadyExists` appends an incrementing numeric suffix until a free slot is found (bounded to 999 retries to avoid infinite loops).

### 2. Log entry struct

Define a serializable struct in `interactive_prd.rs`:

```rust
#[derive(Serialize)]
struct PrdDebugLogEntry {
    timestamp: String,           // chrono::Utc::now().to_rfc3339()
    backend_spec: String,        // e.g. "claude(opus)"
    label: String,               // e.g. "draft-attempt-1", "review-attempt-2-of-3"
    prompt_chars: usize,         // prompt length for quick scanning
    prompt: String,              // full prompt text (possibly truncated, see §7)
    raw_output: Option<String>,  // None if backend errored before producing output
    error: Option<String>,       // None if backend succeeded
    validation: Option<String>,  // e.g. "ok", "missing: Summary, Testing Strategy", "parse_failed: expected value at line 3 column 5"
}
```

### 3. Logging helper function

Add a `write_debug_log()` function in `interactive_prd.rs`:

```rust
fn write_debug_log(log_dir: &Path, entry: &PrdDebugLogEntry) {
    // 1. Create log_dir (create_dir_all), eprintln! on failure and return.
    // 2. Format filename: "{compact_timestamp}-{label}.json"
    // 3. Serialize entry to JSON (serde_json::to_string_pretty).
    // 4. Write atomically via tempfile + rename, or direct write (best-effort).
    // 5. On any I/O error: eprintln!("prd: debug log write failed: {err}") and return.
    // Never panic or return Err.
}
```

### 4. Log context struct

Introduce a small context struct to avoid threading 5+ extra parameters through every function:

```rust
struct PrdLogCtx<'a> {
    log_dir: PathBuf,           // pre-computed: {data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}/logs/
    writer_spec: &'a str,       // e.g. "claude(opus)" for labeling writer calls
    reviewer_spec: &'a str,     // e.g. "claude(sonnet)" for labeling reviewer calls
}
```

The transition entry points (`do_pending_to_awaiting_answers`, `do_awaiting_answers_to_awaiting_feedback`, `do_awaiting_feedback_check`) already have access to `config: &PrdPollConfig` (which provides `data_dir`, `owner`, `repo`, `writer_backend`, `reviewer_backend`) and `issue.number`. They construct a `PrdLogCtx` and pass it down to all backend-calling helpers.

### 5. Per-retry logging in `run_review_with_retry` (prd/quick.rs modification)

The core review retry loop in `prd/quick.rs` (lines 204–235) must be modified to support per-attempt logging. The existing function signature takes `(backend, prompt)` and internally performs up to 3 attempts — the raw malformed output from each failed attempt is currently embedded into the next retry prompt and then lost. A wrapper-only approach in `interactive_prd.rs` cannot capture individual retry attempts because it only sees the final result.

**Approach:** Add an optional callback parameter to `run_review_with_retry`:

```rust
pub async fn run_review_with_retry(
    backend: Arc<dyn Backend>,
    prompt: String,
    on_attempt: Option<&dyn Fn(u8, &str, &str, Result<&ReviewFeedback, &str>)>,
    //                        ^attempt ^prompt ^raw_output ^parse result or error
) -> Result<ReviewFeedback> {
    let mut current_prompt = prompt;

    for attempt in 1..=3_u8 {
        let raw = backend.execute(&current_prompt).await?;
        let parse_result = parse_review_feedback(&raw);

        // Fire callback before acting on result
        if let Some(cb) = &on_attempt {
            match &parse_result {
                Ok(fb) => cb(attempt, &current_prompt, &raw, Ok(fb)),
                Err(e) => cb(attempt, &current_prompt, &raw, Err(&e.to_string())),
            }
        }

        match parse_result {
            Ok(feedback) => return Ok(feedback),
            Err(parse_error) => {
                if attempt == 3 { /* existing error return */ }
                current_prompt = /* existing reformat prompt using raw */;
            }
        }
    }
    unreachable!()
}
```

The callback is `Option` so that all existing callers (including `quick.rs` internal usage) pass `None` and are unaffected. The `interactive_prd.rs` wrapper `run_review_with_retry_sync()` constructs a closure that calls `write_debug_log()` for each attempt, capturing the raw output and parse error before the retry prompt overwrites them.

**Why this is necessary:** The acceptance criteria require that malformed review output is captured *before* it is discarded. The raw output from attempt N is only available inside the retry loop — by the time `run_review_with_retry` returns (success or final failure), intermediate raw outputs have been consumed into retry prompts. Only per-attempt instrumentation satisfies the requirement.

### 6. Instrumentation points

| Function | Call sites to instrument | Label pattern |
|---|---|---|
| `generate_questions_with_timeout()` | 2 `run_backend_sync` calls + 1 synthesis call | `question-gen-a`, `question-gen-b`, `synthesis` |
| `generate_draft_from_answers_with_timeout()` | `run_draft_with_section_retry_sync` + revision loop | `draft-attempt-N`, `revision-N` |
| `revise_draft_from_feedback()` | `run_draft_with_section_retry_sync` + revision loop | `feedback-draft-attempt-N`, `feedback-revision-N` |
| `run_draft_with_section_retry_sync()` | inner `run_backend_sync` calls (up to DRAFT_SECTION_RETRIES+1) | passes through parent label with attempt suffix |
| `run_review_with_retry_sync()` | delegates to `run_review_with_retry` with callback | `review-attempt-{N}-of-3`, `feedback-review-attempt-{N}-of-3` |

Each `run_backend_sync` call site constructs a `PrdDebugLogEntry` with:
- `backend_spec` from `PrdLogCtx.writer_spec` or `PrdLogCtx.reviewer_spec` as appropriate
- `label` from the table above
- `raw_output` = the Ok result, or `None` if the backend errored
- `error` = the Err message, or `None` on success
- `validation` = result of `check_spec_sections()` or parse status as applicable

### 7. Prompt truncation option

For extremely large prompts (e.g. full-repo context), honor a `RALPH_PRD_LOG_TRUNCATE` env var (default: unlimited). When set to a byte count, truncate the `prompt` field and append `... [truncated at {N} bytes, full length: {M}]`. The `prompt_chars` field always reflects the *original* full length so that truncation is evident.

## Files & Modules

| File | Change |
|---|---|
| `src/daemon/interactive_prd.rs` | Add `PrdDebugLogEntry` struct, `PrdLogCtx` struct, `write_debug_log()` function. Modify `generate_questions_with_timeout()`, `generate_draft_from_answers_with_timeout()`, `generate_revision_from_feedback_with_timeout()`, `run_draft_with_section_retry_sync()`, `run_review_with_retry_sync()` to accept `&PrdLogCtx` and emit log entries at each backend call. |
| `src/daemon/interactive_prd.rs` (transition functions) | Construct `PrdLogCtx` in `do_pending_to_awaiting_answers`, `do_awaiting_answers_to_awaiting_feedback`, `do_awaiting_feedback_check` and thread it into the backend-calling helpers. |
| `src/prd/quick.rs` | Add optional `on_attempt` callback parameter to `run_review_with_retry()`. Existing callers pass `None`. Fire callback with `(attempt_number, prompt, raw_output, parse_result)` on each attempt. |
| `src/prd/quick.rs` (call sites) | Update existing callers of `run_review_with_retry()` (within `QuickPrdPipeline`) to pass `None` for the new callback parameter. |
| No other files modified | `backend/mod.rs`, state serialization, and `ralph auto`/`ralph run` orchestration are untouched. |

## Testing Strategy

### Conformance tests (in `src/validate/tests_interactive_prd.rs`)

New conformance tests following the project's existing `ConformanceTest` pattern:

- **`debug_log_file_creation`**: Call `write_debug_log()` with a valid entry in a tempdir. Assert the file is created, is valid JSON, and contains all expected fields (timestamp, backend_spec, label, prompt, raw_output, validation).
- **`debug_log_graceful_failure`**: Call `write_debug_log()` targeting a read-only directory. Assert no panic or error propagation (function returns silently), and an `eprintln!` message is produced.
- **`debug_log_timestamp_collision_handling`**: Call `write_debug_log()` twice within the same second with the same label. Assert both files are created with distinct names (suffix disambiguation).
- **`debug_log_prompt_truncation`**: Set `RALPH_PRD_LOG_TRUNCATE` env var, call `write_debug_log()` with a large prompt. Assert the `prompt` field is truncated with marker text and `prompt_chars` reflects the original length.
- **`debug_log_captures_malformed_review_output`**: Using a mock backend that returns unparseable JSON, invoke `run_review_with_retry` with an `on_attempt` callback. Assert the callback fires for each attempt with the full raw malformed output and the parse error string before the retry or final failure.
- **`debug_log_question_gen_produces_three_entries`**: Using mock backends, invoke `generate_questions_with_timeout()` with a `PrdLogCtx` pointing to a tempdir. Assert exactly 3 log files are created (question-gen-a, question-gen-b, synthesis) with correct labels.
- **`debug_log_draft_review_cycle`**: Using mock backends (writer produces a valid draft, reviewer returns approved=true), invoke `generate_draft_from_answers_with_timeout()` with a `PrdLogCtx`. Assert log files are created for the draft attempt and review attempt(s).
- **`debug_log_no_impact_on_state_file`**: Run a full transition with logging enabled. Assert the state file at `{issue_number}.json` is identical in structure to what it would be without logging (no new fields, no path changes).

### Existing test preservation

All existing `interactive_prd` conformance tests continue to pass. Functions that gain a `&PrdLogCtx` parameter will need updated call sites in existing tests. In test contexts, construct a `PrdLogCtx` pointing to a tempdir (or `/dev/null`-equivalent no-op path) so that log writes are harmless and test assertions are unaffected. The `on_attempt` callback in `run_review_with_retry` is `Option<&dyn Fn(...)>` so existing callers pass `None` with no behavior change.

### Manual testing

Run the daemon against a test issue with an intentionally misconfigured backend (e.g. one that returns HTML instead of markdown). Confirm:
1. Log files appear in the expected directory with timestamp-prefixed names.
2. The raw garbage output is present in full in the `raw_output` field.
3. The `validation` field captures the specific parse/section error.
4. The workflow still proceeds (retries, error accumulation) exactly as before.

## Out of Scope

- Structured tracing framework (e.g. `tracing` crate) — future work if log volume warrants it.
- Log rotation or cleanup — logs accumulate per-issue; manual deletion or future GC.
- Logging for `ralph auto` / `ralph run` backend calls — separate feature.
- Real-time log streaming or dashboard — files on disk only.
- Modifying `run_review_with_retry` return type to expose intermediate attempts (the callback approach is less invasive than changing the return type).