### Summary
Add debug logging to the interactive PRD workflow so every backend invocation persists raw streamed output plus lightweight metadata to:

`<data_dir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>/logs/`

Use existing `LogWriter` and `execute_with_log` APIs unchanged. Preserve existing workflow behavior (retries, timeouts, parsing/validation logic, errors, and `last_error` semantics).

### In Scope
1. `src/daemon/interactive_prd.rs`
2. `src/prd/quick.rs`
3. Unit tests for touched behavior

### Out of Scope
- Changes to `src/output_log.rs` or `src/backend/mod.rs`
- State schema changes
- Log rotation/cleanup
- Structured JSON logs
- Full prompt logging (only hash + preview)

### Required Implementation

#### 1) Canonical log directory
Add helper in `interactive_prd.rs`:

```rust
fn log_dir(data_dir: &Path, owner: &str, repo: &str, issue_number: u32) -> PathBuf
```

Path must resolve exactly to:
`<data_dir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>/logs/`

#### 2) Prompt summary helper (single shared behavior)
Define one helper and reuse it consistently for all logged backend calls:

```rust
fn write_prompt_summary(log: &mut LogWriter, prompt: &str)
```

Behavior:
- Compute SHA-256 of full prompt bytes (hex).
- Compute byte length via `prompt.as_bytes().len()`.
- Preview is first 500 Unicode characters (`prompt.chars().take(500)`), not bytes.
- Write inline to the `.log` file before backend execution with a stable marker block.

#### 3) `run_backend_sync` logging support
Change signature in `interactive_prd.rs` to:

```rust
fn run_backend_sync(
    backend: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
    log_writer: Option<&mut LogWriter>,
) -> Result<String>
```

Behavior:
- If `log_writer` is `Some`, before execute:
1. Write attempt separator with `is_fallback = log_writer.attempt() > 0`.
2. Write prompt summary.
- Replace `backend.execute(prompt)` with `backend.execute_with_log(prompt, log_writer)`.
- `None` path must preserve previous behavior exactly.

#### 4) `run_draft_with_section_retry_sync` logging
Change signature to accept `&mut LogWriter`.
- Route every backend attempt through `run_backend_sync(..., Some(log_writer))`.
- After each `check_spec_sections` call, append exactly one validation line:
- PASS: `--- validation: PASS ---`
- FAIL: `--- validation: FAIL missing=[...] ---`
- Keep retry counts/logic/error behavior unchanged.

#### 5) Add logged review retry function in `quick.rs`
Add:

```rust
pub async fn run_review_with_retry_logged(
    backend: Arc<dyn Backend>,
    prompt: String,
    log_writer: &mut LogWriter,
) -> Result<ReviewFeedback>
```

Requirements:
- Mirror `run_review_with_retry` logic exactly (3 attempts, same correction-prompt content, same terminal parse-failure error text).
- For each attempt:
1. Write separator (`is_fallback = log_writer.attempt() > 0`).
2. Write prompt summary.
3. Execute with `backend.execute_with_log(&current_prompt, Some(log_writer)).await`.
4. Append parse annotation line:
- OK: `--- review-parse: OK approved=<bool> ---`
- FAIL: `--- review-parse: FAIL error=<msg> ---`
- Do not modify existing `run_review_with_retry`; keep it for existing callers.

#### 6) `run_review_with_retry_sync`
Change signature in `interactive_prd.rs` to accept `&mut LogWriter` and call `run_review_with_retry_logged`.
- Preserve timeout/runtime behavior.
- If needed for `!Send`, use `LocalSet` with current-thread runtime only (no multi-thread runtime change).

#### 7) Thread `issue_number` and open log writers at entry points
Add `issue_number: u32` parameter to:
1. `generate_questions_with_timeout`
2. `generate_draft_from_answers_with_timeout`
3. `generate_revision_from_feedback_with_timeout`

Update all call sites accordingly.

Open `LogWriter`s per phase using:
- `project_id = format!("issue-{issue_number}")`
- `log_dir(...)` from step 1

Roles/files required:
- Questions flow: `questions-a`, `questions-b`, `questions-synthesis`
- Draft flow: `draft-writer`, `draft-reviewer`
- Revision flow: `revision-writer`, `revision-reviewer`

### Acceptance Criteria
- Every backend call in interactive PRD generation/review paths writes raw streamed output via `execute_with_log` when logging is enabled.
- Logs are written only under `<data_dir>/<owner>/<repo>/.ralph/interactive-prd/<issue_number>/logs/`.
- Each logged backend attempt includes separator + prompt summary before execution.
- Fallback flag uses `log_writer.attempt() > 0` everywhere.
- Draft/revision section checks append `validation` PASS/FAIL lines.
- Review parse handling appends `review-parse` OK/FAIL lines for every attempt, including correction retries.
- Logs exist even when backend execution, section validation, or review parsing fails.
- Existing behavior (timeouts, retries, parse logic, error text, `last_error`) is unchanged.
- No API changes to `LogWriter` or backend traits.

### Tests
Add focused tests (deterministic mocks only):
1. `run_backend_sync` with `Some(log_writer)` writes separator, prompt summary, and raw output.
2. `run_backend_sync` with `None` creates no log file.
3. `write_prompt_summary` handles multibyte UTF-8 safely and limits preview to 500 chars.
4. `run_draft_with_section_retry_sync` logs multiple attempts with correct fallback progression and validation PASS/FAIL lines.
5. `run_review_with_retry_logged` logs parse-fail then parse-success across retries.
6. `run_review_with_retry_logged` logs all 3 failed parse attempts and returns the same terminal error text as existing behavior.
7. Entry-point tests verify expected role-based log files are created in canonical directory for questions/draft/revision flows.

Run `cargo test` and ensure existing tests remain green.