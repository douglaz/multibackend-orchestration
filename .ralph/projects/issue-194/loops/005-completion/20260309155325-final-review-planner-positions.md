---
artifact: final-review-planner-positions
loop: 5
project: issue-194
backend: claude(opus)
role: planner
created_at: 2026-03-09T15:53:25Z
---

Confirmed: `pending_amendment_count` is only called once in the entire orchestrator, at line 739. There is no second check before the completion return at line 2797.

# Planner Positions

## Amendment: FR-194-001

### Position
ACCEPT

### Rationale
Verified by reading the source. The pending amendment queue is checked exactly once, at `orchestrator.rs:739`, during the planner's completion decision. After that check passes, the orchestrator proceeds through the completing phase (and optionally final review/acceptance QA). The status is set to `ProjectStatus::Completed` at lines 2604 or 2637, and the successful return happens at line 2797-2810 — with no second `pending_amendment_count` call anywhere in between.

This is a real correctness gap: if a user enqueues an amendment via `ralph amend` while the completing or final_review phases are executing (which can involve multiple backend calls and significant wall-clock time), the run will return `"project completed"` without ever processing that amendment. The amendment silently remains in the queue file. A final guard at line ~2797 (before the `Ok(OrchestrationResult { ... })` return) checking `pending_amendment_count` and either erroring or looping back to planning is a straightforward and correct fix.

## Amendment: FR-194-002

### Position
ACCEPT

### Rationale
Verified by reading `tests/amend_cli.rs:190-222`. The test is named `amend_cli_multiple_amendments_drain_in_order` — clearly claiming it validates ordering. However, lines 218-221 collect the IDs into a `Vec` and then assert only with `ids.contains(...)` for each of the three IDs. This proves membership (all three are present) but not ordering. If `drain_amendment_queue` returned items in reverse order or any shuffled order, this test would still pass.

The fix is trivial and the problem is real: the test name makes a contract claim ("drain in order") that the assertions do not enforce. Replacing the three `contains` checks with `assert_eq!(ids, vec!["EXT-MULTI-0", "EXT-MULTI-1", "EXT-MULTI-2"])` would make the test actually verify what it claims.
