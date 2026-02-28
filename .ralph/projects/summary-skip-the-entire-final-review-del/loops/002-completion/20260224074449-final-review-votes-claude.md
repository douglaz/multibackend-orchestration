---
artifact: final-review-votes
loop: 2
project: summary-skip-the-entire-final-review-del
backend: claude
role: final_reviewer
created_at: 2026-02-24T07:44:49Z
---

# Vote Results

## Amendment: FR-CAP-SKIP-ORCH-TESTS-001

### Vote
ACCEPT

### Rationale
The amendment is straightforward and well-justified. The core logic change — cap `0` now means "skip final review entirely" — is intentional per the feature design. Tests that were written against the old semantics (where cap `0` still allowed a first run before failing) are now genuinely broken, not flaky. They need to be updated.

The proposed fix is sensible on all three points:

1. **Bumping the cap to `1` for resume/invalidation tests** preserves their original testing intent (verifying proposal caching and config-mismatch invalidation) while letting deliberation actually run under the new boundary rules. This is the minimal change needed.

2. **Keeping/adjusting assertions** rather than gutting them ensures we don't lose coverage for the behaviors those tests were originally designed to verify.

3. **Adding an explicit `max_final_review_restarts = 0` integration test** for the immediate-skip path is a good addition. The new boundary behavior is a meaningful semantic change and deserves its own dedicated test coverage at the integration layer, not just implicit coverage through unit tests on the orchestrator guard.

The planner's position is sound — the tests are asserting obsolete semantics, and the fix aligns them with the intentional design change without losing their original purpose.
