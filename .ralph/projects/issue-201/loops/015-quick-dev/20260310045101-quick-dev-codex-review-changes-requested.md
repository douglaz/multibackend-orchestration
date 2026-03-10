---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T04:51:01Z
---

# Review: CHANGES REQUESTED

1. **High: staged PR-review amendments can be permanently stranded after daemon restart**
   - **Where:** [src/daemon/runtime.rs:792](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:792), [src/daemon/runtime.rs:2633](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2633), [src/daemon/runtime.rs:1199](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1199), [src/daemon/runtime.rs:1460](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1460), [src/daemon/pr_review.rs:531](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:531)
   - **Why this is wrong:**  
     If the daemon crashes after `completed -> in-progress` label swap but before successful PR-review resume dispatch, startup reconciliation converts `in-progress -> ready`. `pr_review_phase` then skips it because it only resumes `ralph:completed`, and `poll_and_claim` dispatches with `DispatchOrigin::Claim`, where staged amendments are not drained. Dedup keys are already persisted, so those comments won’t be re-enqueued; fixes are never applied.
   - **Spec impact:** Violates the staged-amendment race-safety/resume guarantee for post-completion PR feedback.
   - **Fix:**  
     Implement one of these (recommended: both for robustness):
     - In `pr_review_phase`, allow staged candidates with `ralph:ready` (not only `ralph:completed`) to re-enter PR-review resume flow, swapping `ready -> in-progress` before dispatch.
     - In `dispatch_task`, drain staged PR-review amendments whenever staged files exist and a project directory is present (not only when `origin == PrReviewResume`), then reset state if anything was drained.
   - **Add test:**  
     Add a validate conformance case simulating restart drift (`completed -> in-progress` then daemon restart to `ready`) and assert staged amendments are drained and applied on next tick.
