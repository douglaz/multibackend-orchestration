---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T12:48:59Z
---

# Review: CHANGES REQUESTED

1. High: PR comment polling can be fully blocked by one malformed/null-author comment payload.
- Code path: [`fetch_pr_review_comments` in github.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2364), raw structs at [`github.rs:2320`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2320), parse points at [`github.rs:2380`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2380), [`2414`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2414), [`2446`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2446), then task skipped in [`pr_review.rs:618-634`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:618).
- Problem: `user` is required in raw structs and parse errors are propagated with `?`. A single unexpected item can make the whole endpoint fail, then `poll_pr_reviews` skips the task, so valid comments are not converted to amendments.
- Fix:
  1. Make raw `user` fields optional (`Option<RawUser>`, `#[serde(default)]`).
  2. Skip entries with missing/empty login instead of failing the whole endpoint.
  3. Handle endpoint parse errors as per-endpoint warnings (continue), not full-task failure.
  4. Add tests with `user: null` mixed with valid comments to confirm valid comments still enqueue.

2. Medium: `dispatch_task` drains staged PR-review amendments before verifying the dispatch can actually resume.
- Code path: drain/reset executes at [`runtime.rs:1456-1492`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1456), resume validation/fail-fast occurs after at [`runtime.rs:1494-1505`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1494).
- Problem: For `PrReviewResume`, side effects (queue copy + state reset) can happen before the function decides resume is invalid (missing `prompt.md`). That creates avoidable state mutation in a dispatch that is immediately rejected.
- Fix:
  1. Move the `PrReviewResume` fail-fast resume check ahead of drain/reset.
  2. Only call `drain_staged_amendments` and `reset_project_state_for_resume` after `resume_existing_project == true`.
  3. Add a regression test where project dir exists but `prompt.md` is missing, and assert no drain/reset side effects occur.
