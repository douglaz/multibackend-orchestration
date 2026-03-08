---
artifact: quick-dev-plan-implement
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T21:34:35Z
---

# Implementation: Final Review Amendments

Both amendments from the final review have been implemented and all tests pass.

## Amendment [P1]: Make Tmux Cancellation Cleanup Synchronous

**Problem:** `TmuxBackend::execute_with_cancel()` returned `Cancelled` immediately on token fire, relying on `TmuxWindowGuard::drop()` which spawns a detached thread — no guarantee the window is actually gone when the method returns.

**Changes to `src/backend/tmux_backend.rs`:**
- Added `active_window: Arc<Mutex<Option<String>>>` field to `TmuxBackend` to track the currently executing tmux window ID
- `execute_raw()` publishes the window ID after creation and clears it on normal cleanup paths (both timeout and success)
- Added `cancel_active_window()` method that:
  1. Takes the window ID from the shared state
  2. Calls `kill_window_best_effort()` to issue `tmux kill-window`
  3. Polls `has_window()` in a loop with 100ms intervals, bounded by a 5-second deadline
- Updated `execute_with_cancel()` to call `cancel_active_window()` before returning `Cancelled`, so the caller can rely on the window being gone
- Drop guard remains as fallback
- Added test `cancel_kills_window_synchronously_before_returning` that verifies kill-window is called, has_window poll occurs, and active_window is cleared

## Amendment [P2]: Fix Non-Hermetic Workspace-Creation Test / Hidden CWD Coupling

**Problem:** `ensure_workspace()` in both `auto.rs` and `quick_dev_auto.rs` used `Workspace::discover()` which walks from ambient process CWD, making the test environment-dependent.

**Changes:**
- **`src/workspace/mod.rs`:** Added `Workspace::discover_from(start: &Path)` that calls `discover_workspace_root(Some(start))` instead of `None` (ambient CWD)
- **`src/cli/auto.rs`:** Changed `ensure_workspace()` None-branch to use `Workspace::discover_from(fallback_cwd)` instead of `Workspace::discover()`
- **`src/cli/quick_dev_auto.rs`:** Same change

This ensures both discovery and creation are rooted at the caller's provided path, never ambient CWD.
