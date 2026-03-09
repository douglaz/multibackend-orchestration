---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T22:53:15Z
---

Now I have all the information I need to write the final review. Let me compile the findings from all agents and my own checks.

# Final Review: AMENDMENTS

## Amendment: KillOnDrop-pgid0

### Problem
In `src/backend/mod.rs:72`, the `KillOnDrop::drop` implementation calls `libc::kill(-(raw), libc::SIGKILL)` where `raw` is derived from the stored pgid. If `pgid` were ever `0`, this would call `kill(0, SIGKILL)`, which sends SIGKILL to **every process in the daemon's own process group** — killing the daemon itself and all co-located processes.

While `pgid=0` cannot occur in the current construction site (line 600-601: `KillOnDrop(child.id())` where `child.id()` returns the child's PID, always >= 2 after `setsid()`), this is a latent safety hazard. A future refactor could introduce a path that constructs `KillOnDrop(Some(0))`, and the resulting daemon self-kill would be extremely difficult to diagnose.

### Proposed Change
Add a defensive guard at `src/backend/mod.rs:64`:
```rust
if let Ok(raw) = i32::try_from(pgid) {
    if raw == 0 {
        return; // Never kill our own process group
    }
    // ...existing SIGKILL logic...
}
```

### Affected Files
- `src/backend/mod.rs` - Add `raw == 0` guard in `KillOnDrop::drop` (line 64) `[P1]`

---

## Amendment: partial-dispatch-rollback-test-rename

### Problem
In `src/validate/tests_daemon_concurrency.rs`, two tests have been semantically changed but retain their original names, making them misleading:

1. `partial_dispatch_rollback` (around line 237): Originally tested that when one task's dispatch fails, sibling tasks are unaffected. Now both tasks receive identical cancellation via `drain_all_children`. The test proves uniform drain behavior, not partial rollback or sibling isolation.

2. `dispatch_failure_explicit_markers` (around line 607): Originally asserted that a dispatch-failure marker was NOT present on the sibling (isolation). Now asserts both markers ARE present — the exact opposite semantic. It now tests uniform drain, not failure isolation.

### Proposed Change
Rename these tests to match their actual current behavior:
- `partial_dispatch_rollback` → `drain_terminates_all_tasks_uniformly` (or similar)
- `dispatch_failure_explicit_markers` → `drain_marks_all_tasks_failed` (or similar)

Alternatively, if sibling isolation is still a desired property, implement new tests that use a mock backend where one task fails (returns error) while the other succeeds, and verify the successful task gets `ralph:completed` while the failing one gets `ralph:failed`.

### Affected Files
- `src/validate/tests_daemon_concurrency.rs` - Rename test functions and update docstrings `[P2]`

---

## Amendment: runtime-no-diff-weakened-assertion

### Problem
In `src/validate/tests_daemon.rs`, the `runtime_no_diff_pr_path` test (around line 1828-1846) was weakened: it now accepts either a `no-diff` marker OR a `failed` marker. This means the test passes even when the task simply fails before reaching the no-diff detection path, which defeats the test's purpose of verifying no-diff handling specifically.

### Proposed Change
If the no-diff path is reachable in-process (i.e., the mock backend can be configured to produce no git diff), the assertion should require the `no-diff` marker specifically. If the no-diff path is not easily testable in-process, the test should be documented as a known gap with a `// TODO` comment, rather than silently accepting any terminal state.

### Affected Files
- `src/validate/tests_daemon.rs` - Tighten assertion in `runtime_no_diff_pr_path` `[P2]`

---

## Summary

The implementation is **well-executed overall**. The migration from subprocess dispatch to in-process tokio tasks is architecturally sound. Key strengths:

- **CWD safety**: All `std::env::set_current_dir()` / `current_dir()` calls removed from library dispatch paths. Explicit `workspace_root` threading throughout (`src/daemon/tasks.rs`, all 4 entry points).
- **Environment sanitization**: `SANITIZED_ENV_VARS` correctly applied at the backend layer (`src/backend/mod.rs:569-572`) for `CliBackend` and at `src/backend/tmux_backend.rs:131-145` for `TmuxBackend`.
- **Cancellation**: `CancellationToken` correctly threaded through orchestrators, with checks between phases and `tokio::select!` in backend execution. `KillOnDrop` guard ensures child process cleanup on future drop.
- **Per-task logging**: `WithSubscriber` correctly used (not `with_default`), ensuring log isolation across tokio thread migrations.
- **Panic handling**: Multi-level isolation via inner `tokio::spawn` in `dispatch_task`, `collect_children`, and `drain_all_children`, with label rollback on panic.
- **`max_backend_retries`**: Clean migration from env var to config field with proper wiring through the entire config pipeline.
- **Dead code removal**: All subprocess spawn functions, `RALPH_DAEMON_BIN` resolution, `ralph_bin` field, and mock scripts fully removed.
- **No compilation issues** with the code structure (no Rust toolchain available to verify, but import analysis shows no stale references).

The three amendments above are: one safety hardening (`[P1]`), and two test accuracy improvements (`[P2]`). No correctness bugs or resource leaks were found in the production code paths.
