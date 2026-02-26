---
artifact: prompt-review
project: task-implement-debug-logging-for-interac
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-26T17:11:17Z
---

# Prompt Review

## Issues Found
- The scope is internally inconsistent: it says interactive PRD only, but also requires `src/prd/quick.rs` changes without explicitly defining compatibility expectations for Quick PRD callers.
- Log path is specified in two ways (`{data_dir}/{owner}/{repo}/...` and `config.repo_clone_path()/...`) with no single source of truth, which can cause logs to land in different places.
- Filename semantics are ambiguous (`phase` + `role` vs examples), so implementations may diverge unless a strict mapping is defined.
- `fallback` flag meaning is unclear in this workflow (retry vs backend fallback), which can produce misleading separators.
- Prompt hash behavior is underspecified (exact bytes, encoding, hex casing), making hashes non-comparable across implementations.
- Validation footer formatting is not fully deterministic (spacing/order), which makes tests brittle.
- The proposed async snippets risk Rust borrow issues with `Option<&mut LogWriter>` across `.await`; this is not called out as an implementation constraint.
- Signature changes have a large blast radius, but backward-compatibility requirements for existing callers/tests are not explicit enough.
- Best-effort logging is stated but not operationalized (what happens after open/write failures), so runtime behavior can vary.
- Validate/conformance coverage is suggested but not fully tied to repo conventions (new validate module + registration), weakening long-term testability.

## Refined Prompt
### Goal
Implement durable debug logging for the **interactive PRD workflow** so raw backend outputs are preserved for postmortem debugging, including malformed outputs currently lost on validation/parse failures.

### Scope
- Primary: `src/daemon/interactive_prd.rs`
- Shared logging helpers: `src/output_log.rs`
- Reviewer retry helper update: `src/prd/quick.rs` (API-compatible; Quick PRD behavior unchanged unless a log writer is passed)

### Non-Goals
- No verbatim prompt logging
- No log rotation/cleanup
- No JSON/structured log migration
- No wiring Quick PRD pipeline to create logs (it should pass `None`)

### Required Behavior
1. Log every interactive PRD backend call:
- Every `run_backend_sync()` call in questions/draft/revision phases must persist raw backend output.
- Logging must occur whether section validation passes or fails.

2. Log every reviewer retry attempt:
- Every internal attempt inside `run_review_with_retry()` must persist raw reviewer output, including parse-failure retries.

3. Log directory and filenames:
- Single source of truth for directory: `config.repo_clone_path().join(".ralph").join("tmp").join("logs")`
- Required filenames:
  - `prd-{issue}-questions-a.log`
  - `prd-{issue}-questions-b.log`
  - `prd-{issue}-questions-synth.log`
  - `prd-{issue}-draft-writer.log`
  - `prd-{issue}-draft-reviewer.log`
  - `prd-{issue}-revision-writer.log`
  - `prd-{issue}-revision-reviewer.log`
- Files must append across retries/revision loops within the same invocation.

4. Attempt separator fields:
- Include: attempt number, sanitized backend label, fallback flag, RFC 3339 timestamp, and `prompt_sha256`.
- `prompt_sha256` = SHA-256 of the exact UTF-8 bytes of the prompt sent, lowercase hex.
- Preserve backward compatibility for existing callers that do not pass a prompt hash.

5. Validation footer (writer outputs only):
- After every `check_spec_sections` call in draft/revision paths, append exactly:
  - `--- validation=pass missing=[] ---`
  - `--- validation=fail missing=[section1,section2] ---`
- Deterministic format: comma-separated names, no spaces.

6. Timeout footer:
- Any timeout in sync wrappers must append timeout footer via `write_timeout_footer`.

7. Best-effort semantics:
- Logging I/O/open failures must never change PRD workflow outcomes.
- Workflow must continue even if logging becomes unavailable.

8. Thread issue number:
- Add/pass `issue_number: u32` through interactive generation functions for log naming/project-id derivation.

### Implementation Requirements
- Extend `LogWriter` in a backward-compatible way:
  - Add prompt-hash-aware attempt separator support (or add a new helper while preserving old behavior).
  - Add `write_attempt_separator_with_prompt(...)` to compute hash internally.
  - Add `format_validation_footer(...)` and `write_validation_footer(...)`.
- Use `execute_with_log(...)` where a writer is available.
- Keep async borrow handling Rust-safe (avoid long-lived mutable borrows across `.await`).
- Keep Quick PRD callers compatible by passing `None`.

### File-Level Changes
- `src/daemon/interactive_prd.rs`
  - Thread `issue_number`
  - Create per-phase/per-role `LogWriter`s
  - Pass writers through backend/reviewer paths
  - Write validation and timeout footers
- `src/output_log.rs`
  - Add prompt-hash separator support
  - Add validation footer formatter/writer
- `src/prd/quick.rs`
  - Update `run_review_with_retry` to accept optional writer and log each attempt via `execute_with_log`
  - Preserve behavior for `None`
- `Cargo.toml`
  - Add `sha2` only if missing

### Required Tests
1. Unit tests (`output_log.rs`)
- Separator includes hash when provided
- Legacy separator unchanged when hash omitted
- Validation footer pass/fail exact strings
- Prompt hash helper writes expected SHA-256 for fixed input

2. Unit tests (`interactive_prd.rs`)
- Success path logs separator + raw output
- Section-failure retries log raw output + fail footer each attempt
- Timeout path writes timeout footer
- Pass footer written on successful section validation
- Expected files created for fixed issue number
- Logging I/O failure does not fail workflow

3. Unit tests (`prd/quick.rs`)
- Parse-failure retries log all attempts
- Final failure after max retries still logs all raw attempts
- `None` writer preserves existing behavior

4. Validate conformance test
- Add `src/validate/tests_interactive_prd_logging.rs`
- Export `pub fn tests() -> Vec<ConformanceTest>`
- Register module in `src/validate/mod.rs`
- End-to-end assertion: files exist at expected paths and contain separator fields, prompt hash, raw output, and validation/timeout footers as applicable

### Verification Commands
- `nix develop -c cargo check`
- `nix develop -c cargo test`
- `nix build -L`
- `./result/bin/ralph validate --bin ./result/bin/ralph`
