## Summary

Add debug logging to the interactive PRD workflow (`interactive_prd.rs`) so that raw backend output from every `run_backend_sync()` call and every reviewer attempt inside `run_review_with_retry()` is persisted to disk. Today, when a backend returns malformed output, the raw text is discarded and only a truncated error message survives in `last_error`. This makes failures like issue #93 impossible to debug after the fact. The main orchestration workflow already solves this with `LogWriter` (in `output_log.rs`); the interactive PRD workflow should adopt the same mechanism.

Additionally, each log entry will include section-validation status (pass/fail with missing section names) and a SHA-256 hash of the prompt sent, so that logs are self-contained for debugging without storing potentially large prompt text verbatim.

## Acceptance Criteria

- Every `run_backend_sync()` call in the interactive PRD workflow writes raw backend output to a log file via `LogWriter`, regardless of whether section validation passes or fails.
- Every reviewer attempt inside `run_review_with_retry()` logs raw reviewer output, including malformed parse-failure attempts that are retried internally.
- Log files are persisted to `{data_dir}/{owner}/{repo}/.ralph/tmp/logs/` using the naming convention `prd-{issue_number}-{phase}-{role}.log`, where `{phase}` is one of `questions`, `draft`, or `revision`. Examples: `prd-103-questions-a.log`, `prd-103-questions-b.log`, `prd-103-questions-synth.log`, `prd-103-draft-writer.log`, `prd-103-draft-reviewer.log`, `prd-103-revision-writer.log`, `prd-103-revision-reviewer.log`.
- Each log entry contains an attempt separator with: attempt number, backend label, fallback flag, RFC 3339 timestamp, and a `prompt_sha256=` field containing the hex-encoded SHA-256 of the prompt — matching and extending the existing `format_attempt_separator` format.
- Each writer log entry includes a validation footer after the raw output: `--- validation={pass|fail} missing=[section1,section2] ---` indicating whether `check_spec_sections` passed and which sections (if any) were missing.
- Timeout events produce a timeout footer line via `write_timeout_footer`.
- Raw backend output is logged even when section validation (`check_spec_sections`) fails.
- Raw reviewer output is logged even when JSON parse (`parse_review_feedback`) fails; all internal retry attempts within `run_review_with_retry` are captured.
- Log files accumulate across retries and revision loops within a single invocation (append mode via `LogWriter`).
- Logging is best-effort; I/O failures do not affect PRD workflow outcomes (matching `LogWriter` semantics).
- The `issue_number` used for log file naming is threaded through the generation functions via a new parameter.

## Technical Approach

### 1. Extend `format_attempt_separator` with prompt hash

Add an optional `prompt_sha256` parameter to `format_attempt_separator` in `output_log.rs`:

```rust
pub fn format_attempt_separator(
    attempt: u32,
    backend_label: &str,
    is_fallback: bool,
    timestamp: &str,
    prompt_sha256: Option<&str>,  // NEW — None preserves existing callers
) -> String {
    let sanitized = sanitize_for_filename(backend_label);
    let fallback_flag = if is_fallback { "fallback=true" } else { "fallback=false" };
    let hash_field = match prompt_sha256 {
        Some(h) => format!(" prompt_sha256={h}"),
        None => String::new(),
    };
    format!("\n--- attempt={attempt} backend={sanitized} {fallback_flag} ts={timestamp}{hash_field} ---\n")
}
```

Update `LogWriter::write_attempt_separator` to accept an optional prompt hash and pass it through. Add a convenience method `write_attempt_separator_with_prompt` that computes the SHA-256 internally:

```rust
pub fn write_attempt_separator_with_prompt(
    &mut self,
    backend_label: &str,
    is_fallback: bool,
    prompt: &str,
) {
    use sha2::{Sha256, Digest};
    let hash = format!("{:x}", Sha256::digest(prompt.as_bytes()));
    self.attempt += 1;
    let timestamp = Utc::now().to_rfc3339();
    let separator = format_attempt_separator(
        self.attempt, backend_label, is_fallback, &timestamp, Some(&hash),
    );
    self.write_bytes(separator.as_bytes());
}
```

Existing callers of `write_attempt_separator` continue to pass `None` for the hash (no changes needed to orchestrator code).

### 2. Add validation footer helper to `LogWriter`

Add a `format_validation_footer` function and corresponding `write_validation_footer` method:

```rust
pub fn format_validation_footer(passed: bool, missing: &[String]) -> String {
    if passed {
        "\n--- validation=pass missing=[] ---\n".to_owned()
    } else {
        let joined = missing.join(",");
        format!("\n--- validation=fail missing=[{joined}] ---\n")
    }
}

// In impl LogWriter:
pub fn write_validation_footer(&mut self, passed: bool, missing: &[String]) {
    let footer = format_validation_footer(passed, missing);
    self.write_bytes(footer.as_bytes());
}
```

### 3. Switch `run_backend_sync` to use `execute_with_log`

Add `log_writer` and `prompt` parameters. Use `execute_with_log` for streaming log capture. Write the attempt separator (with prompt hash) before the call and timeout footer on timeout:

```rust
fn run_backend_sync(
    backend: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
    log_writer: Option<&mut LogWriter>,  // NEW
) -> Result<String> {
    // ... existing timeout check ...
    if let Some(lw) = log_writer.as_mut() {
        let is_fallback = lw.attempt() > 0;
        lw.write_attempt_separator_with_prompt(backend.name(), is_fallback, prompt);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| { /* ... */ })?;
    let result = rt.block_on(async {
        tokio::time::timeout(remaining, backend.execute_with_log(prompt, log_writer)).await
    });
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(err)) => Err(/* ... */),
        Err(_) => {
            if let Some(lw) = log_writer {
                lw.write_timeout_footer(&chrono::Utc::now().to_rfc3339());
            }
            Err(RalphError::InteractivePrdFailed("PRD backend timeout exceeded".to_owned()))
        }
    }
}
```

**Note on mutable borrow through async:** `execute_with_log` takes `Option<&mut LogWriter>`. Since `run_backend_sync` creates a dedicated single-threaded tokio runtime and blocks on it, the `&mut LogWriter` borrow remains valid for the entire async block. No lifetime issues arise because the runtime is scoped.

### 4. Modify `run_review_with_retry` to accept a `LogWriter`

The core issue is that `run_review_with_retry` (in `prd/quick.rs`) calls `backend.execute()` internally and discards raw output on parse failure. To capture all retry attempts, modify `run_review_with_retry` to accept `Option<&mut LogWriter>` and use `execute_with_log` instead of `execute`:

```rust
pub async fn run_review_with_retry(
    backend: Arc<dyn Backend>,
    prompt: String,
    log_writer: Option<&mut LogWriter>,  // NEW
) -> Result<ReviewFeedback> {
    let mut current_prompt = prompt;
    for attempt in 1..=3_u8 {
        if let Some(lw) = log_writer.as_deref_mut() {
            let is_fallback = lw.attempt() > 0;
            lw.write_attempt_separator_with_prompt(backend.name(), is_fallback, &current_prompt);
        }
        let raw = backend.execute_with_log(&current_prompt, log_writer.as_deref_mut()).await?;
        match parse_review_feedback(&raw) {
            Ok(feedback) => return Ok(feedback),
            Err(parse_error) => {
                if attempt == 3 {
                    return Err(/* ... */);
                }
                current_prompt = format!(/* correction prompt with {raw} */);
            }
        }
    }
    unreachable!()
}
```

Since `run_review_with_retry` is also called from the quick PRD pipeline (`QuickPrdPipeline`), the new parameter uses `Option` so existing callers can pass `None` without changes.

Update `run_review_with_retry_sync` to create a `LogWriter` reference and thread it through:

```rust
fn run_review_with_retry_sync(
    reviewer: &CliBackend,
    prompt: String,
    deadline: std::time::Instant,
    log_writer: Option<&mut LogWriter>,  // NEW
) -> Result<crate::prd::quick::ReviewFeedback> {
    // ... existing timeout check ...
    let rt = /* ... */;
    let backend: Arc<dyn Backend> = Arc::new(reviewer.clone());
    let result = rt.block_on(async {
        tokio::time::timeout(
            remaining,
            run_review_with_retry(backend, prompt, log_writer),
        ).await
    });
    match result {
        Ok(Ok(feedback)) => Ok(feedback),
        Ok(Err(err)) => Err(/* ... */),
        Err(_) => {
            if let Some(lw) = log_writer {
                lw.write_timeout_footer(&chrono::Utc::now().to_rfc3339());
            }
            Err(RalphError::InteractivePrdFailed("PRD backend timeout exceeded".to_owned()))
        }
    }
}
```

### 5. Thread `issue_number` and `LogWriter` through the generation functions

Each generation function receives a new `issue_number: u32` parameter (available at every call site from `issue.number` or `state.issue_number`) and constructs its own `LogWriter` instances. The log directory is derived from `config`:

```rust
let log_dir = config.repo_clone_path().join(".ralph").join("tmp").join("logs");
```

**`generate_questions_with_timeout(config, issue_text, issue_number)`**:
- Create `LogWriter` per question backend: `prd-{issue}-questions-a.log`, `prd-{issue}-questions-b.log`, and synthesis: `prd-{issue}-questions-synth.log`.
- Pass `Some(&mut writer)` to each `run_backend_sync` call.

**`generate_draft_from_answers_with_timeout(config, issue_text, questions_text, user_answers, issue_number)`**:
- Create writer log: `prd-{issue}-draft-writer.log`, reviewer log: `prd-{issue}-draft-reviewer.log`.
- Pass writer log to `run_draft_with_section_retry_sync` and all revision `run_backend_sync` calls.
- Pass reviewer log to `run_review_with_retry_sync`.

**`generate_revision_from_feedback_with_timeout(config, current_draft, aggregated_feedback, issue_number)`**:
- Create writer log: `prd-{issue}-revision-writer.log`, reviewer log: `prd-{issue}-revision-reviewer.log`.
- Same threading pattern as draft generation.

### 6. Thread LogWriter through intermediate functions

Update `run_draft_with_section_retry_sync` to accept and pass through the writer log, and write validation footers after each `check_spec_sections` call:

```rust
fn run_draft_with_section_retry_sync(
    writer: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
    log_writer: Option<&mut LogWriter>,  // NEW
) -> Result<String> {
    for attempt in 0..=DRAFT_SECTION_RETRIES {
        let raw = run_backend_sync(writer, prompt, deadline, log_writer.as_deref_mut())?;
        let (cleaned, missing) = check_spec_sections(&raw);
        if let Some(lw) = log_writer.as_deref_mut() {
            lw.write_validation_footer(missing.is_empty(), &missing);
        }
        if missing.is_empty() {
            return Ok(cleaned);
        }
        if attempt == DRAFT_SECTION_RETRIES {
            return Err(/* ... */);
        }
    }
    unreachable!()
}
```

Similarly, in the revision loops within `generate_draft_from_answers_with_timeout` and `generate_revision_from_feedback_with_timeout`, write validation footers after each `check_spec_sections` call on the writer log.

### 7. Project ID convention

Use `prd-{issue_number}` as the `project_id` for `LogWriter::open`. The `loop_number` parameter is `None` since interactive PRD does not have numbered loops — the attempt separator's `attempt=N` counter within each file handles retry numbering. The role encodes both phase and role: `questions-a`, `questions-b`, `questions-synth`, `draft-writer`, `draft-reviewer`, `revision-writer`, `revision-reviewer`.

## Files & Modules

| File | Change |
|---|---|
| `src/daemon/interactive_prd.rs` | Main changes: add `issue_number` parameter to `generate_questions_with_timeout`, `generate_draft_from_answers_with_timeout`, `generate_revision_from_feedback_with_timeout`; add `log_writer` parameter to `run_backend_sync`, `run_draft_with_section_retry_sync`, `run_review_with_retry_sync`; construct `LogWriter` instances in generation functions; write validation footers after `check_spec_sections` calls |
| `src/output_log.rs` | Add optional `prompt_sha256` parameter to `format_attempt_separator`; add `write_attempt_separator_with_prompt` method; add `format_validation_footer` function and `write_validation_footer` method on `LogWriter` |
| `src/prd/quick.rs` | Add `Option<&mut LogWriter>` parameter to `run_review_with_retry`; switch from `backend.execute()` to `backend.execute_with_log()` inside the retry loop; write attempt separators before each attempt; update `QuickPrdPipeline` callers to pass `None` |
| `src/backend/mod.rs` | No changes needed — `execute_with_log` already exists on the `Backend` trait |
| `Cargo.toml` | Add `sha2` crate dependency (for prompt hashing) |

## Testing Strategy

### Unit tests in `output_log.rs`

- **`format_attempt_separator_with_prompt_hash`**: Verify the separator includes `prompt_sha256=<hex>` when provided.
- **`format_attempt_separator_without_prompt_hash`**: Verify backward-compatible output when `prompt_sha256` is `None`.
- **`format_validation_footer_pass`**: Verify output is `--- validation=pass missing=[] ---`.
- **`format_validation_footer_fail`**: Verify output includes `validation=fail` and comma-separated missing section names.
- **`write_attempt_separator_with_prompt_computes_hash`**: Verify the SHA-256 of a known prompt appears in the log file.

### Unit tests in `interactive_prd.rs`

Extend the existing `#[cfg(test)]` module:

- **`run_backend_sync_logs_output_on_success`**: Call `run_backend_sync` with a mock backend and a `LogWriter` pointing at a tempdir; verify the log file contains the attempt separator (with prompt hash) and raw output.
- **`run_backend_sync_logs_output_on_section_failure`**: Call `run_draft_with_section_retry_sync` with a backend that returns incomplete specs; verify all retry attempts appear in the log and each has a `validation=fail` footer with the correct missing sections.
- **`run_backend_sync_logs_timeout`**: Use a backend that sleeps past the deadline; verify the timeout footer is written.
- **`run_draft_with_section_retry_logs_validation_pass`**: Backend returns a complete spec on first try; verify `validation=pass missing=[]` footer in the log.
- **`generate_draft_creates_log_files`**: Call `generate_draft_from_answers_with_timeout` with mock backends and `issue_number=42`; verify both `prd-42-draft-writer.log` and `prd-42-draft-reviewer.log` exist in the log directory.
- **`generate_questions_creates_separate_logs`**: Call `generate_questions_with_timeout` with `issue_number=42`; verify three log files (`prd-42-questions-a.log`, `prd-42-questions-b.log`, `prd-42-questions-synth.log`).
- **`generate_revision_creates_log_files`**: Call `generate_revision_from_feedback_with_timeout` with `issue_number=42`; verify `prd-42-revision-writer.log` and `prd-42-revision-reviewer.log`.
- **`log_writer_io_failure_does_not_affect_workflow`**: Pass a `LogWriter` opened against an invalid path (e.g., `/dev/null/nonexistent`) to `run_backend_sync`; verify the backend call still succeeds and returns output normally despite logging being disabled.

### Unit tests in `prd/quick.rs`

- **`run_review_with_retry_logs_all_attempts`**: Call `run_review_with_retry` with a mock backend that returns unparseable output on attempts 1-2 and valid JSON on attempt 3; pass a `LogWriter` and verify all three raw outputs appear in the log file.
- **`run_review_with_retry_logs_parse_failure_attempts`**: Mock backend returns unparseable output on all 3 attempts; verify all 3 raw outputs are logged even though the function returns an error.
- **`run_review_with_retry_none_log_writer`**: Pass `None` for `log_writer`; verify no panic and same behavior as before (backward compatibility with `QuickPrdPipeline` callers).

### Existing tests

The `Option<&mut LogWriter>` parameter is `None` in existing tests that don't care about logging, preserving backward compatibility. Tests that use `setup_mock_backends_stable` or `PrdPollConfig` directly need only minor updates to pass `None` for the new parameter. The `run_review_with_retry` callers in `QuickPrdPipeline` pass `None`.

### Validate tests (integration)

- **`validate_interactive_prd_creates_debug_logs`**: End-to-end test using the existing validate harness: configure a mock interactive PRD run with a backend that produces a valid spec, then verify log files exist under `.ralph/tmp/logs/` with the expected naming convention and contain attempt separators, prompt hashes, raw output, and validation footers.

## Out of Scope

- **Verbatim prompt logging**: Prompts are not logged verbatim due to their potentially large size. The SHA-256 hash enables prompt correlation (the same hash always means the same prompt was sent) without bloating log files. Full prompt logging can be added as a follow-up if needed.
- **Log rotation or cleanup**: Log files accumulate indefinitely. A separate cleanup mechanism (e.g., TTL-based pruning) is out of scope.
- **Structured/JSON log format**: Logs use the existing plain-text format with attempt separators. Migration to structured logging is a separate concern.
- **Exposing logs via GitHub comments or API**: Logs are local debug artifacts only.
- **Quick PRD logging**: The quick PRD workflow (`src/prd/quick.rs` `QuickPrdPipeline`) is a separate code path. While `run_review_with_retry` gains an `Option<&mut LogWriter>` parameter, `QuickPrdPipeline` callers pass `None` and are not wired up for logging. This can be addressed separately.