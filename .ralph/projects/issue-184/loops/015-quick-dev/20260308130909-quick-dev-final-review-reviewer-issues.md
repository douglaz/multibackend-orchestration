---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T13:09:09Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Preserve PGID-Based Cleanup After Child Exit

### Problem
`CliBackend::execute_streaming` waits for child completion before stderr drain (`src/backend/mod.rs:735-742`).  
If stderr drain times out (`src/backend/mod.rs:794-809`), it calls `kill_and_reap_child`, but that function only group-kills when `child.id()` is present (`src/backend/mod.rs:887-923`). After `wait()`, `child.id()` can be unavailable, so fallback path only does `child.kill()` (`src/backend/mod.rs:924-932`), which does not kill surviving descendants. The guard is then disarmed (`src/backend/mod.rs:814`), so leaked descendants can persist.

### Proposed Change
Store the spawned process-group leader PID/PGID at spawn time and use that stored PGID for timeout cleanup, even if `child.id()` is unavailable later.  
In the stderr-timeout branch, perform explicit PGID SIGTERM/SIGKILL cleanup based on stored PGID before disarming the drop guard. Add a regression test where leader exits but a descendant keeps stderr open, and assert descendant termination.

### Affected Files
- `src/backend/mod.rs` - keep and use spawned PGID for post-exit cleanup; add regression test for descendant leak path.

## Amendment: [P2] Timeout-Abort Path in `drain_all_children` Is Not Actually Tested

### Problem
Test `drain_all_children_aborts_stuck_tasks_after_timeout` (`src/daemon/runtime.rs:3952`) claims timeout-abort behavior, but the spawned task exits immediately after cancellation (`src/daemon/runtime.rs:3995-3999`). `drain_all_children` cancels all tasks at entry (`src/daemon/runtime.rs:2001-2004`), so this test only verifies cooperative cancellation and never exercises deadline/`join_handle.abort()` fallback (`src/daemon/runtime.rs:2014-2018`).

### Proposed Change
Add a real timeout test path with an injectable short drain deadline (or test-only helper), plus a non-cooperative task that ignores cancellation. Assert that timeout is reached, `join_handle.abort()` is used, and terminal failure handling is applied.

### Affected Files
- `src/daemon/runtime.rs` - add timeout-test seam/helper and a true abort-path unit test.

## Amendment: [P3] Env-Sanitization Test Uses Unsynchronized Global Env and Timing-Sensitive Script

### Problem
`cli_backend_strips_claudecode_from_subprocess_env` mutates process-global env (`set_var`/`remove_var`) without synchronization (`src/backend/mod.rs:2515,2530`). In parallel tests, this can race with other env users.  
Also, the script does not consume stdin (`src/backend/mod.rs:2504-2510`) while backend execution always writes prompt data to stdin (`src/backend/mod.rs:616-622`), making `Broken pipe` timing-sensitive failures possible.

### Proposed Change
Guard env mutation with a test-global mutex and restore prior env state via RAII guard. Update test script to consume stdin (`cat >/dev/null`) before printing result so prompt write is deterministic.

### Affected Files
- `src/backend/mod.rs` - harden env-sanitization test against parallel env races and stdin timing issues.

---
