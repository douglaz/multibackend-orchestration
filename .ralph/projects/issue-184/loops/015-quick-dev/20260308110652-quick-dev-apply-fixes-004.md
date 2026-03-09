---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 4
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T11:06:52Z
---

# Applied Fixes for Review Feedback

## P1: Cancellation hard-abort can leave backend descendants running

**Root cause:** Both `KillOnDrop` and `kill_and_reap_child` used only `waitpid`/`child.wait()` to detect process exit, which only tracks the **leader PID**. If the leader exits on SIGTERM but a descendant ignores it, no SIGKILL escalation was sent to the group.

**Fix applied in `src/backend/mod.rs`:**

- **`KillOnDrop::drop`**: Now combines `waitpid(leader, WNOHANG)` (to reap the leader zombie and clear it from the process table) with `kill(-pgid, 0)` (to check true group liveness including descendants). The polling loop in the grace-period thread also does both operations each iteration.

- **`kill_and_reap_child`**: Replaced `tokio::time::timeout(grace, child.wait())` with a polling loop that calls `child.try_wait()` (to reap the leader zombie via tokio) then `kill(-pgid, 0)` (to check group liveness). If any process in the group survives the 5s grace period, SIGKILL is sent to the entire group.

- **Regression test added**: `kill_and_reap_child_kills_descendants_that_survive_leader` — spawns a parent that exits on SIGTERM with a child that ignores SIGTERM. Verifies the descendant is killed via SIGKILL escalation after the grace period.

## P2: CLI still bypasses new daemon task entrypoints

**Root cause:** All four CLI `execute()` functions (`auto.rs`, `run.rs`, `quick_dev_auto.rs`, `quick_dev_run.rs`) created orchestrators directly instead of calling the shared library entry points in `daemon/tasks.rs`.

**Fix applied:**

- **`src/daemon/tasks.rs`**: Extended all four param structs (`AutoTaskParams`, `RunTaskParams`, `QuickDevAutoTaskParams`, `QuickDevRunTaskParams`) with the full set of orchestrator options (backend overrides, tmux, skip_commit, dry_run, etc.). Updated task functions to pass all options through to `RunOptions`/`QuickDevRunOptions`. Removed unused `global_config` field. Added backend spec validation and dry_run support to `run_auto_task`.

- **`src/cli/auto.rs`**: Refactored `execute()` to call `tasks::run_auto_task()` with `CancellationToken::new()`. Removed duplicated PRD, project creation, and orchestration logic. Kept `ensure_workspace` for CLI workspace discovery/creation.

- **`src/cli/run.rs`**: Refactored `execute()` to call `tasks::run_run_task()`.

- **`src/cli/quick_dev_auto.rs`**: Refactored `execute()` to call `tasks::run_quick_dev_auto_task()`. Removed duplicated `slugify_idea`, `truncate_idea_for_name`, PRD, and orchestration logic.

- **`src/cli/quick_dev_run.rs`**: Refactored `execute()` to call `tasks::run_quick_dev_run_task()`.

- **`src/daemon/runtime.rs`**: Updated `dispatch_task()` to construct the extended param structs (daemon-specific fields default to `None`/`false`). Removed unused `global_config` binding.
