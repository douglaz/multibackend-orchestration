---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T14:46:55Z
---

# Apply-Fixes: PR-Review Resume and Stranded-Issue Recovery

## Changes Made

### Fix 1: `ready + staged + no marker` issues are no longer permanently skipped

**`src/daemon/runtime.rs` (pr_review_phase resume selection, ~line 2754)**

The `ralph:ready` resume condition now accepts staged amendments in addition to the resume-pending marker:

```rust
// Before: only marker
} else if labels.iter().any(|l| l == "ralph:ready")
    && has_resume_pending_marker(...)

// After: marker OR staged amendments  
} else if labels.iter().any(|l| l == "ralph:ready")
    && (has_marker || has_staged)
```

The marker-setting logic was also broadened: previously only set for `ralph:completed`, now set whenever no marker exists (`!has_marker`), covering the `ready + staged + no marker` path.

The swap-failure marker-cleanup logic was updated correspondingly: `from_label == "ralph:completed"` → `!has_marker` (only clear markers that were freshly created in the current cycle).

### Fix 2: Stranded issues (`marker + no lifecycle label`) are now recovered

**`src/daemon/runtime.rs` (pr_review_phase resume selection, ~line 2760)**

Added a new branch in the `from_label` selection that detects `no_lifecycle && has_marker`. When hit, it re-adds `ralph:ready` via `add_label_with_retry`, logs recovery, and proceeds with the normal swap path. If the re-add fails, the iteration is skipped (marker persists for retry).

### New validate tests

**`src/validate/tests_pr_review.rs`**

1. **`pr_review::ready_staged_no_marker_dispatches`** — Sets up a `ralph:ready` issue with staged amendments but NO resume-pending marker. Verifies that `pr_review_phase` dispatches the task (label swap + drain).

2. **`pr_review::stranded_no_lifecycle_recovered_via_marker`** — Sets up an issue with NO lifecycle label but WITH a resume-pending marker and staged amendments. Verifies recovery: `ralph:ready` is re-added, label swap to `ralph:in-progress` occurs, and amendments are drained.

### Compilation

`cargo check` passes with no errors.
