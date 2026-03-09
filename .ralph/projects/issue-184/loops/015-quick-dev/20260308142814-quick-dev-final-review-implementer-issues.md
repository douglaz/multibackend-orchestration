---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T14:28:14Z
---

Excellent tests. The drain tests are thorough - they verify both the cooperative cancellation path and the force-abort path, and even check that the aborted task stops producing side effects.

Now I have a comprehensive understanding of all changes. Let me compile the final review:

# Final Review: AMENDMENTS

## Summary

The implementation is well-structured and addresses all core acceptance criteria. The migration from subprocess dispatch to in-process tokio tasks is sound: `dispatch_task()` correctly spawns tasks via `spawn_inprocess_task()`, `collect_children()` properly uses `JoinHandle::is_finished()`, `kill_aborted_children()` correctly cancels tokens, and `drain_all_children()` implements cooperative-then-forced shutdown. CWD safety is correctly enforced throughout — all library paths are explicit, `Workspace::load()` is used instead of `discover()`, and `set_cwd()` is called with correct repo roots. Environment sanitization is applied at the right layer in `CliBackend::execute_streaming()`. The `KillOnDrop` guard correctly protects against orphaned backend processes on cancellation. Per-task tracing isolation via `WithSubscriber` is the correct approach.

Two amendments are identified:

## Amendment: A1

### Problem

**`[P1]`** In `KillOnDrop::drop()` (`src/backend/mod.rs:114`), the reaper thread calls `libc::waitpid(raw, std::ptr::null_mut(), 0)` — a **blocking** waitpid (flags=0). After SIGKILL, the process normally exits immediately, making this benign in practice. However, if the child process is stuck in an uninterruptible kernel state (D state, e.g. NFS hang), this thread will block indefinitely. Since `KillOnDrop` is a fire-and-forget safety net, a thread that blocks forever is a resource leak (one OS thread consumed per stuck backend process, never released).

By contrast, the async `kill_and_reap_child()` at line 878 correctly uses the non-blocking `child.try_wait()` pattern. The reaper thread should match this defensive style.

### Proposed Change

Change the final `waitpid` in the reaper thread (line 114) to use `WNOHANG` instead of blocking flags `0`, consistent with the pattern already used at lines 75 and 99. The zombie will be reaped by tokio's internal reaper if the WNOHANG call misses it, so no cleanup is lost.

```rust
// Line 114: Change from:
libc::waitpid(raw, std::ptr::null_mut(), 0);
// To:
libc::waitpid(raw, std::ptr::null_mut(), libc::WNOHANG);
```

### Affected Files
- `src/backend/mod.rs:114` - Change `waitpid` flags from `0` to `libc::WNOHANG`

---

## Amendment: A2

### Problem

**`[P2]`** The `drain_all_children()` force-abort path (`src/daemon/runtime.rs:2073-2081`) calls `complete_task()` **after** removing the `TaskHandle` from the map and **outside** panic isolation. If `complete_task` panics (e.g. a GitHub API serialization bug), the panic propagates to the drain loop and prevents subsequent tasks from being completed, leaving them stuck as `ralph:in-progress`.

By contrast, the normal `collect_children()` path (lines 1857-1873) wraps `complete_task` in an inner `tokio::spawn` for panic isolation with an explicit `ralph:failed` rollback on panic (lines 1897-1903). The drain force-abort path lacks this protection.

### Proposed Change

Wrap the `complete_task()` call in `drain_all_children_with_deadline()` at lines 2073-2081 in panic-isolation matching the pattern from `collect_children()`:

```rust
// Replace the bare complete_task call with panic-isolated version:
let config_clone = config.clone();
let repo_root_lock_clone = repo_root_lock.clone();
let tid = task_id.clone();
let inner = tokio::spawn(async move {
    complete_task(&config_clone, issue_number, &tid, "ralph:failed", false, &repo_root_lock_clone).await;
});
if let Err(join_err) = inner.await {
    eprintln!("warning: complete_task panicked for {task_id} during drain: {join_err}");
    if let Err(rollback_err) = github::swap_lifecycle_label(
        &config.owner, &config.repo, issue_number,
        "ralph:in-progress", "ralph:failed",
    ).await {
        eprintln!("warning: drain panic rollback failed for {task_id}: {rollback_err}");
    }
}
```

### Affected Files
- `src/daemon/runtime.rs:2073-2081` - Wrap `complete_task` in panic isolation

---
