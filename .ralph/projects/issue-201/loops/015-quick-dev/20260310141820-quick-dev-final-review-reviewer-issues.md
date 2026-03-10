---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T14:18:20Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] PRR-CLAIM-BYPASS-AFTER-READY-ROLLBACK

### Problem
When PR-review resume fails for a `ralph:ready` + marker task, the code rolls labels back to `ralph:ready`, then the same loop’s normal claim phase can immediately reclaim and dispatch it as `DispatchOrigin::Claim`, bypassing the resume-only safety path.

Evidence:
- PR-review runs before claim phase: [runtime.rs:853](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:853), [runtime.rs:884](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:884)
- PR-review rollback can restore `ralph:ready`: [runtime.rs:2771](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2771)
- Claim path accepts `ralph:ready` without checking marker/staged PR-review state: [runtime.rs:1110](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1110), [runtime.rs:1131](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1131)

This undermines the newly added fail-fast protection for resume-only dispatch.

### Proposed Change
In `poll_and_claim`, skip claiming `ralph:ready` issues when either:
- PR-review resume marker exists for that task, or
- staged PR-review amendments exist for that task.

Let `pr_review_phase` exclusively own those issues. Add a conformance regression test for `ready + marker + missing project` proving no fallback Claim dispatch occurs in the same iteration.

### Affected Files
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - add guard in claim flow.
- [src/validate/tests_pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - add regression test for same-cycle claim bypass.

## Amendment: [P2] PRR-MULTI-LIFECYCLE-NORMALIZATION-MISSING

### Problem
`pr_review_phase` fetches labels and resumes based on `completed`/`ready`, but does not normalize multi-lifecycle states first. If an issue has inconsistent lifecycle labels, this code can resume from an ambiguous state and compound label corruption.

Evidence:
- Labels fetched and used directly: [runtime.rs:2657](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2657), [runtime.rs:2679](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2679)
- Existing normalization helper exists but is not used here: [github.rs:1404](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:1404)

### Proposed Change
Before choosing `from_label` in `pr_review_phase`, compute lifecycle labels and run the same multi-label normalization policy as claim flow (`normalize_multi_lifecycle_labels`), then skip this cycle for that issue.

### Affected Files
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - add lifecycle normalization branch in PR-review phase.
- [src/validate/tests_pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - add coverage for multi-lifecycle input.

## Amendment: [P3] TEST-DOES-NOT-EXERCISE-PRODUCTION-REPLY-FILTER

### Problem
`parse_pull_comments_filters_replies` validates a manual `.filter()` over raw structs instead of calling `parse_pull_comments`, so it can pass even if production reply filtering regresses.

Evidence:
- Test body manually filters parsed raw JSON: [github.rs:3398](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:3398)

### Proposed Change
Rewrite this test to call `parse_pull_comments(...)` directly and assert the reply is excluded from returned `PrReviewComment`s.

### Affected Files
- [src/daemon/github.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs) - update unit test to verify production logic directly.
