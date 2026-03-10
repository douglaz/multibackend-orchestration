---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T08:39:40Z
---

# Review: CHANGES REQUESTED
1. **High: dedup does not guarantee “exactly once” across poll cycles/restarts.**  
At [`src/daemon/pr_review.rs:579`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:579), staging happens before persisted dedup update, and at [`src/daemon/pr_review.rs:121`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:121) staged filenames are timestamp-based.  
If the daemon crashes between stage and save (or save fails at [`src/daemon/pr_review.rs:594`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:594)), the same GitHub comment can be staged again with a new filename, creating duplicate amendments. This violates the acceptance criterion for exact-once enqueue semantics.

   **Fix:** make staging idempotent per comment key (deterministic filename from `{endpoint}:{id}` or amendment id, created with `create_new`), and treat existing staged file as already enqueued. Then persist dedup state; on save failure, do not emit additional staged duplicates on next cycle.

2. **Medium: PR-review resume path is over-broad for `ralph:ready` and can reset non-completed quick-dev state.**  
At [`src/daemon/runtime.rs:2636`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2636), `pr_review_phase` resumes both `ralph:completed` and `ralph:ready`.  
At [`src/daemon/runtime.rs:1456`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1456), any dispatch with staged amendments resets project state; for quick-dev this forces `plan_and_implement` via [`src/daemon/pr_review.rs:266`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:266).  
That can mutate legitimately resumable non-completed `ready` projects (outside the feature scope, which targets completed-project post-PR feedback).

   **Fix:** gate reset/resume logic to completed-project resumes only, or require an explicit “pr-review-resume-pending” marker before treating `ralph:ready` as restart-drift from a completed PR-review resume.

I ran targeted checks (`cargo test pr_review` and `validate --filter pr_review::`), and those pass; the issues above are edge-case correctness/scope gaps against the spec guarantees.
