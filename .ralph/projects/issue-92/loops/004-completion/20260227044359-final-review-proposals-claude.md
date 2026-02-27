---
artifact: final-review-proposals
loop: 4
project: issue-92
backend: claude
role: final_reviewer
created_at: 2026-02-27T04:43:59Z
---

All 38 other integration tests pass. The only failure is `dedup_invariant_issue_processed_at_most_once` which is directly caused by this PR's addition of the `ensure_waiting_feedback_label` call in `do_pending_to_awaiting`.

Let me now provide the final review:

# Final Review: AMENDMENTS

## Summary

The implementation of the `ralph:waiting-feedback` label lifecycle is well-structured and correct in its core logic. The label constants, detection helpers, `ensure_waiting_feedback_label` helper, waiting-state reconciliation, and terminal removal paths are all correctly implemented with proper ordering guarantees. Unit tests (793 pass) and integration tests (38/39 pass) are solid.

However, there is one regression introduced by the PR and one test weakness worth noting.

## Amendment: DEDUP-REGRESSION-01

### Problem
The `dedup_invariant_issue_processed_at_most_once` integration test (`tests/daemon_interactive_prd.rs:2654`) passes on master but fails on this branch. The test counts **all** `gh issue edit` calls as a proxy for "how many times issue #50 was processed." Before this PR, the `Pending->AwaitingAnswers` path made 1 edit call (removing `ralph:prd`). After this PR, `ensure_waiting_feedback_label` at `src/daemon/interactive_prd.rs:1051` adds a second edit call (`--add-label ralph:waiting-feedback`), so the counter reaches 2 instead of the expected 1.

The test assertion at line 2776 fails:
```
assertion `left == right` failed: issue #50 should be processed exactly once per tick, but was processed 2 times
```

This causes `nix build -L` to fail because the integration test suite is included in the nix build.

### Proposed Change
Update the test's expected count from 1 to 2 at line 2776, and update the comment at lines 2770-2773 to explain that the Pending->AwaitingAnswers path now makes 2 edit calls (remove `ralph:prd` + add `ralph:waiting-feedback`). The dedup invariant still holds — the issue is processed once, it just has 2 label mutations now.

Alternatively, the test could be restructured to count only `--remove-label ralph:prd` calls as the processing marker, which would keep the expected count at 1 and be more resilient to future label additions. Either approach is acceptable.

### Affected Files
- `tests/daemon_interactive_prd.rs` - Update expected count and/or counting logic in `dedup_invariant_issue_processed_at_most_once` (lines 2770-2778)

---
