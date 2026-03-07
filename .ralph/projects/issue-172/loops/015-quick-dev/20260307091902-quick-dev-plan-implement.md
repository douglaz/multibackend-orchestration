---
artifact: quick-dev-plan-implement
loop: 15
project: issue-172
backend: claude
role: implementer
created_at: 2026-03-07T09:19:02Z
---

# Pre-Commit Checks: Review Amendment Fixes

## Changes Made

### Amendment A1: Fix pipe deadlock in `run_command_with_timeout`

**File: `src/daemon/process.rs`**

**Problem**: `run_command_with_timeout` piped stdout/stderr but only read them after the process exited via `wait_with_output()`. If child output fills OS pipe buffers (~64KB), the child blocks on write, never exits, and gets killed on timeout — producing false failures that route passing code into repeated fix loops.

**Fix**: Spawn two reader threads that drain stdout/stderr concurrently while the process runs. The timeout loop polls `try_wait()` as before, but pipe buffers never fill because readers consume output in real time. On timeout or error, threads are joined to collect partial output. The `Output` struct is assembled manually from the collected buffers.

**Regression test**: `run_command_with_timeout_high_output_no_false_timeout` generates ~280KB of stdout via `seq 1 50000` (well above the 64KB pipe buffer) and asserts the command completes successfully without false timeout.

### Amendment A2: Strengthen `disabled_skips_checks` test

**File: `src/validate/tests_pre_commit_checks.rs`**

**Problem**: The test disabled fmt/clippy, but its mock script never created a `Cargo.toml`, so cargo checks were skipped regardless of config — the test passed for the wrong reason.

**Fix**: Added `disabled_checks_mock_script()` that creates a `Cargo.toml` + badly formatted `src/main.rs` (`fn main(){println!("hello");}`) in the worktree. With checks disabled, the loop commits despite the bad formatting. The existing `fmt_failure_triggers_reloop` test serves as the control case: the same kind of fixture with checks enabled triggers the pre-commit failure/reloop path.

### Test Results
- All 966 lib unit tests pass
- All 5 `workflow::pre_commit_checks` unit tests pass
- High-output regression test passes
- Compilation clean with no warnings
