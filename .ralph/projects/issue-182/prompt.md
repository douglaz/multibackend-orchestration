## Summary

When the daemon process dies (crash, OOM, signal), child processes (`ralph quick-dev-auto` / `ralph auto`) continue running as orphans. On restart, `reconcile_in_progress_labels()` (line 963 of `src/daemon/runtime.rs`) blindly resets all `ralph:in-progress` labels to `ralph:ready` without checking whether the spawned child processes survived. This causes (a) labels that don't reflect actual task state, (b) duplicate dispatch of the same issue, and (c) orphaned children whose terminal label transitions (`ralph:completed`/`ralph:failed`) never fire because `collect_children` never runs for them.

This spec adds PID/PGID-based orphan detection by extending the existing `TaskMetadata` persistence layer so the daemon can distinguish "crashed with no survivor" from "crashed but child still running" during startup reconciliation, and can adopt surviving orphans into a dedicated tracking map for eventual terminalization through the standard `complete_task` flow.

## Acceptance Criteria

1. **PID/PGID persistence on dispatch**: Inside `dispatch_task()`, immediately after the child process is spawned and before the `ChildHandle` is constructed, the child's PID and PGID are persisted to the existing `TaskMetadata` JSON file (`.ralph/daemon/tasks/{task_id}.json`) by extending `TaskMetadata` with `pid: Option<u32>` and `pgid: Option<u32>` fields. This ensures the PID is on disk before `dispatch_task` returns, closing the crash window between spawn and persistence.
2. **Smart reconciliation**: On startup reconciliation, for each `ralph:in-progress` issue, the daemon loads its `TaskMetadata`. If `pid` and `pgid` are present and the process is verified alive (using `process::pid_exists` and the new `process::pgid_exists`), the issue is **not** reset to `ralph:ready`.
3. **Orphan adoption into dedicated map**: Surviving orphan processes are tracked in a separate `adopted_orphans: HashMap<u32, OrphanInfo>` map (keyed by issue number), **not** in `children`. `OrphanInfo` stores `pid`, `pgid`, and `task_id`. This avoids attempting to reconstruct a `ChildHandle` (which requires a live `tokio::process::Child`, watcher handles, and other fields that cannot be recovered from a bare PID).
4. **Dead-process reconciliation**: If `TaskMetadata` contains a PID/PGID but the process is dead (or the PGID check fails), reconciliation proceeds normally (reset to `ralph:ready`) and the PID/PGID fields are cleared from the metadata file.
5. **PID/PGID cleared on child completion**: In `collect_children()`, after removing the child from `children` and before calling `complete_task`, the PID/PGID fields are cleared from `TaskMetadata` via a load-modify-save cycle.
6. **PID/PGID cleared on dispatch failure/panic**: In `poll_and_claim()` Stage 3, on `DispatchOutcome::Failure` and `DispatchOutcome::Panic`, the PID/PGID fields are defensively cleared from `TaskMetadata` (the child may or may not have been spawned before the failure).
7. **No duplicate dispatch**: `poll_and_claim` skips issues present in `adopted_orphans` (new guard alongside the existing `children.contains_key` check at line 1107).
8. **Adopted orphans count toward `max_concurrent`**: The slot calculation (line 851) uses `children.len() + adopted_orphans.len()` as the active count, preventing over-subscription.
9. **Orphan terminalization through `complete_task`**: When an adopted orphan's process exits (detected via `pid_exists`/`pgid_exists` polling each main-loop iteration), the daemon determines a terminal label using a heuristic (merged PR → `ralph:completed`, otherwise → `ralph:failed`), then routes the transition through `complete_task()` to preserve all side effects: completion comment, PR flow, label swap, and worktree cleanup.
10. **`daemon abort` handles adopted orphans**: `kill_aborted_children` is extended to also query labels for adopted orphans and terminate their process groups via `terminate_process_group` when no longer `ralph:in-progress`.
11. **PID/PGID write failure is best-effort**: If `save_task_metadata` fails when persisting PID/PGID, a warning is logged but dispatch is not rolled back. The child is already running; on next restart, reconciliation will find no PID and reset the label normally.
12. **PGID validation mitigates PID reuse**: Liveness checks require both `pid_exists(pid)` and `pgid_exists(pgid)` to return true, and that the stored `pid == pgid` (since all daemon children are session leaders via `setsid()`). This makes false-positive adoption from PID reuse highly unlikely — a reused PID would also need to be a session leader with a matching PGID.

## Technical Approach

### 1. Extend `TaskMetadata` (reuse existing persistence)

Add `pid` and `pgid` fields to `TaskMetadata` (`src/daemon/runtime.rs` lines 708–713):

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TaskMetadata {
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub pgid: Option<u32>,
}
```

The `#[serde(default)]` annotations ensure backward compatibility with existing metadata files that lack these fields. No new persistence path is introduced — the existing `.ralph/daemon/tasks/{task_id}.json` files carry all durable per-task state.

All callers of `save_task_metadata` must use a load-modify-save pattern to avoid clobbering fields set by other writers (e.g., the draft-PR watcher saving `pr_url` must preserve `pid`/`pgid`, and the dispatch spawn saving `pid`/`pgid` must preserve `pr_url`).

### 2. Write PID/PGID inside `dispatch_task` (close crash window)

In `dispatch_task()` (`src/daemon/runtime.rs` line ~1595), immediately after the child process is spawned (`spawned.pid` and `spawned.pgid` are known) and **before** constructing the `ChildHandle` return value:

```rust
// Persist PID/PGID immediately so a crash between here and
// ChildHandle insertion is recoverable on next startup.
{
    let mut meta = load_task_metadata(&workspace_root, &task_id);
    meta.pid = Some(spawned.pid);
    meta.pgid = Some(spawned.pgid);
    save_task_metadata(&workspace_root, &task_id, &meta);
}
```

This is placed inside `dispatch_task` rather than in `poll_and_claim` Stage 3 (where `children.insert` happens) to eliminate the crash window: if the daemon dies after spawn but before Stage 3, the PID is already on disk.

### 3. Add `pgid_exists` to `process.rs` (reuse existing utility pattern)

Add a `pgid_exists` function to `src/daemon/process.rs` alongside the existing `pid_exists` (line 504), following the same `kill`-based pattern:

```rust
/// Check if a process group with the given PGID exists.
pub fn pgid_exists(pgid: u32) -> bool {
    if pgid <= 1 {
        return false;
    }
    let Ok(raw_pgid) = i32::try_from(pgid) else {
        return false;
    };
    // Sending signal 0 to -pgid checks the entire process group.
    match kill(Pid::from_raw(-raw_pgid), None) {
        Ok(_) => true,
        Err(Errno::EPERM) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => false,
    }
}
```

This reuses the existing `nix` imports and EPERM/ESRCH handling from `pid_exists`. The negative PID convention targets the process group rather than a single process.

### 4. `OrphanInfo` struct and adopted-orphans map

Add a lightweight struct in `src/daemon/runtime.rs`:

```rust
/// Metadata for an orphaned child process adopted after daemon restart.
struct OrphanInfo {
    pid: u32,
    pgid: u32,
    task_id: String,
}
```

In the `run()` function (line 822), initialize alongside `children`:

```rust
let mut children: HashMap<u32, ChildHandle> = HashMap::new();
let mut adopted_orphans: HashMap<u32, OrphanInfo> = HashMap::new();
```

### 5. Smart reconciliation with orphan adoption

Replace the current `reconcile_in_progress_labels()` (lines 963–1000) with a new signature:

```rust
async fn reconcile_in_progress_labels(
    config: &DaemonRuntimeConfig,
    adopted_orphans: &mut HashMap<u32, OrphanInfo>,
) -> Result<()>
```

For each `ralph:in-progress` issue:

1. Derive `task_id` via `format_task_id(&config.owner, &config.repo, issue.number)`.
2. Call `load_task_metadata(&config.workspace_root, &task_id)`.
3. If `meta.pid` and `meta.pgid` are both `Some`, and `meta.pid == meta.pgid` (session-leader invariant), and `process::pid_exists(pid)` and `process::pgid_exists(pgid)` both return true:
   - **Skip label reset**. Insert into `adopted_orphans`.
   - Log: `"reconcile: adopting orphan for issue #{} (pid={}, pgid={})"`.
4. Otherwise:
   - Proceed with `ralph:in-progress` → `ralph:ready` swap (existing behavior).
   - If metadata had stale PID/PGID, clear them via load-modify-save.
   - Remove stale PID file if present.

### 6. Orphan polling in main loop

In the main loop (after `collect_children`, before `poll_and_claim`), add an orphan-polling phase:

```rust
poll_adopted_orphans(config, &mut adopted_orphans, &repo_root_lock).await;
```

The `poll_adopted_orphans` function iterates over `adopted_orphans` and for each entry:

1. Check `process::pid_exists(pid) && process::pgid_exists(pgid)`.
2. **If still alive**: do nothing (label stays `ralph:in-progress`).
3. **If dead**: remove from `adopted_orphans`, determine terminal label, and route through `complete_task()`.

**Terminal label heuristic for orphans** (since exit code is unrecoverable):
- Load `TaskMetadata` to get the task's branch name (derivable from `task_id` via the existing `format_branch_name` convention).
- Call `github::find_existing_pr(&config.owner, &config.repo, &branch)` to check for a PR.
- If a PR exists and is merged → `"ralph:completed"`.
- Otherwise → `"ralph:failed"`.

This routes through the existing `complete_task()` function (line 2029), which handles: completion comment posting, PR flow (on success), `ralph:in-progress` → terminal label swap, and worktree cleanup. This preserves all side effects that the normal `collect_children` path performs.

After `complete_task` returns, clear PID/PGID from `TaskMetadata`.

### 7. Slot calculation includes adopted orphans

In `poll_and_claim` caller (line 851), change:

```rust
let active_count = (children.len() + adopted_orphans.len()) as u32;
```

### 8. Skip adopted orphans in `poll_and_claim`

After the existing `children.contains_key` guard (line 1107), add:

```rust
if adopted_orphans.contains_key(&issue.number) {
    continue;
}
```

This requires threading `adopted_orphans` as a parameter to `poll_and_claim`.

### 9. Clear PID/PGID on child completion

In `collect_children()` Stage 2 (line 1759), after removing the child from `children`:

```rust
{
    let mut meta = load_task_metadata(&config.workspace_root, &task_id);
    meta.pid = None;
    meta.pgid = None;
    save_task_metadata(&config.workspace_root, &task_id, &meta);
}
```

### 10. Clear PID/PGID on dispatch failure/panic

In `poll_and_claim()` Stage 3, in the `DispatchOutcome::Failure` and `DispatchOutcome::Panic` arms (lines 1236–1275), defensively clear PID/PGID:

```rust
{
    let task_id = format_task_id(&config.owner, &config.repo, issue_number);
    let mut meta = load_task_metadata(&config.workspace_root, &task_id);
    if meta.pid.is_some() || meta.pgid.is_some() {
        meta.pid = None;
        meta.pgid = None;
        save_task_metadata(&config.workspace_root, &task_id, &meta);
    }
}
```

### 11. Extend `kill_aborted_children` for adopted orphans

Extend `kill_aborted_children` (line 1855) to accept `&mut HashMap<u32, OrphanInfo>` in addition to `children`. For each adopted orphan, query its issue labels concurrently (same pattern as the existing `children` loop). If no longer `ralph:in-progress`:

```rust
process::terminate_process_group(orphan.pgid, Duration::from_secs(10)).await;
// Clear metadata and remove from map
let mut meta = load_task_metadata(&config.workspace_root, &orphan.task_id);
meta.pid = None;
meta.pgid = None;
save_task_metadata(&config.workspace_root, &orphan.task_id, &meta);
adopted_orphans.remove(&issue_number);
```

No watcher teardown is needed for adopted orphans (they have no watcher handles).

## Files & Modules

| File | Changes |
|------|---------|
| `src/daemon/runtime.rs` | Extend `TaskMetadata` with `pid`/`pgid` fields. Add `OrphanInfo` struct. Add `adopted_orphans` map to `run()`. Modify `reconcile_in_progress_labels` signature to accept `config` + `adopted_orphans`, add PID/PGID liveness checks and orphan adoption logic. Add `poll_adopted_orphans` function for main-loop orphan reaping with `complete_task` routing. Modify slot calculation to include `adopted_orphans.len()`. Thread `adopted_orphans` through `poll_and_claim` and add skip guard. Add PID/PGID write inside `dispatch_task` after spawn. Add PID/PGID clear in `collect_children` Stage 2 and `poll_and_claim` Stage 3 failure/panic arms. Extend `kill_aborted_children` to handle adopted orphans. |
| `src/daemon/process.rs` | Add `pgid_exists(pgid: u32) -> bool` function alongside existing `pid_exists`. |
| `src/daemon/mod.rs` | No changes — `ChildHandle` struct unchanged. Adopted orphans use the separate `OrphanInfo` struct. |
| `src/daemon/worktree.rs` | No changes. |

## Testing Strategy

### Unit tests (`src/daemon/process.rs`)

1. **`test_pgid_exists_current_process`** — Call `pgid_exists` with the current process's PGID (via `getpgrp()`). Assert returns true.
2. **`test_pgid_exists_dead_group`** — Call `pgid_exists` with `u32::MAX - 1` (almost certainly no such group). Assert returns false.
3. **`test_pgid_exists_boundary`** — Call `pgid_exists(0)` and `pgid_exists(1)`. Assert both return false (guard clause).

### Unit tests (`src/daemon/runtime.rs`)

4. **`test_task_metadata_pid_roundtrip`** — Create a `TaskMetadata` with `pid: Some(12345)`, `pgid: Some(12345)`, `pr_url: Some(...)`. Save to a temp directory via `save_task_metadata`, load via `load_task_metadata`. Assert all fields round-trip correctly.
5. **`test_task_metadata_backward_compat`** — Write a JSON file with only `{"pr_url": "https://..."}` (no `pid`/`pgid` keys). Load via `load_task_metadata`. Assert `pid` and `pgid` are `None` (serde default).
6. **`test_task_metadata_concurrent_field_preservation`** — Save metadata with `pid`/`pgid` set. Load, set `pr_url`, save again. Reload and assert `pid`/`pgid` are preserved (validates load-modify-save pattern).

### Conformance tests (`src/validate/`)

7. **`test_reconciliation_skips_live_orphan`** — Using the `src/validate/` harness: spawn a long-running `sleep` child via `setsid`, persist its PID/PGID to `TaskMetadata`, label an issue `ralph:in-progress`, run `reconcile_in_progress_labels`. Assert: label is NOT reset, issue appears in `adopted_orphans`. Kill the child and clean up.
8. **`test_reconciliation_resets_dead_orphan`** — Persist a PID/PGID for a non-existent process to `TaskMetadata`, label an issue `ralph:in-progress`, run `reconcile_in_progress_labels`. Assert: label IS reset to `ralph:ready`, PID/PGID fields are cleared from metadata.
9. **`test_pid_reuse_rejected_by_pgid_mismatch`** — Persist metadata with `pid` set to the current process's PID but `pgid` set to a non-existent value (simulating PID reuse where the new process has a different PGID). Assert: reconciliation treats this as dead and resets the label.
10. **`test_adopted_orphan_counts_toward_max_concurrent`** — Set `max_concurrent = 1`, adopt one orphan, run `poll_and_claim`. Assert: no new issues are claimed (0 available slots).
11. **`test_no_duplicate_dispatch_for_adopted_orphan`** — Adopt an orphan for issue #N, then run `poll_and_claim` with issue #N in the `ralph:in-progress` poll results. Assert: issue #N is skipped, no dispatch attempted.
12. **`test_pid_lifecycle_dispatch_to_collect`** — Run a full `dispatch_task` → `collect_children` cycle. Assert: `TaskMetadata` has `pid`/`pgid` set after dispatch, and `pid`/`pgid` are `None` after collect.
13. **`test_abort_kills_adopted_orphan`** — Adopt an orphan, swap its label away from `ralph:in-progress` externally, run `kill_aborted_children`. Assert: orphan's process group is terminated, orphan removed from map.
14. **`test_orphan_terminalization_routes_through_complete_task`** — Adopt an orphan whose PID refers to a process that has already exited. Run `poll_adopted_orphans`. Assert: `complete_task` side effects fire (completion comment posted, label swapped to terminal state, worktree cleanup triggered).

### Scenario test (crash-timing)

15. **`test_crash_after_spawn_before_stage3`** — Simulate the critical crash window: call `dispatch_task` directly (which writes PID/PGID to metadata), but do NOT insert the returned `ChildHandle` into `children` (simulating a crash before Stage 3 processes the result). Then run `reconcile_in_progress_labels`. Assert: the live child is detected via metadata, adopted into `adopted_orphans`, and label is not reset. Kill the child and clean up.

## Out of Scope

- **Atomic label-aware dispatch (Option B from the issue)** — The intermediate `ralph:dispatching` label adds protocol complexity. PID-based detection solves the immediate problem with less risk. Option B can be layered on later if needed.
- **Cross-host orphan detection** — PID/PGID liveness checks only work when the daemon restarts on the same host. Multi-host deployments are not addressed.
- **Exit code recovery for orphans** — OS-level exit code is lost once the parent process dies. The merged-PR heuristic is a pragmatic approximation; exact exit code recovery is not pursued.
- **Signal handler / graceful-shutdown PID cleanup** — Adding a signal handler to proactively clear PID/PGID from metadata on graceful shutdown is a nice-to-have but not required; the liveness check on restart handles stale entries.
- **`reconcile_worktrees` integration** — `reconcile_worktrees()` (`src/daemon/worktree.rs` line 525) is not currently called from the daemon runtime (`src/daemon/runtime.rs`). If it is integrated in the future, the `active_task_ids` list passed to it should include task IDs from `adopted_orphans` to prevent premature worktree removal. This is left to the future integration work.
- **Process start-time validation** — Checking `/proc/{pid}/stat` for the process start time would further reduce PID-reuse false positives but is Linux-specific. The PGID + session-leader invariant check provides sufficient mitigation for the single-host deployment model.