---
artifact: final-review-planner-positions
loop: 7
project: implement-an-event-driven-multi-turn-prd
backend: claude(opus)
role: planner
created_at: 2026-02-22T06:48:21Z
---

Now I have full context. Let me provide my positions.

# Planner Positions

## Amendment: PRD-AMEND-20260222-01

### Position
ACCEPT

### Rationale
The analysis is correct and the bug is real. Looking at `advance_issue` (line 357-384), the `get_or_fetch_bot_login(bot_login_cache)?` call on lines 375 and 379 uses `?` to propagate the error directly, which means the error bypasses `finish_transition` entirely. The transition functions (`transition_awaiting_answers_to_awaiting_feedback`, `transition_awaiting_feedback`) are never entered, so `finish_transition` is never called, `error_count` is never incremented, and `last_error` is never persisted.

This directly violates the master prompt's "Any state -> Failed" rule which states: "Recoverable errors increment `error_count`; retry next tick. Unrecoverable condition: `error_count >= 3` for the same stage." A repeated `gh api user` failure would produce infinite retries with no failure escalation and no persistent record of the errors.

The proposed fix — routing bot-login resolution through the transition error handling path — is the correct approach. The error should be captured by `finish_transition` / `apply_transition_result` so it participates in the retry-exhaustion accounting. The request for regression tests covering this scenario is appropriate.

## Amendment: PRD-AMEND-20260222-02

### Position
ACCEPT

### Rationale
The analysis identifies a genuine partial-failure window with real consequences. At lines 800-811, the approval flow calls `remove_label_with_retry("ralph:prd-active")` first, then `add_label_with_retry("ralph:prd-done")` second. Both are inside `do_approval_transition` which is wrapped by `finish_transition`, so a failure on the second call correctly increments `error_count` and keeps the state as `AwaitingFeedback`.

However, `poll_and_advance_prd` (line 318-354) only polls for issues with labels `ralph:prd` or `ralph:prd-active`. If `ralph:prd-active` was already removed but `ralph:prd-done` was not yet added, the issue has neither polled label. On the next tick the issue won't be discovered by polling, so the retry can never fire. The persisted state says `AwaitingFeedback` with an incremented error count, but the daemon will never revisit it — it's effectively orphaned.

The proposed fix — adding `ralph:prd-done` first, then removing `ralph:prd-active` second — is a simple and effective mitigation. In the partial-failure case where add succeeds but remove fails, the issue retains `ralph:prd-active` and remains pollable on the next tick. The only downside is a brief window where both labels coexist, which is harmless since the persisted state is still `AwaitingFeedback` and will complete the transition on retry.

The same label-ordering principle is already used in the failure transition (line 1226-1230), where `remove_label_with_retry` for `ralph:prd-active` is done as best-effort after the `ralph:prd-failed` label is added. Consistency with that pattern further supports this change.
