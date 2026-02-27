## Summary

Add debug logging to the interactive PRD workflow (`src/daemon/interactive_prd.rs`) to capture raw backend output from every `run_backend_sync()` call and every reviewer backend call inside `run_review_with_retry()`. Currently, when backends produce malformed output (e.g., missing required spec sections), the raw text is discarded and only error messages survive in the state file. This makes diagnosing PRD failures extremely difficult. The fix writes each backend call's raw output to a log file using the existing `LogWriter` from `src/output_log.rs`, following the same append-mode pattern already used by `ralph auto`/`ralph run`. For reviewer output, logging is added inside `prd::quick::run_review_with_retry()` itself (since that function encapsulates the retry loop and backend calls), accepting an optional `LogWriter` parameter to maintain backward compatibility with the `ralph quick-prd` code path.

## Acceptance Criteria

- [ ] Every `run_backend_sync()` call in the interactive PRD workflow writes its raw output to a log file before any validation
- [ ] Every `run_review_with_retry()` call logs raw reviewer output for each parse attempt (up to 3), captured inside the retry loop
- [ ] Logs persist even when `check_spec_sections()` validation fails and output is discarded
- [ ] Each log entry includes: timestamp (via `write_attempt_separator`), backend name, unsanitized backend spec string (as a separate `backend_spec=` line, since `write_attempt_separator` sanitizes the label), and full raw output
- [ ] The prompt sent to the backend is logged (first 500 chars truncated at a char boundary + total byte length, to avoid multi-KB prompts bloating logs)
- [ ] Validation pass/fail status and any missing sections are recorded after each raw output entry for writer/draft calls that go through `check_spec_sections()`
- [ ] For question-generation calls (no section validation), log entries record `--- validation: n/a ---`
- [ ] For timeout and execution error paths, log entries record `--- execution: timeout ---` or `--- execution: error <message> ---` when no output is available
- [ ] Log files are written to `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/logs/issue-{number}-{role}.log` (one per role: `questions-a`, `questions-b`, `synthesis`, `writer`, `reviewer`)
- [ ] Log location is adjacent to the existing state file (`{issue_number}.json`) for discoverability
- [ ] Logging failures never cause the PRD workflow to fail (best-effort, matching `LogWriter` semantics)

## Technical Approach

**Reuse `output_log::LogWriter`** — the existing `LogWriter` in `src/output_log.rs` already provides:
- Append-mode file opening with `create_dir_all` for parent directories
- Attempt separators with timestamps, backend labels, and fallback flags
- Best-effort semantics (I/O errors warn but never propagate)
- `write_str()` / `write_bytes()` for raw content

### 1. Create a log directory helper

Add a function `prd_log_dir()` in `interactive_prd.rs`:
```rust
fn prd_log_dir(data_dir: &Path, owner: &str, repo: &str) -> PathBuf {
    data_dir.join(owner).join(repo).join(".ralph").join("interactive-prd").join("logs")
}
```
This places logs at `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/logs/`, directly alongside the existing state files at `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{N}.json`.

### 2. Add char-boundary-safe prompt truncation helper

Add a helper that truncates at a char boundary to avoid panicking on multi-byte UTF-8:
```rust
fn truncate_prompt_for_log(prompt: &str, max_chars: usize) -> &str {
    if prompt.len() <= max_chars {
        return prompt;
    }
    // Find the last char boundary at or before max_chars bytes
    let mut end = max_chars;
    while end > 0 && !prompt.is_char_boundary(end) {
        end -= 1;
    }
    &prompt[..end]
}
```
Usage at each log site:
```rust
log.write_str(&format!("prompt ({} bytes): {}\n",
    prompt.len(),
    truncate_prompt_for_log(prompt, 500)
));
```

### 3. Add unsanitized backend spec line

Since `write_attempt_separator()` sanitizes the backend label (e.g., `claude(opus)` → `claude_opus`), add an explicit unsanitized line after each separator to satisfy the requirement for full backend spec:
```rust
log.write_attempt_separator(backend.name(), is_fallback);
log.write_str(&format!("backend_spec={}\n", backend_spec_str));
```
Where `backend_spec_str` is the raw spec string (e.g., `"claude(opus)"`) from `config.writer_backend` or `config.reviewer_backend`.

### 4. Instrument `run_backend_sync()` — add `LogWriter` parameter

Change the signature to accept an optional log writer:
```rust
fn run_backend_sync(
    backend: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
    log: Option<&mut LogWriter>,
) -> Result<String>
```

Use `execute_with_log()` instead of `execute()` to capture streaming output during execution, matching the `ralph auto`/`ralph run` pattern for raw process output fidelity:
```rust
let result = rt.block_on(async {
    tokio::time::timeout(remaining, backend.execute_with_log(prompt, log_writer)).await
});
```

Note: since `execute_with_log` requires `Option<&mut LogWriter>` and the function already receives one, thread it through. The `CliBackend::execute_with_log` implementation calls `execute_streaming(prompt, log_writer)` which writes raw process output as it arrives, providing the same fidelity as existing agent-output artifacts.

Before the backend call, log the prompt summary and backend spec. After the call, log the returned output string and status. On timeout/error, log the error:
```rust
if let Some(log) = log.as_mut() {
    log.write_str(&format!("prompt ({} bytes): {}\n",
        prompt.len(), truncate_prompt_for_log(prompt, 500)));
}
// ... execute backend ...
match &result {
    Ok(output) => {
        if let Some(log) = log.as_mut() {
            log.write_str(output);
            log.write_str("\n");
        }
    }
    Err(_) => {
        if let Some(log) = log.as_mut() {
            log.write_str("--- execution: error ---\n");
        }
    }
}
```

For timeout specifically:
```rust
Err(_timeout) => {
    if let Some(log) = log.as_mut() {
        log.write_str("--- execution: timeout ---\n");
    }
    Err(RalphError::InteractivePrdFailed("PRD backend timeout exceeded".to_owned()))
}
```

### 5. Instrument `run_review_with_retry()` in `prd/quick.rs` — add optional `LogWriter`

This is the key change to address Review Issue #1. The function currently encapsulates the retry loop and backend calls internally, returning only parsed `ReviewFeedback`. To log raw reviewer output, modify its signature:

```rust
pub async fn run_review_with_retry(
    backend: Arc<dyn Backend>,
    prompt: String,
    log: Option<&mut LogWriter>,
) -> Result<ReviewFeedback>
```

Inside the loop, before each `backend.execute()`, write the attempt separator and prompt summary. After each call, write the raw output. On parse success, log `--- parse: ok ---`; on parse failure, log `--- parse: fail <error> ---`.

Update `run_review_with_retry_sync()` to accept and forward the `LogWriter`:
```rust
fn run_review_with_retry_sync(
    reviewer: &CliBackend,
    prompt: String,
    deadline: std::time::Instant,
    log: Option<&mut LogWriter>,
) -> Result<crate::prd::quick::ReviewFeedback>
```

The `log` is passed into the async block for `run_review_with_retry()`. Since `LogWriter` is `!Send`, and the tokio runtime here is single-threaded (`new_current_thread`), this is safe within the `block_on` call.

Backward compatibility: the `ralph quick-prd` code path (the only other caller of `run_review_with_retry`) passes `None` for the log parameter.

### 6. Instrument `run_draft_with_section_retry_sync()` — add `LogWriter` parameter

```rust
fn run_draft_with_section_retry_sync(
    writer: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
    log: Option<&mut LogWriter>,
) -> Result<String>
```

After each `check_spec_sections()` call, write validation status:
```rust
if missing.is_empty() {
    log.write_str("--- validation: pass ---\n");
} else {
    log.write_str(&format!("--- validation: fail missing=[{}] ---\n", missing.join(", ")));
}
```

### 7. Instrument `generate_questions_with_timeout()`

Add `issue_number: u32` and `data_dir`, `owner`, `repo` parameters (or a struct bundling them). The caller in `do_pending_to_awaiting` already has all of these. Open three `LogWriter` instances:
- `questions-a` for backend A output
- `questions-b` for backend B output
- `synthesis` for synthesis output

After each backend call, since question generation has no section validation, log:
```
--- validation: n/a ---
```

### 8. Instrument `generate_draft_from_answers_with_timeout()`

Add `issue_number: u32` and log path components. The caller `do_awaiting_answers_to_awaiting_feedback` has `issue.number`. Open two `LogWriter` instances:
- `writer` for all writer backend calls (initial draft + revisions)
- `reviewer` for all reviewer backend calls

Pass writer log to `run_draft_with_section_retry_sync()` and `run_backend_sync()` (for revision calls). Pass reviewer log to `run_review_with_retry_sync()`.

### 9. Instrument `generate_revision_from_feedback_with_timeout()`

Add `issue_number: u32` and log path components. The caller `do_awaiting_feedback` has `state.issue_number`. Same `writer`/`reviewer` log pattern as draft generation.

### 10. Parameter threading approach

Rather than adding many individual parameters, introduce a small struct to bundle log context:

```rust
struct PrdLogContext<'a> {
    log_dir: PathBuf,
    issue_number: u32,
    writer_spec: &'a str,
    reviewer_spec: &'a str,
}
```

This is constructed once in each caller (`do_pending_to_awaiting`, `do_awaiting_answers_to_awaiting_feedback`, `do_awaiting_feedback`) from `PrdPollConfig` and `issue_number`, then passed to the generate functions. The generate functions use it to open `LogWriter` instances and access unsanitized backend spec strings. This avoids signature bloat and keeps the log context cohesive.

## Files & Modules

- **`src/daemon/interactive_prd.rs`** — Primary change. Add `prd_log_dir()` helper, `truncate_prompt_for_log()` helper, `PrdLogContext` struct. Thread `LogWriter` through `run_backend_sync()`, `run_draft_with_section_retry_sync()`, `run_review_with_retry_sync()`. Instrument `generate_questions_with_timeout()`, `generate_draft_from_answers_with_timeout()`, `generate_revision_from_feedback_with_timeout()` with log writers. Add `issue_number` and log context parameters to each generate function.
- **`src/prd/quick.rs`** — Add optional `log: Option<&mut LogWriter>` parameter to `run_review_with_retry()`. Log raw reviewer output and parse results inside the retry loop. Update existing callers (the async `ralph quick-prd` path) to pass `None`.
- **`src/output_log.rs`** — No changes needed. Existing `LogWriter`, `log_path_for_role`, `ensure_log_parent`, and `format_attempt_separator` are sufficient as-is.
- **`src/validate/tests_interactive_prd.rs`** — Add conformance tests for log creation, discoverability, and persistence (see Testing Strategy).

## Testing Strategy

- **Conformance test: `prd_log_dir` path construction** — Verify the returned path matches `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/logs/`.
- **Conformance test: `truncate_prompt_for_log` char boundary safety** — Test with ASCII strings, multi-byte UTF-8 (e.g., emoji, CJK characters), empty strings, and strings shorter than the limit. Verify no panics and that the output is valid UTF-8.
- **Conformance test: log file creation and content** — Use `tempdir`, create a `LogWriter` via `prd_log_dir`, simulate the write sequence (separator → backend_spec line → prompt summary → raw output → validation status), and assert file contents contain all expected fields including unsanitized backend spec.
- **Conformance test: validation status formatting** — Test the formatting of pass/fail/n/a lines with missing section lists. Verify `--- validation: pass ---`, `--- validation: fail missing=[...] ---`, and `--- validation: n/a ---` formats.
- **Conformance test: error/timeout log entries** — Verify that `--- execution: timeout ---` and `--- execution: error ---` entries are written when backend calls fail.
- **Conformance test: log persistence on validation failure** — Use `tempdir`, simulate a `run_draft_with_section_retry_sync` call with a mock backend that returns output missing sections, and assert that the log file contains the raw output and the `validation: fail` line even though the overall call returns an error.
- **Conformance test: log file discoverability** — Assert log files are created in the `logs/` subdirectory adjacent to the state file path, and that filenames follow the `issue-{N}-{role}.log` pattern.
- **Conformance test: reviewer log capture** — Verify that `run_review_with_retry()` with a `LogWriter` logs raw output for each parse attempt, including parse failure details.
- **Extend existing `run_draft_with_section_retry_sync` tests** — The existing tests at lines ~3288 and ~3350 use mock backends. Extend them to pass a `LogWriter` (backed by a tempdir) and assert that raw output is logged even when validation fails.
- **Existing `LogWriter` tests remain unchanged** — The `output_log.rs` tests already cover append semantics, attempt numbering, and disabled-writer behavior.

## Out of Scope

- Log rotation or size limits — PRD workflows are infrequent; logs are small relative to backend output artifacts in `ralph auto`.
- Structured/JSON log format — Plain text with separators matches the existing `LogWriter` convention.
- Logging from the async `run_review_with_retry` callers in `ralph quick-prd` — the `quick-prd` code path passes `None` for the log writer parameter. It can be instrumented in a follow-up if needed.
- Exposing log paths in the GitHub issue comments or state file — can be a follow-up enhancement.
- Configuration to enable/disable PRD debug logging — always-on matches the `ralph auto` artifact pattern and keeps the implementation minimal.
- Logging the full process stderr stream separately — `execute_with_log()` already writes raw process output (stdout) to the `LogWriter` during streaming; stderr is captured in error messages. Full stderr logging can be added later if needed.