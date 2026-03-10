---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 2
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T15:40:46Z
---

All three changes are in place and compile successfully. Here's the summary:

# Applied fix: transient GitHub API errors no longer destroy staged amendments

## Changes made

### `src/daemon/runtime.rs` (lines 1156-1218)
- Replaced `.unwrap_or(false)` with explicit `match` on the `Result` from `is_pr_open()`
- Introduced tri-state logic via `Option<bool>`:
  - `Some(true)` — PR is open, `pr_review_phase` owns it → skip (existing behavior)
  - `Some(false)` — PR definitively closed/missing → clear artifacts (existing behavior)
  - `None` — transient API error → log warning and `continue` without clearing anything
- Only cases with no `pr_url` or unparseable PR URL resolve to `Some(false)` (deterministic, no API call involved)

### `src/validate/tests_pr_review.rs`
- Updated mock gh script: `MOCK_GH_PR_STATE=error` now causes `exit 1` with a simulated error message, allowing tests to exercise the transient failure path
- Added new test `pr_review::transient_api_error_preserves_staged` that:
  - Sets up task metadata, staged amendments, and resume-pending marker
  - Configures `MOCK_GH_PR_STATE=error` to simulate API failure
  - Presents the issue as `ralph:ready` so `poll_and_claim` encounters it
  - Asserts staged amendments are **preserved** (not cleared)
  - Asserts resume-pending marker is **preserved**
  - Asserts the warning message is logged
  - Asserts the issue was **not** dispatched via claim
