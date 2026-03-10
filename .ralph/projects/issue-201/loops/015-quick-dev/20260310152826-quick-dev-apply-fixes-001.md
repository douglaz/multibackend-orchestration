---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T15:28:26Z
---

Both fixes are applied. Here's the summary:

# Applied Reviewer Fixes

## Fix 1 (High): Claim-path guard narrowed to prevent permanent starvation

**File:** `src/daemon/runtime.rs:1130-1199`

The guard in `poll_and_claim` now only skips claim dispatch when PR-review can truly own the issue:
- Task metadata exists with a `pr_url`
- That PR is confirmed open via `github::is_pr_open()`

If either condition fails (no task metadata, no `pr_url`, or PR is closed/merged), the guard clears stale marker/staged artifacts and allows normal claim dispatch, with a warning log. This prevents `ralph:ready` issues from being permanently blocked when `pr_review_phase` cannot actually dispatch them.

Added `clear_staged_amendments()` to `pr_review.rs` to support cleanup of stale staging artifacts.

## Fix 2 (Medium): Marker-clear logic made conservative for ambiguous failures

**File:** `src/daemon/runtime.rs:2876-2893`

Changed `unwrap_or(true)` to `== Some(true)` for `from_label_restored`. Now the resume-pending marker is only cleared when label restoration is *confirmed* (`Some(true)`). For ambiguous cases:
- `None` (remove step failed — label may be absent due to concurrent removal)
- `Some(false)` (rollback explicitly failed)

...the marker is preserved so restart recovery can detect and retry the stranded state next cycle.
