---
artifact: final-review-proposals
loop: 15
project: implement-an-event-driven-multi-turn-prd
backend: codex
role: final_reviewer
created_at: 2026-02-22T10:17:10Z
---

# Final Review: AMENDMENTS

## Amendment: PRD-TERM-RETRY-001

### Problem
`Done` can be durably persisted before label cleanup completes, and then never retried:

- `do_approval_transition` saves terminal `Done` before removing `ralph:prd-active` (`src/daemon/interactive_prd.rs:860`, `src/daemon/interactive_prd.rs:867`, `src/daemon/interactive_prd.rs:878`).
- If that remove fails, `finish_transition` still keeps/saves terminal state and returns an error (`src/daemon/interactive_prd.rs:1020`, `src/daemon/interactive_prd.rs:1027`, `src/daemon/interactive_prd.rs:1065`).
- Later ticks skip terminal issues immediately (`src/daemon/interactive_prd.rs:379`), so cleanup is never retried.

This can strand issues with both `ralph:prd-active` and `ralph:prd-done`, violating the required label outcome.

### Proposed Change
Make terminal label cleanup retryable:

- Add terminal reconciliation in `advance_issue` for `Done`/`Failed` states when poll-visible labels still exist.
- Or, if post-save cleanup fails in `do_approval_transition`, roll state back to retryable non-terminal state and persist that rollback.
- Route cleanup failures through `error_count`/retry exhaustion semantics.

### Affected Files
- `src/daemon/interactive_prd.rs` - ensure terminal cleanup failures are retried instead of permanently short-circuited.
- `tests/daemon_interactive_prd.rs` - add integration case for `ralph:prd-active` remove failure after `Done` save.
- `src/validate/tests_interactive_prd.rs` - add conformance test for terminal cleanup retry behavior.

## Amendment: PRD-FAILED-ACTIONS-002

### Problem
`Failed` transition currently drops required side-effect failures:

- Failure comment posting is best-effort and ignored (`src/daemon/interactive_prd.rs:1407`, `src/daemon/interactive_prd.rs:1419`).
- Adding `ralph:prd-failed` is ignored (`src/daemon/interactive_prd.rs:1423`).
- Removing `ralph:prd-active` / `ralph:prd` is ignored (`src/daemon/interactive_prd.rs:1445`, `src/daemon/interactive_prd.rs:1446`).

With terminal short-circuiting (`src/daemon/interactive_prd.rs:379`), these missed actions may never be corrected, which conflicts with required failed-state actions.

### Proposed Change
Treat failed-state side effects as required and retryable:

- Propagate errors for posting `status-failed` and adding `ralph:prd-failed` into retry accounting.
- Reconcile label removals with retries (or terminal reconciliation) instead of silently ignoring failures.
- Keep issue retry-visible until required failed-state side effects complete or retry exhaustion is reached.

### Affected Files
- `src/daemon/interactive_prd.rs` - remove best-effort `let _ =` handling for required failed-state actions.
- `tests/daemon_interactive_prd.rs` - add integration tests for transient failures in failed-state comment/label operations.
- `src/validate/tests_interactive_prd.rs` - add conformance coverage for failed-state side-effect reliability.
