---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 4
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T04:29:30Z
---

All changes look correct. Here's a summary:

# Applied Reviewer Fixes

## Fix 1 (High): Scoped PR-review drain/reset/purge to `DispatchOrigin::PrReviewResume`

- Added `DispatchOrigin` enum with `Claim` and `PrReviewResume` variants (`runtime.rs:1363-1370`)
- Added `origin: DispatchOrigin` parameter to `dispatch_task` (`runtime.rs:1381`)
- Normal claim flow passes `DispatchOrigin::Claim` (`runtime.rs:1215`)
- PR-review resume flow passes `DispatchOrigin::PrReviewResume` (`runtime.rs:2666`)
- Drain/reset block now only executes when `origin == DispatchOrigin::PrReviewResume` (`runtime.rs:1460`)
- Purge now only executes when `drained_count > 0` (`runtime.rs:1773`)

This prevents normal claim dispatches from accidentally draining or purging staged amendments, and prevents purging when no amendments were actually drained (e.g., when the project dir doesn't exist).

## Fix 2 (Medium): Shared PR-open cache across poll and dispatch phases

- `poll_pr_reviews` now accepts `pr_open_cache: &mut HashMap<u32, bool>` (`pr_review.rs:413`)
- It checks/populates the cache before making API calls (`pr_review.rs:444-466`)
- `pr_review_phase` creates the cache before calling `poll_pr_reviews` and reuses it during the dispatch candidate loop (`runtime.rs:2498-2500`)
- Eliminated redundant `is_pr_open` API calls for the same PR within a single poll cycle

Build passes and all 22 `pr_review` tests pass.
