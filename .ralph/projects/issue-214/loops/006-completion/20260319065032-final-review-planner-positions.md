---
artifact: final-review-planner-positions
loop: 6
project: issue-214
backend: codex(gpt-5.4-xhigh)
role: planner
created_at: 2026-03-19T06:50:32Z
---

Reviewing the cited daemon and GitHub helper paths first to verify whether the error boundary and state accounting behave the way the amendment claims.
I have the success path and helper contract. Next I’m checking the phase loop, cap enforcement, and existing validate coverage to see whether a readback failure really can overshoot the configured per-cycle limit or leave stale state until a later cycle.
The validate file already has cap and posting-failure cases, so I’m checking whether they miss the specific split between “post succeeded” and “metadata fetch failed” that the amendment points out.
The current tests only cover hard comment-command failure. I’m reading the mock `gh` implementation now to confirm there is no existing test path for “post succeeds, follow-up comment fetch fails.”
I’ve confirmed the current helper really can surface a successful post as an error if the follow-up `issue view --json comments` step fails. I’m pulling exact line references now so the verdict is grounded in the current implementation rather than the amendment summary.
I’ve got enough to judge the bug itself. I’m checking the mock `gh` test harness with line numbers once more so I can state precisely whether validate coverage already exercises this failure mode.
# Planner Positions

## Amendment: ORACLE-REVIEW-FR-001

### Position
ACCEPT

### Rationale
[oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L149) enforces `daemon_oracle_review_max_per_cycle` via `success_count`, and [oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L229) only increments that counter and persists `state.mark_reviewed(...)` in the `Ok(_)` arm. The helper called there does not treat a successful `gh issue comment` as the success boundary: after the zero-exit post at [github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2230), it immediately does a readback at [github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2257), and that path propagates `gh issue view --json comments` failures from [github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2122) and [github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L1923). So a real post followed by transient readback failure is currently logged as `comment post failed`, does not advance state, and does not consume the cap.

That is a real correctness bug, not just a logging issue. With multiple eligible PRs, the loop at [oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L150) can continue past the configured limit because the first successful post did not increment `success_count`. Separating `already_exists` from `posted` is also technically justified: the helper can return an existing bot comment in the pre-post race window at [github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2217), and the caller currently counts that the same as a fresh post at [oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L240).

The validate gap is also real. Existing tests cover the normal cap path at [tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L619) and hard `issue comment` failure at [tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L923), but the mock `gh` script only supports failing `issue comment` and always succeeds on comment readback at [tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L1155) and [tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L1179).
