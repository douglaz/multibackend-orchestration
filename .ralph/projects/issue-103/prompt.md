### Summary
Add persistent, structured debug logging for Interactive PRD backend calls so malformed or failed backend outputs are always recoverable from disk. Logging must be best-effort and must not change existing workflow behavior or state file layout.

### Goal
Capture every Interactive PRD backend attempt (including each retry attempt inside review retry logic) with enough data to debug failures in production.

### In Scope
- `src/daemon/interactive_prd.rs`
- `src/prd/quick.rs` (retry-attempt callback support)
- New validate tests for this feature and registration in `src/validate/mod.rs`

### Out of Scope
- `ralph auto` / `ralph run` artifact changes
- Log rotation/cleanup
- Tracing framework migration
- State schema/path changes

### Required Behavior

### 1) Log location and state invariants
- Keep state file unchanged at: `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json`
- Write logs under: `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}/logs/`
- Create directories on demand with best-effort semantics.

### 2) One JSON file per backend attempt
- Every backend attempt in Interactive PRD writes exactly one log file, including failures.
- For review retries, each retry attempt writes its own file before retry/discard logic executes.

### 3) Log filename format and collision handling
- Filename format: `{timestamp}-{label}.json`
- Timestamp format for filename: `YYYYMMDDTHHMMSSZ` (UTC)
- Use create-new semantics and resolve collisions by appending `-NNN` (`001`..`9999`) before label.
- If still failing, emit `eprintln!` and continue workflow (no panic, no error propagation).

### 4) Log entry schema (stable and testable)
Use a structured schema (no free-form validation strings):

```rust
#[derive(Serialize)]
struct PrdDebugLogEntry {
    timestamp: String,          // RFC3339 UTC
    backend_spec: String,       // e.g. "claude(opus)"
    label: String,              // e.g. "question-gen-a", "review-attempt-2-of-3"
    prompt_chars: usize,        // original prompt char count
    prompt: String,             // full or truncated prompt
    raw_output: Option<String>, // Some on backend success, None on transport/runtime error
    error: Option<String>,      // Some on backend transport/runtime error, None otherwise
    validation: ValidationResult,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ValidationResult {
    NotChecked,
    Ok,
    MissingSections { missing: Vec<String> },
    ReviewParseFailed { error: String },
}
```

### 5) Prompt truncation
- Honor `RALPH_PRD_LOG_TRUNCATE` as optional max bytes.
- Default: unlimited.
- Truncate safely at UTF-8 boundary; append marker: `... [truncated at N bytes, full length: M bytes]`.
- `prompt_chars` must always reflect original untruncated character count.

### 6) Instrumentation points and labels
Instrument all Interactive PRD backend calls with deterministic labels:

- `question-gen-a`
- `question-gen-b`
- `synthesis`
- `draft-attempt-{N}`
- `draft-review-attempt-{N}-of-3`
- `draft-revision-{N}`
- `feedback-draft-attempt-{N}`
- `feedback-review-attempt-{N}-of-3`
- `feedback-revision-{N}`

If function names differ in current code, instrument the equivalent call paths:
- question generation path
- draft generation + section-retry path
- review path
- feedback revision path

### 7) Review retry per-attempt hook in `src/prd/quick.rs`
Add optional per-attempt callback to `run_review_with_retry` so each attempt is observable before retry logic mutates prompt/state.

Preferred shape:

```rust
pub struct ReviewAttemptEvent {
    pub attempt: u8,
    pub prompt: String,
    pub raw_output: String,
    pub parse_error: Option<String>, // None if parse succeeded
}

pub async fn run_review_with_retry(
    backend: Arc<dyn Backend>,
    prompt: String,
    on_attempt: Option<&mut dyn FnMut(ReviewAttemptEvent)>,
) -> Result<ReviewFeedback>
```

Requirements:
- Callback fires on every attempt immediately after parse attempt, before retry/discard handling.
- Existing callers remain behaviorally unchanged by passing `None`.

### 8) Error handling and workflow safety
- Logging is best-effort only.
- Any logging failure must only emit `eprintln!` and continue.
- No panics introduced.
- No behavior change to decision logic, retry counts, or transitions.

### File Changes Required
- `src/daemon/interactive_prd.rs`: add log structs/helpers, thread logging context through backend-calling functions, emit logs at all required call sites.
- `src/prd/quick.rs`: add optional attempt callback and invoke per attempt.
- `src/validate/tests_interactive_prd_logging.rs`: add new conformance tests.
- `src/validate/mod.rs`: register new validate test module.

### Acceptance Criteria
- All Interactive PRD backend attempts produce log files with required fields.
- Every review retry attempt (up to 3) is logged individually before retry/discard.
- Malformed review output and missing-section draft outputs are captured in `raw_output` plus structured `validation`.
- Backend transport/runtime errors produce entries with `raw_output = None` and populated `error`.
- Log path matches required directory; state file path/content contract is unchanged.
- Logging failures do not block workflow.
- Existing non-interactive-prd behavior remains unchanged.

### Testing Requirements

### Validate tests
Add conformance tests covering:
- Log file creation and schema validity.
- Collision handling (same-second same-label creates distinct files).
- Prompt truncation behavior and metadata correctness.
- Per-attempt callback capture for malformed review output (all attempts logged).
- Question-generation path emits 3 expected labels.
- Draft + review path emits expected draft/review labels.
- No state-file path/schema regression.

### Additional tests
- Unit tests for filename generation/collision helper.
- Unit tests for UTF-8-safe truncation helper.

### Verification commands
- `nix develop -c cargo check`
- `nix develop -c cargo test`
- `nix build -L`
- `./result/bin/ralph validate --bin ./result/bin/ralph`