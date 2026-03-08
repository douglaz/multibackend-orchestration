---
artifact: quick-dev-plan-implement
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T15:56:48Z
---

# Implementation Complete

All three amendments from the final review have been addressed:

## [P0] Cancellation Returns Before Backend Subprocesses Are Reaped

**Problem:** `KillOnDrop` spawned a detached reaper thread, allowing the orchestrator to return `Cancelled` before backend processes were actually dead.

**Fix:**
1. **Added `execute_with_cancel` to the `Backend` trait** (`src/backend/mod.rs:139-154`) with a default implementation that races `execute_with_log` against the cancellation token. `CliBackend` overrides it to handle cancellation internally.

2. **Integrated cancellation into `execute_streaming`** (`src/backend/mod.rs:523`) — added `cancel: &CancellationToken` parameter and a third `Cancelled` branch in the internal `tokio::select!`. On cancellation, the function calls `kill_and_reap_child()` (async, awaited) before returning, guaranteeing backend processes are dead.

3. **Simplified `KillOnDrop` to SIGKILL-only emergency fallback** (`src/backend/mod.rs:39-73`) — no more detached reaper thread. This only fires on unexpected future drops (task abort), not on normal cancellation.

4. **Updated orchestrator call sites** — `orchestrator.rs:6097` and `quick_dev_orchestrator.rs:1440` now call `backend.execute_with_cancel()` instead of wrapping `execute_with_log` in `tokio::select!`.

## [P2] Concurrency Test Ordering Assertions

**Problem:** `concurrent_dispatch_evidence` assertions didn't prove concurrent execution.

**Fix:** Added ordering constraint (`tests_daemon_concurrency.rs:663-694`) — verifies that the LAST dispatch message appears at a byte offset BEFORE the FIRST terminal-state message. Under sequential execution, this ordering is impossible.

## [P2] Mixed-Outcome Dispatch Isolation

**Problem:** Dispatch-failure isolation tests expected both issues to fail.

**Fix:** Added `mixed_outcome_claim_isolation` test (`tests_daemon_concurrency.rs:799-893`) with a `daemon_mock_gh_mixed_outcome_script` (`mock_scripts.rs:4039-4163`) where `MOCK_GH_CLAIM_FAIL_ISSUE=901` causes claim failure for issue 901 while issue 900 proceeds. Asserts:
- Issue 900 dispatched, 901 not dispatched
- Issue 900 reaches terminal state independently
- No `ralph:failed` label for 901 (never claimed)

**All 966 unit tests + 28 integration tests pass with 0 failures.**
