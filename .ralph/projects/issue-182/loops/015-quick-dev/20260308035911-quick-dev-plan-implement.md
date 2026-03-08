---
artifact: quick-dev-plan-implement
loop: 15
project: issue-182
backend: claude
role: implementer
created_at: 2026-03-08T03:59:11Z
---

The changes are clean and address both review issues. Here's a summary:

# Implementation Summary

## P3 Fix: `pid_lifecycle_dispatch_to_collect` - Missing "set after dispatch" assertion

**Files changed:**
- `src/validate/mock_scripts.rs` — Added `daemon_mock_ralph_meta_snapshot_script()` helper that creates a mock ralph script which sleeps briefly (to let the parent write PID/PGID), snapshots the metadata file to a capture path, then exits.
- `src/validate/tests_daemon_orphan.rs` — Updated `pid_lifecycle_dispatch_to_collect` to:
  1. Use the metadata-snapshotting mock ralph instead of the instant-exit mock
  2. **Phase 1 (new):** Assert the snapshot file exists and contains `pid: Some(...)` and `pgid: Some(...)`, proving PID/PGID were set during dispatch while the child was alive
  3. **Phase 2 (existing):** Assert PID/PGID are `None` after completion/collection

This catches regressions where `dispatch_task` fails to persist PID/PGID.

## P2 Fix: `crash_after_spawn_before_stage3` - Doesn't exercise dispatch

**File changed:** `src/validate/tests_daemon_orphan.rs` — Replaced the terse doc comment with comprehensive documentation explaining:
- Why manual spawn + metadata write is used (simulating a mid-dispatch crash isn't possible through the daemon binary)
- How `pid_lifecycle_dispatch_to_collect` covers the dispatch persistence path
- That this test focuses on reconciliation logic (adoption vs label reset)
- That format compatibility is guaranteed by using the same `TaskMetadata` struct and `save_task_metadata` function as `dispatch_task`
