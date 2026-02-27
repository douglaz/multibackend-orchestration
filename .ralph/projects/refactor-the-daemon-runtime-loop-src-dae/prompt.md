### Objective
Refactor the daemon runtime path to idiomatic async Rust on Tokio while preserving existing CLI behavior and daemon semantics.

### Scope
1. `src/daemon/runtime.rs`
2. `src/daemon/refine.rs`
3. `src/daemon/process.rs`
4. `src/cli/daemon.rs`
5. `src/cli/mod.rs`

### Non-Goals
1. No native async rewrite of `src/daemon/github.rs` or `src/daemon/worktree.rs`.
2. No TaskStore internal redesign or locking model changes.
3. No new daemon features (signals, cancellation framework, webhook mode, etc.).
4. No conformance test behavior changes.

### Required Behavior Invariants
1. `ralph daemon start` keeps current externally observable behavior: polling, claiming, spawning `ralph auto`, collecting exits, PR flow, and labels.
2. `--single-iteration` still drains all active children to terminal state before exiting.
3. CAS-style task state transitions remain intact.
4. Process group isolation is preserved for daemon children (`setsid` at spawn) and abort flow (`terminate_process_group()` SIGTERM/SIGKILL escalation).
5. Existing CLI interface and config behavior remain unchanged.

### Required Code Changes
1. Propagate async from CLI daemon entry:
   1. `cli::daemon::execute` becomes async.
   2. `execute_start` becomes async.
   3. `cli/mod.rs` awaits daemon execute in the daemon match arm.
   4. `execute_status` and `execute_abort` remain synchronous.
2. Convert runtime loop to async:
   1. `runtime::run` becomes `async fn`.
   2. Replace poll sleeps with `tokio::time::sleep(...).await`.
   3. Replace child-drain sleep loop with `tokio::time::sleep(...).await`.
3. Convert prompt refinement path:
   1. `refine::refine_prompt` becomes async.
   2. Remove `block_in_place` and nested `Handle::block_on`.
   3. Await backend execution directly.
4. Convert process spawning:
   1. Use `tokio::process::Command`.
   2. `ActiveChild` and `SpawnedChild` hold `tokio::process::Child`.
   3. Capture child PID immediately after spawn with `child.id().ok_or(...)`.
   4. Store captured `u32` PID/PGID in task state and never rely on re-reading `Child::id()` later.
5. Kill/reap behavior in async runtime:
   1. Use Tokio child kill/wait semantics (`kill().await` or equivalent).
   2. Cleanup paths remain best-effort with warn-and-continue behavior.
6. Blocking call policy:
   1. In async runtime code, wrap all TaskStore blocking operations in `tokio::task::spawn_blocking`.
   2. In async runtime code, wrap all calls into synchronous `github` and `worktree` helpers in `spawn_blocking`.
   3. Keep `github.rs` and `worktree.rs` function signatures synchronous.
7. `spawn_blocking` error handling:
   1. Introduce a helper that converts `JoinError` into `RalphError::Orchestration`.
   2. Do not use `.unwrap()` on `spawn_blocking(...).await` results.
   3. For required operations, propagate the mapped error.
   4. For best-effort cleanup/label/comment paths, log warning and continue.

### Explicit Acceptance Criteria
1. No use of `block_in_place` or `Handle::current().block_on()` in daemon refine/runtime path.
2. No `thread::sleep` in async daemon runtime path (`runtime.rs` loop and child drain).
3. Runtime child processes are managed via Tokio process APIs.
4. TaskStore operations from async runtime are executed via `spawn_blocking`.
5. Synchronous GitHub/worktree calls from async runtime are executed via `spawn_blocking`.
6. PID/PGID handling is correct with Tokio `Child::id() -> Option<u32>`.
7. `cargo test` passes.
8. `cargo clippy` has no new warnings.
9. Daemon validate suite passes unchanged.
10. No behavioral regressions in single-iteration draining and cleanup.

### Verification Commands
1. `nix develop -c cargo check`
2. `nix develop -c cargo test`
3. `nix develop -c cargo clippy --all-targets --all-features -- -D warnings`
4. `nix build -L`
5. `./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon`

### Implementation Constraints
1. Prefer minimal diff outside scoped files.
2. Preserve current logging style and error taxonomy (`RalphError`).
3. Preserve best-effort semantics in cleanup/label update paths.
4. Do not modify `src/validate/` expectations unless strictly required for compatibility.

### Deliverables
1. Code changes in scoped files implementing async conversion and blocking isolation.
2. Short change summary with rationale by file.
3. Verification output summary for each required command, including pass/fail status.