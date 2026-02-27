Now I have all the context needed. Let me produce the updated specification addressing all four review issues.

---

## Summary

Convert the daemon runtime loop (`src/daemon/runtime.rs`) and its supporting modules from synchronous, thread-blocking code to idiomatic async Rust using the existing tokio runtime. The primary motivations are: (1) eliminate the `block_in_place` / `Handle::current().block_on()` anti-pattern in `refine.rs`, (2) replace `thread::sleep` polling with `tokio::time::sleep` so the runtime thread is available for other work, and (3) use `tokio::process::Command` for child process management to enable `.await`-based child collection instead of busy-loop polling. External behavior, CLI interface, and TaskStore file-locking semantics remain unchanged.

## Acceptance Criteria

1. `ralph daemon start` produces identical external behavior: polls GitHub issues, claims, spawns `ralph auto` children, collects results, creates PRs, and updates labels exactly as before.
2. `refine_prompt()` no longer uses `block_in_place()` or `Handle::current().block_on()`. The backend `.execute()` call is `.await`ed directly.
3. The main poll loop in `runtime::run()` uses `tokio::time::sleep()` instead of `thread::sleep()`.
4. `drain_all_children()` uses `tokio::time::sleep()` instead of `thread::sleep(50ms)`.
5. Child processes are spawned via `tokio::process::Command`, and `ActiveChild` holds a `tokio::process::Child`.
6. All `TaskStore` file-locking calls (`load`, `save`, `with_exclusive_tasks`, `update_task`) are wrapped in `tokio::task::spawn_blocking()` at the async call sites in `runtime.rs`. The `TaskStore` implementation itself is unchanged.
7. All `std::process::Command` calls in `github.rs` and `worktree.rs` are invoked via `spawn_blocking()` from async callers (Option A). The functions themselves remain synchronous.
8. Process group isolation is preserved: `setsid` via `pre_exec`, SIGTERM/SIGKILL escalation via `terminate_process_group()`.
9. CAS-style atomic task state transitions are preserved.
10. `--single-iteration` mode still waits for all children to reach terminal state before exiting.
11. `cargo test` passes with no new failures. Conformance tests in `src/validate/` are unmodified.
12. `cargo clippy` produces no new warnings.
13. `ralph validate --bin ./result/bin/ralph --filter daemon` passes with no regressions. This is the primary behavioral gate since the daemon's external contract is exercised by these conformance tests.
14. `tokio::process::Child::id()` (returning `Option<u32>`) is handled correctly: PID/PGID are captured immediately after spawn (when `id()` is guaranteed `Some`), and all downstream code uses the captured `u32` values from `SpawnedChild`, never re-querying the tokio `Child`.
15. `spawn_blocking` `JoinError`s are mapped to `RalphError::Orchestration` via a helper, never `.unwrap()`ed. Cleanup paths (`kill`, `wait`, label updates, worktree removal) remain best-effort with `warn + continue` semantics matching current behavior.

## Technical Approach

### Phase 1: Propagate `async` from `cli::run()` down to `daemon::runtime::run()`

The call chain is: `main()` (already async) → `cli::run()` (already async) → `daemon::execute()` (sync) → `execute_start()` (sync) → `runtime::run()` (sync).

- Make `daemon::execute()` async and `.await` on `execute_start()`.
- Make `execute_start()` async and `.await` on `runtime::run()`.
- Update the match arm in `cli/mod.rs` line 300 to `.await` the `daemon::execute()` call.
- `execute_status()` and `execute_abort()` remain synchronous — they run simple sync operations and return immediately. `execute()` calls them directly (no `.await`) since they don't return futures.

### Phase 2: Convert `runtime::run()` to async

- Change signature to `pub async fn run(...)`.
- Replace `thread::sleep(Duration::from_secs(poll_seconds))` on line 78 with `tokio::time::sleep(...).await`.
- Replace the `thread::sleep(50ms)` loop in `drain_all_children()` (line 447) with `tokio::time::sleep(50ms).await`.
- Change `ActiveChild.child` from `std::process::Child` to `tokio::process::Child`.
- In `collect_children()`, use `child.try_wait()` on `tokio::process::Child`. Note: `tokio::process::Child::try_wait()` is synchronous (returns `io::Result<Option<ExitStatus>>`, no `.await` needed), identical semantics to `std::process::Child::try_wait()`.
- In `drain_all_children()` force-kill path (lines 455-458), `child.start_kill()` replaces `child.kill()`. `tokio::process::Child::start_kill()` is synchronous and non-blocking — it sends SIGKILL but does not wait for the process to exit. Follow with `child.wait().await` to reap. Since this is a best-effort cleanup path, wrap both in `let _ =` to match current error-suppression behavior. Alternatively, use `child.kill().await` (the async version) wrapped in `let _ =`.
- Similarly in `dispatch_task()` lines 340-342 (kill just-spawned child on concurrent abort): use `child.start_kill()` + `child.wait().await` or `child.kill().await`, both wrapped in `let _ =`.

### Phase 3: Handle `tokio::process::Child::id()` returning `Option<u32>` (Review Issue #1)

`tokio::process::Child::id()` returns `Option<u32>` — it returns `None` once the child has been polled to completion. In `std::process::Child`, `id()` returns `u32` unconditionally.

**Approach:** PID/PGID are only read immediately after `spawn()` in `spawn_ralph_auto()`, at which point the child is guaranteed alive and `id()` returns `Some`. The fix:

```rust
let pid = child.id().ok_or_else(|| {
    RalphError::Orchestration("child exited immediately after spawn".into())
})?;
let pgid = pid;  // After setsid(), PID == PGID
```

This is the only place `child.id()` is called. All downstream code (`dispatch_task` lines 310-311, `abort_task` line 163, `terminate_process_group`) uses the `u32` values captured in `SpawnedChild.pid` / `SpawnedChild.pgid` or `DaemonTask.child_pid` / `DaemonTask.child_pgid`. These are plain `u32` / `Option<u32>` fields and are unaffected by the tokio `Child` type change.

**No other call site re-queries `child.id()`** — it is only called once in `spawn_ralph_auto()`.

### Phase 4: Introduce `spawn_blocking` error handling helper (Review Issue #2)

Add a small helper in `runtime.rs` to map `JoinError` from `spawn_blocking`:

```rust
/// Unwrap a `spawn_blocking` result, mapping `JoinError` to `RalphError`.
fn sb_unwrap<T>(result: std::result::Result<T, tokio::task::JoinError>) -> T {
    // JoinError only occurs if the blocking task panics or the runtime
    // shuts down. Both are unrecoverable — propagate the panic.
    match result {
        Ok(v) => v,
        Err(err) => {
            if let Ok(reason) = err.try_into_panic() {
                std::panic::resume_unwind(reason);
            }
            // Runtime shutdown — nothing meaningful to do. Surface as
            // RalphError so callers can propagate.
            panic!("tokio runtime shut down during spawn_blocking");
        }
    }
}
```

Usage pattern for error-propagating call sites (store operations, github calls that use `?`):

```rust
let tasks = sb_unwrap(tokio::task::spawn_blocking({
    let store = store.clone();
    move || store.load()
}).await)?;
```

Usage pattern for best-effort call sites (cleanup, label updates, worktree removal):

```rust
// Best-effort: log warning and continue (matches current behavior)
let result = tokio::task::spawn_blocking({
    let store = store.clone();
    let task_id = task_id.to_owned();
    move || store.update_task(&task_id, |t| { t.pr_url = Some(url); Ok(()) })
}).await;
if let Ok(inner) = result {
    if let Err(err) = inner {
        eprintln!("warning: failed to update PR URL: {err}");
    }
}
```

**Rationale for `resume_unwind` instead of mapping to `RalphError`:** `JoinError` from `spawn_blocking` only happens if (a) the closure panics or (b) the runtime shuts down. Case (a) should propagate the panic to preserve backtraces. Case (b) is non-recoverable. Neither can happen during normal operation, so this path is unreachable in practice — but we must handle it to satisfy the type system. This is strictly better than `.unwrap()` because the panic message is preserved rather than replaced with a generic "called unwrap on Err" message.

**Specific error-handling contracts by call site category:**

| Category | Current behavior | Async behavior |
|----------|-----------------|----------------|
| Store CAS operations (`with_exclusive_tasks`, `update_task` in dispatch/complete) | `?` propagates | `sb_unwrap(...).await?` propagates identically |
| Store reads (`load()`) | `?` propagates | `sb_unwrap(...).await?` propagates identically |
| GitHub operations in `poll_and_claim` | `?` propagates | `sb_unwrap(...).await?` propagates identically |
| GitHub operations in `complete_task` | `if let Err` logs warning, continues | `spawn_blocking` result: log warning, continue |
| `child.kill()` / `child.wait()` in cleanup | `let _ =` ignores errors | `let _ = child.kill().await` ignores errors |
| Worktree cleanup | void, logs internally | `spawn_blocking`, ignore outer `JoinError` |
| Label updates | void, best-effort | `spawn_blocking`, ignore outer `JoinError` |

### Phase 5: Wrap blocking calls — complete call site inventory (Review Issue #3)

Every blocking call in `runtime.rs` must be wrapped in `spawn_blocking`. The complete inventory, organized by function:

**`run()` (lines 43-82):**
- L45: `reconcile_tasks(store)` — entire function becomes async, wraps store call internally
- L46: `reconcile_worktrees(store, config)` — becomes async, wraps store + worktree calls
- L52: `adopt_pending_tasks(store, config, &mut children)` — becomes async
- L56, L69: `collect_children(store, config, &mut children)` — becomes async (calls `complete_task` which has blocking operations)
- L63: `poll_and_claim(store, config, &mut children, slots)` — becomes async
- L74: `drain_all_children(store, config, &mut children)` — becomes async
- L78: `thread::sleep(...)` → `tokio::time::sleep(...).await`

**`reconcile_tasks()` (lines 86-106):**
- L87: `store.with_exclusive_tasks(...)` — wrap in `spawn_blocking`

**`reconcile_worktrees()` (lines 108-128):**
- L110: `store.load()` — wrap in `spawn_blocking`
- L116: `worktree::reconcile_worktrees(...)` — wrap in `spawn_blocking`

**`adopt_pending_tasks()` (lines 131-165):**
- L136: `store.load()` — wrap in `spawn_blocking`
- L148: `fetch_and_persist_raw_idea(store, &task)` — becomes async (wraps internal calls)
- L159: `dispatch_task(store, config, children, &task)` — becomes async

**`poll_and_claim()` (lines 168-242):**
- L174: `github::poll_issues(...)` — wrap in `spawn_blocking`
- L182: `store.load()` — wrap in `spawn_blocking`
- L199: `github::claim_issue(...)` — wrap in `spawn_blocking`
- L225: `store.with_exclusive_tasks(...)` — wrap in `spawn_blocking`
- L234: `dispatch_task(...)` — becomes async

**`dispatch_task()` (lines 251-357):**
- L264: `worktree::create_worktree(...)` — wrap in `spawn_blocking`
- L270: `fetch_and_persist_raw_idea(...)` — becomes async
- L275: `refine::refine_prompt(...)` — becomes async (Phase 6)
- L290-302: `github::post_idempotent_comment(...)` — wrap in `spawn_blocking`
- L305: `process::spawn_ralph_auto(...)` — now uses `tokio::process::Command`, is async natively
- L312: `store.with_exclusive_tasks(...)` — wrap in `spawn_blocking`
- L341-342: `child.kill()` / `child.wait()` — become async (tokio `Child`)
- L343: `worktree::remove_worktree(...)` — wrap in `spawn_blocking`

**`fetch_and_persist_raw_idea()` (lines 359-380):**
- L360: `github::fetch_issue_body(...)` — wrap in `spawn_blocking`
- L371: `store.update_task(...)` — wrap in `spawn_blocking`

**`collect_children()` (lines 396-430):**
- L404: `active.child.try_wait()` — synchronous on `tokio::process::Child`, no wrapping needed
- L428: `complete_task(...)` — becomes async (has many blocking operations inside)

**`drain_all_children()` (lines 435-463):**
- L443: `collect_children(...)` — becomes async
- L447: `thread::sleep(50ms)` → `tokio::time::sleep(50ms).await`
- L457-458: `child.kill()` / `child.wait()` — become async (tokio `Child`)
- L460: `complete_task(...)` — becomes async

**`complete_task()` (lines 470-557):**
- L479: `store.with_exclusive_tasks(...)` — wrap in `spawn_blocking`
- L509, L554: `cleanup_worktree(...)` — becomes async
- L529: `github::post_idempotent_comment(...)` — wrap in `spawn_blocking`
- L542: `handle_pr_flow(...)` — becomes async
- L546: `github::update_terminal_labels_best_effort(...)` — wrap in `spawn_blocking`

**`cleanup_worktree()` (lines 559-568):**
- L567: `worktree::remove_worktree(...)` — wrap in `spawn_blocking`

**`handle_pr_flow()` (lines 570-658):**
- L586: `github::has_diff(...)` — wrap in `spawn_blocking`
- L596: `github::post_idempotent_comment(...)` — wrap in `spawn_blocking`
- L611: `github::push_branch(...)` — wrap in `spawn_blocking`
- L620: `github::find_existing_pr(...)` — wrap in `spawn_blocking`
- L624: `store.update_task(...)` — wrap in `spawn_blocking`
- L642: `github::create_pr(...)` — wrap in `spawn_blocking`
- L646: `store.update_task(...)` — wrap in `spawn_blocking`

**Total: 35 blocking call sites** across 13 functions, all enumerated above.

### Phase 6: Convert `refine.rs`

- Change `refine_prompt()` to `pub async fn refine_prompt(...)`.
- Remove `tokio::task::block_in_place(|| Handle::current().block_on(...))` wrapper.
- Directly `.await` the `backend.execute(&prompt)` call.
- All callers (in `dispatch_task`) are now async, so `.await` works.

### Phase 7: Convert `process.rs` to use `tokio::process::Command`

- `spawn_ralph_auto()` returns `SpawnedChild` with `tokio::process::Child`.
- `build_ralph_auto_command()` returns `tokio::process::Command` instead of `std::process::Command`.
- `pre_exec(setsid)` works identically on `tokio::process::Command` (it delegates to `std::os::unix::process::CommandExt` via `Deref`).
- PID capture uses `.id().ok_or_else(...)` returning `Result<u32>` (see Phase 3).
- `terminate_process_group()` stays synchronous (pure libc `kill()` calls + `thread::sleep` for the SIGTERM→SIGKILL escalation window). It is always called from `abort_task()` in the sync CLI path (`daemon::execute_abort()`), **not** from the async runtime loop. The runtime loop does not call `terminate_process_group()` — it uses `child.kill()` directly for its own children. Therefore, no async conversion or `spawn_blocking` wrapping is needed for `terminate_process_group()`.

### Phase 8: Propagate `async` to `cli/daemon.rs`

- `execute()` becomes `pub async fn execute(args: DaemonArgs) -> Result<()>`.
- `execute_start()` becomes `async fn execute_start(args: DaemonStartArgs) -> Result<()>`.
- `execute_status()` and `execute_abort()` remain sync — they are short-lived operations with no event loop. They are called directly (not `.await`ed) from the `match` in `execute()`.
- Startup blocking calls in `execute_start()` (`Workspace::discover()`, `effective_daemon_config()`, `resolve_repo_from_gh()`, `resolve_git_root()`) run once before the event loop and are fine to leave blocking. They execute in sub-millisecond time (file reads) or single-digit seconds (one `gh` / `git` subprocess) and do not benefit from async conversion. No `spawn_blocking` wrapper needed.

### Phase 9: Update `ActiveChild` kill paths

In `dispatch_task()` lines 340-343, when a just-spawned child must be killed because the task was concurrently aborted:
```rust
let mut child = spawned.child;
let _ = child.kill().await;  // tokio::process::Child::kill() is async, sends SIGKILL + waits
```
Note: `tokio::process::Child::kill()` is `async fn kill(&mut self) -> io::Result<()>` — it sends SIGKILL and waits for the process to exit. This replaces the sync `child.kill()` + `child.wait()` two-step.

Similarly, in `drain_all_children()` force-kill path (line 455-458):
```rust
let _ = active.child.kill().await;  // replaces kill() + wait()
```

Both paths use `let _ =` to maintain current best-effort error suppression.

## Files & Modules

| File | Change Type | Description |
|------|------------|-------------|
| `src/daemon/runtime.rs` | **Major** | `run()` → `async fn run()`. All 13 internal helpers become async. `ActiveChild` holds `tokio::process::Child`. `thread::sleep` → `tokio::time::sleep`. 35 blocking call sites wrapped in `spawn_blocking`. Add `sb_unwrap` helper for JoinError handling. |
| `src/daemon/refine.rs` | **Moderate** | `refine_prompt()` → `async fn`. Remove `block_in_place`/`block_on`. Direct `.await` on backend. |
| `src/daemon/process.rs` | **Moderate** | `spawn_ralph_auto()` uses `tokio::process::Command`. `SpawnedChild` holds `tokio::process::Child`. `child.id()` → `.id().ok_or_else(...)` for `Option<u32>` handling. `terminate_process_group()` unchanged (only called from sync abort path). |
| `src/cli/daemon.rs` | **Minor** | `execute()` → `async fn`. `execute_start()` → `async fn`. `.await` on `runtime::run()`. Startup blocking calls left as-is. |
| `src/cli/mod.rs` | **Trivial** | Line 300: `daemon::execute(args)` → `daemon::execute(args).await`. |
| `src/daemon/mod.rs` | **None** | TaskStore unchanged. `abort_task()` and `update_abort_labels_best_effort()` remain sync (called from sync CLI paths). `terminate_process_group_if_present()` remains sync. |
| `src/daemon/github.rs` | **None** | Functions stay sync; called via `spawn_blocking` from `runtime.rs`. |
| `src/daemon/worktree.rs` | **None** | Functions stay sync; called via `spawn_blocking` from `runtime.rs`. |
| `src/validate/` | **None** | Conformance tests invoke binary via subprocess; unaffected. |

## Testing Strategy

1. **Primary behavioral gate: conformance tests.** Run `ralph validate --bin ./result/bin/ralph --filter daemon` (or the Nix equivalent via `nix develop -c cargo test` which includes these). These tests exercise the full daemon lifecycle via subprocess and are the authoritative check that external behavior is preserved. They must pass unchanged — no modifications to test code or expectations.

2. **Existing unit tests in `process.rs`:** The `spawn_command_uses_long_idea_flag` test calls `build_ralph_auto_command()` which now returns `tokio::process::Command`. `tokio::process::Command` exposes `get_args()` via `Deref<Target=std::process::Command>`, so the assertion `cmd.get_args().collect()` works identically. No `#[tokio::test]` needed — the function is sync (it just constructs a command). Verify this compiles.

3. **Existing unit tests in `refine.rs`:** `build_refinement_prompt` and `validate_output` tests are purely sync data transformations. No changes needed.

4. **Existing unit tests in `mod.rs`:** `resolve_task_index` tests and serialization round-trip tests are purely data-oriented. No changes needed.

5. **Conformance tests (`src/validate/`):** These are subprocess-based (`Command::new(ralph_bin)`) and completely decoupled from internal async/sync details. Run unchanged and verify full pass.

6. **Full `cargo test` and `cargo clippy`:** Run the complete suite. No new failures or warnings permitted.

7. **Manual integration test:** Run `ralph daemon start --single-iteration --repo <test-repo>` and verify:
   - Tasks are claimed, dispatched, collected
   - PR flow works end-to-end
   - Process exits cleanly after all children finish
   - No `thread 'main' panicked` or nested runtime errors in stderr

8. **Deadlock/runtime verification:** Confirm:
   - No nested runtime panics (the `block_in_place` removal eliminates the primary risk)
   - `spawn_blocking` calls don't deadlock under `--single-iteration` mode where operations run sequentially
   - No `Cannot start a runtime from within a runtime` errors

## Out of Scope

- Converting `github.rs` and `worktree.rs` functions to native async (Option B) — deferred to a follow-up.
- Converting `abort_task()`, `update_abort_labels_best_effort()`, or `terminate_process_group_if_present()` in `daemon/mod.rs` to async — these are called from the sync `execute_abort()` CLI path and don't need the change.
- Adding cancellation token / graceful shutdown signal handling (e.g., `tokio::signal::ctrl_c()`). This is a separate feature.
- Changing the `TaskStore` locking mechanism or making it async-native (e.g., `tokio::sync::RwLock` with async file I/O). The current `fs2` file-lock approach is correct and works fine via `spawn_blocking`.
- Converting `resolve_repo_from_gh()` or `resolve_git_root()` in `cli/daemon.rs` to async — these run once at startup before the event loop; blocking is fine.
- Performance optimization of the poll loop (e.g., event-driven GitHub webhook triggers instead of polling). Orthogonal concern.
- Adding `tokio::select!`-based child collection across multiple children — acceptable follow-up optimization but not required for v1. The `try_wait()` polling approach is semantically identical and simpler.
- Converting `terminate_process_group()` to async — it is only called from the sync `abort_task()` path. Its internal `thread::sleep(100ms)` loop is fine since it runs on a sync CLI code path, not the async runtime loop.