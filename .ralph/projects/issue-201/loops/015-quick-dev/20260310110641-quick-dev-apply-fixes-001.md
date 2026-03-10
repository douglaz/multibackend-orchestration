---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T11:06:41Z
---

All changes look correct. Here's a summary of the two fixes applied:

**Fix 1 (High) — Resume marker preserved on `ralph:ready` recovery failures:**
- `runtime.rs:2720` and `runtime.rs:2777`: Both `clear_resume_pending_marker` calls are now guarded by `if from_label == "ralph:completed"`. When recovering from `ralph:ready` (pre-existing marker), the marker persists through transient label-swap and dispatch failures, keeping retry recovery intact.

**Fix 2 (Medium) — `has_staged_amendments()` ignores temp files:**
- `pr_review.rs:248-268`: The function now only counts entries with a `.json` extension, filtering out `.tmp` crash artifacts that could trigger no-op resume attempts.
- `pr_review.rs:1069-1095`: New unit test `has_staged_amendments_ignores_tmp_files` verifies that a staging dir with only `.tmp` files returns `false`, and returns `true` once a real `.json` file is added.
