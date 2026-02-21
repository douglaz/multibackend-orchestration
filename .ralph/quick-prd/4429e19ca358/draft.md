## Summary

The test `parse_retry_attempts_four_with_session_followup_and_token_metrics` fails intermittently with only 1 of 4 expected `token-metrics` tracing events captured. The failure is most reliably triggered under `nix build` (static musl, `x86_64-unknown-linux-musl`, sandboxed, release mode) but is **not musl-specific** — it is reproducible on glibc by running both `parse_retry_attempts_` tests in parallel with default test threads (`cargo test workflow::orchestrator::tests::parse_retry_attempts_`). The flake disappears with `--test-threads=1`, confirming parallel test interference as the mechanism.

**Root cause: tracing's global callsite interest cache poisoned by a parallel test that lacks a scoped subscriber.**

The `tracing` crate (0.1.44, tracing-core 0.1.36) assigns each `info!(...)` macro invocation a unique static `DefaultCallsite` whose `Interest` is cached globally and permanently. In `log_parse_retry_token_metrics`, the 8-arm `match` creates 8 independent callsites. The sibling test `parse_retry_attempts_are_three_without_session` calls `execute_with_parse_retries` — which internally calls `log_parse_retry_token_metrics` — **without** a scoped subscriber. When this test runs concurrently (or before) the metrics test:

1. The sibling test's thread hits one or more `info!(...)` callsites in `log_parse_retry_token_metrics`.
2. With no subscriber active (only the default no-op), tracing caches `Interest::never()` for those callsites.
3. This cache is **global and permanent** — `with_default()` does not invalidate already-cached callsites.
4. When the metrics test later enters its `with_default(subscriber, ...)` scope, events from the poisoned callsites are silently dropped because the cached `Interest::never()` short-circuits dispatch before the scoped subscriber is consulted.

The musl/nix environment makes the race more likely due to different thread scheduling and TLS initialization timing, but the bug is fundamentally a parallel-test callsite-cache-poisoning issue.

**Key insight:** Consolidating to a single callsite reduces the probability of the race but does **not** eliminate it — the single callsite can itself be poisoned if the sibling test thread registers it first. The fix must either (a) ensure the sibling test cannot poison callsites used by the metrics test, or (b) bypass the tracing interest cache entirely for test capture.

## Acceptance Criteria

- Test passes consistently under `nix build` (static musl, sandboxed, release mode)
- Test passes consistently under `cargo test` (debug and release modes, glibc)
- Test passes consistently when run in parallel with `parse_retry_attempts_are_three_without_session` using default thread count
- All 4 token-metrics events are captured correctly across all build configurations
- No regressions in other tests in the `workflow::orchestrator::tests` module
- No change to production-path tracing output format or semantics (the `info!()` calls in `log_parse_retry_token_metrics` must produce identical structured fields in production as they do today)

## Technical Approach

### Recommended: Thread-local callback side-channel for test capture

**Rationale:** The tracing interest cache is global, permanent, and cannot be reliably scoped to a single test thread. Any fix that relies on the `tracing` subscriber dispatch path (`MetricsCaptureLayer` + `with_default`) remains vulnerable to callsite cache poisoning from parallel tests. The only way to **guarantee** capture regardless of callsite cache state is to bypass the tracing dispatch for test verification entirely.

**Implementation:**

#### Step 1: Add a test-only thread-local callback hook

Add a `#[cfg(test)]` thread-local in `src/workflow/orchestrator.rs` that `log_parse_retry_token_metrics` writes to when set, alongside (not instead of) its existing `info!()` calls:

```rust
#[cfg(test)]
thread_local! {
    static METRICS_TEST_SINK: std::cell::RefCell<Option<Box<dyn FnMut(TokenMetricsRecord)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenMetricsRecord {
    attempt: u8,
    session_reused: bool,
    tokens_in: Option<u64>,
    tokens_out: Option<u64>,
    cached_in: Option<u64>,
}
```

#### Step 2: Emit to the sink from `log_parse_retry_token_metrics`

At the **top** of `log_parse_retry_token_metrics`, before the existing `match` block, add:

```rust
#[cfg(test)]
{
    METRICS_TEST_SINK.with(|sink| {
        if let Some(ref mut f) = *sink.borrow_mut() {
            f(TokenMetricsRecord {
                attempt,
                session_reused,
                tokens_in: normalized.tokens_in,
                tokens_out: normalized.tokens_out,
                cached_in: normalized.cached_in,
            });
        }
    });
}
```

The existing 8-arm `match` with `info!()` calls remains **unchanged** — production logging output is not affected.

#### Step 3: Rewrite the test to use the thread-local sink

Replace `MetricsCaptureLayer` / `with_default` in `parse_retry_attempts_four_with_session_followup_and_token_metrics` with:

```rust
let captured: Arc<Mutex<Vec<TokenMetricsRecord>>> = Arc::new(Mutex::new(Vec::new()));
let captured_clone = captured.clone();

METRICS_TEST_SINK.with(|sink| {
    *sink.borrow_mut() = Some(Box::new(move |record| {
        captured_clone.lock().expect("sink lock").push(record);
    }));
});

let result = runtime.block_on(execute_with_parse_retries(
    // ... same args as today ...
));

// Clear the sink after the test runs.
METRICS_TEST_SINK.with(|sink| { *sink.borrow_mut() = None; });
```

Since `execute_with_parse_retries` runs on a `new_current_thread` tokio runtime inside `runtime.block_on(...)`, all async work executes on the same OS thread that set the thread-local. The sink is guaranteed to capture all 4 calls.

#### Step 4: Update assertions

Assertions change from field-presence detection (`tokens_in_seen`) to direct value inspection:

```rust
let events = captured.lock().expect("lock").clone();
assert_eq!(events.len(), 4);
assert_eq!(events.iter().map(|e| e.attempt).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
assert_eq!(events.iter().map(|e| e.session_reused).collect::<Vec<_>>(), vec![false, true, false, false]);

// Attempt 1: structured usage present
assert!(events[0].tokens_in.is_some() && events[0].tokens_out.is_some() && events[0].cached_in.is_some());
// Attempts 2-4: no structured usage
for event in events.iter().skip(1) {
    assert!(event.tokens_in.is_none() && event.tokens_out.is_none() && event.cached_in.is_none());
}
```

#### Step 5: Remove dead test infrastructure

Delete `ParseRetryMetricsEvent`, `MetricsVisitor`, and `MetricsCaptureLayer` — they are only used by the one test being rewritten. (Confirmed: grep shows no other usage.)

### Why this approach is safe

- **No production code change:** The `#[cfg(test)]` thread-local and `TokenMetricsRecord` are compiled out of release builds entirely. The 8-arm `info!()` match is untouched.
- **No tracing dependency in test path:** The side-channel bypasses callsite caching, subscriber dispatch, and thread-local subscriber scoping. It cannot be poisoned by parallel tests.
- **Thread-local is scoped correctly:** The `new_current_thread` tokio runtime ensures all 4 `log_parse_retry_token_metrics` calls run on the test thread. No cross-thread leakage is possible.
- **Simpler test code:** The test shrinks — no `Registry`, no `Layer`, no `Visit` impl, no `with_default`.

### Why not single-callsite consolidation (previous Option A)

Collapsing the 8-arm match to one `info!()` was the previous recommendation. This is **insufficient** for two reasons:

1. **Residual flake risk:** If the sibling test thread executes `log_parse_retry_token_metrics` first and hits the single callsite before the metrics test's `with_default` scope is entered, `Interest::never()` is cached for that one callsite. Result: 0/4 events captured (worse than the current 1/4).

2. **Production log schema change:** Emitting `tokens_in = 0` with a companion `tokens_in_present = false` changes the production tracing output format, which conflicts with the acceptance criterion of no production output change. The `tracing::field::Empty` approach (which genuinely omits the field) requires the 8-arm match pattern.

Single-callsite consolidation is a valid simplification of `log_parse_retry_token_metrics` but does not fix the test flake and is therefore **out of scope** for this change.

## Files & Modules

| File | Lines | Change |
|------|-------|--------|
| `src/workflow/orchestrator.rs` | ~4660 (before `log_parse_retry_token_metrics`) | Add `#[cfg(test)] thread_local! METRICS_TEST_SINK` and `#[cfg(test)] struct TokenMetricsRecord` |
| `src/workflow/orchestrator.rs` | 4668 (top of `log_parse_retry_token_metrics`) | Add `#[cfg(test)]` block that writes to `METRICS_TEST_SINK` before the existing `match` |
| `src/workflow/orchestrator.rs` | 6523–6610 | Delete `ParseRetryMetricsEvent`, `MetricsVisitor`, `MetricsCaptureLayer` (no longer needed) |
| `src/workflow/orchestrator.rs` | 6667–6750 | Rewrite test to use `METRICS_TEST_SINK` instead of `MetricsCaptureLayer` + `with_default` |

No other files need modification. The `tracing` and `tracing-subscriber` dependency versions remain unchanged. The 8-arm `match` in `log_parse_retry_token_metrics` (lines 4669–4770) is **not modified**.

## Testing Strategy

### 1. Targeted parallel-interference verification (primary)

The specific failure mode is parallel execution of the two `parse_retry_attempts_` tests. Verify the fix eliminates this:

```bash
# Run both parse_retry_attempts_ tests together with default parallelism (multi-threaded).
# Repeat 20 times to confirm no intermittent failures.
for i in $(seq 1 20); do
  cargo test workflow::orchestrator::tests::parse_retry_attempts_ 2>&1 | tail -1
done
```

This must produce 20 consecutive passes. This command reproduces the flake on glibc prior to the fix.

### 2. Full module test suite

```bash
cargo test workflow::orchestrator::tests       # debug, glibc
cargo test --release workflow::orchestrator::tests  # release, glibc
```

Both must pass with no regressions across all 43 tests in the module.

### 3. Musl / nix build verification

```bash
# The original failure environment. Run 5 consecutive builds.
for i in $(seq 1 5); do nix build 2>&1 | tail -3; done
```

All 5 must pass. This is the environment where the flake was first observed.

### 4. Regression guard

- Confirm `ParseRetryMetricsEvent`, `MetricsVisitor`, and `MetricsCaptureLayer` have no other callers before deletion (already verified: only used by the one test).
- Confirm no other test references `METRICS_TEST_SINK` or `TokenMetricsRecord` (they are new, so no existing references exist).
- Confirm `parse_retry_attempts_are_three_without_session` continues to pass — it does not use tracing capture and is unaffected by this change.

## Out of Scope

- Upgrading `tracing` or `tracing-core` crate versions
- Changing the nix build configuration or musl target
- Modifying the tokio runtime setup (already correct: `new_current_thread`)
- Adding `#[serial]` or other test-ordering constraints
- Changing the `SequencedBackend` test mock or `BackendRegistry` behavior
- Refactoring `execute_with_parse_retries` beyond adding the `#[cfg(test)]` sink call
- Adding a global tracing subscriber in the test harness
- Changing the production log output format or the 8-arm `info!()` match structure
- Consolidating the 8-arm match to a single callsite (valid simplification but orthogonal to this fix)
- Wrapping `parse_retry_attempts_are_three_without_session` in a `with_default` subscriber (would fix the poisoning for *this* pair but not protect against future tests)
