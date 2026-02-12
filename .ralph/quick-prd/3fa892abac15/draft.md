Now I have all the context needed. Here is the complete engineering specification:

---

## Summary

Add configurable exponential backoff retry logic to `Backend::execute()` so that transient failures (timeouts, rate limits, temporary API errors) are automatically retried before surfacing an error. Today, retry logic lives in `execute_with_timeout_retries()` in the orchestrator (hardcoded to 3 attempts, timeout-only, no jitter). This feature pushes a generalized, configurable `RetryPolicy` into the backend layer itself, replaces the hardcoded orchestrator function, adds jitter to prevent thundering-herd problems, and classifies errors as retryable vs. non-retryable.

## Acceptance Criteria

1. A `RetryPolicy` struct exists with `max_retries: u32` (default 3) and `base_delay: Duration` (default 1s), implementing `Default`.
2. `Backend::execute()` calls are wrapped so that `BackendTimeout` and `BackendCommandFailed` (when details contain transient indicators like "connection refused", "rate limit", "503", "429") are retried; all other errors propagate immediately.
3. Backoff delay is `base_delay * 2^attempt` with ±25% uniform random jitter per attempt.
4. Each retry emits a `tracing::warn!` log with backend name, attempt number, computed delay, and error summary.
5. When retries are exhausted, the final error is returned wrapped in `BackendRetriesExhausted { backend, attempts, last_error }`.
6. The existing `execute_with_timeout_retries()` in `orchestrator.rs` is replaced by the new mechanism — no duplicate retry layers.
7. `RetryPolicy` is configurable per-backend via `CliBackend` / `TmuxBackend` constructors; defaults apply if unset.
8. `MockBackend` is unaffected (no retry wrapping on mocks).
9. The `rand` crate is added to `Cargo.toml` for jitter computation.

## Technical Approach

### 1. Add `rand` dependency

Add `rand = "0.8"` to `Cargo.toml` `[dependencies]`.

### 2. Define `RetryPolicy` (`src/backend/retry.rs`, new file)

```rust
use std::time::Duration;
use rand::Rng;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_secs(1),
        }
    }
}

impl RetryPolicy {
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base = self.base_delay.as_millis() as u64 * 2_u64.pow(attempt);
        let jitter_range = base / 4; // ±25%
        let jitter = rand::thread_rng().gen_range(0..=jitter_range * 2);
        Duration::from_millis(base - jitter_range + jitter)
    }
}
```

### 3. Add error classification (`src/backend/retry.rs`)

Add a free function `is_retryable(err: &RalphError) -> bool` that returns `true` for:
- `BackendTimeout { .. }` — always retryable
- `BackendCommandFailed { details, .. }` — retryable when `details` contains any of: `"connection refused"`, `"rate limit"`, `"429"`, `"503"`, `"502"`, `"timed out"`, `"temporarily unavailable"` (case-insensitive)

All other variants return `false`.

### 4. Add `BackendRetriesExhausted` error variant (`src/error.rs`)

```rust
#[error("backend retries exhausted for {backend} after {attempts} attempts: {last_error}")]
BackendRetriesExhausted {
    backend: String,
    attempts: u32,
    last_error: String,
},
```

### 5. Implement `execute_with_retries()` (`src/backend/retry.rs`)

An async function that wraps any `Arc<dyn Backend>`:

```rust
pub async fn execute_with_retries(
    backend: &dyn Backend,
    prompt: &str,
    policy: &RetryPolicy,
) -> Result<String> {
    let mut last_err = None;
    for attempt in 0..=policy.max_retries {
        match backend.execute(prompt).await {
            Ok(output) => return Ok(output),
            Err(e) if is_retryable(&e) && attempt < policy.max_retries => {
                let delay = policy.delay_for_attempt(attempt);
                tracing::warn!(
                    backend = backend.name(),
                    attempt = attempt + 1,
                    max = policy.max_retries,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "retrying backend execution"
                );
                tokio::time::sleep(delay).await;
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(RalphError::BackendRetriesExhausted {
        backend: backend.name().to_owned(),
        attempts: policy.max_retries + 1,
        last_error: last_err.map(|e| e.to_string()).unwrap_or_default(),
    })
}
```

### 6. Store `RetryPolicy` on backends

Add a `retry_policy: RetryPolicy` field to `CliBackend` and `TmuxBackend`. Initialize from defaults in constructors. Expose a builder method `with_retry_policy(mut self, policy: RetryPolicy) -> Self`.

### 7. Replace orchestrator retry logic

In `src/workflow/orchestrator.rs`:
- Remove `execute_with_timeout_retries()` entirely.
- At each former call site, call `execute_with_retries(backend.as_ref(), prompt, &backend_retry_policy)` instead.
- Map `BackendRetriesExhausted` to `BackendTimeoutExhausted` at the orchestrator level if needed for backward compatibility, or update callers to handle the new variant.
- The `BackendTimeoutExhausted` variant can be deprecated/removed if no other code path produces it.

### 8. Register the module

Add `pub mod retry;` to `src/backend/mod.rs`.

## Files & Modules

| File | Change |
|---|---|
| `Cargo.toml` | Add `rand = "0.8"` dependency |
| `src/backend/retry.rs` | **New** — `RetryPolicy`, `is_retryable()`, `execute_with_retries()` |
| `src/backend/mod.rs` | Add `pub mod retry;`, add `retry_policy` field to `CliBackend`, builder method |
| `src/backend/tmux_backend.rs` | Add `retry_policy` field to `TmuxBackend`, pass through to construction |
| `src/error.rs` | Add `BackendRetriesExhausted` variant |
| `src/workflow/orchestrator.rs` | Remove `execute_with_timeout_retries()`, call `execute_with_retries()` at call sites |

## Testing Strategy

1. **Unit tests in `src/backend/retry.rs`**:
   - `test_delay_for_attempt_exponential` — verify delay doubles per attempt (check within ±25% jitter bounds).
   - `test_is_retryable_timeout` — `BackendTimeout` returns `true`.
   - `test_is_retryable_transient_command_failure` — `BackendCommandFailed` with "connection refused" / "429" / "503" returns `true`.
   - `test_is_not_retryable_auth_failure` — `BackendCommandFailed` with "invalid API key" returns `false`.
   - `test_is_not_retryable_other_variants` — `ParseError`, `Validation`, etc. return `false`.
   - `test_execute_with_retries_succeeds_on_third_attempt` — `MockBackend` that fails twice then succeeds; verify exactly 3 calls.
   - `test_execute_with_retries_exhausted` — `MockBackend` that always times out; verify `BackendRetriesExhausted` returned with correct `attempts`.
   - `test_execute_with_retries_non_retryable_immediate` — `MockBackend` returning a non-retryable error; verify only 1 call made.
   - `test_zero_retries_policy` — `RetryPolicy { max_retries: 0, .. }` makes exactly 1 attempt.

2. **Integration-level**: Existing orchestrator tests in `src/workflow/orchestrator.rs` continue to pass. Update any tests that assert on `BackendTimeoutExhausted` to expect `BackendRetriesExhausted` instead (or verify the mapping).

3. **MockBackend enhancement**: Add an `ErrorMockBackend` (or extend the existing `MockBackend`) that can return a sequence of `Result<String>` values (not just `Ok(String)`) to support retry testing without real backends.

## Out of Scope

- **Circuit breaker pattern** — no "open/half-open" state tracking across multiple calls.
- **Per-error-type backoff tuning** — all retryable errors use the same exponential curve.
- **Retry budget / rate limiting** — no global cap on total retries across concurrent calls.
- **Configuration via `.ralph/index.json`** — retry policy is set programmatically only; config-file support is a follow-up.
- **Retry logic for `health_check()`** — only `execute()` is wrapped.
- **Metrics / structured retry telemetry** — `tracing::warn!` logs only; no counters or histograms.