I now have a thorough understanding of the complete picture. Here's my engineering specification:

---

## Summary

The test `parse_retry_attempts_four_with_session_followup_and_token_metrics` fails intermittently under `nix build` (static musl, `x86_64-unknown-linux-musl`) with only 1 of 4 expected `token-metrics` tracing events captured, while passing consistently under glibc builds.

The root cause is **tracing's per-callsite interest caching** interacting with musl's thread-local storage behavior. Each `info!(...)` invocation in `log_parse_retry_token_metrics` corresponds to a distinct static `DefaultCallsite`. In the 8-arm `match` inside that function, each arm has its own callsite. The first call (attempt 1, all tokens present) hits the `(Some, Some, Some)` arm and its callsite gets registered with the test's `MetricsCaptureLayer` subscriber. Attempts 2–4 (no tokens) hit the `(None, None, None)` arm — a *different* callsite that may have been previously registered by another test running on a parallel thread (where no scoped subscriber was active), causing its interest to be cached as `Interest::never()`. Under musl's different TLS initialization timing, the `SCOPED_COUNT` fast-path check or the `CURRENT_STATE.try_with()` call can race with callsite registration, permanently marking those callsites as disabled.

The fix is to **bypass the interest-caching problem entirely** by collecting metrics through a direct, non-tracing side channel in the test, or by restructuring `log_parse_retry_token_metrics` so all code paths use a single callsite (single `info!()` call) that cannot be independently cached.

## Acceptance Criteria

- Test passes consistently in `nix build` environment (static musl, sandboxed, release mode)
- Test continues to pass in `cargo test` (debug and release modes, glibc)
- All 4 token-metrics events are captured correctly across all build configurations
- No regressions in other tests in the `workflow::orchestrator::tests` module
- No change to production-path tracing output format or semantics

## Technical Approach

### Option A (Recommended): Consolidate to a single `info!()` callsite

Refactor `log_parse_retry_token_metrics` to emit a single `info!()` call regardless of which token fields are present. The current 8-arm match produces 8 different static callsites — each independently subject to interest caching. A single callsite eliminates the possibility that different arms get different cached interest values.

**Implementation:**

1. Replace the 8-arm match in `log_parse_retry_token_metrics` (lines 4669–4770) with a single `info!()` call that always includes all fields, using `tracing::field::Empty` for absent values:

```rust
fn log_parse_retry_token_metrics(
    role: &str,
    phase: &str,
    loop_number: u32,
    attempt: u8,
    backend: &str,
    session_reused: bool,
    normalized: &crate::backend::output_normalizer::NormalizedOutput,
) {
    let tokens_in = normalized.tokens_in;
    let tokens_out = normalized.tokens_out;
    let cached_in = normalized.cached_in;

    // Single callsite: all field names are always declared, values use
    // tracing::field::Empty when the Option is None.  This avoids the
    // per-callsite interest-caching divergence that caused musl failures.
    info!(
        role = role,
        phase = phase,
        loop_number = loop_number,
        attempt = attempt,
        backend = backend,
        session_reused = session_reused,
        tokens_in = tokens_in.unwrap_or(0),
        tokens_out = tokens_out.unwrap_or(0),
        cached_in = cached_in.unwrap_or(0),
        tokens_in_present = tokens_in.is_some(),
        tokens_out_present = tokens_out.is_some(),
        cached_in_present = cached_in.is_some(),
        "parse-retry normalization metrics"
    );
}
```

However, this changes the semantics of `tracing::field::Empty` to actual `0` values in production logs. If preserving the `Empty` field behavior in production is required, use approach A2:

**A2 — Keep `Empty` semantics with a value-set span trick:**

Use `tracing::Span::current()` with `record()` to set optional fields dynamically on a single callsite:

```rust
fn log_parse_retry_token_metrics(...) {
    let span = tracing::info_span!(
        "parse-retry-metrics",
        role = role,
        phase = phase,
        loop_number = loop_number,
        attempt = attempt,
        backend = backend,
        session_reused = session_reused,
        tokens_in = tracing::field::Empty,
        tokens_out = tracing::field::Empty,
        cached_in = tracing::field::Empty,
    );
    if let Some(v) = normalized.tokens_in { span.record("tokens_in", v); }
    if let Some(v) = normalized.tokens_out { span.record("tokens_out", v); }
    if let Some(v) = normalized.cached_in { span.record("cached_in", v); }

    let _entered = span.enter();
    info!("parse-retry normalization metrics");
}
```

This still produces a single `info!()` callsite. The `MetricsVisitor` in the test must be updated to read fields from the span context rather than the event directly.

**Simplest recommended path:** Use approach A (non-A2) — emit all fields as concrete values (`0` for absent) with companion `*_present` bools. This is the smallest change with the most predictable cross-platform behavior. Update `MetricsVisitor` to read `tokens_in_present` / `tokens_out_present` / `cached_in_present` bools instead of detecting field presence.

2. Update `MetricsVisitor` to match the new field structure — read `tokens_in_present` etc. as bools via `record_bool`.

3. Update the test assertions: `tokens_in_seen` / `tokens_out_seen` / `cached_in_seen` now map to the `*_present` boolean fields rather than field-existence detection.

### Option B (Alternative): Direct side-channel for test capture

Instead of relying on tracing dispatch, pass an optional metrics sink (`Arc<Mutex<Vec<...>>>`) into `execute_with_parse_retries` that the function writes to directly. This bypasses the tracing subscriber entirely for test verification. However, this adds a test-only parameter to production code, which is less clean.

**Recommendation: Option A** — it fixes the root cause (multi-callsite interest divergence), simplifies the function from 110 lines to ~20, and eliminates the class of musl/TLS bugs entirely.

## Files & Modules

| File | Change |
|------|--------|
| `src/workflow/orchestrator.rs` lines 4660–4771 | Rewrite `log_parse_retry_token_metrics`: collapse 8-arm match into single `info!()` call |
| `src/workflow/orchestrator.rs` lines 6523–6582 | Update `ParseRetryMetricsEvent`, `MetricsVisitor`, `MetricsCaptureLayer` to match new field structure |
| `src/workflow/orchestrator.rs` lines 6667–6750 | Update test assertions to use new `*_present` boolean fields |

No other files need modification. The `tracing` and `tracing-subscriber` dependency versions remain unchanged.

## Testing Strategy

1. **Existing test (modified):** `parse_retry_attempts_four_with_session_followup_and_token_metrics` — verify all 4 events are captured with correct `attempt`, `session_reused`, and token-presence fields.

2. **Local verification:**
   - `cargo test workflow::orchestrator::tests` (debug mode, glibc) — must pass
   - `cargo test --release workflow::orchestrator::tests` (release mode, glibc) — must pass
   - Run the full test suite to check for regressions in the orchestrator module

3. **Musl verification:**
   - `nix build` — the target environment where the bug manifests. Must pass consistently.
   - Run at least 5 consecutive `nix build` invocations to confirm no intermittent failures.

4. **Regression guard:** Ensure no other tests in the module depend on the old 8-arm `info!()` structure or field-presence detection semantics. (Grep confirms only one test uses `MetricsCaptureLayer`.)

## Out of Scope

- Upgrading `tracing` or `tracing-core` crate versions
- Changing the nix build configuration or musl target
- Modifying the tokio runtime setup (already correct: `new_current_thread`)
- Adding `#[serial]` or other test-ordering constraints (unnecessary with single-callsite fix)
- Changing the `SequencedBackend` test mock or `BackendRegistry` behavior
- Refactoring `execute_with_parse_retries` beyond the metrics logging function
- Adding a global tracing subscriber in the test harness
- Changing the production log output format beyond the `Empty` → `0` + `*_present` field change