---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T03:35:31Z
---

# Apply Reviewer-Requested Fixes for PR Review Feature

All four reviewer-requested changes have been applied and verified against the full test suite (1341 tests, 0 failures).

## Fix 1 (High): Deferred reactivation now retries on subsequent cycles

**`src/daemon/runtime.rs` — `pr_review_phase()`**

Previously, `pr_review_phase` returned early when `poll_results` was empty (line 2470), so tasks deferred due to capacity constraints were never retried. Now the function builds a combined candidate list from two sources:
1. Tasks with newly-discovered amendments from `poll_pr_reviews()`
2. Tasks with previously-staged amendments (discovered via `has_staged_amendments()` scan of `.ralph/daemon/pr-review-amendments/*`)

This ensures deferred tasks are retried every cycle until capacity frees up and they can be dispatched.

## Fix 2 (High): Robust paginated JSON merge with `serde_json::Deserializer`

**`src/daemon/github.rs` — `merge_paginated_json_arrays()`**

Replaced the naive bracket-counting approach with `serde_json::Deserializer::from_str().into_iter()` streaming. This correctly handles brackets inside JSON string values (e.g. comment bodies containing `[` or `]`). The function now returns `Result<String>` instead of `String`, propagating parse errors.

**`fetch_pr_review_comments()`** — All three endpoint parse calls now use `.map_err()` to propagate errors with descriptive messages instead of silently falling back via `unwrap_or_default()`.

Added 2 new tests: `merge_paginated_json_arrays_brackets_in_strings` and `merge_paginated_json_arrays_invalid_json_returns_error`.

## Fix 3 (Medium): Resume path skips refinement and issue mutations

**`src/daemon/runtime.rs` — `dispatch_task()`**

When `resume_existing_project == true`, the entire refinement/title-update/body-update/comment-posting block is now skipped. The issue already has real content from the original dispatch, and `raw_idea` may be a placeholder string during PR review resume.

## Fix 4 (Medium): Validate conformance tests for PR review

**New file: `src/validate/tests_pr_review.rs`** — 5 conformance tests registered in `src/validate/mod.rs`:

| Test | Covers |
|------|--------|
| `pr_review::whitelist_filters_comments` | Only whitelisted users produce staged amendments; self-comments and non-whitelisted are ignored |
| `pr_review::completed_project_resumes_with_state_reset` | State reset to `in_progress` for regular projects + staging/drain roundtrip |
| `pr_review::dedup_across_restart` | Dedup state persists across simulated daemon restarts |
| `pr_review::capacity_deferral_preserves_staged` | Staged amendments survive capacity exhaustion and drain on later cycles |
| `pr_review::quick_dev_resume_resets_phase` | Quick-dev projects get `status=in_progress` + `quick_dev_phase=codex_review` + `current_phase=reviewing` |
