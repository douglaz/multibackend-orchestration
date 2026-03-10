---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 6
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T04:55:27Z
---

# Apply-Fixes: Staged PR-Review Amendments Stranded After Daemon Restart

## Changes Made

### 1. `pr_review_phase` now resumes `ralph:ready` issues (runtime.rs:2633)

Previously, `pr_review_phase` only checked for `ralph:completed` labels, meaning tasks that drifted to `ralph:ready` after startup reconciliation (`in-progress → ready`) were permanently skipped.

Now the code checks for **both** `ralph:completed` and `ralph:ready`, selecting the appropriate `from_label` for the label swap. The dispatch failure revert also uses the correct `from_label` instead of hardcoding `ralph:completed`.

### 2. `dispatch_task` drains staged amendments for all dispatch origins (runtime.rs:1456-1490)

Previously, staged amendment draining was gated on `origin == DispatchOrigin::PrReviewResume`. If `poll_and_claim` dispatched a `ralph:ready` task with `DispatchOrigin::Claim`, staged amendments would never be drained.

Now the drain logic runs unconditionally whenever `has_staged_amendments()` returns true, regardless of the dispatch origin. This handles the restart-drift case where `poll_and_claim` picks up the task instead of `pr_review_phase`.

### 3. New conformance test: `pr_review::restart_drift_ready_drains_staged` (tests_pr_review.rs)

Simulates the exact restart-drift scenario:
- Completed project had label swapped to `in-progress` before crash
- Startup reconciliation converts `in-progress → ready`
- Dedup keys are already persisted (comments won't be re-enqueued)
- Staged amendments exist from pre-crash polling

Asserts that:
- `pr_review_phase` picks up the `ralph:ready` task
- Label swap `ready → in-progress` occurs
- Staged amendments are drained during dispatch

### 4. Updated existing test assertions

Updated 3 existing test assertions that matched `"pr-review: resuming completed task"` to match the new log message format `"pr-review: resuming ralph:completed task"`.
