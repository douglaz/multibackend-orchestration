Now I have a thorough understanding of the codebase. Let me address each of the 6 review issues and produce the revised specification.

Key findings that inform the revision:

1. **Review #1 (tmux "timed out")**: Confirmed — tmux produces `"disappeared or timed out"` which matches the `"timed out"` indicator. And `TmuxBackend::name()` delegates to the inner backend, so you can't distinguish tmux vs CLI by name. Need a structural fix.
2. **Review #2 (JitterStrategy API mismatch)**: AC #8 says function parameter, but implementation puts it on `RetryPolicy`. Need to align — keeping it on `RetryPolicy` is cleaner.
3. **Review #3 (overflow)**: `max_retries + 1` can overflow at `u32::MAX`, and `as_millis() as u64` can truncate u128.
4. **Review #4 (scope of rollout)**: 5 direct `backend.execute()` calls in prd modules are unwrapped.
5. **Review #5 (configurability)**: Feature idea says "configurable" but only code-level defaults exist.
6. **Review #6 (test gaps)**: Missing tmux "timed out" regression test, missing orchestrator-level test, tests should avoid real sleeps.

Here is the revised specification:

---

## Summary

Add configurable exponential backoff retry logic to `Backend::execute()` so that transient failures (timeouts, rate limits, temporary API errors) are automatically retried before surfacing an error. Today, retry logic lives in `execute_with_timeout_retries()` in the orchestrator (hardcoded to 3 total attempts, timeout-only, no jitter). This feature introduces a `RetryPolicy` struct and an `execute_with_retries()` free function in a new `src/backend/retry.rs` module that replaces the orchestrator's hardcoded function. The new mechanism adds jitter to prevent thundering-herd problems, classifies errors as retryable vs. non-retryable, and caps backoff delay to keep runtime bounded. Retry policy is passed explicitly at call sites (the orchestrator and prd modules), not stored on backend structs — backends remain stateless with respect to retry behavior.

## Acceptance Criteria

1. A `RetryPolicy` struct exists with `max_retries: u32` (default 2) and `base_delay: Duration` (default 1s), implementing `Default`. The field `max_retries` means the number of **retries after the initial attempt**; total attempts = `max_retries + 1`. With the default of 2, there are 3 total attempts — matching today's behavior exactly. `max_retries` is clamped to `u32::MAX - 1` in the constructor to prevent overflow when computing total attempts.
2. `Backend::execute()` calls are wrapped by `execute_with_retries()` so that `BackendTimeout` errors are always retried. `BackendCommandFailed` is retried **only when** the `details` field contains transient HTTP/network indicators (case-insensitive): `"connection refused"`, `"rate limit"`, `"429"`, `"503"`, `"502"`. The strings `"timed out"` and `"temporarily unavailable"` are **not** used as indicators because tmux produces `BackendCommandFailed` with `"disappeared or timed out"` (at `tmux_backend.rs:239`), and since `TmuxBackend::name()` delegates to the inner backend's name, there is no way to distinguish tmux vs. CLI transport at the error level. Removing these two indicators ensures tmux command failures are never incorrectly retried.
3. Backoff delay is `min(base_delay * 2^attempt, max_delay)` with ±25% uniform random jitter, where `max_delay` defaults to 30 seconds. Arithmetic uses `checked_shl` + `saturating_mul` to prevent overflow. Duration-to-millisecond conversion uses `u64::try_from(millis_u128).unwrap_or(u64::MAX)` to avoid silent truncation from `as u64` casting on u128 values.
4. Each retry emits a `tracing::warn!` log with backend name, attempt number (1-indexed), total max attempts, computed delay in ms, and error summary.
5. When retries are exhausted, the final error is returned wrapped in `BackendRetriesExhausted { backend, phase, attempts, last_error }`, where `attempts` is the total number of attempts made.
6. The existing `execute_with_timeout_retries()` in `orchestrator.rs` is replaced by calls to `execute_with_retries()` — no duplicate retry layers.
7. `RetryPolicy` is passed explicitly at call sites in the orchestrator and prd modules (via `RetryPolicy::default()`). It is **not** stored on `CliBackend`, `TmuxBackend`, or the `Backend` trait. Backends remain unaware of retry behavior.
8. `RetryPolicy` contains a `jitter: JitterStrategy` field (enum: `Uniform`, `None`) defaulting to `Uniform`. A `without_jitter()` builder method returns a copy with `JitterStrategy::None` for deterministic testing. There is no separate jitter parameter on `execute_with_retries()`.
9. The `rand` crate is added to `Cargo.toml` for jitter computation.
10. All `backend.execute()` call sites are wrapped: the 3 calls inside `execute_with_parse_retries()` in the orchestrator, plus the 5 direct calls in `src/prd/pipeline.rs`, `src/prd/gaps.rs`, and `src/prd/quick.rs`. This ensures retry coverage across all backend invocations.
11. Runtime/user configurability of retry parameters (via CLI flags, environment variables, or `.ralph/index.json`) is explicitly **out of scope**. "Configurable" in this feature means code-level configurability: callers can construct `RetryPolicy` with custom values. Runtime configuration is a follow-up.

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
    /// Total number of attempts (initial + retries), clamped to prevent overflow.
    fn total_attempts(&self) -> u32 {
        self.max_retries.saturating_add(1)
    }

    /// Compute the delay before the given retry attempt (0-indexed).
    /// Uses saturating arithmetic and caps at `max_delay`.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base_ms = u64::try_from(self.base_delay.as_millis()).unwrap_or(u64::MAX);
        let multiplier = 1_u64.checked_shl(attempt).unwrap_or(u64::MAX);
        let uncapped_ms = base_ms.saturating_mul(multiplier);
        let max_ms = u64::try_from(self.max_delay.as_millis()).unwrap_or(u64::MAX);
        let capped_ms = uncapped_ms.min(max_ms);

        match self.jitter {
            JitterStrategy::None => Duration::from_millis(capped_ms),
            JitterStrategy::Uniform => {
                let jitter_range = capped_ms / 4; // ±25%
                if jitter_range == 0 {
                    return Duration::from_millis(capped_ms);
                }
                let jitter = rand::thread_rng().gen_range(0..=jitter_range * 2);
                Duration::from_millis(
                    capped_ms.saturating_sub(jitter_range).saturating_add(jitter),
                )
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

**Review issue #1 fix (tmux "timed out"):** Resolved in the error classifier (section 3 below) by removing `"timed out"` and `"temporarily unavailable"` from the indicator list. Since `TmuxBackend::name()` delegates to `self.inner.name()`, we cannot distinguish transport type from the error alone. Removing the ambiguous indicators is the simplest correct fix. The `BackendTimeout` variant (produced by `tokio::time::timeout` in `CliBackend::execute`) is a separate enum variant and is always retried regardless of transport — this is correct because tmux does not produce `BackendTimeout` (it produces `BackendCommandFailed` with "disappeared or timed out" text instead).

**Review issue #2 fix (AC/API alignment):** Jitter strategy lives on `RetryPolicy.jitter`, not as a separate parameter to `execute_with_retries()`. AC #8 is revised to match.

**Review issue #3 fix (overflow):** Three changes: (a) `total_attempts()` uses `saturating_add(1)` instead of `+1`; (b) `as_millis() as u64` replaced with `u64::try_from(...).unwrap_or(u64::MAX)` to handle u128 values safely; (c) jitter arithmetic uses `saturating_sub`/`saturating_add`.

### 3. Add error classification (`src/backend/retry.rs`)

```rust
use crate::error::RalphError;

/// Transient HTTP/network indicators in `BackendCommandFailed` details.
/// These are chosen to match CliBackend stderr output from HTTP-based tools
/// while avoiding false positives from tmux error messages.
///
/// Notably excluded:
/// - "timed out": tmux produces "disappeared or timed out" in BackendCommandFailed
/// - "temporarily unavailable": too generic, could match non-HTTP contexts
///
/// BackendTimeout (the enum variant) is always retried regardless of this list.
const TRANSIENT_INDICATORS: &[&str] = &[
    "connection refused",
    "rate limit",
    "429",
    "503",
    "502",
];

pub fn is_retryable(err: &RalphError) -> bool {
    match err {
        RalphError::BackendTimeout { .. } => true,
        RalphError::BackendCommandFailed { details, .. } => {
            let lower = details.to_lowercase();
            TRANSIENT_INDICATORS.iter().any(|ind| lower.contains(ind))
        }
        _ => false,
    }
}
```

**Review issue #1 fix (tmux feasibility):** The `"timed out"` and `"temporarily unavailable"` indicators are removed from the transient indicator list. This eliminates the false positive where tmux's `"disappeared or timed out"` message would incorrectly trigger retries. The remaining indicators (`"connection refused"`, `"rate limit"`, `"429"`, `"503"`, `"502"`) are HTTP/network-specific strings that tmux error messages do not produce. Tmux errors contain patterns like `"tmux command exited with code 1"` or `"tmux window '...' disappeared or timed out"` — neither of which matches the remaining indicators.

The trade-off: CLI backends that produce `"timed out"` in stderr (e.g., `curl: (28) Operation timed out`) will not have those `BackendCommandFailed` errors retried. However, actual timeouts from `tokio::time::timeout` in `CliBackend::execute()` produce the `BackendTimeout` variant, which is always retried. This is an acceptable narrowing.

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

The `phase` field is preserved from the existing `BackendTimeoutExhausted` variant to maintain parity for error messages and caller context. The `attempts` field is `u32` (widened from `u8` in the old variant) to match `RetryPolicy::max_retries`.

### 5. Implement `execute_with_retries()` (`src/backend/retry.rs`)

```rust
pub async fn execute_with_retries(
    backend: &dyn Backend,
    role: &str,
    phase: &str,
    prompt: &str,
    policy: &RetryPolicy,
) -> Result<String> {
    let total_attempts = policy.total_attempts();

    for attempt in 0..total_attempts {
        match backend.execute(prompt).await {
            Ok(output) => return Ok(output),
            Err(e) => {
                let is_last = attempt + 1 >= total_attempts;

                if !is_retryable(&e) {
                    // Non-retryable error — propagate immediately
                    return Err(e);
                }

                if is_last {
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
    // Safety fallback matching existing codebase pattern.
    Err(RalphError::Orchestration(
        "unexpected retry control-flow error".to_owned(),
    ))
}
```

**Review issue #3 fix (control-flow correctness):** The logic is restructured into two clear branches: (1) non-retryable errors propagate immediately, (2) retryable errors on the last attempt return `BackendRetriesExhausted`. This eliminates the original bug where the last retryable failure could fall through to return the raw error.

### 6. Replace orchestrator retry logic and wrap prd call sites

**In `src/workflow/orchestrator.rs`:**
- Remove `execute_with_timeout_retries()` entirely.
- Update the 3 internal call sites in `execute_with_parse_retries()` to call `execute_with_retries(backend.as_ref(), role, phase, prompt, &RetryPolicy::default())`.
- Remove the `BackendTimeoutExhausted` variant from `src/error.rs` since it is fully superseded by `BackendRetriesExhausted`. Update `exit_code()` match arms accordingly.
- Update any orchestrator tests that assert on `BackendTimeoutExhausted` to use `BackendRetriesExhausted`.

**In `src/prd/pipeline.rs`** (line 194):
- Replace `self.backend.execute(&prompt).await?` with `execute_with_retries(self.backend.as_ref(), "prd-writer", "prd-generation", &prompt, &RetryPolicy::default()).await?`.

**In `src/prd/gaps.rs`** (line 127):
- Replace `backend.execute(&prompt).await?` with `execute_with_retries(backend.as_ref(), "gap-analyst", "gap-analysis", &prompt, &RetryPolicy::default()).await?`.

**In `src/prd/quick.rs`** (lines 196, 338, 394):
- Replace each `backend.execute(...)` / `self.writer.execute(...)` call with the corresponding `execute_with_retries(...)` call using appropriate `role` and `phase` strings:
  - Line 196: `role = "spec-writer"`, `phase = "spec-generation"`
  - Line 338: `role = "spec-writer"`, `phase = "spec-revision"`
  - Line 394: `role = "spec-writer"`, `phase = "spec-generation"`

### 7. Register the module

Add `pub mod retry;` to `src/backend/mod.rs`.

## Files & Modules

| File | Change |
|---|---|
| `Cargo.toml` | Add `rand = "0.8"` dependency |
| `src/backend/retry.rs` | **New** — `RetryPolicy`, `JitterStrategy`, `TRANSIENT_INDICATORS`, `is_retryable()`, `execute_with_retries()` |
| `src/backend/mod.rs` | Add `pub mod retry;` |
| `src/error.rs` | Add `BackendRetriesExhausted` variant; remove `BackendTimeoutExhausted` |
| `src/workflow/orchestrator.rs` | Remove `execute_with_timeout_retries()`; update 3 call sites in `execute_with_parse_retries()` to use `execute_with_retries()` with `RetryPolicy::default()` |
| `src/prd/pipeline.rs` | Wrap 1 `backend.execute()` call with `execute_with_retries()` |
| `src/prd/gaps.rs` | Wrap 1 `backend.execute()` call with `execute_with_retries()` |
| `src/prd/quick.rs` | Wrap 3 `backend.execute()` / `self.writer.execute()` calls with `execute_with_retries()` |

Note: `CliBackend`, `TmuxBackend`, and `MockBackend` are **not modified**. Retry policy is external to backends.

## Testing Strategy

All retry unit tests use `JitterStrategy::None` (via `RetryPolicy::default().without_jitter()`) for deterministic assertions, and use zero or near-zero `base_delay` (e.g., `Duration::from_millis(1)`) to avoid real sleep latency. Tests that verify jitter bounds are the exception.

1. **Unit tests in `src/backend/retry.rs`** (`#[cfg(test)] mod tests`):
   - `test_delay_for_attempt_exponential_no_jitter` — With `base_delay: 1s, jitter: None`, verify delays are exactly 1000ms, 2000ms, 4000ms, 8000ms for attempts 0–3.
   - `test_delay_for_attempt_capped_at_max` — With `base_delay: 1s, max_delay: 5s, jitter: None`, verify attempt 3 (8s uncapped) returns 5000ms.
   - `test_delay_for_attempt_with_jitter_bounds` — With `jitter: Uniform`, run 100 iterations for attempt 0 with `base_delay: 1000ms` and verify all results fall within 750ms–1250ms (±25%).
   - `test_delay_for_attempt_overflow_saturates` — With `base_delay: 1s`, verify attempt 63 and attempt 100 both return `max_delay` without panicking.
   - `test_total_attempts_saturates` — With `max_retries: u32::MAX`, verify `total_attempts()` returns `u32::MAX` (not 0 from overflow).
   - `test_is_retryable_timeout` — `BackendTimeout` returns `true`.
   - `test_is_retryable_transient_command_failure` — `BackendCommandFailed` with each indicator string (`"connection refused"`, `"429"`, `"503"`, `"502"`, `"rate limit"`) returns `true`.
   - `test_is_retryable_case_insensitive` — `BackendCommandFailed` with `"Connection Refused"` and `"Rate Limit"` returns `true`.
   - `test_is_not_retryable_tmux_exit_code` — `BackendCommandFailed` with `"tmux command exited with code 1 (command='claude')"` returns `false`.
   - **`test_is_not_retryable_tmux_disappeared_or_timed_out`** — `BackendCommandFailed` with `"tmux window 'claude' (id=@1) for backend 'claude' disappeared or timed out before the exit file was written."` returns `false`. *(Review issue #6 — regression test for tmux false positive.)*
   - `test_is_not_retryable_other_variants` — `ParseError`, `Validation`, `Io`, etc. return `false`.
   - `test_execute_with_retries_succeeds_on_third_attempt` — Mock that fails twice with `BackendTimeout` then succeeds; uses `base_delay: 1ms, jitter: None`; verify exactly 3 calls and `Ok` result.
   - `test_execute_with_retries_exhausted_wraps_error` — Mock that always returns `BackendTimeout`; with `max_retries: 2, base_delay: 1ms, jitter: None`; verify `BackendRetriesExhausted` is returned with `attempts: 3`, `phase` matches input, and `last_error` contains the timeout message.
   - `test_execute_with_retries_non_retryable_immediate` — Mock returning `Validation` error; verify only 1 call made and original error propagated (not wrapped).
   - `test_zero_retries_policy` — `RetryPolicy { max_retries: 0, .. }` makes exactly 1 attempt; on `BackendTimeout`, returns `BackendRetriesExhausted` with `attempts: 1`.
   - `test_retryable_command_failure_retried` — Mock returning `BackendCommandFailed` with `"429"` twice then succeeds; uses `base_delay: 1ms, jitter: None`; verify 3 calls.
   - `test_command_failure_timed_out_not_retried` — Mock returning `BackendCommandFailed` with `"server timed out"` once; verify only 1 call made and the original `BackendCommandFailed` error propagated immediately (not wrapped in `BackendRetriesExhausted`). *(Confirms "timed out" in BackendCommandFailed details is not retried.)*

2. **Mock backend for retry testing**: Create a `SequenceMockBackend` in `src/backend/retry.rs` (inside `#[cfg(test)] mod tests`) that holds a `Mutex<VecDeque<Result<String>>>` and returns them sequentially, tracking call count via `AtomicU32`. This is distinct from the existing `MockBackend` (which only returns `Ok` values) and lives only in the test module.

3. **Orchestrator-level regression test** (`src/workflow/orchestrator.rs` tests):
   - **`test_parse_retries_uses_new_retry_mechanism`** — Using `MockBackend`, verify that after removing `execute_with_timeout_retries`, the parse-retry flow still makes 3 total attempts on timeout and produces `BackendRetriesExhausted` (not `BackendTimeoutExhausted`). This confirms the replacement is behavior-preserving. *(Review issue #6 — orchestrator-level regression.)*
   - Update any existing tests that assert on `BackendTimeoutExhausted` to expect `BackendRetriesExhausted`.

4. **Sleep avoidance**: All `execute_with_retries` tests use `base_delay: Duration::from_millis(1)` and `jitter: None`, keeping actual sleep time negligible (≤2ms total across all retries). This avoids test flakiness and keeps the suite fast without requiring `tokio::time::pause()`. *(Review issue #6 — fast/stable tests.)*

## Out of Scope

- **Runtime/user configurability** — retry parameters are not configurable via CLI flags, environment variables, or `.ralph/index.json`. "Configurable" in this feature means code-level: callers construct `RetryPolicy` with desired values. Adding runtime config surfaces (CLI `--max-retries`, config file fields) is a follow-up. *(Review issue #5 — clarified.)*
- **Circuit breaker pattern** — no "open/half-open" state tracking across multiple calls.
- **Per-error-type backoff tuning** — all retryable errors use the same exponential curve.
- **Retry budget / rate limiting** — no global cap on total retries across concurrent calls.
- **Retry logic for `health_check()`** — only `execute()` is wrapped.
- **Metrics / structured retry telemetry** — `tracing::warn!` logs only; no counters or histograms.
- **Storing `RetryPolicy` on backend structs** — retry is a caller concern, not a backend concern. Per-backend policy customization via config is a follow-up.
- **Tmux stderr capture or transport-aware error variants** — not needed. The transient indicator list is scoped to avoid tmux false positives without requiring structural changes to error types or tmux backend.
- **Retrying `"timed out"` in `BackendCommandFailed` details** — intentionally excluded to avoid tmux false positives. Actual timeouts are caught by the `BackendTimeout` variant.