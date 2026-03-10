---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 3
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T09:27:51Z
---

All changes are clean and compile. Here's a summary of what was done:

# Applied Review Fix: Gate drain/purge to PrReviewResume only

**Problem:** `drain_staged_amendments` and `purge_staged_amendments` ran for both `DispatchOrigin::Claim` and `DispatchOrigin::PrReviewResume`. A normal `ralph:ready` claim dispatch could drain+purge staged PR-review amendments without resetting quick-dev state, causing the quick-dev short-circuit to skip processing and lose staged feedback.

**Changes:**

1. **`src/daemon/runtime.rs:1456-1492`** — Wrapped the entire drain block in `if origin == DispatchOrigin::PrReviewResume { ... } else { 0 }`. This ensures:
   - Drain only runs on PR-review resume dispatches
   - Purge is naturally gated (since `drained_count` is `0` for Claim dispatches)
   - The `should_reset` variable is removed since reset now always runs when drain succeeds (it's already inside the PrReviewResume branch)

2. **`src/validate/tests_pr_review.rs`** — Added regression test `pr_review::claim_dispatch_does_not_drain_staged` that:
   - Pre-stages a PR-review amendment
   - Triggers a normal Claim dispatch via `ralph:ready` label
   - Asserts the staged amendment survives (not drained or purged)
