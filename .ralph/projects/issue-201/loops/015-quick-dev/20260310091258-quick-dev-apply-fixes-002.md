---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 2
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T09:12:58Z
---

All 425 tests pass, including `pr_review::restart_drift_ready_drains_staged` which now shows `ok`.

# Fix: create resume-pending marker in `restart_drift_ready_drains_staged` test

## Changes made

**`src/validate/tests_pr_review.rs`** — 3 edits:

1. **Import** `has_resume_pending_marker` and `set_resume_pending_marker` from `crate::daemon::pr_review` (line 7-9)

2. **Create marker before daemon start** (line 879-883): Added `set_resume_pending_marker(&ws_root, "acme-widgets-42")` after dedup state setup. This simulates the real scenario where the previous PR-review resume created the marker before the daemon crashed. Without it, the runtime gate at `runtime.rs:2642-2646` skips `ralph:ready` tasks that lack the marker.

3. **Assert marker cleared after dispatch** (line 944-948): Added `!has_resume_pending_marker(...)` assertion to verify the marker is properly cleaned up after successful dispatch.

`nix build -L` passes with all 425 conformance tests green.
