---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T21:26:10Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Make Tmux Cancellation Cleanup Synchronous

### Problem
`TmuxBackend::execute_with_cancel()` returns `Cancelled` immediately on token fire, but relies on `TmuxWindowGuard::drop()` to kill the window via a detached thread (`src/backend/tmux_backend.rs:533-545`, `src/backend/tmux_backend.rs:414-427`).  
There is no await or success check, so the task can reach terminal state while the backend process in tmux is still running.

### Proposed Change
Implement explicit cancel cleanup that waits (bounded) for window termination before returning:
1. On cancel, call async window kill (`kill_window_best_effort`), then poll `has_window` for up to 5s.
2. Keep drop-guard kill as fallback only.
3. Add a cancellation test that proves no tmux window/process remains when `execute_with_cancel` returns.

### Affected Files
- `src/backend/tmux_backend.rs` - synchronous cancel cleanup + test coverage for termination guarantees.

## Amendment: [P2] Fix Non-Hermetic Workspace-Creation Test / Hidden CWD Coupling

### Problem
`cli::auto::tests::ensure_workspace_creates_workspace_when_missing` is environment-dependent and fails when any ancestor `.ralph` exists (`src/cli/auto.rs:313-322`).  
`ensure_workspace()` ignores its injected fallback path for discovery and uses ambient process CWD via `Workspace::discover()` (`src/cli/auto.rs:127-133`; same pattern in `src/cli/quick_dev_auto.rs:62-69`).

### Proposed Change
Use discovery rooted at the provided fallback path (not ambient CWD), then load/create accordingly.  
Also make the test isolate CWD (or use explicit-root path) so it only validates the intended code path.

### Affected Files
- `src/cli/auto.rs` - discovery should use injected fallback root; harden test.
- `src/cli/quick_dev_auto.rs` - align discovery behavior with `auto`.

---
