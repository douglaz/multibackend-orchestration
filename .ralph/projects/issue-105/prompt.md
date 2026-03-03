### Objective
Refactor the daemon runtime loop in `src/daemon/runtime.rs` so independent I/O-heavy work runs concurrently while preserving existing behavior, safety, and deterministic single-iteration semantics.

### Scope
- Modify only daemon runtime behavior and related tests.
- Keep `children` as `&mut HashMap<u32, ChildHandle>` (no `Arc<Mutex<...>>`).
- Preserve current phase order in the main loop: `kill_aborted_children` -> `collect_children` -> `auto_rebase_phase` -> `poll_and_claim` -> `collect_children`.
- Decouple PRD from that loop only in continuous mode.

### Required Behavior
- PRD execution model:
- If `config.prd_enabled && config.single_iteration`, run exactly one inline PRD tick before dispatch-related work in that iteration; do not start a PRD background task.
- If `config.prd_enabled && !config.single_iteration`, start one background PRD task with `tokio::spawn`.
- Continuous-mode PRD task must run one tick immediately on start, then sleep `poll_seconds` between ticks.
- Use `CancellationToken` to stop new PRD ticks on shutdown.
- Add `prd_shutdown_timeout_secs` to runtime config (default `60`, minimum `1`).
- On shutdown: cancel token, await PRD task with timeout, call `handle.abort()` on timeout, log warning, and continue shutdown.
- `kill_aborted_children`:
- Query labels concurrently via `JoinSet`, capped at `max(1, config.max_concurrent)`.
- Keep kill/termination application sequential against `children`.
- On query failure, log and skip that issue for this cycle (best effort).
- `auto_rebase_phase`:
- Keep candidate discovery and merge-metadata queries sequential.
- Preserve existing break-on-first-merge-info-failure behavior.
- Execute rebase operations concurrently up to `config.max_rebases_per_cycle` via `JoinSet`.
- Apply outcomes to `children` sequentially.
- `poll_and_claim`:
- Keep claim/label swap and idea extraction sequential per issue.
- After claim stage, dispatch claimed issues concurrently up to available `slots` (current slot logic remains authoritative).
- Change `dispatch_task` to return `Result<ChildHandle>` and stop mutating `children` directly.
- Caller inserts successful `ChildHandle` into `children`.
- Caller must preserve rollback behavior on dispatch failure: swap `ralph:in-progress` -> `ralph:failed` per failed issue.
- `collect_children`:
- Keep child status scan sequential.
- For each finished child: preserve existing order `watcher_cancel -> watcher_join -> draft_pr_cancel -> draft_pr_join`; keep `print_log_tail` for failed children.
- Run `complete_task` concurrently across finished children via `JoinSet`.
- Log task panic/error and continue processing remaining children.
- Git lock safety:
- Create one shared `Arc<tokio::sync::Semaphore>` with 1 permit in `run()`.
- Acquire permit only for repo-root git operations that touch shared state (`fetch` in repo root, worktree add/remove/prune, project-branch sync, repo readiness).
- Do not hold permit across long worktree-local operations (`git rebase`, `git push` inside a worktree).
- Pass semaphore into functions that need repo-root git operations (including dispatch/rebase paths).

### Non-Goals
- No conversion of `children` to shared synchronized state.
- No global GitHub API rate limiter redesign.
- No retry/backoff redesign for read-only GitHub queries.
- No CLI surface changes.

### Implementation Notes
- Replace line-number-driven implementation references with function-name-driven edits.
- Keep log style consistent with existing daemon warnings.
- Prefer snapshot -> execute concurrent I/O -> apply results pattern for all `children` interactions.

### Acceptance Criteria
- Continuous mode no longer blocks dispatch/collection on PRD phase execution.
- Single-iteration mode runs exactly one inline PRD tick and no background PRD task.
- PRD shutdown follows: cancel -> bounded await -> explicit abort on timeout -> warning log.
- `kill_aborted_children` label fetches are concurrent with configured cap.
- `auto_rebase_phase` rebases execute concurrently (capped), while metadata queries remain sequential with early-stop semantics preserved.
- `poll_and_claim` dispatches multiple claimed issues concurrently up to `slots`.
- Dispatch failure rollback label swap remains intact per issue.
- `collect_children` runs completion calls concurrently while preserving per-child watcher teardown order and failure log tail behavior.
- Repo-root git operations are serialized via a shared semaphore, preventing git index lock contention.
- Main loop ordering is preserved exactly as listed above.

### Tests
- Update/add tests so behavior is verifiable, not inferred:
- Existing unit and integration tests related to daemon runtime continue to pass.
- Add/adjust integration coverage for:
- concurrent dispatch of 2+ issues in one cycle,
- dispatch failure rollback for one of multiple claimed issues,
- single-iteration PRD determinism (inline, no background task),
- concurrent rebase+dispatch without git lock contention errors.
- Add/adjust validate conformance coverage under `src/validate/` for the new daemon concurrency behavior and register it in `src/validate/mod.rs`.
- Run:
- `nix develop -c cargo test`
- `nix build -L`
- `./result/bin/ralph validate --bin ./result/bin/ralph`

### Deliverables
- Code changes in daemon runtime and any required config plumbing.
- Test changes (unit/integration/validate) covering the acceptance criteria.
- Short implementation summary in PR description mapping each acceptance criterion to code paths/tests.