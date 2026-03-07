---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 3
project: issue-172
backend: claude
role: implementer
created_at: 2026-03-07T08:16:07Z
---

# Apply-Fixes: Reviewer-Requested Changes

## Changes Made

### Fix 1 (High): `infer_phase_iteration` loses pre-commit re-review iteration after resume

**Root cause:** `Phase::Reviewing` iteration was inferred solely from `reviews.last()`, ignoring pre-commit response iterations that increment the phase counter at runtime without adding a `ReviewExchange`.

**Changes:**

- **`src/project/state.rs:173`** — Added `latest_pre_commit_response_iteration: Option<u32>` field to `FeatureLoopArtifacts`, tracking the highest `impl-pre-commit-response` iteration for reconstruction.

- **`src/project/lifecycle.rs:1040-1050`** — Updated `infer_phase_iteration` for `Phase::Reviewing` to use `max(review_next, pre_commit_response_next)` instead of only `review_next`.

- **`src/project/lifecycle.rs:717`** — Populated `latest_pre_commit_response_iteration` from `pre_commit_responses.keys().last()` during `reconstruct_feature_loop`.

- **`src/workflow/orchestrator.rs:1252`** — Set `latest_pre_commit_response_iteration` at runtime when writing a pre-commit response artifact.

- **All `FeatureLoopArtifacts` initializers** (`state.rs`, `orchestrator.rs` ×2, `summary.rs` ×2, `tests/state.rs` ×9, `tests/status_history.rs`) — Added `latest_pre_commit_response_iteration: None`.

### Fix 2 (Medium): Reconstruction resurrects stale approval after pre-commit failure

**Root cause:** `reconstruct_feature_loop` unconditionally restored the latest `review-approved.md` artifact, even when a newer pre-commit failure had cleared it at runtime.

**Changes:**

- **`src/project/lifecycle.rs:724-731`** — Added `effective_approval`: when `pending_pre_commit_feedback` is `Some`, the approval is set to `None`, mirroring the runtime behavior at `orchestrator.rs:1991`.

- **`src/project/lifecycle.rs:778,800`** — Changed `completed_at`, `status`, and `approval` in the artifacts struct to use `effective_approval` instead of the raw `approval` reference. The original `approval` reference is still available for reviewer backend extraction.

### Tests Added

1. **`infer_reviewing_iteration_accounts_for_pre_commit_response`** — Verifies `Phase::Reviewing` iteration correctly uses `max(review_next, pre_commit_response_next)`, including when pre-commit response iteration exceeds review exchanges.

2. **`reconstruction_invalidates_stale_approval_on_pending_pre_commit_failure`** — Verifies that approval, status, and completed_at are cleared when a pre-commit failure has no matching response.

3. **`reconstruction_preserves_approval_when_pre_commit_failure_is_responded`** — Verifies approval is preserved when the pre-commit failure has a matching `impl-pre-commit-response` artifact.
