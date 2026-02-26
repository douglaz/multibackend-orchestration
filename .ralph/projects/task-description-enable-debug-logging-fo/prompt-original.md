## Summary

Add debug logging to the interactive PRD workflow (`interactive_prd.rs`) by threading a `LogWriter` through all backend call paths so that raw output, timestamps, backend identity, prompts, and validation results are persisted to `<data_dir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>/logs/`. Currently, `run_backend_sync` calls `backend.execute(prompt)` which discards streaming output, and `run_review_with_retry_sync` wraps an async retry loop (`run_review_with_retry`) that calls `backend.execute()` up to 3 times without any logging. The fix modifies `run_backend_sync` to accept `Option<&mut LogWriter>` and call `execute_with_log`, adds a new `run_review_with_retry_logged` async function that mirrors the existing retry logic while streaming each attempt through `execute_with_log`, and opens `LogWriter` instances in the four entry-point functions that produce backend calls. Prompt summaries are written inline to the `.log` file (not companion files) as a header block before each backend invocation. The existing `LogWriter` and `execute_with_log` APIs are used without modification.

## Acceptance Criteria

- [ ] All `run_backend_sync()` call sites in `interactive_prd.rs` write raw backend output to log files via `LogWriter` and `execute_with_log`
- [ ] `run_review_with_retry_sync()` logs raw reviewer backend output for every attempt (including parse-retry correction prompts) via a new `run_review_with_retry_logged` async function
- [ ] Log files are written to `<data_dir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>/logs/`
- [ ] Each log file contains: attempt separator (timestamp, backend name, fallback flag per `format_attempt_separator`), full raw streaming output captured by `execute_with_log`, and is named by workflow phase using `log_path_for_role`
- [ ] A prompt summary (first 500 chars truncated at a char boundary + SHA-256 hash + byte length) is written inline to the `.log` file before each backend call
- [ ] Attempt separators use `is_fallback = log_writer.attempt() > 0`, matching orchestrator parity
- [ ] After `check_spec_sections` returns, a `--- validation: PASS ---` or `--- validation: FAIL missing=[...] ---` line is appended to the log
- [ ] After `parse_review_feedback` returns, a `--- review-parse: OK approved=<bool> ---` or `--- review-parse: FAIL error=<msg> ---` line is appended to the log
- [ ] Logs persist even when section validation, review parsing, or backend execution fails
- [ ] Logs are written synchronously before validation (`check_spec_sections` / `parse_review_feedback`) runs — the `LogWriter` captures streaming output during `execute_with_log`, guaranteeing persistence before the execute call returns
- [ ] No changes to existing validation logic, error handling, or `last_error` state behavior
- [ ] Existing tests pass; new unit tests cover `LogWriter` integration in the modified functions

## Technical Approach

### 1. Canonical log directory path

All log files are written under a single canonical path derived from the existing `state_path` helper:

```rust
fn log_dir(data_dir: &Path, owner: &str, repo: &str, issue_number: u32) -> PathBuf {
    data_dir
        .join(owner)
        .join(repo)
        .join(".ralph")
        .join("interactive-prd")
        .join(issue_number.to_string())
        .join("logs")
}
```

This places logs at `<data_dir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>/logs/`, which is a sibling directory to the existing state file at `<data_dir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>.json`. All sections of this spec use this single path definition.

### 2. Modify `run_backend_sync` to accept `Option<&mut LogWriter>`

**New signature:**

```rust
fn run_backend_sync(
    backend: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
    log_writer: Option<&mut LogWriter>,
) -> Result<String>
```

**Body changes:**
- Before calling the backend, if `log_writer` is `Some`, write an attempt separator using `is_fallback = lw.attempt() > 0` (matching the orchestrator's pattern in `execute_with_timeout_retries` at orchestrator.rs:5373) and call `write_prompt_summary`.
- Replace `backend.execute(prompt)` with `backend.execute_with_log(prompt, log_writer)`. Since `execute_with_log` accepts `Option<&mut LogWriter>`, the `None` case preserves the current behavior exactly.
- All existing call sites that do not yet have a `LogWriter` pass `None`, preserving behavior until they are updated.

### 3. Prompt summary helper (char-boundary-safe)

```rust
fn write_prompt_summary(log: &mut LogWriter, prompt: &str) {
    use sha2::{Sha256, Digest};
    let hash = hex::encode(Sha256::digest(prompt.as_bytes()));
    let end = prompt.len().min(500);
    // Find the nearest char boundary at or before the 500-byte mark
    let end = (0..=end).rev().find(|&i| prompt.is_char_boundary(i)).unwrap_or(0);
    let preview = &prompt[..end];
    log.write_str(&format!(
        "\n--- prompt hash={hash} len={} preview:\n{preview}\n---\n",
        prompt.len()
    ));
}
```

The truncation uses `str::is_char_boundary()` to find the nearest valid UTF-8 boundary at or before 500 bytes, preventing panics on multi-byte characters. This is written inline to the `.log` file, not to a companion file — the `LogWriter` API does not support alternate file extensions, and a separate file adds complexity without debugging value.

### 4. Add `run_review_with_retry_logged` async function in `src/prd/quick.rs`

The original spec proposed calling `backend.execute_with_log()` directly inside `run_review_with_retry_sync`, bypassing the parse-retry logic. This is incorrect — it would change failure rates and error surfaces. Instead, add a new async function that mirrors `run_review_with_retry` but accepts and threads a `LogWriter`:

```rust
pub async fn run_review_with_retry_logged(
    backend: Arc<dyn Backend>,
    prompt: String,
    log_writer: &mut LogWriter,
) -> Result<ReviewFeedback> {
    let mut current_prompt = prompt;

    for attempt in 1..=3_u8 {
        let is_fallback = log_writer.attempt() > 0;
        log_writer.write_attempt_separator(backend.name(), is_fallback);
        write_prompt_summary(log_writer, &current_prompt);

        let raw = backend.execute_with_log(&current_prompt, Some(log_writer)).await?;
        match parse_review_feedback(&raw) {
            Ok(feedback) => {
                log_writer.write_str(&format!(
                    "\n--- review-parse: OK approved={} ---\n",
                    feedback.approved
                ));
                return Ok(feedback);
            }
            Err(parse_error) => {
                log_writer.write_str(&format!(
                    "\n--- review-parse: FAIL error={parse_error} ---\n"
                ));
                if attempt == 3 {
                    return Err(RalphError::QuickPrdFailed(format!(
                        "failed to parse review feedback after 3 attempts: {parse_error}"
                    )));
                }
                current_prompt = format!(
                    "CRITICAL: Your previous review response could not be parsed.\n\n\
                     Error: {parse_error}\n\n\
                     Return ONLY a single fenced JSON block with this exact schema:\n\
                     ```json\n\
                     {{\"approved\": true/false, \"issues\": [{{\"area\": \"...\", \"feedback\": \"...\"}}]}}\n\
                     ```\n\
                     Use valid JSON, no prose before or after the fenced block.\n\n\
                     Previous response:\n---\n{raw}\n---\n"
                );
            }
        }
    }

    unreachable!("loop should return or error before reaching this point")
}
```

This preserves the exact same retry logic (3 attempts, correction prompts, same error messages) while adding: (a) attempt separator before each call, (b) prompt summary before each call, (c) streaming output capture via `execute_with_log`, (d) review-parse result annotation after each call. The `is_fallback` flag correctly uses `log_writer.attempt() > 0`, so attempt 1 has `fallback=false` and attempts 2-3 have `fallback=true`, matching orchestrator parity.

The original `run_review_with_retry` remains unchanged for callers that do not need logging.

### 5. Modify `run_review_with_retry_sync` to thread `LogWriter`

**New signature:**

```rust
fn run_review_with_retry_sync(
    reviewer: &CliBackend,
    prompt: String,
    deadline: std::time::Instant,
    log_writer: &mut LogWriter,
) -> Result<ReviewFeedback>
```

**Body changes:** Replace the call to `run_review_with_retry(backend, prompt)` with `run_review_with_retry_logged(backend, prompt, log_writer)`. The `LogWriter` is passed by mutable reference into the async block. No other changes to error handling, timeout wrapping, or runtime creation.

Note: `LogWriter` is `!Send` but this is safe because `run_review_with_retry_sync` creates a `new_current_thread` runtime (not multi-threaded), so the `&mut LogWriter` never crosses thread boundaries. If the compiler requires it, the async block can be wrapped in a `LocalSet`.

### 6. Open `LogWriter` instances in the four entry-point functions

Each entry-point function needs `issue_number`, `owner`, and `repo` to derive the log path. `owner` and `repo` are already on `PrdPollConfig`. `issue_number` is available at each call site (lines 1061, 1203, 1369) but is not passed to the generation functions. Add `issue_number: u32` as a parameter to: `generate_questions_with_timeout`, `generate_draft_from_answers_with_timeout`, `generate_revision_from_feedback_with_timeout`.

**6a. `generate_questions_with_timeout`** — opens 3 LogWriters:

```rust
let log_dir = log_dir(&config.data_dir, &config.owner, &config.repo, issue_number);
let project_id = format!("issue-{issue_number}");

let mut log_a = LogWriter::open(&log_dir, &project_id, None, "questions-a");
// ... run_backend_sync(&backend_a, &prompt, deadline, Some(&mut log_a))

let mut log_b = LogWriter::open(&log_dir, &project_id, None, "questions-b");
// ... run_backend_sync(&backend_b, &prompt, deadline, Some(&mut log_b))

let mut log_synth = LogWriter::open(&log_dir, &project_id, None, "questions-synthesis");
// ... run_backend_sync(&backend_a, &synthesis_prompt, deadline, Some(&mut log_synth))
```

Produces files: `issue-42-questions-a.log`, `issue-42-questions-b.log`, `issue-42-questions-synthesis.log`.

**6b. `generate_draft_from_answers_with_timeout`** — opens 2 LogWriters:

```rust
let log_dir = log_dir(&config.data_dir, &config.owner, &config.repo, issue_number);
let project_id = format!("issue-{issue_number}");

let mut writer_log = LogWriter::open(&log_dir, &project_id, None, "draft-writer");
let mut reviewer_log = LogWriter::open(&log_dir, &project_id, None, "draft-reviewer");
```

- `writer_log` is passed to `run_draft_with_section_retry_sync` and to the revision `run_backend_sync` call inside the review loop. All writer attempts (initial + section retries + revision attempts) go to the same LogWriter, producing continuous attempt numbering with correct fallback flags.
- `reviewer_log` is passed to `run_review_with_retry_sync`. All reviewer parse-retry attempts go to the same LogWriter.

This requires `run_draft_with_section_retry_sync` to also accept `&mut LogWriter`:

```rust
fn run_draft_with_section_retry_sync(
    writer: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
    log_writer: &mut LogWriter,
) -> Result<String>
```

Inside the loop, each call to `run_backend_sync` passes `Some(log_writer)`, and after `check_spec_sections` returns, the validation result is annotated in the log.

**6c. `generate_revision_from_feedback_with_timeout`** — same structure as 6b:

```rust
let mut writer_log = LogWriter::open(&log_dir, &project_id, None, "revision-writer");
let mut reviewer_log = LogWriter::open(&log_dir, &project_id, None, "revision-reviewer");
```

**6d. Validation result per call type.** The "validation result" annotation varies by context:

| Call type | Validation annotation |
|---|---|
| `run_backend_sync` in question generation | No validation annotation (questions are not section-validated) |
| `run_backend_sync` via `run_draft_with_section_retry_sync` | `--- validation: PASS ---` or `--- validation: FAIL missing=[Summary, ...] ---` |
| `run_backend_sync` for revision in review loop | `--- validation: PASS ---` or `--- validation: FAIL missing=[...] ---` |
| `run_review_with_retry_logged` | `--- review-parse: OK approved=true ---` or `--- review-parse: FAIL error=<msg> ---` |

### 7. Parameter threading summary

| Function | New parameters | Source of values |
|---|---|---|
| `generate_questions_with_timeout` | `issue_number: u32` | Caller (`transition_pending_to_awaiting_answers`) has `issue.number` |
| `generate_draft_from_answers_with_timeout` | `issue_number: u32` | Caller (`transition_awaiting_answers_to_awaiting_feedback`) has `issue.number` |
| `generate_revision_from_feedback_with_timeout` | `issue_number: u32` | Caller (`transition_awaiting_feedback`) has `issue_number` local |
| `run_backend_sync` | `log_writer: Option<&mut LogWriter>` | Created by each entry-point function |
| `run_draft_with_section_retry_sync` | `log_writer: &mut LogWriter` | Passed from entry-point function |
| `run_review_with_retry_sync` | `log_writer: &mut LogWriter` | Passed from entry-point function |
| `run_review_with_retry_logged` (new) | `log_writer: &mut LogWriter` | Passed from `run_review_with_retry_sync` |

`owner` and `repo` are already available on `PrdPollConfig` (passed to all entry-point functions). `data_dir` is `config.data_dir`. No changes to `PrdPollConfig` or `InteractivePrdState` are needed.

## Files & Modules

| File | Changes |
|---|---|
| `src/daemon/interactive_prd.rs` | Add `log_dir()` helper. Modify `run_backend_sync` signature to accept `Option<&mut LogWriter>` and call `execute_with_log`. Modify `run_draft_with_section_retry_sync` to accept `&mut LogWriter`, pass it to `run_backend_sync`, and annotate validation results. Modify `run_review_with_retry_sync` to accept `&mut LogWriter` and call `run_review_with_retry_logged` instead of `run_review_with_retry`. Add `issue_number: u32` parameter to `generate_questions_with_timeout`, `generate_draft_from_answers_with_timeout`, `generate_revision_from_feedback_with_timeout`. In each of those functions, derive log directory, open `LogWriter` instances per phase, and pass them through the call chain. Add `write_prompt_summary` helper with char-boundary-safe truncation. Add validation result annotations after `check_spec_sections` calls in the draft/revision flow. Update the 3 call sites in transition functions to pass `issue_number`. |
| `src/prd/quick.rs` | Add `run_review_with_retry_logged` async function that mirrors `run_review_with_retry` but accepts `&mut LogWriter`, writes attempt separators, prompt summaries, streams output via `execute_with_log`, and annotates parse results. Move or re-export `write_prompt_summary` here if needed for the logged review function. Original `run_review_with_retry` remains unchanged. |
| `src/output_log.rs` | No changes — existing `LogWriter::open`, `write_attempt_separator`, `write_str`, `write_bytes`, `attempt()`, and `format_attempt_separator` are sufficient. |
| `src/backend/mod.rs` | No changes — `execute_with_log` already exists on both the trait and `CliBackend`. |

## Testing Strategy

1. **Unit test: `run_backend_sync` writes to log via `execute_with_log`** — Create a `CliBackend` with a mock command (e.g., `echo "hello"`), call `run_backend_sync` with a `LogWriter` opened to a `tempdir()`, assert the log file contains: (a) an attempt separator line matching `--- attempt=1 backend=... fallback=false ts=... ---`, (b) the raw output `hello`, (c) the prompt summary with correct SHA-256 hash and preview text. Verify the file path is `<tempdir>/issue-1-test.log` using a deterministic `project_id` and `role`.

2. **Unit test: `run_backend_sync` with `None` log writer produces no files** — Call `run_backend_sync` with `log_writer: None`, assert the temp directory remains empty. This verifies backward compatibility.

3. **Unit test: prompt summary is char-boundary-safe** — Call `write_prompt_summary` with a prompt containing multi-byte UTF-8 characters (e.g., emoji at byte position 499), assert no panic and the preview ends at a valid char boundary.

4. **Unit test: `run_draft_with_section_retry_sync` logs all retry attempts with correct fallback flag** — Create a `CliBackend` that returns an incomplete spec (3 of 6 sections) on the first `DRAFT_SECTION_RETRIES` calls and a complete spec on the final call. Assert the log file contains multiple attempt separators with `fallback=false` for the first and `fallback=true` for subsequent attempts, each followed by the raw output. Assert the log contains `--- validation: FAIL missing=[...] ---` for failed attempts and `--- validation: PASS ---` for the final successful attempt.

5. **Unit test: `run_review_with_retry_logged` logs all parse-retry attempts** — Create a mock backend (implementing `Backend` trait) that returns unparseable text on attempt 1, then valid JSON on attempt 2. Assert the log file contains: (a) two attempt separators with correct fallback flags, (b) prompt summary for each attempt (including the correction prompt on attempt 2), (c) raw output for both attempts, (d) `--- review-parse: FAIL error=... ---` after attempt 1 and `--- review-parse: OK approved=... ---` after attempt 2.

6. **Unit test: logs persist on backend timeout/error** — Call `run_backend_sync` with a backend that times out (e.g., `sleep 999` with a 1-second deadline). Assert the log file still contains the attempt separator and prompt summary (written before the backend call), plus the timeout footer written by `execute_with_log`.

7. **Unit test: `generate_questions_with_timeout` creates 3 log files** — Call with mock backends, assert exactly 3 log files are created in `<tempdir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>/logs/` named `issue-<N>-questions-a.log`, `issue-<N>-questions-b.log`, `issue-<N>-questions-synthesis.log`. Verify each contains at least one attempt separator and raw output.

8. **Unit test: `generate_draft_from_answers_with_timeout` creates writer and reviewer logs** — Call with mock backends (writer returns complete spec, reviewer returns `{"approved": true, "issues": []}`). Assert 2 log files: `issue-<N>-draft-writer.log` and `issue-<N>-draft-reviewer.log`. Verify `draft-writer.log` contains validation annotation and `draft-reviewer.log` contains review-parse annotation.

9. **Unit test: `generate_revision_from_feedback_with_timeout` log path is correct** — Call with mock backends, assert log files are written to `<data_dir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>/logs/` with `revision-writer` and `revision-reviewer` roles.

10. **Unit test: reviewer parse-retry with 3 failures logs all 3 attempts** — Create a mock backend that always returns unparseable text. Call `run_review_with_retry_logged`, expect an error. Assert the log file contains 3 attempt separators, 3 prompt summaries (original + 2 correction prompts), 3 raw outputs, and 3 `review-parse: FAIL` annotations.

11. **Existing tests remain green** — Run `cargo test` to verify no regressions in validation logic, state transitions, error handling, or existing mock-backend test infrastructure. The 3 call sites in transition functions are updated to pass `issue_number` which is already available as a local variable.

## Out of Scope

- **Log rotation or cleanup** — No automatic deletion of old log files; this can be addressed separately.
- **Structured/JSON log format** — Logs use the existing plaintext separator format for consistency with orchestrator logs.
- **Async streaming to log in `run_backend_sync`** — `run_backend_sync` creates a single-threaded tokio runtime; the `execute_with_log` integration handles streaming within that runtime. No restructuring of the runtime is needed.
- **Companion `.prompt` files** — Prompt summaries are written inline to the `.log` file. The `LogWriter` API produces `.log` filenames only, and inline logging is sufficient for debugging without adding file management complexity.
- **Modifying `run_review_with_retry` async internals** — The original function is preserved unchanged. A new `run_review_with_retry_logged` function is added alongside it, duplicating the retry logic with logging integrated. This avoids breaking existing callers (e.g., `quick.rs` workflows).
- **Changing the state file schema** — No new fields on `InteractivePrdState`.
- **Logging prompts in full** — Only a SHA-256 hash + char-boundary-safe 500-byte preview is logged to avoid disk bloat.
- **Changes to `execute_with_log` trait or `LogWriter` API** — The existing API is sufficient; no modifications needed.
- **Logging in `poll_and_advance_prd` or transition functions** — Logging is scoped to the backend execution layer (generation/review functions), not the state machine layer.