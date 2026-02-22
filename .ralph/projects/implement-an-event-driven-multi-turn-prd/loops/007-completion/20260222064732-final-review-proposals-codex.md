---
artifact: final-review-proposals
loop: 7
project: implement-an-event-driven-multi-turn-prd
backend: codex
role: final_reviewer
created_at: 2026-02-22T06:47:32Z
---

# Final Review: AMENDMENTS

## Amendment: PRD-AMEND-20260222-01

### Problem
Errors while resolving bot identity are not counted toward the PRD workflow retry/failure state machine.

`advance_issue` resolves bot login with `?` before entering transition wrappers (`src/daemon/interactive_prd.rs:375`, `src/daemon/interactive_prd.rs:379`), but retry accounting happens inside `finish_transition`/`apply_transition_result` (`src/daemon/interactive_prd.rs:920`, `src/daemon/interactive_prd.rs:936`, `src/daemon/interactive_prd.rs:944`).  
Result: repeated `gh api user` failures never increment `error_count`, never persist `last_error`, and never transition to `Failed`, violating the retry-exhaustion rule.

### Proposed Change
Route bot-login resolution through transition error handling so failures are wrapped by `finish_transition` and persisted:

1. Move `get_or_fetch_bot_login(...)` into the transition wrapper result path (or compose it into the `result` passed to `finish_transition`).
2. Ensure login-resolution failures increment `error_count` and can trigger `Failed` after 3 consecutive failures.
3. Add regression tests for repeated bot-login failure in `AwaitingAnswers` and `AwaitingFeedback`.

### Affected Files
- `src/daemon/interactive_prd.rs` - include bot-login failures in transition retry accounting.
- `src/validate/tests_interactive_prd.rs` - add conformance case for repeated `gh api user` failure reaching `Failed`.
- `tests/daemon_interactive_prd.rs` - add integration coverage for login-failure retry exhaustion.

## Amendment: PRD-AMEND-20260222-02

### Problem
Approval label updates can orphan an issue from polling when partial GitHub failure occurs.

`do_approval_transition` removes `ralph:prd-active` before adding `ralph:prd-done` (`src/daemon/interactive_prd.rs:800`, `src/daemon/interactive_prd.rs:807`).  
Polling only scans `ralph:prd` and `ralph:prd-active` (`src/daemon/interactive_prd.rs:322`, `src/daemon/interactive_prd.rs:338`).

If `--add-label ralph:prd-done` fails after `--remove-label ralph:prd-active` succeeds, state stays non-terminal (`AwaitingFeedback` + incremented error) but the issue no longer has a polled label, so it will not be retried. This breaks restart/retry safety.

### Proposed Change
Make approval label mutation boundary-safe:

1. Add `ralph:prd-done` first.
2. Remove `ralph:prd-active` second.
3. Keep retry semantics so partial failures remain pollable (`ralph:prd-active` still present) and can recover on next tick.
4. Add regression tests for partial failure during approval label swap.

### Affected Files
- `src/daemon/interactive_prd.rs` - reorder approval label operations to preserve poll visibility on partial failure.
- `src/validate/tests_interactive_prd.rs` - add conformance test for approval label-swap partial failure recovery.
- `tests/daemon_interactive_prd.rs` - add integration test where add/remove label failure is injected mid-approval.
