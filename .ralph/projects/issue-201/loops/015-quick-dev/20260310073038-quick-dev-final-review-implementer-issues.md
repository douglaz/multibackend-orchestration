---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T07:30:38Z
---

Good. The mock returns `ralph-bot\n`, `parse_authenticated_login` trims it to `ralph-bot`. The self-comment filtering works correctly.

I've now completed a thorough review of all source changes. Let me compile my findings.

# Final Review: AMENDMENTS

## Amendment: A1

### Problem
**[P2]** In `pr_review_phase` (runtime.rs:2536-2547), when building `DispatchCandidate` entries from `poll_results`, the `pr_number` is resolved by searching `all_tasks` (a second call to `discover_tasks_with_prs`). If the matching task is not found, `pr_number` defaults to `0`. A candidate with `pr_number == 0` bypasses the PR-open guard at line 2574 (`if candidate.pr_number > 0`), allowing dispatch even if the PR has been closed between the `poll_pr_reviews` call and the dispatch loop.

While `poll_pr_reviews` already checked PR-open state (and cached it), the `poll_results` struct includes `issue_number` but not `pr_number`. If the task metadata file were concurrently deleted between the two `discover_tasks_with_prs` calls, or if the file is corrupt on the second read, `pr_num` silently falls to `0` and the safety check is skipped.

The simplest fix is to include `pr_number` in `PrReviewPollResult` so it never needs a second lookup.

### Proposed Change
Add `pr_number: u32` field to `PrReviewPollResult` in `src/daemon/pr_review.rs` (populated from `task_info.pr_number` at line 589). Then use `r.pr_number` directly in `pr_review_phase` instead of the `all_tasks.find()` fallback.

### Affected Files
- `src/daemon/pr_review.rs` - Add `pr_number` to `PrReviewPollResult`, populate in `poll_pr_reviews`
- `src/daemon/runtime.rs` - Use `r.pr_number` directly in the `poll_results` → `DispatchCandidate` mapping (lines 2536-2547)

---

## Amendment: A2

### Problem
**[P3]** `reset_project_state_for_resume` in `src/daemon/pr_review.rs:280` uses a direct `fs::write` for `state.json`, unlike `PrReviewState::save` which uses the atomic temp-file + rename pattern (lines 69-83). A crash during `fs::write` could leave a truncated `state.json` that is unparseable, causing the next dispatch attempt to fail with a JSON parse error. Since `state.json` is the project's canonical state, corruption here could permanently block the project from resuming without manual intervention.

### Proposed Change
Use the same atomic write pattern (write to `state.json.tmp`, then `fs::rename`) that `PrReviewState::save` uses.

### Affected Files
- `src/daemon/pr_review.rs` - Change `reset_project_state_for_resume` to use temp-file + rename for `state.json` writes

---

## Summary

The implementation is well-structured, with solid error handling, correct dedup semantics, and comprehensive testing (24 unit tests + 8 conformance tests). The phase ordering in the main loop is correct (`pr_review_phase` runs before `poll_and_claim`, preventing double-dispatch). The atomic dedup-state writes, copy-then-purge staged amendment pattern, and label revert on dispatch failure are all sound safety measures.

Key files verified:
- `src/daemon/pr_review.rs` — Dedup, staging, state reset, polling, comment conversion
- `src/daemon/github.rs` — `fetch_pr_review_comments`, `is_pr_open`, `merge_paginated_json_arrays` (streaming JSON parser handles brackets in strings correctly)
- `src/daemon/runtime.rs` — `pr_review_phase`, `dispatch_task` integration, `DispatchOrigin` plumbing
- `src/config/global.rs` — `daemon_pr_review_whitelist` config with TOML roundtrip test
- `src/config/mod.rs` — `EffectiveDaemonConfig` plumbing
- `src/project/amendments.rs` — `AmendmentSource::PrReview` variant with kebab-case serde
- `src/validate/tests_pr_review.rs` — 8 conformance tests covering whitelist filtering, dedup, capacity deferral, quick-dev resume, dispatch failure, stale counters, and restart drift

The two amendments above are low-severity improvements. A1 eliminates a fragile secondary lookup; A2 hardens a state write against crash corruption.
