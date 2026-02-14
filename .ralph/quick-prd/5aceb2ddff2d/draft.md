I have everything I need. Here's the spec:

---

## Summary

Convert the daemon runtime loop (`src/daemon/runtime.rs`) and its supporting modules from synchronous, thread-blocking code to idiomatic async Rust using the existing tokio runtime. The primary motivations are: (1) eliminate the `block_in_place` / `Handle::current().block_on()` anti-pattern in `refine.rs`, (2) replace `thread::sleep` polling with `tokio::time::sleep` so the runtime thread is available for other work, and (3) use `tokio::process::Command` for child process management to enable `.await`-based child collection instead of busy-loop polling. External behavior, CLI interface, and TaskStore file-locking semantics remain unchanged.

## Acceptance Criteria

1. `ralph daemon start` produces identical external behavior: polls GitHub issues, claims, spawns `ralph auto` children, collects results, creates PRs, and updates labels exactly as before.
2. `refine_prompt()` no longer uses `block_in_place()` or `Handle::current().block_on()`. The backend `.execute()` call is `.await`ed directly.
3. The main poll loop in `runtime::run()` uses `tokio::time::sleep()` instead of `thread::sleep()`.
4. `drain_all_children()` uses `tokio::time::sleep()` or `tokio::time::interval()` instead of `thread::sleep(50ms)`.
5. Child processes are spawned via `tokio::process::Command`, and `ActiveChild` holds a `tokio::process::Child`.
6. All `TaskStore` file-locking calls (`load`, `save`, `with_exclusive_tasks`, `update_task`) are wrapped in `tokio::task::spawn_blocking()` at the async call sites in `runtime.rs`. The `TaskStore` implementation itself is unchanged.
7. All `std::process::Command` calls in `github.rs` and `worktree.rs` are invoked via `spawn_blocking()` from async callers, OR the functions themselves are converted to async using `tokio::process::Command`. Either approach is acceptable; choose whichever minimizes churn.
8. Process group isolation is preserved: `setsid` via `pre_exec`, SIGTERM/SIGKILL escalation via `terminate_process_group()`.
9. CAS-style atomic task state transitions are preserved.
10. `--single-iteration` mode still waits for all children to reach terminal state before exiting.
11. `cargo test` passes with no new failures. Conformance tests in `src/validate/` are unmodified.
12. `cargo clippy` produces no new warnings.

## Technical Approach

### Phase 1: Propagate `async` from `cli::run()` down to `daemon::runtime::run()`

The call chain is: `main()` (already async) → `cli::run()` (already async) → `daemon::execute()` (sync) → `execute_start()` (sync) → `runtime::run()` (sync).

- Make `daemon::execute()` async and `.await` on `execute_start()`.
- Make `execute_start()` async and `.await` on `runtime::run()`.
- Update the match arm in `cli/mod.rs` line 300 to `.await` the `daemon::execute()` call.

### Phase 2: Convert `runtime::run()` to async

- Change signature to `pub async fn run(...)`.
- Replace `thread::sleep(Duration::from_secs(poll_seconds))` on line 78 with `tokio::time::sleep(...).await`.
- Replace the `thread::sleep(50ms)` loop in `drain_all_children()` (line 447) with `tokio::time::sleep(50ms).await`.
- Change `ActiveChild.child` from `std::process::Child` to `tokio::process::Child`.
- In `collect_children()`, use `child.try_wait()` on `tokio::process::Child` (it has the same non-blocking `try_wait()` method, no `.await` needed).
- In `drain_all_children()`, since the function now lives in async context, optionally restructure to `tokio::select!` on child futures for more efficient waiting — but the polling approach with `tokio::time::sleep` is acceptable for v1.

### Phase 3: Wrap blocking TaskStore operations

Every call to `store.load()`, `store.with_exclusive_tasks(...)`, and `store.update_task(...)` in `runtime.rs` must be wrapped:

```rust
let tasks = tokio::task::spawn_blocking({
    let store = store.clone();
    move || store.load()
}).await.unwrap()?;
```

`TaskStore` already derives `Clone` (it only holds a `PathBuf`). No changes to `TaskStore` itself.

Functions that call TaskStore (`reconcile_tasks`, `reconcile_worktrees`, `adopt_pending_tasks`, `poll_and_claim`, `dispatch_task`, `complete_task`, `fetch_and_persist_raw_idea`) become async and use `spawn_blocking` for their store calls.

### Phase 4: Convert `process.rs` to use `tokio::process::Command`

- `spawn_ralph_auto()` returns `SpawnedChild` with `tokio::process::Child`.
- `build_ralph_auto_command()` returns `tokio::process::Command` instead of `std::process::Command`.
- `pre_exec(setsid)` works identically on `tokio::process::Command` (it delegates to the same `CommandExt` trait).
- `terminate_process_group()` stays synchronous (pure libc calls) but its `thread::sleep(100ms)` loop should use `tokio::time::sleep` if called from async context — wrap in `spawn_blocking` or make it async with `tokio::time::sleep`.

### Phase 5: Convert `refine.rs`

- Change `refine_prompt()` to `pub async fn refine_prompt(...)`.
- Remove `tokio::task::block_in_place(|| Handle::current().block_on(...))` wrapper.
- Directly `.await` the `backend.execute(&prompt)` call.
- All callers (in `dispatch_task`) are now async, so `.await` works.

### Phase 6: Handle `github.rs` and `worktree.rs` blocking calls

Two acceptable strategies (prefer whichever produces less diff):

**Option A (recommended): `spawn_blocking` at call sites.** Leave `github.rs` and `worktree.rs` functions synchronous. Wrap each call in `runtime.rs` with `spawn_blocking`. Example:
```rust
let issues = tokio::task::spawn_blocking({
    let owner = config.owner.clone();
    let repo = config.repo.clone();
    let labels = config.labels.clone();
    move || github::poll_issues(&owner, &repo, &labels)
}).await.unwrap()?;
```

**Option B: Convert to async.** Change all `std::process::Command` in `github.rs`/`worktree.rs` to `tokio::process::Command` and make the functions async. This is cleaner long-term but larger diff.

The spec recommends Option A for v1 to minimize blast radius, with Option B as a follow-up.

### Phase 7: Update `ActiveChild` kill path

In `dispatch_task()` lines 340-343, when a just-spawned child must be killed because the task was concurrently aborted:
```rust
let mut child = spawned.child;
child.kill().await?;  // tokio::process::Child::kill() is async
```
Similarly, in `drain_all_children()` force-kill path (line 457): `child.kill().await` and `child.wait().await`.

## Files & Modules

| File | Change Type | Description |
|------|------------|-------------|
| `src/daemon/runtime.rs` | **Major** | `run()` → `async fn run()`. All internal helpers become async. `ActiveChild` holds `tokio::process::Child`. `thread::sleep` → `tokio::time::sleep`. TaskStore calls wrapped in `spawn_blocking`. |
| `src/daemon/refine.rs` | **Moderate** | `refine_prompt()` → `async fn`. Remove `block_in_place`/`block_on`. Direct `.await` on backend. |
| `src/daemon/process.rs` | **Moderate** | `spawn_ralph_auto()` uses `tokio::process::Command`. `SpawnedChild` holds `tokio::process::Child`. `terminate_process_group()` either made async or wrapped in `spawn_blocking` at call sites. |
| `src/cli/daemon.rs` | **Minor** | `execute()` → `async fn`. `execute_start()` → `async fn`. `.await` on `runtime::run()`. |
| `src/cli/mod.rs` | **Trivial** | Line 300: `daemon::execute(args)` → `daemon::execute(args).await`. |
| `src/daemon/mod.rs` | **None** | TaskStore unchanged. `abort_task()` and `update_abort_labels_best_effort()` remain sync (called from sync CLI paths). |
| `src/daemon/github.rs` | **None (v1)** | Functions stay sync; called via `spawn_blocking` from `runtime.rs`. |
| `src/daemon/worktree.rs` | **None (v1)** | Functions stay sync; called via `spawn_blocking` from `runtime.rs`. |
| `src/validate/` | **None** | Conformance tests invoke binary via subprocess; unaffected. |

## Testing Strategy

1. **Existing unit tests**: Tests in `refine.rs` (pure functions like `build_refinement_prompt`, `validate_output`, `create_backend`) do not need `#[tokio::test]` since they test sync helpers. The `refine_prompt()` function itself is not unit-tested (it requires a live backend).

2. **Existing unit tests in `process.rs`**: `build_ralph_auto_command` test needs updating if the function now returns `tokio::process::Command` — verify the same assertions work (both types expose `get_args()`).

3. **Existing unit tests in `mod.rs`**: `resolve_task_index` tests and serialization tests are purely data-oriented; no changes needed.

4. **Conformance tests** (`src/validate/`): Run via subprocess (`Command::new(ralph_bin)`), completely decoupled from internal async/sync details. Verify they pass unchanged.

5. **Manual integration test**: Run `ralph daemon start --single-iteration --repo <test-repo>` and verify:
   - Tasks are claimed, dispatched, collected
   - PR flow works
   - Process exits cleanly after all children finish

6. **Regression check**: Run `cargo test` full suite. Run `cargo clippy`. No new warnings or failures.

7. **Deadlock/runtime verification**: Confirm no nested runtime panics occur (the `block_in_place` removal eliminates the primary risk). Verify `spawn_blocking` calls don't deadlock under `--single-iteration` mode where all operations run sequentially.

## Out of Scope

- Converting `github.rs` and `worktree.rs` functions to native async (Option B) — deferred to a follow-up.
- Converting `abort_task()` or `update_abort_labels_best_effort()` in `daemon/mod.rs` to async — these are called from the sync `execute_abort()` CLI path and don't need the change.
- Adding cancellation token / graceful shutdown signal handling (e.g., `tokio::signal::ctrl_c()`). This is a separate feature.
- Changing the `TaskStore` locking mechanism or making it async-native (e.g., `tokio::sync::RwLock` with async file I/O). The current `fs2` file-lock approach is correct and works fine via `spawn_blocking`.
- Converting `resolve_repo_from_gh()` or `resolve_git_root()` in `cli/daemon.rs` to async — these run once at startup before the event loop; blocking is fine.
- Performance optimization of the poll loop (e.g., event-driven GitHub webhook triggers instead of polling). Orthogonal concern.
- Adding `tokio::process::Child` `.wait().await` based child collection using `tokio::select!` across multiple children — acceptable follow-up optimization but not required for v1. The `try_wait()` polling approach is semantically identical and simpler.