Now I have all the context needed. Let me produce the revised specification addressing all 7 review issues.

---

## Summary

Add configurable exponential backoff retry logic to `Backend::execute()` so that transient failures (timeouts, rate limits, temporary API errors) are automatically retried before surfacing an error. Today, retry logic lives in `execute_with_timeout_retries()` in the orchestrator (hardcoded to 3 total attempts, timeout-only, no jitter). This feature introduces a `RetryPolicy` struct and an `execute_with_retries()` free function in a new `src/backend/retry.rs` module that replaces the orchestrator's hardcoded function. The new mechanism adds jitter to prevent thundering-herd problems, classifies errors as retryable vs. non-retryable, and caps backoff delay to keep runtime bounded. Retry policy is passed explicitly at call sites (the orchestrator), not stored on backend structs — backends remain stateless with respect to retry behavior.

## Acceptance Criteria

1. A `RetryPolicy` struct exists with `max_retries: u32` (default 2) and `base_delay: Duration` (default 1s), implementing `Default`. The field `max_retries` means the number of **retries after the initial attempt**; total attempts = `max_retries + 1`. With the default of 2, there are 3 total attempts — matching today's behavior exactly.
2. `Backend::execute()` calls are wrapped by `execute_with_retries()` so that `BackendTimeout` errors are always retried. `BackendCommandFailed` is retried **only for `CliBackend`** (not tmux) when the `details` field contains transient indicators (case-insensitive): `"connection refused"`, `"rate limit"`, `"429"`, `"503"`, `"502"`, `"timed out"`, `"temporarily unavailable"`. Tmux `BackendCommandFailed` errors (which contain exit codes, not HTTP status text) are never retried.
3. Backoff delay is `min(base_delay * 2^attempt, max_delay)` with ±25% uniform random jitter, where `max_delay` defaults to 30 seconds. Arithmetic uses saturating multiplication to prevent overflow.
4. Each retry emits a `tracing::warn!` log with backend name, attempt number (1-indexed), total max attempts, computed delay in ms, and error summary.
5. When retries are exhausted, the final error is returned wrapped in `BackendRetriesExhausted { backend, phase, attempts, last_error }`, where `attempts` is the total number of attempts made.
6. The existing `execute_with_timeout_retries()` in `orchestrator.rs` is replaced by calls to `execute_with_retries()` — no duplicate retry layers.
7. `RetryPolicy` is passed explicitly at call sites in the orchestrator (via `RetryPolicy::default()`). It is **not** stored on `CliBackend`, `TmuxBackend`, or the `Backend` trait. Backends remain unaware of retry behavior.
8. `execute_with_retries()` accepts a `JitterStrategy` parameter (enum: `Uniform`, `None`) to support deterministic testing without jitter.
9. The `rand` crate is added to `Cargo.toml` for jitter computation.

## Technical Approach

### 1. Add `rand` dependency

Add `rand = "0.8"` to `Cargo.toml` `[dependencies]`.

### 2. Define `RetryPolicy` and `JitterStrategy` (`src/backend/retry.rs`, new file)

```rust
use std::time::Duration;
use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitterStrategy {
    /// ±25% uniform random jitter (production default)
    Uniform,
    /// No jitter — deterministic delays for testing
    None,
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub jitter: JitterStrategy,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,           // 2 retries = 3 total attempts
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            jitter: JitterStrategy::Uniform,
        }
    }
}

impl RetryPolicy {
    /// Compute the delay before the given retry attempt (0-indexed).
    /// Uses saturating arithmetic and caps at `max_delay`.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base_ms = self.base_delay.as_millis() as u64;
        let multiplier = 1_u64.checked_shl(attempt).unwrap_or(u64::MAX);
        let uncapped_ms = base_ms.saturating_mul(multiplier);
        let max_ms = self.max_delay.as_millis() as u64;
        let capped_ms = uncapped_ms.min(max_ms);

        match self.jitter {
            JitterStrategy::None => Duration::from_millis(capped_ms),
            JitterStrategy::Uniform => {
                let jitter_range = capped_ms / 4; // ±25%
                if jitter_range == 0 {
                    return Duration::from_millis(capped_ms);
                }
                let jitter = rand::thread_rng().gen_range(0..=jitter_range * 2);
                Duration::from_millis(capped_ms - jitter_range + jitter)
            }
        }
    }

    /// Return a copy with jitter disabled (for testing).
    pub fn without_jitter(mut self) -> Self {
        self.jitter = JitterStrategy::None;
        self
    }
}
```

**Addressing review issue #1 (architecture/ownership):** `RetryPolicy` is a plain data struct, not stored on backends. It is passed explicitly to `execute_with_retries()` at each call site in the orchestrator. Backends remain stateless with respect to retries. There is no retry-policy accessor on the `Backend` trait.

**Addressing review issue #6 (overflow/cap):** `delay_for_attempt` uses `checked_shl` + `saturating_mul` to prevent overflow and caps at `max_delay` (default 30s).

### 3. Add error classification (`src/backend/retry.rs`)

```rust
use crate::error::RalphError;

pub fn is_retryable(err: &RalphError) -> bool {
    match err {
        RalphError::BackendTimeout { .. } => true,
        RalphError::BackendCommandFailed { details, .. } => {
            // Only retry command failures that contain HTTP/network
            // transient indicators. Tmux errors contain exit codes and
            // tmux-specific messages (e.g. "tmux command exited with
            // code 1"), which will not match these patterns — so tmux
            // failures are effectively never retried here.
            let lower = details.to_lowercase();
            lower.contains("connection refused")
                || lower.contains("rate limit")
                || lower.contains("429")
                || lower.contains("503")
                || lower.contains("502")
                || lower.contains("timed out")
                || lower.contains("temporarily unavailable")
        }
        _ => false,
    }
}
```

**Addressing review issue #5 (tmux feasibility):** The substring-based classification is scoped to the `details` field content. Tmux `BackendCommandFailed` errors produce messages like `"tmux command exited with code 1 (command='claude')"` or `"tmux window '...' disappeared..."` — none of which contain the transient indicator substrings. CliBackend errors include stderr output, which for HTTP-based tools (claude CLI, codex CLI) will contain phrases like "429", "rate limit", or "connection refused". This means retry classification works correctly for CliBackend without any tmux stderr capture changes.

### 4. Add `BackendRetriesExhausted` error variant (`src/error.rs`)

```rust
#[error("backend retries exhausted for {backend} during {phase} after {attempts} attempts: {last_error}")]
BackendRetriesExhausted {
    backend: String,
    phase: String,
    attempts: u32,
    last_error: String,
},
```

The `phase` field is preserved from the existing `BackendTimeoutExhausted` variant to maintain parity for error messages and caller context.

### 5. Implement `execute_with_retries()` (`src/backend/retry.rs`)

```rust
pub async fn execute_with_retries(
    backend: &dyn Backend,
    role: &str,
    phase: &str,
    prompt: &str,
    policy: &RetryPolicy,
) -> Result<String> {
    let total_attempts = policy.max_retries + 1;

    for attempt in 0..total_attempts {
        match backend.execute(prompt).await {
            Ok(output) => return Ok(output),
            Err(e) => {
                let is_last = attempt + 1 >= total_attempts;

                if !is_retryable(&e) || is_last {
                    if is_last && is_retryable(&e) {
                        // Exhausted all retries on a retryable error
                        tracing::warn!(
                            role = role,
                            backend = backend.name(),
                            "backend retries exhausted"
                        );
                        return Err(RalphError::BackendRetriesExhausted {
                            backend: backend.name().to_owned(),
                            phase: phase.to_owned(),
                            attempts: total_attempts,
                            last_error: e.to_string(),
                        });
                    }
                    // Non-retryable error — propagate immediately
                    return Err(e);
                }

                let delay = policy.delay_for_attempt(attempt);
                tracing::warn!(
                    role = role,
                    backend = backend.name(),
                    attempt = attempt + 1,
                    max_attempts = total_attempts,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "retrying backend execution"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }

    // Unreachable: the loop always returns within total_attempts iterations.
    // But for safety, match the existing pattern in the codebase.
    Err(RalphError::Orchestration(
        "unexpected retry control-flow error".to_owned(),
    ))
}
```

**Addressing review issue #2 (attempt semantics):** `max_retries: 2` means 2 retries after the first attempt = 3 total attempts. The loop runs `0..total_attempts` where `total_attempts = max_retries + 1`. With default `max_retries: 2`, the loop runs attempts 0, 1, 2 (3 total), matching the existing `1..=3` behavior exactly.

**Addressing review issue #3 (control-flow correctness):** The original pseudocode had a bug where the last retryable failure took the `Err(e)` arm (because `attempt == policy.max_retries` failed the `attempt < policy.max_retries` guard) and returned the raw error instead of wrapping it. The revised code explicitly checks `is_last && is_retryable(&e)` and returns `BackendRetriesExhausted` in that case. Non-retryable errors still propagate immediately via the else branch.

### 6. Replace orchestrator retry logic

In `src/workflow/orchestrator.rs`:
- Remove `execute_with_timeout_retries()` entirely.
- Update `execute_with_parse_retries()` and its 3 internal call sites to call `execute_with_retries(backend.as_ref(), role, phase, prompt, &RetryPolicy::default())` instead.
- Remove the `BackendTimeoutExhausted` variant from `src/error.rs` since it is fully superseded by `BackendRetriesExhausted`. Update `exit_code()` match arms if needed.
- Update any orchestrator tests that assert on `BackendTimeoutExhausted` to use `BackendRetriesExhausted`.

### 7. Register the module

Add `pub mod retry;` to `src/backend/mod.rs`.

## Files & Modules

| File | Change |
|---|---|
| `Cargo.toml` | Add `rand = "0.8"` dependency |
| `src/backend/retry.rs` | **New** — `RetryPolicy`, `JitterStrategy`, `is_retryable()`, `execute_with_retries()` |
| `src/backend/mod.rs` | Add `pub mod retry;` |
| `src/error.rs` | Add `BackendRetriesExhausted` variant; remove `BackendTimeoutExhausted` |
| `src/workflow/orchestrator.rs` | Remove `execute_with_timeout_retries()`; update 3 call sites in `execute_with_parse_retries()` to use `execute_with_retries()` with `RetryPolicy::default()` |

Note: `CliBackend`, `TmuxBackend`, and `MockBackend` are **not modified**. Retry policy is external to backends.

## Testing Strategy

1. **Unit tests in `src/backend/retry.rs`** (all use `JitterStrategy::None` for deterministic assertions unless testing jitter specifically):
   - `test_delay_for_attempt_exponential_no_jitter` — With `base_delay: 1s, jitter: None`, verify delays are exactly 1s, 2s, 4s, 8s for attempts 0–3.
   - `test_delay_for_attempt_capped_at_max` — With `base_delay: 1s, max_delay: 5s, jitter: None`, verify attempt 3 (8s uncapped) returns 5s.
   - `test_delay_for_attempt_with_jitter_bounds` — With `jitter: Uniform`, run 100 iterations for attempt 0 with `base_delay: 1000ms` and verify all results fall within 750ms–1250ms (±25%).
   - `test_delay_for_attempt_overflow_saturates` — With `base_delay: 1s`, verify attempt 63 and attempt 100 both return `max_delay` without panicking.
   - `test_is_retryable_timeout` — `BackendTimeout` returns `true`.
   - `test_is_retryable_transient_command_failure` — `BackendCommandFailed` with each indicator string ("connection refused", "429", "503", "502", "rate limit", "timed out", "temporarily unavailable") returns `true`.
   - `test_is_retryable_case_insensitive` — `BackendCommandFailed` with "Connection Refused" and "Rate Limit" returns `true`.
   - `test_is_not_retryable_tmux_exit_code` — `BackendCommandFailed` with `"tmux command exited with code 1 (command='claude')"` returns `false`.
   - `test_is_not_retryable_other_variants` — `ParseError`, `Validation`, `Io`, etc. return `false`.
   - `test_execute_with_retries_succeeds_on_third_attempt` — Mock that fails twice with `BackendTimeout` then succeeds; verify exactly 3 calls and `Ok` result.
   - `test_execute_with_retries_exhausted_wraps_error` — Mock that always returns `BackendTimeout`; with `max_retries: 2`, verify `BackendRetriesExhausted` is returned with `attempts: 3`, `phase` matches input, and `last_error` contains the timeout message.
   - `test_execute_with_retries_non_retryable_immediate` — Mock returning `Validation` error; verify only 1 call made and original error propagated.
   - `test_zero_retries_policy` — `RetryPolicy { max_retries: 0, .. }` makes exactly 1 attempt; on `BackendTimeout`, returns `BackendRetriesExhausted` with `attempts: 1`.
   - `test_retryable_command_failure_retried` — Mock returning `BackendCommandFailed` with "429" twice then succeeds; verify 3 calls.

2. **Mock backend for retry testing**: Create a `SequenceMockBackend` in `src/backend/retry.rs` (test-only, inside `#[cfg(test)] mod tests`) that holds a `Vec<Result<String>>` and returns them sequentially, tracking call count. This is distinct from the existing `MockBackend` (which only returns `Ok` values) and lives only in the test module.

3. **Integration-level**: Existing orchestrator tests continue to pass. Update any tests that assert on `BackendTimeoutExhausted` to expect `BackendRetriesExhausted` instead.

**Addressing review issue #4 (mock conflict):** The existing `MockBackend` is not modified and does not participate in retry logic. A separate test-only `SequenceMockBackend` is created inside `#[cfg(test)]` in the retry module specifically for retry testing. This backend can return a sequence of `Result<String>` values (including errors), enabling precise control over failure/success patterns without affecting the production `MockBackend`.

**Addressing review issue #7 (test completeness):** The `JitterStrategy::None` mode enables deterministic delay assertions. Tests cover: jitter bounds verification, overflow/saturation, case-insensitive matching, tmux-style error non-matching, exhaustion wrapping with correct fields, and zero-retry edge case.

## Out of Scope

- **Circuit breaker pattern** — no "open/half-open" state tracking across multiple calls.
- **Per-error-type backoff tuning** — all retryable errors use the same exponential curve.
- **Retry budget / rate limiting** — no global cap on total retries across concurrent calls.
- **Configuration via `.ralph/index.json`** — retry policy is set programmatically only; config-file support is a follow-up.
- **Retry logic for `health_check()`** — only `execute()` is wrapped.
- **Metrics / structured retry telemetry** — `tracing::warn!` logs only; no counters or histograms.
- **Storing `RetryPolicy` on backend structs** — retry is an orchestrator concern, not a backend concern. Per-backend policy customization via config is a follow-up.
- **Tmux stderr capture** — not needed; tmux error messages naturally don't match transient indicators.