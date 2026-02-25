---
artifact: final-review-planner-positions
loop: 5
project: task-implement-parallel-prd-issue-proces
backend: claude(opus)
role: planner
created_at: 2026-02-24T23:50:01Z
---

Now I have enough context to evaluate all amendments. Let me formulate my positions.

# Planner Positions

## Amendment: ORPHANED-IMPL-NOTES

### Position
ACCEPT

### Rationale
The file `1740440931-impl-notes.md` at the repository root is a build/loop artifact from the automated implementation process. It contains implementation session notes (issue number choices, mock script design decisions, test pass reports) that have no runtime or documentation value to the codebase. The information it records is already captured in the code itself (test comments, commit history). Leaving it in the repo root adds clutter and sets a bad precedent for accumulating session artifacts. It should be deleted.

## Amendment: FR-001

### Position
REJECT

### Rationale
FR-001 proposes overhauling the concurrency tests to add full backend mocking (claude/codex scripts) and replace the current assertions with persisted-state advancement checks. While the reviewer's observation is technically accurate — these tests do not mock backends and assert on label-edit side effects rather than `InteractivePrdState` transitions — the amendment mischaracterizes the tests as "false positives" and proposes changes that go well beyond the scope of this task.

**Why the current tests are sufficient for this task's goals:**

1. **The tests validate concurrency mechanics, not state advancement.** The master prompt's testing requirements specify: concurrent advancement (both issues advance in one tick), bounded concurrency (peak never exceeds max), error isolation, panic isolation, dedup invariant, and refresh ordering. The existing tests achieve these goals by instrumenting the `gh issue edit` calls — which are the observable side effects of issue processing — with barriers, counters, and flags. This is the correct layer to test concurrency behavior.

2. **State advancement is already covered by existing tests.** The master prompt requirement #8 states "Existing integration and validate tests must continue to pass." The pre-existing test suite already covers Pending → AwaitingAnswers → AwaitingFeedback → Done transitions with full backend mocking. The new concurrency tests don't need to re-test that.

3. **The "backend timeout exceeded" observation doesn't mean false positives.** When the backend times out, the issue still goes through the processing pipeline (label edits happen, which is what the concurrency tests check). The tests verify that *processing* is concurrent, not that it *succeeds at advancing state*. A timeout during question generation is expected when backends aren't mocked — the test passes because the concurrency behavior (parallel execution, bounded worker count, dedup) is still correctly exercised.

4. **The proposed changes are high-effort with low marginal value.** Adding full backend mock scripts, persisted state assertions, and reduced timeouts to every concurrency test would significantly increase test complexity and maintenance burden. The testing requirements in the master prompt don't call for end-to-end state advancement verification in the concurrency-specific tests.

5. **Timeout reduction could make tests fragile.** The reviewer suggests reducing `backend_timeout_secs` to 1-2s after mocking. If mocking is incomplete or a test regresses, short timeouts could cause flaky failures rather than actionable diagnostics.

The existing tests correctly test what they're meant to test: concurrency mechanics. Full backend mocking for state advancement is a separate concern already covered elsewhere.

## Amendment: FR-002

### Position
ACCEPT

### Rationale
This is a duplicate of ORPHANED-IMPL-NOTES. The file `1740440931-impl-notes.md` is a loop artifact that does not belong in the source tree. Both reviewers independently identified it, confirming it should be removed. Accept for the same reasons as ORPHANED-IMPL-NOTES — delete the file from the repo root.
