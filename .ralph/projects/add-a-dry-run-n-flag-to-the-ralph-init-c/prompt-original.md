Here's a summary of how the revised spec addresses each review issue:

**Issue 1 — Missing validate conformance tests:** Added a full "Validate conformance tests" subsection to the Testing Strategy. Four new tests are registered in `src/validate/tests_init.rs`: `dry_run_prints_actions` (checks all stdout lines and that no files are created), `dry_run_short_flag` (confirms `-n` works), `dry_run_rejects_nonempty_dir` (exit code 2 parity), and `dry_run_rejects_file_target` (exit code 1 for non-directory target). All use the existing harness pattern (`catch_unwind` + `TestResult`) and assertion helpers (`assert_stdout_contains`, `assert_path_not_exists`, `assert_exit_code`).

**Issue 2 — Behavior fidelity / drift risk:** Replaced the hard-coded `print_dry_run` function with a shared action-plan approach. Introduced `TEMPLATE_FILES` and `LEGACY_LINKS` constants, plus an `InitAction` enum and `plan_actions()` function. Both the dry-run path (iterates actions to print) and the real `create_workspace` path (iterates the same constants to write) use the same source of truth. If a template or symlink is added/removed, both paths update automatically.

**Issue 3 — Edge cases and output parity:** Expanded acceptance criteria to explicitly specify:
- Dry-run does **not** print `initialized workspace at ...`
- Validation parity for three edge cases: non-empty directory (exit 2), non-directory target (exit 1), unreadable target (exit 1)
- Added `validate_target()` extraction so the dry-run path runs identical validation before the branch
- Added corresponding test cases for each edge case