The revised specification has been written. Here's a summary of how each review issue was addressed:

**Issue #1 — Auto-path feasibility**: Happy-path tests now use `auto_mock_script()` (which handles quick-PRD prompts) instead of `standard_mock_script()`. The mock ralph script invokes the real `ralph auto` binary, which correctly flows through spec writer → reviewer → reviser before orchestration.

**Issue #2 — QA/review rollback semantics**: Corrected. Rollback removes the loop entirely from `state.json` via `state.remove_loop()`. Tests renamed to `*_fails` (not `*_new_loop`). With `--loops 1`, after rollback there are 0 loops remaining, so the orchestrator exits non-zero and the daemon marks the task `failed`. Assertions check for empty loops array and absent loop directory.

**Issue #3 — Timeout coverage gap**: Added `backend_timeout_exhausted_fails_task` test. Uses `sleep 30` mock backend with `backends.<name>.timeout_seconds` set to 2 via config. Verifies `BackendTimeoutExhausted` propagation and `failed` task state.

**Issue #4 — Reformatter fallback scope**: Clarified that reformatter is parse-error-driven only. Non-zero exit → `BackendCommandFailed` (no reformatter). Timeout → `BackendTimeoutExhausted` (no reformatter). Empty output → same-backend retry then reformatter attempt. Tests match these semantics. Added explicit Out of Scope entry.

**Issue #5 — State transition completeness**: Acknowledged that mid-phase snapshots require binary instrumentation, which is out of scope. Final-state validation is the primary mechanism. Added explicit Out of Scope entry. Noted that `tests_run.rs` already covers per-phase transitions.

**Issue #6 — Implementation response format**: Added `impl_response_artifact_on_review_feedback` test. Verifies `*-impl-response-001.md` with correct YAML frontmatter (`artifact`, `iteration`, `role`) and body content.

**Issue #7 — Documentation**: Added `docs/validate-e2e.md` to Files & Modules and acceptance criteria. Spec describes 5 sections: architecture overview, test-to-requirement mapping, running instructions, troubleshooting, and adding-new-tests template.

**Issue #8 — Mock child recursion risk**: Added dedicated "Mock ralph script design" section. The mock `RALPH_DAEMON_BIN` embeds `h.ralph_bin` absolute path at generation time. New `e2e_mock_ralph_script(ralph_bin: &Path)` helper. Clear code example showing `exec /absolute/path/to/ralph auto "$@"`.

**Issue #9 — PR metadata verification**: Added `pr_metadata_verification` test and `e2e_mock_gh_logging_script()` that logs all `pr create` arguments and body-file contents. Assertions verify `--title` (ralph: prefix), `--body-file` (Closes #N, diff stat, project ref), `--head`, and `--repo`.