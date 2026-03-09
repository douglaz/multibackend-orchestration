---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T20:13:11Z
---

# Apply Fixes for Review Feedback

## Changes Made

### P1: Daemon-level conformance test for env sanitization path
**File:** `src/validate/tests_daemon.rs`

Added `daemon::inprocess_env_sanitization` test that:
- Creates a daemon harness and configures a backend script that checks for `CLAUDECODE` in its environment and writes `SANITIZED` or `LEAKED:<value>` to a report file
- Sets `CLAUDECODE=should-not-leak` in the `daemon_env()` call
- Dispatches a daemon task in-process via `--single-iteration`
- Reads the report file and asserts `SANITIZED` is present and `LEAKED` is absent

This validates the full daemon dispatch → workspace load → orchestrator → backend subprocess chain, complementing the existing unit test in `src/backend/mod.rs:2552`.

### P2: Daemon-level conformance test for per-task log isolation
**File:** `src/validate/tests_daemon_concurrency.rs`

Added `daemon_concurrency::per_task_log_isolation` test that:
- Creates a backend script that emits a unique `TASK_MARKER_<PID>` per invocation and writes the marker to a marker directory file
- Dispatches two issues (800, 801) concurrently with `--max-concurrent 4`
- Locates each task's log file at `.ralph/tmp/logs/acme-widgets-{800,801}.log`
- Reads all marker files and checks which markers appear in which log files
- Asserts each log contains at least one marker from its own task
- Asserts no marker appears in both log files (cross-contamination check)

Both tests compile cleanly (`cargo check` passes).
