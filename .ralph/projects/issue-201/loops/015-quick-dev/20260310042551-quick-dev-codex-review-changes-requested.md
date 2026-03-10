---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T04:25:51Z
---

# Review: CHANGES REQUESTED

1. High: staged amendments can be lost due over-broad dispatch integration and unconditional purge  
   - `dispatch_task` is called from both normal claim flow and PR-review resume flow ([runtime.rs:1199](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1199), [runtime.rs:2650](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2650)).  
   - New PR-review drain/reset runs unconditionally inside `dispatch_task` ([runtime.rs:1445](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1445)). If project dir is missing, drain count is `0` ([runtime.rs:1462](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1462)), but staged files are still purged unconditionally after spawn ([runtime.rs:1757](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1757)).  
   - This can drop staged PR-review amendments without ever enqueuing them in-project, violating the spec’s resume/staging safety goals.  
   - Fix: make PR-review drain/reset/purge explicit to PR-review resume dispatch only (e.g., `DispatchOrigin::PrReviewResume`), and only purge when `drained_count > 0`. If resumed project metadata exists but project dir/prompt is missing, keep staged files and fail/defer instead of purging.

2. Medium: PR-open state is not cached across the full poll cycle (redundant API calls)  
   - `poll_pr_reviews` checks `is_pr_open` per task ([pr_review.rs:441](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:441)).  
   - `pr_review_phase` checks `is_pr_open` again for staged candidates ([runtime.rs:2560](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2560)).  
   - For “already staged, no new comments” tasks, this duplicates open-state API calls in the same cycle, which misses the spec’s per-cycle open-state caching intent.  
   - Fix: share one `HashMap<u32, bool>` cache between polling and dispatch phases (or have `poll_pr_reviews` return/cache PR-open results for reuse in `pr_review_phase`).
