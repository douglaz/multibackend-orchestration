---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T06:21:16Z
---

# Apply-Fixes: Reviewer-Requested Changes

All 5 reviewer-requested changes have been applied. Here's a summary of each fix:

## Fix 1 (High): Backend CWD for orchestrator execution
**Files:** `orchestrator.rs`, `quick_dev_orchestrator.rs`

Added `registry.set_cwd(self.workspace.root.parent().map(|p| p.to_path_buf()))` immediately after `BackendRegistry::new()` in both orchestrators, ensuring backend subprocesses run in the correct worktree directory.

## Fix 2 (High): `RALPH_MAX_BACKEND_RETRIES` env removal + quick-dev retry threading
**Files:** `orchestrator.rs`, `quick_dev_orchestrator.rs`

- Removed env var fallback from `max_backend_retries()` — now uses `None` → default 3, `Some(v)` → clamped to 10.
- Added `max_backend_retries: Option<u8>` parameter to `run_phase_machine()` and `execute_backend()` in quick-dev orchestrator.
- Implemented retry logic in quick-dev's `execute_backend()` with exponential backoff (matching the orchestrator pattern).
- Added local `max_backend_retries()` helper to quick-dev module.
- Updated all 5 `execute_backend` call sites to pass the parameter through.
- Migrated unit tests from env-var-based to `configured` parameter-based assertions.

## Fix 3 (High): Phase-loop cancellation checks + graceful kill sequence
**Files:** `orchestrator.rs`, `quick_dev_orchestrator.rs`, `backend/mod.rs`

- Added `if self.cancel.is_cancelled() { return Err(RalphError::Cancelled); }` at the top of both phase loops.
- Changed `KillOnDrop` from immediate SIGKILL to graceful sequence: SIGTERM → poll `waitpid(WNOHANG)` for up to 5 seconds → SIGKILL + reap.

## Fix 4 (Medium): CWD-dependent API removal
**Files:** `cli/auto.rs`, `cli/quick_dev_auto.rs`, `cli/quick_prd.rs`, `prd/quick.rs`, `orchestrator.rs`

- Replaced `registry.set_cwd(Some(std::env::current_dir()?))` with `registry.set_cwd(Some(repo_root.clone()))` (derived from `workspace.root.parent()`) in both CLI entry points.
- Replaced `quick_prd.run()` with `quick_prd.run_in(repo_root)` in all 3 CLI callers.
- Removed the zero-arg `run()` method from `QuickPrdPipeline` (which called `current_dir()`), making `run_in()` the sole public API.
- Removed `current_dir()` fallback in `loop_dir_hint` computation.

## Fix 5 (Medium): Daemon conformance test migration
**File:** `validate/tests_daemon.rs`

Migrated all 4 dispatch variant tests from subprocess arg-capture assertions to in-process stderr-based assertions:
- `quick_label_fresh_dispatches_quick_dev_auto` — asserts `stderr.contains("starting fresh with quick-dev-auto")`
- `quick_label_resume_dispatches_quick_dev_run` — asserts `stderr.contains("resuming with quick-dev-run")`
- `no_quick_label_fresh_dispatches_auto` — asserts `stderr.contains("starting fresh with auto")`
- `no_quick_label_resume_dispatches_run` — asserts `stderr.contains("resuming with run")`

Removed `RALPH_DAEMON_BIN`, mock ralph scripts, and args-log file reads from all 4 tests.
