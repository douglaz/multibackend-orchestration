---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T08:44:54Z
---

# Apply reviewer-requested fixes for PR review polling

## Changes made

### Issue 1 (High): Dedup does not guarantee "exactly once" across poll cycles/restarts

**Root cause:** `stage_amendment` used timestamp-based filenames (`{timestamp}-{sanitized_id}.json`), so a crash between staging and dedup-state persist would produce a duplicate file with a new timestamp on the next poll cycle.

**Fix in `src/daemon/pr_review.rs:121-133`:**
- Changed filename to **deterministic** format: `{sanitized_amendment_id}.json` (no timestamp)
- Added `path.exists()` check — if the file already exists, staging is a no-op
- This makes staging **idempotent per comment key**: even if dedup state save fails after staging, the next poll cycle will find the existing file and skip it, preventing duplicate amendments

### Issue 2 (Medium): PR-review resume path is over-broad for `ralph:ready`

**Root cause:** `pr_review_phase` resumed both `ralph:completed` and `ralph:ready` issues unconditionally. Combined with `dispatch_task` resetting project state whenever staged amendments existed (regardless of dispatch origin), this could mutate legitimately resumable non-completed `ralph:ready` projects.

**Fix — three parts:**

1. **`src/daemon/pr_review.rs:309-354`** — Added resume-pending marker functions (`set_resume_pending_marker`, `has_resume_pending_marker`, `clear_resume_pending_marker`) that write/check/remove a marker file at `.ralph/daemon/pr-review-pending/{task_id}.marker`

2. **`src/daemon/runtime.rs:2636-2660`** — Gated `ralph:ready` resume to **only** fire when the resume-pending marker exists (restart-drift recovery). Set marker before label swap from `ralph:completed`; clear after successful dispatch.

3. **`src/daemon/runtime.rs:1456-1490`** — Changed `_origin` → `origin` and gated `reset_project_state_for_resume` to only run when `origin == DispatchOrigin::PrReviewResume`. Normal `Claim` dispatches still drain staged amendments (safe copy) but do **not** reset project state.

### Tests added

- `stage_amendment_is_idempotent` — stages the same amendment twice, asserts exactly one file exists
- `resume_pending_marker_roundtrip` — set/check/clear lifecycle of the marker file
