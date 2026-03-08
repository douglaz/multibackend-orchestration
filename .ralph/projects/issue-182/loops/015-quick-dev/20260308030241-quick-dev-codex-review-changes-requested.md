---
artifact: quick-dev-codex-review
loop: 15
project: issue-182
backend: codex
role: reviewer
created_at: 2026-03-08T03:02:41Z
---

# Review: CHANGES REQUESTED

1. High: orphan terminalization uses a branch that does not match the task PR flow.
- Evidence:
  - Orphan heuristic is hardcoded to `ralph/daemon/{task_id}` in [runtime.rs:1085](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs:1085).
  - Dispatch explicitly syncs worktrees onto `ralph/issue-<n>` via [runtime.rs:1537](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs:1537) and [branch.rs:108](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/git/branch.rs:108).
  - Completion PR flow resolves PRs from the current worktree branch in [runtime.rs:3041](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs:3041).
- Impact: adopted orphans can be mislabeled `ralph:failed` even when a merged PR exists for the actual task branch.
- Fix:
  - In `poll_adopted_orphans`, derive merge status from persisted `TaskMetadata.pr_url` first (parse PR number, query PR state).
  - Fallback to the project branch (`resolve_branch_name(..., issue-{n})`) instead of only `ralph/daemon/{task_id}`.
  - Then call `complete_task` with the corrected terminal label.

2. Medium: conformance coverage is incomplete for key spec paths.
- Evidence: only 7 orphan conformance tests are registered in [tests_daemon_orphan.rs:10](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs:10).
- Missing high-risk paths from the spec include:
  - orphan terminalization through `complete_task` side effects,
  - crash window “spawned but not inserted” adoption path,
  - dispatch failure/panic PID cleanup verification.
- Fix:
  - Add conformance tests for those scenarios in [tests_daemon_orphan.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs) and register them in [mod.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/mod.rs).
