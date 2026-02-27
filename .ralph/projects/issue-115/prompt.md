## Summary
Add always-on debug logging for the interactive PRD workflow so every backend call preserves raw output for diagnosis, including malformed responses that later fail validation/parsing.

## Goal
Ensure interactive PRD failures are diagnosable from log files alone by recording prompt summaries, backend identity, raw output (when available), and outcome markers for execution, validation, and review parsing.

## Scope
- `src/daemon/interactive_prd.rs`
- `src/prd/quick.rs`
- `src/validate/tests_interactive_prd.rs`
- `src/validate/mod.rs` (register new tests)
- No behavior changes required in `src/output_log.rs` unless a compile/runtime gap is discovered.

## Required Behavior

1. Capture all interactive PRD backend calls
- Every `run_backend_sync()` call in interactive PRD must write a log entry before validation/parsing.
- Every reviewer backend attempt inside `prd::quick::run_review_with_retry()` must write a log entry for each retry attempt (up to current retry limit).

2. Log location and file naming
- Logs must be stored at:
  `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/logs/`
- Filenames must be:
  `issue-{number}-{role}.log`
- Roles:
  `questions-a`, `questions-b`, `synthesis`, `writer`, `reviewer`
- This path is canonical and must be used consistently.

3. Entry structure per attempt
- Use `LogWriter::write_attempt_separator(...)` for timestamped attempt boundaries.
- Immediately after the separator, write:
  `backend_spec=<unsanitized spec string>`
- Write prompt summary:
  `prompt (<byte_len> bytes): <first 500 chars, UTF-8 safe>`
- Record raw backend output if available.
- Record outcome lines as applicable:
  - `--- execution: ok ---`
  - `--- execution: timeout ---`
  - `--- execution: error <message> ---`
  - `--- validation: pass ---`
  - `--- validation: fail missing=[section1, section2] ---`
  - `--- validation: n/a ---`
  - `--- parse: ok ---`
  - `--- parse: fail <message> ---`

4. Prompt truncation rule
- Truncate by character count (500 chars), not bytes.
- Truncation must be UTF-8 safe (no invalid slicing/panic).
- Log total byte length of the original prompt.

5. Validation/parsing semantics
- Calls that run `check_spec_sections()` must always log validation pass/fail and missing sections.
- Calls without section validation (question generation, synthesis, reviewer parsing attempts) must log `--- validation: n/a ---`.
- Reviewer parse attempts must log parse status for each attempt.

6. Best-effort logging
- Logging must never fail the PRD workflow.
- Any file-open/write errors must be swallowed per current `LogWriter` semantics (warn-only behavior).

7. Backward compatibility
- `ralph quick-prd` path must remain functional without requiring logging setup.
- Add optional logging parameters so existing call sites can pass `None`.

## Implementation Requirements

1. `interactive_prd.rs`
- Add helper:
  `fn prd_log_dir(data_dir: &Path, owner: &str, repo: &str) -> PathBuf`
- Add UTF-8-safe prompt truncation helper for 500-char preview.
- Add a log context struct to avoid parameter explosion, containing at minimum:
  `log_dir`, `issue_number`, `writer_backend_spec`, `reviewer_backend_spec`.
- Thread optional `LogWriter` through:
  `run_backend_sync(...)`
  `run_draft_with_section_retry_sync(...)`
  `run_review_with_retry_sync(...)`
- For `run_backend_sync`, avoid duplicate raw-output writes:
  if streaming logging is used, do not append the same full output again.
- Open and use role-specific log writers in:
  `generate_questions_with_timeout(...)`
  `generate_draft_from_answers_with_timeout(...)`
  `generate_revision_from_feedback_with_timeout(...)`

2. `prd/quick.rs`
- Update `run_review_with_retry(...)` to accept optional logging input and raw reviewer backend spec.
- Inside retry loop, log each attempt’s separator, backend spec, prompt summary, raw output (if any), and parse result.
- Preserve existing behavior when logging is `None`.

## Acceptance Criteria
- Every interactive PRD `run_backend_sync()` call writes an attempt entry before validation.
- Every reviewer retry attempt logs raw output (or execution failure marker) and parse status.
- Raw output remains logged even when section validation later fails.
- Each entry includes timestamped separator, backend name label from separator, explicit unsanitized `backend_spec=...`, prompt summary, and outcome markers.
- Timeout/error paths log explicit execution markers with error message when available.
- Validation markers are present and correct (`pass`, `fail missing=[...]`, `n/a`).
- Logs are created under canonical path with `issue-{N}-{role}.log` naming.
- Logging failures do not fail workflow execution.
- `quick-prd` callers remain compatible by passing `None`.

## Test Requirements
Add conformance coverage in `src/validate/tests_interactive_prd.rs` and register in `src/validate/mod.rs`:

1. Path and naming
- Verifies logs are created at the canonical directory and filename pattern.

2. Writer validation-failure persistence
- Mock writer returns malformed draft; assert raw output + `validation: fail` are logged even when overall step errors.

3. Reviewer retry logging
- Mock reviewer fails parse on first attempt and succeeds/fails later; assert one log entry per attempt with raw output and parse status.

4. Question/synthesis logging
- Assert `validation: n/a` entries for question-generation/synthesis roles.

5. Timeout/error logging
- Assert `execution: timeout` and `execution: error <message>` markers when backend fails without usable output.

6. UTF-8 prompt truncation safety
- Validate 500-char truncation behavior with multibyte input and correct byte-length reporting.

7. Backward compatibility
- Verify `quick-prd` path still works when review logging parameter is `None`.

## Out of Scope
- Log rotation/retention policies.
- JSON/structured log format.
- Surfacing log paths in issue comments or persisted state.
- Separate stderr stream artifacts beyond existing error-message handling.