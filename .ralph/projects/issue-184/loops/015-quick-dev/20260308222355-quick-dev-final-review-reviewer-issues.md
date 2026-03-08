---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T22:23:55Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Make `daemon_concurrency::per_task_log_isolation` deterministic under in-process single-iteration cancellation

### Problem
`daemon_concurrency::per_task_log_isolation` fails reproducibly. The test runs daemon with `--single-iteration` at [tests_daemon_concurrency.rs:1263](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:1263) and expects backend markers in task logs at [tests_daemon_concurrency.rs:1350](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:1350).  
But `drain_all_children_with_deadline()` now cancels all tasks immediately at drain start ([runtime.rs:2019](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:2019)), so backend markers are not guaranteed to be emitted before cancellation. This leaves logs empty and breaks conformance.

### Proposed Change
Stabilize this test against the new runtime semantics:
1. Add deterministic task-start tracing markers in task entrypoints (e.g., in `run_auto_task` / `run_run_task` / quick-dev variants) so every spawned task emits at least one per-task log record immediately.
2. Update `per_task_log_isolation` to assert isolation using those deterministic markers (or run in continuous mode and terminate after marker observation), rather than backend output markers that depend on task progress beyond dispatch.

### Affected Files
- [src/daemon/tasks.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs) - emit deterministic per-task start marker(s) via `tracing`.
- [src/validate/tests_daemon_concurrency.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs) - assert log isolation using deterministic task markers.

## Amendment: [P1] Fix invalid assumption in `daemon::dispatch_fresh_issue_passes_project_id`

### Problem
`daemon::dispatch_fresh_issue_passes_project_id` fails at [tests_daemon.rs:2652](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:2652), asserting `.ralph/projects/issue-500` exists in the worktree.  
That assumption is incompatible with new single-iteration behavior: drain now cancels all active tasks before completion ([runtime.rs:2019](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:2019)), so project creation is not guaranteed to happen before task cancellation.

### Proposed Change
Update the test to verify only guarantees that remain valid in single-iteration mode:
- dispatch happened,
- normalized project-id was passed (already asserted in stderr),
- worktree was created.  
Remove (or move to a non-single-iteration test) assertions requiring project state directory creation before cancellation.

### Affected Files
- [src/validate/tests_daemon.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs) - remove/relocate project-directory existence assertions for this single-iteration dispatch test.
