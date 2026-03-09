---
artifact: final-review-votes
loop: 5
project: issue-194
backend: claude
role: final_reviewer
created_at: 2026-03-09T15:53:55Z
---

Both claims fully verified. Here is my review:

# Vote Results

## Amendment: FR-194-001

### Vote
ACCEPT

### Rationale
Confirmed by source inspection. `pending_amendment_count` is called exactly once in the orchestrator, at line 739, during the planner's completion decision. After that single check, the orchestrator proceeds through completing, optional acceptance QA, and optional final review — all of which can involve multiple backend calls and significant wall-clock time. `ProjectStatus::Completed` is set at line 2604 or 2637 with no subsequent queue re-check, and the return at line 2797-2817 happily reports "project completed" regardless of any amendments that arrived in the interim.

This is a genuine correctness bug. A user running `ralph amend` while the completing or final_review phase is executing will have their amendment silently ignored for that run. Adding a guard before the completed return path at line ~2797 is the right fix — it's a minimal, targeted change that closes a real race window. The proposed conformance test is also appropriate to prevent regression.

## Amendment: FR-194-002

### Vote
ACCEPT

### Rationale
Confirmed by reading lines 190-222 of `tests/amend_cli.rs`. The test is named `amend_cli_multiple_amendments_drain_in_order`, explicitly claiming it verifies ordering. However, lines 218-221 collect IDs into a `Vec` and only assert membership via `ids.contains(...)` for each of the three IDs. This would pass even if the drain returned items in reverse or arbitrary order, making the "in order" contract claim unenforceable.

The fix is trivial — replace the three `contains` calls with a single `assert_eq!(ids, vec!["EXT-MULTI-0", "EXT-MULTI-1", "EXT-MULTI-2"])` — and the risk is zero. A test that claims to verify ordering but doesn't is worse than useless; it gives false confidence.
