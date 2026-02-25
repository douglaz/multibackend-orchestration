### Objective
Reduce conformance-suite wall-clock time in timeout/streaming-heavy tests without reducing behavioral coverage or weakening assertions.

### Scope
Implement only these changes:
1. Retry-count override for timeout retries.
2. Faster active-streaming mock timing.
3. Fast validate harness setup helpers using production Rust APIs (not CLI subprocesses).
4. Migration of `tests_streaming.rs` and `tests_e2e_conformance.rs` to fast helpers where semantically safe.
5. New/updated tests for retry override behavior and deterministic env-unset behavior.

Do not modify `tests_init.rs`, `tests_project.rs`, `tests_auto_init.rs`, or tests whose purpose is validating CLI command behavior.

### Required Behavior
For `RALPH_MAX_BACKEND_RETRIES` in `execute_with_timeout_retries`:
- Unset -> `3`
- `1..=10` -> exact value
- `0` -> reject and default to `3`
- Non-numeric -> default to `3`
- Numeric `>10` -> clamp to `10`

Implementation note: parse as an integer type that can represent values above `u8` so values like `11` and `256` are both treated as numeric and clamped to `10`.

### Required Code Changes
1. `src/workflow/orchestrator.rs`
- Add a helper (for example `max_backend_retries() -> u8`) implementing the behavior table above.
- Read retry count once per `execute_with_timeout_retries` invocation and reuse it for loop bounds and exhaustion checks.
- Replace hardcoded retry count `3` with the computed value in timeout retry logic.

2. `src/validate/mock_scripts.rs`
- In `active_streaming_planner_mock_script`, change active stream cadence from 8 chunks at `sleep 0.3` to 6 chunks at `sleep 0.2`.
- Keep timeout invariants valid: per-chunk interval below idle timeout and total stream duration above idle timeout.

3. `src/config/...` and `src/cli/config.rs`
- Extract global config mutation logic into a shared `pub(crate)` helper (for example `set_global_config_value`) near `GlobalConfig`.
- Refactor CLI config set path to delegate to this shared helper.
- Do not expose `cli::config` internals publicly.

4. `src/validate/harness.rs`
- Add fast helpers with stable names:
- `init_workspace_fast`
- `create_project_fast`
- `set_config_fast`
- `setup_mock_backends_fast`
- Add a command helper that supports env removals for child processes (for example `ralph_env_with_removals(..., env_removals)`).
- `set_config_fast` must target global scope only in this batch.

5. `src/validate/tests_streaming.rs`
- Migrate setup flow to fast helpers where behavior is equivalent.
- Update chunk assertions from `chunk-8` to `chunk-6`.
- Apply `RALPH_MAX_BACKEND_RETRIES=1` only in tests where reducing retry count does not change test intent.

6. `src/validate/tests_e2e_conformance.rs`
- Migrate setup flow to fast helpers where behavior is equivalent.
- Add conformance tests:
- unset var -> defaults to 3 (must explicitly remove env var in child process)
- set to `1` -> one attempt
- set to `0` -> defaults to 3
- set to invalid string -> defaults to 3
- Keep `11` clamp verification in unit tests, not conformance tests.

### Test Requirements
1. Unit tests in orchestrator module for env parsing/clamping matrix, including `11` and `256`.
2. Conformance tests for unset/1/0/invalid behavior using attempt-count assertions from deterministic planner logs.
3. Full conformance gate must pass:
- `./target/debug/ralph validate --bin ./target/debug/ralph` or equivalent built binary path in this repo.
4. No regressions in existing assertions outside expected `chunk-8` -> `chunk-6` updates.

### Performance Verification
Provide before/after timing evidence for affected tests on the same machine/run context.
- Target outcome: timeout-heavy cases materially reduced (typically ~6-7s to ~1-2s) and active-streaming cases roughly halved (~2.4s to ~1.2s).
- Treat timing as evidence, not a flaky hard CI assertion.

### Out of Scope
- Changing timeout durations or backoff strategy.
- Changing parse-retry envelope behavior.
- Parallelizing the validate framework.
- Batch 2+ migrations in other validate suites.
- Making `cli::config` module public.
- Introducing project-scoped fast config writes in this batch.

### Definition of Done
1. All required code changes are implemented in the specified modules.
2. New tests for retry override behavior are added and passing.
3. Full validate command passes.
4. Implementation notes include:
- list of changed files
- summary of semantic parity decisions
- before/after timing evidence for migrated timeout/streaming tests