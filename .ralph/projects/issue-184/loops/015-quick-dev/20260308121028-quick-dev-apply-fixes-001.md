---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T12:10:28Z
---

# Applied Reviewer-Requested Fixes

## P1: Cancellation can fail to kill backend descendants

**File:** `src/backend/mod.rs` (lines 790–821)

**Problem:** `kill_guard.disarm()` was called *before* `collect_stderr(...).await`. If cancellation dropped the future during stderr drain, the guard was already disarmed and descendant processes could survive.

**Fix:** Restructured the `Completed(Ok(...))` branch:
1. Guard stays **armed** while draining stderr
2. Stderr drain is wrapped in `tokio::time::timeout(5s, ...)` — if descendants hold the pipe open, we don't hang indefinitely
3. On timeout, calls `kill_and_reap_child()` to SIGKILL the process group, then proceeds with empty stderr
4. `kill_guard.disarm()` is called only **after** all async cleanup, immediately before the synchronous return with no further `.await`

## P2: `RALPH_MAX_BACKEND_RETRIES` env reading removed from CLI paths

**Problem:** Four CLI modules still read `RALPH_MAX_BACKEND_RETRIES` from env, preserving old behavior instead of config/arg-driven control per spec #10.

**Fix:**
- **Removed** `parse_max_backend_retries_env()` from all 4 CLI files: `auto.rs`, `run.rs`, `quick_dev_auto.rs`, `quick_dev_run.rs`
- **Added** `--max-backend-retries` CLI arg (`Option<u8>`) to `AutoArgs`, `RunArgs`, `QuickDevAutoArgs`, `QuickDevRunArgs`
- CLI execute functions now pass `args.max_backend_retries` directly (or `None` for default 3)
- **Updated conformance tests** (`tests_e2e_conformance.rs`): switched from env var manipulation to `--max-backend-retries` CLI arg; the "invalid string" test now validates clap rejection
- **Updated streaming tests** (`tests_streaming.rs`): 3 occurrences of `ralph_env([...], &[("RALPH_MAX_BACKEND_RETRIES", "1")])` → `ralph(["run", ..., "--max-backend-retries", "1"])`
