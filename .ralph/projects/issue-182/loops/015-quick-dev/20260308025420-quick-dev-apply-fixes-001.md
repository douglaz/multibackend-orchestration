---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-182
backend: claude
role: implementer
created_at: 2026-03-08T02:54:20Z
---

# Apply-Fixes: Reviewer Changes Applied

All three reviewer-requested changes have been applied:

## 1. [HIGH] PID/PGID metadata cleared on all child-removal paths

- **`kill_aborted_children` child branch** (`runtime.rs:2086-2091`): Added load-modify-save to clear `pid`/`pgid` from `TaskMetadata` after killing the child and tearing down watchers.
- **`drain_all_children` forced-kill branch** (`runtime.rs:2150-2155`): Added load-modify-save to clear `pid`/`pgid` from `TaskMetadata` after force-killing and before `complete_task`.

## 2. [MEDIUM] Session-leader invariant enforced in `poll_adopted_orphans`

- `runtime.rs:1069-1074`: Added `info.pid != info.pgid` as a dead-detection condition alongside the existing `pid_exists`/`pgid_exists` checks. An orphan where `pid != pgid` is now treated as dead (PID reuse mitigation).

## 3. [MEDIUM] Test coverage added

**Unit tests in `process.rs`** (3 tests):
- `test_pgid_exists_current_process` — verifies current process's PGID is detected as alive
- `test_pgid_exists_dead_group` — verifies non-existent PGID returns false
- `test_pgid_exists_boundary` — verifies guard clause rejects 0 and 1

**Unit tests in `runtime.rs`** (3 tests):
- `test_task_metadata_pid_roundtrip` — verifies all fields survive save/load
- `test_task_metadata_backward_compat` — verifies old JSON without `pid`/`pgid` defaults to `None`
- `test_task_metadata_concurrent_field_preservation` — verifies load-modify-save preserves other writers' fields

**Conformance tests in `tests_daemon_orphan.rs`** (7 tests, registered in `mod.rs`):
- `reconciliation_skips_live_orphan` — live child is adopted, not reset
- `reconciliation_resets_dead_orphan` — dead PID is reset, metadata cleared
- `pid_reuse_rejected_by_pgid_mismatch` — mismatched pid/pgid treated as dead
- `adopted_orphan_counts_toward_max_concurrent` — orphan fills slot, new issue skipped
- `no_duplicate_dispatch_for_adopted_orphan` — adopted orphan not re-dispatched
- `pid_lifecycle_dispatch_to_collect` — PID/PGID set on dispatch, cleared on collect
- `abort_kills_adopted_orphan` — externally aborted orphan is killed, metadata cleared

**Verification**: `cargo check` passes, all 6 unit tests pass.
