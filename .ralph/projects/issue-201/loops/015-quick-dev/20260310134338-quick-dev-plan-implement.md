---
artifact: quick-dev-plan-implement
loop: 15
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T13:43:38Z
---

# Implementation Complete

All 5 review amendments have been addressed:

## Changes Made

### P1: Atomic task metadata persistence + corrupt metadata handling
- **`src/daemon/runtime.rs`**: Made `save_task_metadata()` use atomic temp-file + rename pattern to prevent crash-interrupted writes from creating corrupt JSON
- **`src/daemon/pr_review.rs`**: Replaced `unwrap_or_default()` in `discover_tasks_with_prs()` with explicit parse error handling that logs warnings for both corrupt JSON and I/O errors, so tasks are not silently lost

### P2: Tighter stage_amendment idempotency validation
- **`src/daemon/pr_review.rs`**: Changed `stage_amendment()` to validate existing files as `AmendmentRequest` with matching `id` and `source: PrReview` (not just any valid JSON). Added `stage_amendment_rewrites_valid_json_but_invalid_amendment` unit test

### P2: Test name correction
- **`src/validate/tests_pr_review.rs`**: Renamed `partial_swap_failure_preserves_marker` → `partial_swap_failure_clears_marker_on_rollback` to match actual assertion behavior (marker is cleared when rollback succeeds)

### P2: Dispatch-success assertion in capacity test
- **`src/validate/tests_pr_review.rs`**: Added assertion that `stderr.contains("dispatched task")` in `capacity_deferral_preserves_staged` to prove capacity constraint caused selective deferral rather than total failure

### P3: Exact key set assertions in whitelist test
- **`src/validate/tests_pr_review.rs`**: Added assertions for exact included keys (`pull_comment:1`, `issue_comment:10`, `review:20`) and explicit absence of excluded keys (`pull_comment:2`, `issue_comment:11`) in `whitelist_filters_comments`

## Verification
- Compilation: clean (zero warnings)
- Full test suite: 1,362 tests pass, 0 failures
