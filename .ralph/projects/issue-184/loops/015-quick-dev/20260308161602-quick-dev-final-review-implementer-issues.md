---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T16:16:02Z
---

Good - the default `execute_with_cancel` trait method races against the token. The CliBackend overrides this (line 970) to call its own `execute_streaming` which has the integrated `tokio::select!` with proper `KillOnDrop` cleanup. This is correct.

Now I have a thorough understanding of all the changes. Let me compile my final review.

# Final Review: AMENDMENTS

## Summary

The implementation is **well-executed overall**. The core migration from subprocess dispatch to in-process tokio tasks is architecturally sound. Key patterns are correct: `KillOnDrop` guard with proper disarm, `WithSubscriber` for per-task log isolation, cooperative cancellation via `CancellationToken`, and explicit CWD wiring through `workspace.root.parent()`. The `SANITIZED_ENV_VARS` application in both `CliBackend` and `TmuxBackend` is thorough. The `drain_all_children` two-phase shutdown and `collect_children` three-stage design handle edge cases well (panic isolation, watcher teardown, label rollback).

I found two real issues worth addressing.

---

## Amendment: A1

### Problem

**`[P2]` Default trait `execute_with_cancel` silently drops child process on cancellation**

The `Backend` trait's default `execute_with_cancel` implementation at `src/backend/mod.rs:98-108` races `execute_with_log` against `cancel.cancelled()` via `tokio::select!`. When the cancel branch wins, the `execute_with_log` future is dropped. However, `execute_with_log` may have spawned a child process internally (this is the non-cancel-aware execution path). Dropping that future does **not** kill the child process — the `KillOnDrop` guard exists only in `CliBackend::execute_streaming`, not in `execute_with_log`.

For the `CliBackend`, this is safe because it overrides `execute_with_cancel` and routes through `execute_streaming` which has the guard. But any other `Backend` implementation using the default trait method (including `TmuxBackend` if it ever gets cancellation, or any future backend) would leak the child process on cancellation.

Currently `TmuxBackend` does not override `execute_with_cancel`, so if it's used with a cancellation token that actually fires, its tmux window process would be orphaned.

### Proposed Change

Either:
1. Add a `// WARNING` comment on the default `execute_with_cancel` trait method documenting that implementors spawning subprocesses MUST override this method, or
2. Have `TmuxBackend` override `execute_with_cancel` to clean up its tmux window on cancellation.

Option 1 is minimal and sufficient since the tmux backend is not currently used in daemon task dispatch (daemon always sets `tmux: None`/disabled).

### Affected Files
- `src/backend/mod.rs:93-108` - Add documentation warning about subprocess cleanup responsibility

---

## Amendment: A2

### Problem

**`[P2]` `drain_all_children_with_deadline` calls `complete_task` outside the handle-removal guard, running even when no handle was found**

At `src/daemon/runtime.rs:2072-2079`, the `complete_task` call at line 2079 is positioned **after** the closing brace of `if let Some(mut handle) = children.remove(&issue_number)` at line 2072. This means `complete_task` runs even if `children.remove()` returns `None` (e.g., if the task was already collected by a prior `collect_children` call during the drain loop).

While this is functionally harmless in practice (it would just attempt a redundant label swap that would no-op), it spawns an unnecessary tokio task, makes a GitHub API call, and masks the logic. The `complete_task` should be inside the `if let Some` block, matching the pattern used for watcher teardown.

### Proposed Change

Move the `complete_task` block (lines 2073-2107) inside the `if let Some(mut handle)` block, before the closing brace at line 2072.

### Affected Files
- `src/daemon/runtime.rs:2038-2110` - Move `complete_task` invocation inside the `if let Some(mut handle)` guard

---
