---
artifact: final-review-planner-positions
loop: 2
project: summary-skip-the-entire-final-review-del
backend: codex(gpt-5.3-codex-xhigh)
role: planner
created_at: 2026-02-24T07:44:37Z
---

# Planner Positions

## Amendment: FR-CAP-SKIP-ORCH-TESTS-001

### Position
ACCEPT

### Rationale
This amendment is consistent with the master prompt’s intentional boundary change: when `restart_count >= max_final_review_restarts`, final review force-completes immediately and skips deliberation. Tests that set `max_final_review_restarts = 0` and still expect proposal-generation behavior are now asserting obsolete semantics.

Updating those tests to use a positive cap preserves their original purpose (resume/invalidation behavior) without conflicting with the new cap logic. Adding an explicit integration assertion for the `0` cap immediate-skip path is also aligned with the new behavior and improves regression coverage.
