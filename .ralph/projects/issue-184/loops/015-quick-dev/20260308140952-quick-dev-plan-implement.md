---
artifact: quick-dev-plan-implement
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T14:09:52Z
---

# Final Review Amendments — Implementation Complete

All four review findings have been addressed:

## Changes Made

### [P0] Drain timeout race — `src/daemon/runtime.rs`
After `join_handle.abort()` in `drain_all_children_with_deadline()`, we now **await the JoinHandle with a 10-second bounded timeout** before calling `complete_task()`. This ensures the task has actually stopped executing before the daemon labels it as terminally failed, preventing races where a task could still be mutating git state when its label transitions.

### [P1] Immediate watcher cancellation on abort — `src/daemon/runtime.rs`
`kill_aborted_children()` now cancels **both `watcher_cancel` and `draft_pr_cancel`** tokens immediately alongside the task's `cancel_token` when marking `aborted_externally`. Previously, watchers continued running until `collect_children()` observed the task completion, which could be delayed if the task didn't exit promptly.

### [P2] Non-cooperative test correctness — `src/daemon/runtime.rs`
Replaced the `tokio::time::sleep(...).await` loop (which is abort-cooperative) with a genuinely non-cooperative body using `tokio::task::spawn_blocking(|| std::thread::sleep(...))`. Added a post-drain assertion using an `AtomicU64` counter: after drain completes, we verify the counter stops advancing, proving the task truly stopped — not just removed from the map.

### [P3] KillOnDrop docstring — `src/backend/mod.rs`
Updated the docstring from "sends SIGKILL to the process group" to accurately describe the two-stage termination: "sends SIGTERM to the process group for cooperative shutdown, then spawns a detached reaper thread that escalates to SIGKILL after a 5-second grace period if the group is still alive."

## Verification
- `cargo check` passes
- All 40 `daemon::runtime::tests` pass, including the updated non-cooperative test
