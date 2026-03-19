---
artifact: final-review-planner-positions
loop: 8
project: issue-214
backend: codex(gpt-5.4-xhigh)
role: planner
created_at: 2026-03-19T07:37:01Z
---

I’m verifying the cited oracle-review paths directly in the repo and checking whether the amendment matches the implemented behavior. First step is reading the GitHub comment matcher, the oracle review flow, and the existing validate coverage around this dedup path.
The matcher currently is exactly as described: bot-author scoped, but using `body.contains(marker)`. I’m tracing where that result feeds state transitions and whether any later path depends on the same loose semantics.
I’ve confirmed the pre-run skip path and the post-comment outcome path both route through the same `contains(marker)` helper. Next I’m checking whether any existing unit tests already cover embedded-marker bodies or whether the amendment correctly identifies a gap.
The validate suite asserts that newly posted oracle comments begin with the marker, but that only covers the produced format. I’m checking whether there’s any test exercising the lookup semantics on existing comments, because that’s where the claimed false positive would occur.
I haven’t found any lookup test for the false-positive case yet. I’m doing one last pass across the repo for any embedded-marker coverage before locking the position.
# Planner Positions

## Amendment: ORACLE-REV-FINAL-001

### Position
ACCEPT

### Rationale
The defect is real as described.

The bot-scoped matcher in [src/daemon/github.rs#L2131](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2131) returns the first bot-authored comment whose body merely `contains(marker)` at [src/daemon/github.rs#L2142](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2142). In the oracle phase, that result is trusted as proof the PR/SHA was already reviewed at [src/daemon/oracle_review.rs#L160](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L160), and the state is immediately persisted via [src/daemon/oracle_review.rs#L171](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L171). The post-comment path has the same problem because `post_bot_comment_with_marker_outcome_with_gh_bin` also dedups through that helper at [src/daemon/github.rs#L2182](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2182), and `oracle_review_phase` marks the PR reviewed on `AlreadyExists` at [src/daemon/oracle_review.rs#L250](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L250).

That is broader than the producer semantics. Oracle review comments are created with the marker as a prefix line, not an arbitrary substring: the marker format is defined at [src/daemon/oracle_review.rs#L286](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L286), and posted bodies are built as `{marker}\n{body_text}` at [src/daemon/github.rs#L2199](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2199) and [src/daemon/github.rs#L2324](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2324). So an exact marker-line check is consistent with what this phase actually emits.

The cited test gap also exists. The validate case at [src/validate/tests_daemon_oracle_review.rs#L669](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L669) only seeds a comment whose body begins with the marker at [src/validate/tests_daemon_oracle_review.rs#L687](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L687). I did not find coverage for a bot-authored comment that embeds the marker later in the body. The existing assertions for newly posted comments also assume prefix semantics at [src/validate/tests_daemon_oracle_review.rs#L385](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L385).

So the amendment identifies a genuine correctness gap: a bot-authored non-oracle comment containing the marker text anywhere can suppress oracle execution and incorrectly advance persisted review state.
