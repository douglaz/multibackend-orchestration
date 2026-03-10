---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T03:42:48Z
---

# Review: CHANGES REQUESTED
1. **High: quick-dev resume path does not actually route staged amendments into execution.**  
[`reset_project_state_for_resume` sets `quick_dev_phase = "codex_review"`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L217 ), but quick-dev drains external amendments only in [`QuickDevPhase::PlanAndImplement`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs#L348 ) and can short-circuit completion logic around [`status/quick_dev_phase`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs#L157 ).  
**Fix:** for quick-dev resumes, set state to a phase that consumes `amendment-queue` (e.g. `quick_dev_phase = "plan_and_implement"` and `current_phase = "implementing"`), then add a validate test that proves a staged PR-review amendment is drained and used by quick-dev on resume.

2. **High: inline review replies are being converted to amendments, but replies are out of scope.**  
[`/pulls/{n}/comments` parsing]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs#L2268 ) does not capture/filter `in_reply_to_id`, and all parsed comments are enqueued in [`fetch_pr_review_comments`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs#L2331 ).  
**Fix:** add `in_reply_to_id: Option<u64>` to `RawPullComment` and skip comments where it is `Some(_)`. Add a unit test with one top-level inline comment + one reply and assert only top-level is returned.

3. **Medium: deferred staged amendments can dispatch even when PR is no longer open.**  
In [`pr_review_phase`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L2491 ), candidates from `has_staged_amendments` are re-dispatched without re-checking PR open state before label swap/dispatch.  
**Fix:** carry `pr_number` in dispatch candidates and gate dispatch on `is_pr_open` for deferred items too (prefer a per-cycle `HashMap<pr_number, bool>` cache to avoid duplicate API calls).

4. **Medium: “exactly once” dedup is not crash/error safe.**  
`poll_pr_reviews` only persists dedup state at end of task processing ([save at]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L511 )); if a later stage/copy/write fails, earlier staged comments may be re-enqueued next cycle. Also [`drain_staged_amendments` ignores remove failures]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L157 ), allowing repeated draining.  
**Fix:** persist state incrementally (or persist partial state before returning on error), and treat `remove_file` failure as an error.

5. **Medium: new validate “conformance” tests are unit-style helper tests, not daemon/binary conformance.**  
[`src/validate/tests_pr_review.rs`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs#L41 ) manually calls helper functions and never runs `daemon start --single-iteration` with mocked GitHub API behavior. This does not validate the integrated runtime flow required by the spec.  
**Fix:** rewrite these cases to use `RalphHarness` + mock `gh` and actually execute daemon ticks, asserting label transitions, staging/drain behavior, dedup across restart, and capacity deferral behavior end-to-end.
