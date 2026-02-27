---
artifact: final-review-proposals
loop: 11
project: implement-an-event-driven-multi-turn-prd
backend: codex
role: final_reviewer
created_at: 2026-02-22T08:40:10Z
---

# Final Review: AMENDMENTS

## Amendment: PRD-AMD-20260222-01

### Problem
Terminal label mutations happen before durable state persistence, which can orphan the workflow if save fails:

- Done path removes `ralph:prd-active` in `src/daemon/interactive_prd.rs:812`, but persistence occurs later in `src/daemon/interactive_prd.rs:952`.
- Failed path removes active/queue labels and adds failed before saving in `src/daemon/interactive_prd.rs:1270` and `src/daemon/interactive_prd.rs:1278`.
- Polling only scans `ralph:prd` and `ralph:prd-active` in `src/daemon/interactive_prd.rs:322` and `src/daemon/interactive_prd.rs:338`.

If save fails after terminal label changes, the issue is no longer poll-visible while on-disk state remains stale/non-terminal, violating restart-safe persistence expectations.

### Proposed Change
Make terminal transitions persistence-safe:

- Keep `ralph:prd-active` until save succeeds, then remove it.
- Or compensate on save failure by re-adding `ralph:prd-active` so the issue is retry-visible.
- Treat save failures as transition errors under `error_count`/retry policy.
- Add explicit tests for save-failure recovery during Done/Failed terminalization.

### Affected Files
- `src/daemon/interactive_prd.rs` - reorder/compensate terminal label updates around persistence and count save failures.
- `tests/daemon_interactive_prd.rs` - add integration tests for terminal save-failure recovery.
- `src/validate/tests_interactive_prd.rs` - add conformance coverage for retry visibility after terminal save failure.

## Amendment: PRD-AMD-20260222-02

### Problem
Marker idempotency accepts marker text from any author, not just the daemon bot:

- Marker lookup is body-only in `src/daemon/github.rs:1263`.
- Marker posting idempotency trusts that lookup in `src/daemon/github.rs:1298`.
- Interactive PRD depends on this for question marker hydration in `src/daemon/interactive_prd.rs:457`.

A user can pre/post a marker comment and block bot posting or poison `questions_comment_id`/timestamps, which conflicts with the bot-identity rule for workflow control.

### Proposed Change
Scope marker idempotency to daemon-authored comments:

- Add bot-scoped marker lookup (`author_login == bot_login`).
- Use bot-scoped lookup for PRD question/draft/status idempotency and metadata hydration.
- Add tests where a user posts spoofed marker text and verify daemon still posts/uses its own marker comment.

### Affected Files
- `src/daemon/github.rs` - add bot-scoped marker lookup and integrate into marker-post helpers.
- `src/daemon/interactive_prd.rs` - pass/use bot identity for marker idempotency checks.
- `tests/daemon_interactive_prd.rs` - add spoofed-marker regression test.
- `src/validate/tests_interactive_prd.rs` - add conformance test for marker spoof resistance.
