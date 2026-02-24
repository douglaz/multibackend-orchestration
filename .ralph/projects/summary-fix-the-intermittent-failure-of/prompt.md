### Objective
Fix the intermittent failure of `parse_retry_attempts_four_with_session_followup_and_token_metrics` by making parse-retry token-metrics logging deterministic across glibc and musl builds.

### Implementation Decision
Implement a single `info!()` callsite in `log_parse_retry_token_metrics` for all token-presence combinations.  
Do not implement side-channel capture or test-only production parameters.

### Scope
Primary edits should be in `src/workflow/orchestrator.rs` (logging function, metrics capture structs/visitor, and related assertions).  
Minimal adjacent edits are allowed only if required by compiler/tests.

### Required Changes
1. Refactor `log_parse_retry_token_metrics` so it emits exactly one tracing event callsite regardless of token `Option` combinations.
2. Preserve existing core context fields and message intent: `role`, `phase`, `loop_number`, `attempt`, `backend`, `session_reused`, and parse-retry normalization metric event message.
3. Standardize token representation in the event payload: always emit numeric `tokens_in`, `tokens_out`, `cached_in` (use `0` when absent), and always emit booleans `tokens_in_present`, `tokens_out_present`, `cached_in_present`.
4. Update `ParseRetryMetricsEvent` and `MetricsVisitor` to consume presence via `record_bool` flags rather than field-existence detection.
5. Update `parse_retry_attempts_four_with_session_followup_and_token_metrics` assertions to validate: exactly 4 events captured, correct attempt/session sequencing, correct token presence flags, and correct numeric values when present.
6. Keep retry/orchestration logic unchanged outside metrics logging and metrics-test capture.

### Non-Goals
- No tracing dependency upgrades.
- No nix/musl toolchain changes.
- No tokio runtime model changes.
- No serial-test ordering controls.
- No refactor of `execute_with_parse_retries` beyond metrics logging/capture implications.

### Acceptance Criteria
1. The target test passes in local glibc debug and release.
2. `workflow::orchestrator::tests` shows no regressions in debug and release.
3. `nix build` passes reliably; run 5 consecutive builds to validate flake removal.
4. The only intentional output semantic change is token representation (`0` defaults + `*_present` booleans).

### Validation Commands
1. `cargo test workflow::orchestrator::tests::parse_retry_attempts_four_with_session_followup_and_token_metrics`
2. `cargo test --release workflow::orchestrator::tests::parse_retry_attempts_four_with_session_followup_and_token_metrics`
3. `cargo test workflow::orchestrator::tests`
4. `cargo test --release workflow::orchestrator::tests`
5. `for i in 1 2 3 4 5; do nix build -L || exit 1; done`

### Delivery Output
1. Summarize code changes by function/struct touched.
2. Report pass/fail for each validation command.
3. Note any residual risk only if environment constraints prevented full validation.