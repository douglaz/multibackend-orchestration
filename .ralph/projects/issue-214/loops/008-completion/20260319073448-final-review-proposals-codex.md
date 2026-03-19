---
artifact: final-review-proposals
loop: 8
project: issue-214
backend: codex
role: final_reviewer
created_at: 2026-03-19T07:34:48Z
---

Reviewing the branch diff and tracing the new daemon phase through config, runtime, GitHub helpers, and tests first. After that I'll validate behavior against the actual code paths and call out only concrete defects.
The worktree only has the issue branch locally, so I’m resolving the review base from available refs before diffing. Then I’ll read the touched modules and tests in parallel.
I’ve identified the branch base as `origin/master` and the change set is concentrated in daemon/config/GitHub plumbing plus a large conformance module. Next I’m tracing the runtime path and reading tests against the exact implementation, not the loop docs.
The core phase is readable enough now; I’m checking two things in depth: whether it actually uses the configured binaries/resources correctly on all paths, and whether the new tests would catch the real failures rather than mocked happy paths.
I’m into edge-case tracing now: process helper contracts, persistence ordering, and whether comment/readback outcomes are classified in a way that can silently lose work or cause repeated expensive runs.
I haven’t seen a clear compile-time break yet, so I’m now cross-checking the config plumbing end to end and running focused tests. If there’s a defect, it’s more likely to be a behavioral edge case than a missing field.
The implementation is coherent enough that I’m validating by execution now. I’m running focused unit tests around the new modules plus the conformance suite that was added for this feature, which should surface any broken plumbing or mocks that pass for the wrong reason.
I’m not finding an obvious source-level defect yet, so I’m checking the actual validate harness registration and a few live conformance cases. That also helps catch cases where the mocks or CLI plumbing give a false sense of coverage.
I found one concrete edge case worth calling out: the marker dedup check is substring-based, not exact, which can suppress reviews on the wrong comment body. I’m pulling exact line references now and checking whether anything else rises above that threshold.
# Final Review: AMENDMENTS

## Amendment: ORACLE-REV-FINAL-001 [P2]

### Problem
The bot-marker dedup check is substring-based instead of exact-marker-based. In [src/daemon/github.rs#L2131](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2131), `find_bot_comment_with_marker_with_gh_bin` matches any bot-authored comment whose body `contains(marker)`. The oracle phase then trusts that result in [src/daemon/oracle_review.rs#L159](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L159) and persists the PR/SHA as reviewed.

That means a bot-authored comment that merely embeds the marker text anywhere in its body, instead of actually being the oracle review comment for that `(pr_number, head_sha)`, will incorrectly suppress oracle execution and advance dedup state. The current conformance coverage in [src/validate/tests_daemon_oracle_review.rs#L669](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L669) only tests the happy path where the marker is the actual leading marker line, so this false-positive case is unguarded.

### Proposed Change
Require an exact marker-line match for oracle review idempotency, not substring containment. The safest fix is to add an exact-match helper for this phase that accepts only comments whose first line is exactly the marker, or whose body starts with `{marker}\n`, and use that helper for both the pre-oracle existence check and the post-comment outcome path. Add unit and validate coverage for a bot-authored comment that contains the marker later in the body and verify oracle still runs.

### Affected Files
- [src/daemon/github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs) - tighten or specialize marker matching semantics
- [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs) - use the exact-marker matcher for oracle dedup/state healing
- [src/validate/tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs) - add conformance coverage for embedded-marker false positives

---

## Context Provided
I reviewed the branch diff against `origin/master...HEAD`, then traced the implementation through [src/daemon/runtime.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/runtime.rs), [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs), [src/daemon/github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs), [src/config/global.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/config/global.rs), [src/config/mod.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/config/mod.rs), [src/cli/config.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/cli/config.rs), [src/cli/daemon.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/cli/daemon.rs), and [src/validate/tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs).

I also ran targeted checks: `cargo test parse_open_prs`, `cargo test oracle_review_state`, `cargo test post_bot_comment_outcome_treats_readback_failure_after_post_as_success`, and targeted validate cases for `eligible_pr_reviewed`, `missing_oracle_binary_does_not_advance_state`, and `comment_readback_failure_still_advances_state`.

## Master Prompt
Audit the completed oracle-review daemon phase for correctness, safety, robustness, resource isolation, persistence behavior, and test validity, using the committed source as the source of truth.

## Summary
One amendment is needed: oracle marker dedup currently accepts substring matches, which can falsely suppress reviews and self-heal state on the wrong comment. Aside from that, the runtime integration, config wiring, persistence ordering, failure isolation, and the exercised unit/validate paths looked sound.
