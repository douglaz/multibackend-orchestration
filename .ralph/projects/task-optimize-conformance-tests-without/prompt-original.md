Now I have all the context needed. Let me write the revised specification addressing each review issue.

Here is the revised specification addressing all six review issues:

---

## Summary

Reduce conformance test suite wall-clock time by addressing three independent bottlenecks—timeout retry count, streaming mock sleep durations, and per-test setup overhead—without weakening any test assertions, coverage, or semantic guarantees. The timeout/streaming tests currently spend 6-7s per test case due to 3 retry attempts against 1s timeouts and 2.4s streaming durations; this can be reduced to 1-2s by allowing test-scoped retry overrides and tightening mock sleep intervals while preserving the invariants they prove.

## Acceptance Criteria

1. `RALPH_MAX_BACKEND_RETRIES` environment variable is read in `execute_with_timeout_retries`, clamped to `1..=10`, defaults to 3 when unset or invalid. **Value `0` is rejected and defaults to 3** (not clamped to 1), matching the "reject 0" requirement.
2. New conformance tests cover: unset (defaults to 3), set to `1`, set to `0` (rejected, defaults to 3), set to non-numeric (defaults to 3). The value `11` clamping is verified via a unit test (not a conformance test) due to backoff cost.
3. `active_streaming_planner_mock_script` reduced from `sleep 0.3` x8 to `sleep 0.2` x6; chunk assertion updated from `chunk-8` to `chunk-6`; timing invariants (`0.2s < 1s timeout`, total `1.2s > 1s timeout`) still hold.
4. Fast harness helpers (`init_workspace_fast`, `create_project_fast`, `setup_mock_backends_fast`, `set_config_fast`) implemented in `harness.rs`, calling production code paths from `src/cli/init.rs` (`create_workspace`), `src/project/lifecycle.rs` (`create_project`), and directly mutating `GlobalConfig`/`ProjectConfig` via their public APIs—without depending on private `cli::config` internals.
5. `tests_streaming.rs` and `tests_e2e_conformance.rs` migrated to use fast helpers and `RALPH_MAX_BACKEND_RETRIES=1` where semantically appropriate.
6. `./target/debug/ralph validate --bin ./target/debug/ralph` passes with no regressions.
7. Measurable runtime reduction: timeout tests from ~6-7s to ~1-2s; streaming active tests from ~2.4s to ~1.2s.
8. No changes to `tests_init.rs`, `tests_project.rs`, `tests_auto_init.rs`, or config CLI behavior tests.

## Technical Approach

### 1. `RALPH_MAX_BACKEND_RETRIES` environment variable (orchestrator.rs)

**Location**: `execute_with_timeout_retries` function at `src/workflow/orchestrator.rs:5366`.

**Change**: Replace the hardcoded `1..=3_u8` loop bound with a value read from `std::env::var("RALPH_MAX_BACKEND_RETRIES")`:

```rust
fn max_backend_retries() -> u8 {
    match std::env::var("RALPH_MAX_BACKEND_RETRIES") {
        Ok(val) => match val.parse::<u8>() {
            Ok(n) if (1..=10).contains(&n) => n,
            Ok(n) if n > 10 => 10,  // clamp >10 → 10
            _ => 3,                  // 0, negative, or non-numeric → default
        },
        Err(_) => 3,                 // unset → default
    }
}
```

**Key design decision (addresses review issue #1)**: `0` is treated identically to non-numeric input — it falls through to the default of `3`. The requirement says "reject 0, default to 3", so `0` is _rejected_ (not silently clamped to a valid value). This avoids the ambiguity between "clamp to 1" (which would silently change `0` to a functional value) versus "reject" (which preserves the documented default). The `_ => 3` arm handles `Ok(0)`, `Ok(n)` where n would be negative (impossible for u8), and `Err(_)` from `parse` (non-numeric).

Line 5394 changes from `for attempt in 1..=3_u8` to `for attempt in 1..=max_backend_retries()`. The `attempt == 3` exhaustion check at line 5408 becomes `attempt == max_retries` (capture the return value before the loop).

**Why this is safe**: The retry count only controls how many times a timed-out backend is re-invoked. Reducing it to 1 in tests means a single timeout → exhaustion, which is exactly what the timeout-exhaustion test proves. The 3-attempt default is preserved for production and all existing non-overridden tests.

### 2. Streaming mock timing reduction (mock_scripts.rs)

**Location**: `active_streaming_planner_mock_script` at `src/validate/mock_scripts.rs:2331`.

**Change**: Modify the loop from `seq 1 8` with `sleep 0.3` to `seq 1 6` with `sleep 0.2`.

Invariant verification:
- Per-chunk interval: `0.2s < 1s` (idle timeout configured in tests) — satisfied
- Total runtime: `6 × 0.2s = 1.2s > 1s` (exceeds timeout) — satisfied
- Total reduction: `2.4s → 1.2s` per invocation

**Assertion update** in `tests_streaming.rs`: Change `content.contains("chunk-8")` to `content.contains("chunk-6")` at lines 380 and 599. The elapsed lower-bound check `elapsed >= Duration::from_millis(1200)` at line 504 remains valid (1.2s ≥ 1.2s).

### 3. Fast harness helpers (harness.rs)

**Location**: New methods on `RalphHarness` in `src/validate/harness.rs`.

These helpers replace the pattern of shelling out to `ralph init`, `ralph project new`, `ralph config set` with direct Rust function calls into the same production code paths. Each test currently executes 5-8 separate ralph binary invocations for setup (init + 3-4 config sets + project new), each paying process spawn + workspace discovery overhead.

**`init_workspace_fast()`**: Calls `crate::cli::init::create_workspace(&self.repo_root.join(".ralph"))`. The `cli::init` module is `pub mod` and `create_workspace` is `pub(crate)`, so it is directly accessible from `validate::harness`. The function takes a `&Path` pointing to the `.ralph` directory (not the repo root), validates the target is empty/nonexistent, plans and executes init actions, and returns a `Workspace`. This matches the semantics of `h.ralph_ok(["init"])` which runs with `current_dir(&self.repo_root)` and `InitArgs.dir` defaults to `.ralph` (relative). **(Addresses review issue #2)**: The path passed is `self.repo_root.join(".ralph")`, not `self.repo_root` itself — `validate_target` requires the directory to be empty or nonexistent, and the repo root is a non-empty git repo.

**`create_project_fast(id, name, prompt)`**: Writes the prompt file to a temp path, constructs a `Workspace` by calling `Workspace::load(self.repo_root.join(".ralph"))`, then calls `crate::project::lifecycle::create_project(&workspace, CreateProjectOptions { id, name, source: PromptSource::File(prompt_path), starting_backend: None })`. Both `create_project`, `CreateProjectOptions`, and `PromptSource` are `pub` in `src/project/lifecycle.rs`. The workspace is loaded from `repo_root/.ralph` (not discovered from cwd), matching the `.ralph` directory created by `init_workspace_fast`. **(Addresses review issue #2)**: Workspace is loaded from the known `.ralph` path, not from `self.repo_root`.

**`setup_mock_backends_fast(script)`**: Calls `set_config_fast` for `backends.claude.command`, `backends.codex.command`, and `backends.gemini.enabled=false`, all with global scope. This reuses the same config mutation path.

**`set_config_fast(key, value)`**: **(Addresses review issues #3 and #5)**: Rather than calling the private `cli::config::execute_set` function (which is inaccessible because `mod config` is private in `src/cli/mod.rs` and `ConfigScope` is a private enum within `config.rs`), this helper directly loads the `Workspace` from the known `.ralph` path and mutates its `config: GlobalConfig` field using `GlobalConfig`'s public field access and `Workspace::save_config()`. Specifically:

```rust
pub fn set_config_fast(&self, key: &str, value: &str) -> Result<()> {
    let ralph_dir = self.repo_root.join(".ralph");
    let mut workspace = Workspace::load(ralph_dir)?;
    // Use the same alias resolution as CLI
    let key = match key {
        "planner_backend" => "workflow.planner_backend",
        "qa_backend" => "workflow.qa_backend",
        k => k,
    };
    set_global_config_value(&mut workspace.config, key, value)?;
    workspace.save_config()?;
    Ok(())
}
```

The `set_global_config_value` is a new `pub(crate)` helper extracted from the existing private `set_global_value` logic in `cli/config.rs` into a shared location (e.g., `src/config/global.rs` where `GlobalConfig` is defined). This function contains the `match key { ... }` dispatch that maps dotted keys to `GlobalConfig` field mutations. Moving this logic is preferable to making the entire `cli::config` module public, since it places config mutation next to the config type definition and avoids exposing CLI-internal types (`ConfigScope`, `resolve_scope`, etc.).

**Scope semantics (addresses review issue #5)**: All existing `setup_mock_backends` calls use `--global` explicitly. The migrated `set_config_fast` helper operates exclusively on global scope, matching these calls exactly. For the inline `h.ralph_ok(["config", "set", key, value])` calls in test setup (e.g., setting `backends.claude.timeout_seconds`), we audit each call site: in `tests_streaming.rs` and `tests_e2e_conformance.rs`, all config-set calls in test setup either (a) use `--global` explicitly, or (b) run _before_ any project is created/activated (so `resolve_scope` would default to global anyway since there is no active project). Therefore, migrating these to `set_config_fast` (global scope) preserves identical semantics. Any call that runs _after_ `create_project` (which auto-activates the project) and _without_ `--global` would need project-scope handling — but no such calls exist in the batch 1 migration targets.

**Visibility requirements**: `cli::init::create_workspace` is already `pub(crate)`. `project::lifecycle::create_project`, `CreateProjectOptions`, and `PromptSource` are already `pub`. `Workspace::load` and `Workspace::save_config` are already `pub`. The only new visibility change is extracting `set_global_config_value` as `pub(crate)` in `src/config/global.rs` (or a nearby shared module) from the existing private `set_global_value` in `cli/config.rs`.

### 4. Incremental migration

**Batch 1** (this PR): `tests_streaming.rs` and `tests_e2e_conformance.rs`
- Replace `h.init_workspace()` → `h.init_workspace_fast()`
- Replace `h.setup_mock_backends(&script)` → `h.setup_mock_backends_fast(&script)`
- Replace `h.create_project(...)` → `h.create_project_fast(...)`
- Replace `h.ralph_ok(["config", "set", ...])` → `h.set_config_fast(key, value)` — only for setup calls verified to use global scope (see scope audit above)
- Add `RALPH_MAX_BACKEND_RETRIES=1` via `h.ralph_env(...)` for timeout tests that don't specifically test retry counts

**Not migrated** in this PR: `tests_init.rs`, `tests_project.rs`, `tests_auto_init.rs`, and any tests whose purpose is verifying CLI invocation behavior (e.g., exit codes from `ralph init` in invalid directories).

## Files & Modules

| File | Change |
|------|--------|
| `src/workflow/orchestrator.rs` | Add `max_backend_retries()` fn; replace hardcoded `3` in `execute_with_timeout_retries` loop (line 5394) and exhaustion check (line 5408) |
| `src/validate/mock_scripts.rs` | `active_streaming_planner_mock_script`: change `seq 1 8`/`sleep 0.3` → `seq 1 6`/`sleep 0.2` (lines 2340-2343) |
| `src/validate/harness.rs` | Add `init_workspace_fast()`, `create_project_fast()`, `setup_mock_backends_fast()`, `set_config_fast()` methods; add `use` imports for `crate::cli::init::create_workspace`, `crate::project::lifecycle::{create_project, CreateProjectOptions, PromptSource}`, `crate::workspace::Workspace` |
| `src/validate/tests_streaming.rs` | Migrate 9 tests to fast helpers; update `chunk-8` → `chunk-6` assertions (lines 380, 599); add `RALPH_MAX_BACKEND_RETRIES=1` env for timeout tests |
| `src/validate/tests_e2e_conformance.rs` | Migrate 6 tests to fast helpers; add `RALPH_MAX_BACKEND_RETRIES=1` env for `backend_timeout_exhausted_fails_task`; add new retry-override conformance tests |
| `src/config/global.rs` (or equivalent `GlobalConfig` location) | Extract `pub(crate) fn set_global_config_value(config: &mut GlobalConfig, key: &str, value: &str) -> Result<()>` from the logic currently in `cli/config.rs:set_global_value` |
| `src/cli/config.rs` | Refactor `set_global_value` to delegate to the new shared `set_global_config_value` fn (no external API changes) |

## Testing Strategy

### New conformance tests for `RALPH_MAX_BACKEND_RETRIES`

Add to `tests_e2e_conformance.rs`:

1. **`retry_override_unset_defaults_to_three`**: Run timeout test **with `RALPH_MAX_BACKEND_RETRIES` explicitly removed from the child process environment** (via `Command::env_remove("RALPH_MAX_BACKEND_RETRIES")` or equivalent harness support), verify 3 attempt separators in planner log (`--- attempt=` count). **(Addresses review issue #6)**: The test must not rely on the host environment lacking this variable; it must explicitly unset it in the child process to be deterministic.

2. **`retry_override_set_to_one`**: Set `RALPH_MAX_BACKEND_RETRIES=1`, verify exactly 1 attempt separator and `BackendTimeoutExhausted` in stderr.

3. **`retry_override_zero_rejected_defaults_to_three`**: Set `RALPH_MAX_BACKEND_RETRIES=0`, verify 3 attempts (rejected, uses default). **(Addresses review issue #1)**: This test name and assertion match the "reject 0, default to 3" contract — not "clamp to 1".

4. **`retry_override_invalid_defaults_to_three`**: Set `RALPH_MAX_BACKEND_RETRIES=abc`, verify 3 attempts (default).

5. **(Addresses review issue #4)**: The `RALPH_MAX_BACKEND_RETRIES=11` clamping case is **not** a conformance test. With exponential backoff (`2^(attempt-1)` seconds between retries), 10 timeout retries would incur `1+2+4+8+16+32+64+128+256+512 = 1023 seconds` of backoff sleep alone, making a conformance test impractical even with a fast-succeeding mock (the mock succeeds immediately, but the backoff sleeps still execute between retry attempts). Instead, add a **unit test** for `max_backend_retries()` in `orchestrator.rs` that validates all clamping/rejection logic without invoking the full retry loop:

```rust
#[cfg(test)]
mod tests {
    use super::max_backend_retries;
    use std::env;

    #[test]
    fn retry_override_parsing() {
        // Unset
        env::remove_var("RALPH_MAX_BACKEND_RETRIES");
        assert_eq!(max_backend_retries(), 3);

        // Valid range
        env::set_var("RALPH_MAX_BACKEND_RETRIES", "1");
        assert_eq!(max_backend_retries(), 1);
        env::set_var("RALPH_MAX_BACKEND_RETRIES", "10");
        assert_eq!(max_backend_retries(), 10);

        // Clamping >10
        env::set_var("RALPH_MAX_BACKEND_RETRIES", "11");
        assert_eq!(max_backend_retries(), 10);
        env::set_var("RALPH_MAX_BACKEND_RETRIES", "255");
        assert_eq!(max_backend_retries(), 10);

        // Rejection of 0
        env::set_var("RALPH_MAX_BACKEND_RETRIES", "0");
        assert_eq!(max_backend_retries(), 3);

        // Non-numeric
        env::set_var("RALPH_MAX_BACKEND_RETRIES", "abc");
        assert_eq!(max_backend_retries(), 3);

        // Cleanup
        env::remove_var("RALPH_MAX_BACKEND_RETRIES");
    }
}
```

Note: This unit test must run serially (not in parallel with other tests that read this env var) since `env::set_var` is process-global. Use `#[serial]` from the `serial_test` crate if parallel unit test execution is a concern, or accept serialization since it's a single fast test.

### Harness support for env-remove (addresses review issue #6)

The existing `ralph_env` method adds env vars but cannot _remove_ them. Add a `ralph_env_remove` helper or extend `ralph_env` to accept an additional `remove: &[&str]` parameter so that the "unset" conformance test can call `command.env_remove("RALPH_MAX_BACKEND_RETRIES")` on the child process. Alternatively, add a dedicated `ralph_clean_env` method that calls `command.env_clear()` and then re-adds only the minimal required variables (PATH, HOME, etc.) — though this is more invasive. The simplest approach is:

```rust
pub fn ralph_env_with_removals<I, S>(
    &self,
    args: I,
    env_vars: &[(&str, &str)],
    env_removals: &[&str],
) -> Result<Output>
```

### Streaming timing verification

After mock timing change, run `active_stream_no_timeout` and `codex_active_stream_no_timeout` and verify:
- `chunk-6` present in log (all chunks emitted)
- No timeout footer
- `elapsed >= Duration::from_millis(1200)` (total runtime exceeds timeout)

### Migration parity

For each migrated test, verify identical assertions pass before and after migration. The migration is mechanical (replacing CLI invocations with direct calls) so the assertions themselves do not change (except the `chunk-8` → `chunk-6` update which is a mock change, not a harness change).

### Full suite gate

All changes gated on: `./target/debug/ralph validate --bin ./target/debug/ralph` passing end-to-end.

## Out of Scope

- **Batch 2-4 migrations** (`tests_run.rs`, `tests_commands.rs`, remaining suites, daemon suites): tracked for follow-up PRs.
- **CLI behavior tests**: `tests_init.rs`, `tests_project.rs`, `tests_auto_init.rs`, and config CLI tests that exist to verify the CLI interface itself must continue to shell out to the binary.
- **Parallelizing test execution**: test parallelism is a separate concern from per-test overhead.
- **Changing timeout values or backoff strategy**: we only add an override for retry _count_, not for timeout duration or backoff multiplier.
- **Changing the 4-attempt parse-retry strategy** in `execute_with_parse_retries`: `RALPH_MAX_BACKEND_RETRIES` only affects the _inner_ `execute_with_timeout_retries` loop, not the outer parse-retry envelope.
- **Async test harness**: the fast helpers call sync production code directly; no async test runtime changes needed.
- **Modifying `idle_timeout_reset_planner_mock_script` or `timeout_hanging_planner_mock_script` timing**: only `active_streaming_planner_mock_script` is modified since it has the most margin for reduction while preserving invariants.
- **Making `cli::config` module public**: the `config` module in `src/cli/mod.rs` is `mod config` (private) and exposes CLI-internal types like `ConfigScope`. Rather than broadening its visibility, we extract the config-mutation logic into the shared `config` crate where `GlobalConfig` lives.
- **Project-scoped `set_config_fast`**: batch 1 migration targets only require global-scope config mutations. Project-scoped fast config helpers will be added in a follow-up if needed by batch 2-4 migrations.
- **Test-only backoff override**: a `RALPH_BACKOFF_OVERRIDE` env var to speed up retry backoff in tests is not included in this PR. The conformance test for `RALPH_MAX_BACKEND_RETRIES=11` clamping is handled by unit tests precisely to avoid this dependency.

---

### Review Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| **#1 (0 contract)** | `0` is rejected (defaults to 3), not clamped to 1. Acceptance criteria #1, `max_backend_retries()` implementation, and test `retry_override_zero_rejected_defaults_to_three` all updated. |
| **#2 (init path)** | `init_workspace_fast()` passes `self.repo_root.join(".ralph")` to `create_workspace`, not `self.repo_root`. `create_project_fast`/`set_config_fast` load workspace via `Workspace::load(self.repo_root.join(".ralph"))`. |
| **#3 (private API)** | `set_config_fast` does NOT call `cli::config::execute_set`. Instead, config mutation logic is extracted into a `pub(crate) set_global_config_value` in the `config` crate where `GlobalConfig` lives. `cli::config::set_global_value` is refactored to delegate to this shared function. |
| **#4 (retry test runtime)** | `retry_override_eleven_clamped_to_ten` moved from conformance to unit test. Backoff of 1023s makes conformance testing infeasible. Unit test validates `max_backend_retries()` parsing directly. |
| **#5 (scope parity)** | `set_config_fast` uses global scope exclusively. Audit confirms all batch 1 migration call sites either pass `--global` or run before project activation (defaulting to global). Documented scope equivalence for each migrated call pattern. |
| **#6 (env determinism)** | `retry_override_unset_defaults_to_three` explicitly removes `RALPH_MAX_BACKEND_RETRIES` from child environment. New `ralph_env_with_removals` harness helper supports `env_remove`. |