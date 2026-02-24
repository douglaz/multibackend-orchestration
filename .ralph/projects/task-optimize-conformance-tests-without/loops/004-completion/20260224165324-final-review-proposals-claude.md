---
artifact: final-review-proposals
loop: 4
project: task-optimize-conformance-tests-without
backend: claude
role: final_reviewer
created_at: 2026-02-24T16:53:24Z
---

# Final Review: NO AMENDMENTS

## Summary

All required code changes are implemented correctly and verified through source code reading:

**1. `src/workflow/orchestrator.rs`** — The `max_backend_retries()` helper (line 5449) correctly implements the full behavior table: unset → 3, valid 1–10 → exact value, 0 → 3, non-numeric → 3, >10 → clamped to 10. It parses as `u32` (handles values like 11 and 256 without overflow) then clamps and casts to `u8`. The `execute_with_timeout_retries` function (line 5366) reads the retry count once per invocation and uses it for both the loop bound (`1..=max_retries`) and the exhaustion check (`attempt >= max_retries`). Nine unit tests with `#[serial]` + `with_retry_env_var` properly restore env state.

**2. `src/validate/mock_scripts.rs`** — `active_streaming_planner_mock_script` changed from 8 chunks at `sleep 0.3` to 6 chunks at `sleep 0.2`. Timing invariants hold: 0.2s < 1s idle timeout, 1.2s total > 1s timeout.

**3. `src/config/global.rs` + `src/config/mod.rs`** — `set_global_config_value` (line 1099) is a comprehensive `pub(crate)` helper that handles all global config keys. `config/mod.rs` re-exports it as `pub(crate)`. The `cli/config.rs` `set_global_value` function (line 382) delegates to it. The `cli::config` module remains private.

**4. `src/validate/harness.rs`** — All four fast helpers are implemented with stable names: `init_workspace_fast` (calls `crate::cli::init::create_workspace`), `create_project_fast` (calls `crate::project::lifecycle::create_project`), `set_config_fast` (calls `set_global_config_value` + `workspace.save_config()`), and `setup_mock_backends_fast` (uses `set_config_fast`). The `ralph_env_with_removals` method (line 316) supports explicit env var removal via `command.env_remove()`. Two harness unit tests verify `env_remove` behavior.

**5. `src/validate/tests_streaming.rs`** — All tests migrated from CLI helpers (`init_workspace`, `setup_mock_backends`, `create_project`) to fast helpers. Chunk assertions updated from `chunk-8` to `chunk-6`. `RALPH_MAX_BACKEND_RETRIES=1` applied only in timeout/cleanup tests (`timeout_cleanup`, `hanging_stall_timeout`, `codex_hanging_stall_timeout`) where reducing retry count doesn't change test intent.

**6. `src/validate/tests_e2e_conformance.rs`** — Four conformance tests added: unset→3 (using `ralph_env_with_removals`), set to 1→1, set to 0→3, invalid string→3. The `assert_planner_attempt_count` helper counts `--- attempt=` separators in planner logs. The `11` clamp test is correctly kept in unit tests only, not conformance. Non-retry tests also migrated to fast helpers.

**7. Cross-cutting verification** — Excluded files (`tests_init.rs`, `tests_project.rs`, `tests_auto_init.rs`) are unmodified. No stray files. `serial_test` correctly in `[dev-dependencies]`. The `chunk-8` in `backend/mod.rs:1972` is an independent low-level unit test, not affected by the conformance mock change. `setup_mock_backends_fast` has semantic parity with the original `setup_mock_backends` (both set only the command, not args). The `pr_metadata_verification` test correctly retains `setup_mock_backends_stable` for Nix-compatible daemon testing.
