---
artifact: final-review-arbiter-ruling
loop: 5
project: task-implement-parallel-prd-issue-proces
backend: claude
role: arbiter
created_at: 2026-02-24T23:54:33Z
---

I now have a clear picture. Let me write my ruling.

# Arbiter Ruling

## Amendment: FR-001

### Ruling
REJECT

### Rationale

This was the one genuinely disputed amendment (2 REJECT vs 1 ACCEPT). After careful code review, I rule REJECT for the following reasons:

**1. The tests are correctly testing concurrency mechanics, not state advancement.**

The concurrency tests (barrier-based concurrent advancement, bounded peak worker count, error isolation, panic isolation, dedup) are specifically designed to validate that `poll_and_advance_prd`'s thread pool correctly parallelizes issue processing. The observable side effect they instrument — `gh issue edit` calls — is the earliest, most reliable signal that an issue entered the processing pipeline. This is the right observation point for concurrency behavior.

**2. The "label edit fires before backend" observation is accurate but irrelevant to what these tests measure.**

Codex correctly notes that in `do_pending_to_awaiting` (line 580), `gh issue edit` (adding `ralph:prd-active`) happens before backend question generation (line 617+). This means "label edit reached" does not prove the full `Pending -> AwaitingAnswers` transition completed. However, that's not what these tests claim to verify. The test comments are explicit: they check that issues are *processed concurrently*, *bounded correctly*, and *isolated from each other's failures*. The label edit is proof that the thread pool dispatched work to the issue — which is exactly the concurrency invariant under test.

**3. State advancement is already covered by the pre-existing test suite.**

Full `Pending -> AwaitingAnswers -> AwaitingFeedback -> Done` transitions with mocked backends are thoroughly tested by the existing integration and conformance tests that predate this task. Duplicating that coverage inside every concurrency test would increase complexity without meaningfully improving confidence.

**4. The 60s runtime concern is real but is a separate optimization, not a correctness issue.**

The tests do take longer than ideal because backends aren't mocked. This is a valid quality-of-life improvement but does not make the tests "false-positive" — the concurrency invariants they check are exercised regardless of backend success. This optimization should be tracked as a separate backlog item, not bundled into a correctness-framed amendment.

**5. The proposed changes are high-cost relative to marginal value.**

Adding full backend mock scripts (claude/codex), deterministic question/synthesis output, persisted state assertions, and reduced timeouts to every concurrency test would significantly increase test complexity and maintenance burden for coverage that already exists elsewhere.
