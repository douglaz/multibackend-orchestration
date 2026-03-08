---
artifact: quick-dev-plan-implement
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T22:31:24Z
---

All changes are clean and focused. Here's a summary:

# Implementation: Fix P1 Review Amendments

## Changes Made

### 1. Deterministic task-start markers (`src/daemon/tasks.rs`)

Added `tracing::info!(... "RALPH_TASK_STARTED")` at the very start of all four task entry points:
- `run_auto_task` (line 127)
- `run_run_task` (line 278)
- `run_quick_dev_auto_task` (line 315)
- `run_quick_dev_run_task` (line 465)

Each marker includes the `variant` and `project_id`/`project` field, making it unique per task. These execute before any async work or cancellation checks, so they are guaranteed to be emitted even when `drain_all_children()` immediately cancels tasks in single-iteration mode.

### 2. Fix `per_task_log_isolation` test (`src/validate/tests_daemon_concurrency.rs`)

Replaced the backend-marker-based assertions (which depended on tasks running long enough for backend invocations) with assertions on the deterministic `RALPH_TASK_STARTED` markers:
- Simplified mock backend to a no-op script (no marker directory needed)
- Asserts each log contains `RALPH_TASK_STARTED` and its own `project_id` (`issue-800`/`issue-801`)
- Asserts no cross-contamination (neither log contains the other's project_id)

### 3. Fix `dispatch_fresh_issue_passes_project_id` test (`src/validate/tests_daemon.rs`)

Removed assertions that `.ralph/projects/issue-500/` and `state.json` exist in the worktree — project creation happens inside the in-process task, which may be cancelled by `drain_all_children()` before it completes. The dispatch and project-id pass-through are already validated via stderr assertions above the removed lines.
