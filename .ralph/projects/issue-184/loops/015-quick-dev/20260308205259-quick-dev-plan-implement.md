---
artifact: quick-dev-plan-implement
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T20:52:59Z
---

# Final Review Amendments — Implementation Complete

All three reviewer amendments have been addressed:

## [P1] Keep Backend Kill Guard Armed Until Non-zero Exit Cleanup
**File:** `src/backend/mod.rs`

Moved `kill_guard.disarm()` from before the `status.success()` check to after it:
- **Non-zero exit path:** Now calls `kill_and_reap_child()` to terminate the process group (including detached descendants) before disarming the guard
- **Success path:** Disarms after all async cleanup is done, as before
- **Regression test added:** `nonzero_exit_with_detached_child_kills_process_group` — spawns a backend that exits 1 after creating a detached child, verifies the descendant is killed

## [P2] Restore E2E Assertions in `pr_metadata_verification`
**File:** `src/validate/tests_e2e_conformance.rs`

Added assertions that check the `gh_log_path` file created by the mock GH logging script:
- If the file exists (daemon called `gh pr create`), verifies `--title`, `--head`, and `--body-file` args are present
- Validates body content contains issue closure marker and Issue Context section
- Existing helper-only assertions are preserved below the E2E assertions

## [P2] Remove Global Env Race in `e2e_pr_create_body_file_verification`
**File:** `src/validate/tests_e2e_conformance.rs`

Replaced bare `set_var`/`remove_var` calls with:
- **`env_mutex()`** — a `OnceLock<Mutex<()>>` that serializes env mutations across parallel tests
- **`EnvGuard`** — RAII struct that captures the previous env var value on construction and restores it on drop (even on panic)
- The test now acquires the mutex lock and creates `EnvGuard` instances for `PATH` and `RALPH_E2E_GH_LOG` — both are automatically restored when the test exits
