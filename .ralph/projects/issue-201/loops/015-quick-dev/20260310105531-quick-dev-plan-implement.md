---
artifact: quick-dev-plan-implement
loop: 15
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T10:55:31Z
---

# Implementation: PR Review Feedback Loop — Reviewer Amendments

## Changes Made

### PRR-001 (P1): Crash-safe `stage_amendment` — `src/daemon/pr_review.rs:107-169`

**Problem**: `stage_amendment` used non-atomic `fs::write` and blindly trusted `path.exists()` as proof of a valid staged file. A crash mid-write could leave corrupt JSON that would never be restaged.

**Fix**:
1. **Atomic writes**: New staged files are written to a `.tmp` suffix first, then atomically renamed to the final path. A crash mid-write leaves only the temp file — the target path stays clean.
2. **Existing file validation**: When `path.exists()` is true, the file is read and parsed as JSON. If malformed (truncated from a previous crash), the idempotent short-circuit is skipped and the file is rewritten atomically.

**New tests**:
- `stage_amendment_recovers_from_malformed_existing_file` — writes corrupt JSON, calls `stage_amendment`, verifies the file is rewritten with valid content.
- `stage_amendment_skips_valid_existing_file` — stages twice with different bodies, verifies the original is preserved (idempotency).

### PRR-002 (P2): Clear resume-pending marker on failure — `src/daemon/runtime.rs:2715-2773`

**Problem**: Resume-pending markers were set before `completed → in-progress` label swap but never cleared on failure paths. Stale markers caused repeated re-dispatch attempts on every poll cycle.

**Fix** — marker is now cleared in two failure paths:
1. **Label swap failure** (line 2715-2720): No label change happened, so clear marker immediately.
2. **Dispatch failure with successful rollback** (line 2766-2773): Issue is back to original label, no in-flight resume, so clear marker.
3. **Dispatch failure with failed rollback** (line 2764-2765): Marker is intentionally **kept** — restart recovery needs it to detect the stuck state.

**Updated conformance test**:
- `dispatch_failure_preserves_staged_amendments` in `src/validate/tests_pr_review.rs` now asserts that the resume-pending marker is cleared after a dispatch failure with successful rollback.
