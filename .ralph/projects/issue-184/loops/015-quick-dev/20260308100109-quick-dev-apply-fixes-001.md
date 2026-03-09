---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T10:01:09Z
---

# Apply reviewer-requested fixes

## Changes made

### Fix 1 (High): External abort can still execute success-side effects

**Problem:** When a task is externally aborted (issue labels changed from `ralph:in-progress` to `ralph:failed` on GitHub), `kill_aborted_children()` only cancels the token. If the task already finished `Ok` before cancellation took effect, `collect_children()` would derive `ralph:completed` and run PR flow (commenting, creating PRs) before attempting the label swap — which would fail because `ralph:in-progress` no longer exists.

**Fix applied across 3 files:**

1. **`src/daemon/mod.rs`** — Added `aborted_externally: Arc<AtomicBool>` field to `TaskHandle`. This flag is shared between `kill_aborted_children` (writer) and `collect_children` (reader).

2. **`src/daemon/runtime.rs` — `kill_aborted_children()`** — Sets `handle.aborted_externally.store(true, SeqCst)` before cancelling the token.

3. **`src/daemon/runtime.rs` — `collect_children()` stage 2** — Checks `aborted_externally` after resolving the `JoinHandle`. When set, forces `terminal_label` to `"ralph:failed"` regardless of the `JoinHandle` result.

4. **`src/daemon/runtime.rs` — `complete_task_attempt()`** — Takes new `externally_aborted: bool` parameter:
   - PR flow guard: `terminal_label == "ralph:completed" && !externally_aborted` (skips PR flow for externally aborted tasks even if terminal_label were somehow "completed")
   - Label swap: when `externally_aborted`, uses idempotent `add_label_with_retry(terminal_label)` instead of `swap_lifecycle_label(in-progress -> terminal)`, avoiding error-looping on a label that was already changed externally
   - Comment: posts with phase "aborted" instead of the terminal label phase

5. **`src/daemon/runtime.rs` — `drain_all_children()`** — Updated `complete_task()` call to pass `externally_aborted: false` (drain timeout is not an external abort).

6. **All `TaskHandle` construction sites** — Added `aborted_externally: Arc::new(AtomicBool::new(false))` (2 sites: `dispatch_task` and the drain test).

### Fix 2 (Medium): Daemon library task path auto-initializes workspace

**Problem:** `load_workspace()` in `tasks.rs:443` fell back to `create_workspace()` if `.ralph/ralph.toml` was missing. This could silently run with default workspace config if the worktree config copy failed, violating the spec's requirement for strict `Workspace::load()` in daemon dispatch.

**Fix:** Removed the `create_workspace()` fallback. `load_workspace()` now unconditionally calls `Workspace::load(ralph_dir)`, which returns an error if `ralph.toml` is missing. CLI code paths that intentionally support auto-init are unaffected.

### Verification

- `cargo check` passes with no new warnings
- All `daemon::runtime::tests::derive_terminal_label_*` tests pass (4/4)
- All `daemon::tasks::tests` pass (2/2)  
- `drain_all_children` test passes
- 238/240 daemon tests pass; the 2 failures are pre-existing in `github::tests::push_branch_with_retry_impl_*` (unrelated test environment issue)
