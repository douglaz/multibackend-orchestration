---
artifact: final-review-planner-positions
loop: 15
project: implement-an-event-driven-multi-turn-prd
backend: claude(opus)
role: planner
created_at: 2026-02-22T10:18:55Z
---

Now I have full clarity on both issues. Let me write my position.

# Planner Positions

## Amendment: PRD-TERM-RETRY-001

### Position
ACCEPT

### Rationale
This amendment identifies a real, confirmed bug. The code path is:

1. `do_approval_transition` sets `state = Done` and saves to disk (line 860-874).
2. If save succeeds, it attempts to remove `ralph:prd-active` (line 878). If this removal fails, the function returns an error.
3. However, the terminal state `Done` is already durably persisted to disk.
4. On the next poll tick, `poll_and_advance_prd` queries issues with `ralph:prd-active` (line 351), so the issue **is** still discovered.
5. But `advance_issue` loads the state file, sees `Done` at line 379, and returns `Ok(())` immediately — never retrying the label removal.

The result is an issue permanently stranded with both `ralph:prd-done` and `ralph:prd-active` labels. This directly violates the master prompt's lifecycle label requirements, which specify that Done issues should have `ralph:prd-active` removed and `ralph:prd-done` added as the final label state.

The proposed fix — adding terminal reconciliation in `advance_issue` for Done/Failed states when poll-visible labels are still present — is the cleanest approach. It adds a small check after the `is_terminal()` early return that inspects whether stale labels need cleanup, making the label operations idempotently retryable without re-running the full transition. This aligns with the existing pattern of idempotent operations throughout the module.

The alternative of rolling back to a non-terminal state on label failure would work but introduces risk of re-executing side effects (duplicate comments, etc.) and conflicts with the design intent of "persist terminal state as the critical durability point."

## Amendment: PRD-FAILED-ACTIONS-002

### Position
ACCEPT

### Rationale
This amendment identifies the same structural flaw in the Failed path, and it is actually **worse** than the Done case because more operations are silently dropped.

The code at lines 1407-1446 shows three categories of best-effort operations:

1. **Error comment posting** (line 1407/1419): `let _ =` silently discards failures. The master prompt requires "Post error comment marker `<!-- ralph:prd:{issue_number}:status-failed -->`" as a mandatory action of the Failed transition.

2. **Adding `ralph:prd-failed`** (line 1423): `let _ =` silently discards. The master prompt requires this label as part of the Failed state's label update.

3. **Removing `ralph:prd-active` and `ralph:prd`** (lines 1445-1446): `let _ =` silently discards after terminal state is saved. Same stranding problem as PRD-TERM-RETRY-001.

The critical difference from the Done path is that `transition_to_failed` is itself a fallback mechanism — it's what happens when error_count reaches 3. If the Failed transition's own side effects are silently dropped with no reconciliation path, the issue enters a state where:
- The state file says `Failed`
- But the GitHub issue may have no failure comment, no `ralph:prd-failed` label, and still carry `ralph:prd-active`
- Users see an active-looking issue with no explanation of what happened

The proposed fix of making these side effects retryable through the same terminal reconciliation pattern recommended in PRD-TERM-RETRY-001 is sound. The comment posting and `ralph:prd-failed` label addition should be tracked as required actions that can be retried, and label removals should follow the same reconciliation pattern. This can be implemented alongside PRD-TERM-RETRY-001 as a unified terminal-state reconciliation block in `advance_issue`.

One nuance: the error comment is less critical than the label operations (labels affect workflow correctness; comments affect observability). It would be reasonable to make label operations strictly required for reconciliation while keeping comment posting as best-effort with a single retry, rather than making comment failures block the entire reconciliation. But the labels must be correct.
