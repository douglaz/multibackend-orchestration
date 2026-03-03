## Summary

Refactor the daemon main loop in `src/daemon/runtime.rs` to run independent phases concurrently. Currently, the loop at line 830 executes `kill_aborted_children()` → `collect_children()` → `auto_rebase_phase()` → `run_prd_phase()` → `poll_and_claim()` → `collect_children()` sequentially. Each phase blocks the entire loop, meaning a slow rebase or PRD advance delays task dispatch and abort detection. This change parallelizes independent work within each phase and moves PRD processing to a background task, reducing per-cycle latency proportional to the number of active children.

PRD processing moves to a background `tokio::spawn` task with explicit single-iteration handling: in `single_iteration` mode, one PRD tick executes synchronously inline before the main loop body (preserving determinism for `tests_interactive_prd` conformance tests), while in continuous mode the background task runs independently. Shutdown uses a `CancellationToken` to signal the background loop, combined with a bounded `JoinHandle` await with timeout since `poll_and_advance_prd` runs on `spawn_blocking` and is not cancellation-cooperative.

Within each phase, concurrent I/O uses `JoinSet` with explicit concurrency caps to bound GitHub API fan-out. The `children` HashMap remains a plain `&mut HashMap<u32, ChildHandle>` using a snapshot-execute-apply pattern to avoid `Arc<Mutex<>>`. Git worktree operations that target the shared repo root are serialized through a `tokio::sync::Semaphore` to prevent git index lock contention.

## Acceptance Criteria

- [ ] PRD phase runs in a background `tokio::spawn` task on its own polling interval, no longer blocking the main loop in continuous mode
- [ ] In `single_iteration` mode, one PRD tick executes synchronously inline before dispatch (preserving deterministic ordering for `tests_interactive_prd` conformance tests), then the background task is not started
- [ ] PRD background task shutdown uses `CancellationToken` to stop the polling loop, followed by `JoinHandle` await with a configurable timeout (default 60s); if the timeout expires, the handle is aborted and a warning is logged
- [ ] `kill_aborted_children()` issues all `fetch_issue_labels()` calls concurrently via `JoinSet`, capped at `max_concurrent` in-flight requests (runtime.rs:1573-1593)
- [ ] `auto_rebase_phase()` executes up to `max_rebases_per_cycle` rebase operations concurrently instead of sequentially (runtime.rs:1851-2057), with metadata queries (PR lookup, merge info) remaining sequential to preserve break-on-first-failure safety semantics
- [ ] `poll_and_claim()` dispatches up to `slots` tasks concurrently instead of one-at-a-time (runtime.rs:1104 loop), with claim-failure rollback (`ralph:in-progress` → `ralph:failed`) preserved per-issue in the caller after JoinSet resolution
- [ ] `collect_children()` runs `complete_task()` calls concurrently for all finished children (runtime.rs:1537-1558), preserving `print_log_tail()` for failed children and the existing watcher-cancel → watcher-join → draft-pr-cancel → draft-pr-join ordering per child before spawning completion work
- [ ] All concurrent git operations against the shared repo root (`fetch`, `worktree add`, `worktree remove`, `worktree prune`, `sync_project_branch`) are serialized through a `tokio::sync::Semaphore(1)` passed to each phase, preventing git index lock contention
- [ ] No regressions: existing unit tests in `runtime.rs:2611+` and integration tests in `tests_daemon.rs`, `tests_daemon_rebase.rs`, `tests_interactive_prd.rs` continue to pass
- [ ] Main loop phase ordering preserved: abort detection → collection → rebase → dispatch → collection (PRD decoupled in continuous mode, inline in single-iteration mode)

## Technical Approach

### 1. PRD Background Task

Move PRD processing out of the main loop into an independent `tokio::spawn` task started before the loop (after line 827). The task runs `run_prd_phase()` on its own interval (reuse `config.poll_seconds`). Use a `CancellationToken` (already in the dependency tree via `tokio_util`) to signal the polling loop to stop.

**Single-iteration mode (review issue #1):** In `single_iteration` mode (runtime.rs:876), PRD must run deterministically within the single cycle. Do _not_ spawn the background task. Instead, keep the existing inline `run_prd_phase()` call for single-iteration mode only:

```rust
// Before the loop:
let prd_cancel = CancellationToken::new();
let prd_handle = if config.prd_enabled && !config.single_iteration {
    let cfg = config.clone();
    let cancel = prd_cancel.clone();
    Some(tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(cfg.poll_seconds)) => {}
            }
            if let Err(err) = run_prd_phase(&cfg).await {
                eprintln!("warning: background PRD phase failed: {err}");
            }
        }
    }))
} else {
    None
};

// Inside the loop, replace the existing PRD block:
if config.prd_enabled && config.single_iteration {
    if let Err(err) = run_prd_phase(config).await {
        eprintln!("warning: interactive PRD phase failed: {err}");
    }
}
```

**Shutdown (review issue #2):** `run_prd_phase()` delegates to `spawn_blocking(interactive_prd::poll_and_advance_prd)`, which uses `std::thread::scope` internally and is not cancellation-cooperative. The `CancellationToken` stops the _polling loop_ from starting new ticks, but cannot interrupt a tick already in progress. On loop exit:

```rust
// After the loop exits:
prd_cancel.cancel();
if let Some(handle) = prd_handle {
    match tokio::time::timeout(Duration::from_secs(60), handle).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => eprintln!("warning: PRD background task panicked: {err}"),
        Err(_) => {
            eprintln!("warning: PRD background task did not finish within 60s, aborting");
            // handle is already dropped, abort happens implicitly
        }
    }
}
```

The 60-second timeout accommodates the longest expected PRD tick (backend calls with `prd_backend_timeout_secs`). If it exceeds the timeout, we log and drop the handle (the spawned task continues until the process exits, which is acceptable since the daemon is shutting down).

**Ordering safety:** The comment at line 846 says PRD runs "before claim/dispatch to prevent dual workflow ownership." This is enforced by the `has_in_progress_prd_label()` check at runtime.rs:1033 in `poll_and_claim()`, which skips issues carrying PRD labels regardless of phase ordering. No additional synchronization needed for continuous mode. For single-iteration mode, the inline execution preserves the original ordering.

### 2. Parallel Label Queries in `kill_aborted_children()` (runtime.rs:1566-1619)

Replace the sequential `for issue_number in issue_numbers` loop (line 1573) with a `JoinSet`. Cap concurrent requests at `config.max_concurrent` to avoid unbounded GitHub API fan-out (review issue #3).

```rust
let mut set = JoinSet::new();
let semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_concurrent as usize));
for issue_number in issue_numbers {
    let owner = config.owner.clone();
    let repo = config.repo.clone();
    let permit = semaphore.clone();
    set.spawn(async move {
        let _permit = permit.acquire().await.unwrap();
        let labels = spawn_blocking_op(move || {
            github::fetch_issue_labels(&owner, &repo, issue_number)
        }).await;
        (issue_number, labels)
    });
}
let mut to_kill = Vec::new();
while let Some(result) = set.join_next().await {
    match result {
        Ok((issue_number, Ok(labels))) => {
            if !labels.iter().any(|l| l == "ralph:in-progress") {
                to_kill.push(issue_number);
            }
        }
        Ok((issue_number, Err(err))) => {
            eprintln!("abort-check: failed to query labels for issue #{issue_number}: {err}");
        }
        Err(err) => {
            eprintln!("abort-check: label query task panicked: {err}");
        }
    }
}
// Kill phase remains sequential (mutates children, calls terminate_process_group_blocking)
```

**Rate limit note (review issue #3):** `fetch_issue_labels()` (github.rs:1339) does _not_ have retry/backoff — it is a single `gh` CLI invocation with no retry loop. This is acceptable for the abort-check path since it is best-effort (a transient failure simply skips the abort check for that child this cycle, and the next cycle retries). The `max_concurrent` cap limits fan-out to the same degree as the existing dispatch cap, preventing burst rate-limit hits.

### 3. Parallel Rebase in `auto_rebase_phase()` (runtime.rs:1839-2057)

Split the function into three phases:

**Phase A — Gather candidates (sequential, reads `children`):** Iterate children, check cooldowns, resolve PR URLs via API (sequential, one at a time), query merge info (sequential). Collect a `Vec<RebaseCandidate>` with all data needed to execute rebases. Apply PR URL back-fills to `children` after each lookup.

**Metadata queries stay sequential (review issue #4):** The current code breaks the loop on the first `query_pr_merge_info()` failure (line 1934) as a rate-limit safety measure. Parallelizing these queries would launch all requests before any failure is observed, defeating early-stop semantics. Since metadata queries are fast (single `gh pr view` per child) and the number of candidates is bounded by `max_rebases_per_cycle`, keeping them sequential has negligible latency impact. Only the actual rebase _execution_ (Phase B) benefits meaningfully from parallelization.

**Phase B — Execute rebases (parallel, serialized git ops):** Spawn up to `max_rebases_per_cycle` rebase operations concurrently via `JoinSet`. Each operation acquires the `repo_git_semaphore` (see Section 6) before running `create_worktree_on_branch`, `execute_rebase`, and `remove_rebase_worktree`. The semaphore serializes git commands that touch the shared repo root while allowing rebase computation within isolated worktrees to overlap with other non-git work.

```rust
struct RebaseCandidate {
    issue_number: u32,
    task_id: String,
    branch: String,
    pr_number: u32,
    rebase_target: String,
    head_sha: String,
    last_failure_sha: Option<String>,
}

enum RebaseOutcome {
    Success { issue_number: u32 },
    Failure { issue_number: u32, head_sha: String, err_msg: String, is_lease: bool },
    WorktreeError { issue_number: u32 },
}
```

**Phase C — Apply results (sequential, mutates `children`):** Update `last_rebase_at`, `last_rebase_failure_sha`, and post failure comments based on collected results.

### 4. Parallel Dispatch in `poll_and_claim()` (runtime.rs:968-1126)

The label-swap claim loop (lines 1048-1066) stays sequential since it needs atomic per-issue claim semantics and raw_idea resolution (PRD spec extraction at lines 1069-1103).

After claiming, collect all claimed issues into a `Vec<ClaimedIssue>`:

```rust
struct ClaimedIssue {
    issue_number: u32,
    raw_idea: String,
}
```

Then dispatch all tasks concurrently via `JoinSet`:

```rust
let mut dispatch_set = JoinSet::new();
for claimed in claimed_issues {
    let cfg = config.clone();
    let repo_sem = repo_git_semaphore.clone();
    dispatch_set.spawn(async move {
        let result = dispatch_task(&cfg, claimed.issue_number, &claimed.raw_idea, repo_sem).await;
        (claimed.issue_number, result)
    });
}

while let Some(join_result) = dispatch_set.join_next().await {
    match join_result {
        Ok((issue_number, Ok(child_handle))) => {
            children.insert(issue_number, child_handle);
        }
        Ok((issue_number, Err(err))) => {
            // Preserve existing error-path behavior (review issue #6):
            // roll back claimed issue from in-progress to failed
            eprintln!("warning: failed to dispatch issue #{issue_number}: {err}");
            let owner = config.owner.clone();
            let repo = config.repo.clone();
            let _ = spawn_blocking_op(move || {
                github::swap_lifecycle_label(&owner, &repo, issue_number, "ralph:in-progress", "ralph:failed")
            }).await;
        }
        Err(err) => {
            eprintln!("warning: dispatch task panicked: {err}");
        }
    }
}
```

**Signature change:** `dispatch_task()` (runtime.rs:1196) changes from:
```rust
async fn dispatch_task(config: &DaemonRuntimeConfig, children: &mut HashMap<u32, ChildHandle>,
    issue_number: u32, raw_idea: &str) -> Result<()>
```
to:
```rust
async fn dispatch_task(config: &DaemonRuntimeConfig, issue_number: u32, raw_idea: &str,
    repo_git_semaphore: Arc<Semaphore>) -> Result<ChildHandle>
```

The function returns `Result<ChildHandle>` instead of inserting into the map directly. It acquires `repo_git_semaphore` for git operations (`ensure_repo_ready`, `create_worktree`, `clean_worktree`, `sync_project_branch`) and releases it before spawning the child process. Non-git operations (prompt refinement, GitHub API calls, process spawning) run without the semaphore.

**Error-path completeness (review issue #6):** The caller (JoinSet resolution loop above) explicitly handles `Err` by swapping `ralph:in-progress` → `ralph:failed`, preserving the existing behavior at lines 1106-1119. This is an acceptance criterion.

### 5. Parallel Completion in `collect_children()` (runtime.rs:1494-1558)

The `try_wait()` scan (lines 1498-1531) remains sequential since it's cheap CPU-only work reading process status. For the completion phase (lines 1537-1557):

**Watcher teardown and log diagnostics (review issue #7):** Preserve the exact existing behavior per child _before_ spawning into the JoinSet. The watcher-cancel → watcher-join → draft-pr-cancel → draft-pr-join ordering and `print_log_tail()` for failed children execute sequentially per child during the removal-from-map phase:

```rust
let mut completion_tasks = Vec::new();
for (issue_number, terminal_label) in finished {
    let task_id = format_task_id(&config.owner, &config.repo, issue_number);
    let Some(mut handle) = children.remove(&issue_number) else { continue };

    // Sequential per-child: cancel and join watchers (preserves existing ordering)
    handle.watcher_cancel.cancel();
    if let Some(jh) = handle.watcher_handle.take() {
        if let Err(err) = jh.await {
            eprintln!("warning: artifact watcher join failed for {task_id}: {err}");
        }
    }
    handle.draft_pr_cancel.cancel();
    if let Some(jh) = handle.draft_pr_handle.take() {
        if let Err(err) = jh.await {
            eprintln!("warning: draft PR watcher join failed for {task_id}: {err}");
        }
    }

    // Sequential per-child: print log tail for failures (preserves diagnostics)
    if terminal_label == "ralph:failed" {
        print_log_tail(&task_id, &handle.log_file);
    }

    completion_tasks.push((issue_number, task_id, terminal_label));
}

// Parallel: complete_task() calls (GitHub label swap + PR flow)
let mut completion_set = JoinSet::new();
for (issue_number, task_id, terminal_label) in completion_tasks {
    let cfg = config.clone();
    let label = terminal_label.to_owned();
    let tid = task_id.clone();
    completion_set.spawn(async move {
        complete_task(&cfg, issue_number, &tid, &label).await;
    });
}
while let Some(result) = completion_set.join_next().await {
    if let Err(err) = result {
        eprintln!("warning: complete_task panicked: {err}");
    }
}
```

This ensures `children.remove()` and watcher teardown happen sequentially (no shared state), then only the `complete_task()` I/O work (label swap, PR flow, comment posting) runs concurrently.

### 6. Git Worktree Safety (review issue #5)

Concurrent dispatch and rebase both invoke git commands against the same repo root: `git fetch`, `git worktree add`, `git worktree remove`, `git worktree prune`, and `sync_project_branch` (which runs `git fetch`, `git branch`, `git reset`). These commands acquire git's internal `.git/index.lock`, and concurrent invocations will fail with "Unable to create lock file".

**Coordination mechanism:** Introduce a `tokio::sync::Semaphore` with 1 permit, allocated in `run()` and passed by `Arc` to each phase:

```rust
let repo_git_semaphore = Arc::new(tokio::sync::Semaphore::new(1));
```

Each function that invokes repo-root-scoped git commands acquires a permit before the git operation and releases it after:

- `dispatch_task()`: acquires for `ensure_repo_ready` + `create_worktree` + `clean_worktree` + `sync_project_branch`, releases before process spawn
- Rebase Phase B: each rebase task acquires for `create_worktree_on_branch` + `fetch` (within `execute_rebase`) + `remove_rebase_worktree`, releases after worktree cleanup

**Why Semaphore(1) and not a Mutex:** The semaphore is acquired across `.await` points (the git operations use `spawn_blocking`), and `tokio::sync::Semaphore` is designed for this. A `std::sync::Mutex` cannot be held across `.await`.

**Worktree-scoped operations are safe without the semaphore:** Operations that run _within_ an already-created worktree (e.g., `git checkout`, `git rebase`, `git push` with `current_dir` set to the worktree path) do not contend on the repo-root index lock because worktrees have their own index files. Only operations that modify the shared `.git/worktrees/` directory or the repo-root index require serialization.

**Refinement:** Git worktree-scoped operations within `execute_rebase` (the actual `git rebase`, `git push`) can release the semaphore early, holding it only for `create_worktree_on_branch` and `remove_rebase_worktree`. This allows multiple rebases' compute phases to overlap even though their worktree setup/teardown is serialized.

### 7. Shared State Strategy

No `Arc<Mutex<>>` needed for `children`. The pattern throughout is:

1. **Snapshot** data from `children` (sequential read)
2. **Execute** concurrent I/O operations (parallel, no children access)
3. **Apply** results back to `children` (sequential write)

This read-execute-write pattern keeps `children` as a plain `HashMap<u32, ChildHandle>` with `&mut` access, avoiding synchronization overhead entirely.

## Files & Modules

| File | Changes |
|---|---|
| `src/daemon/runtime.rs` | Main changes: (1) Refactor `run()` to create `repo_git_semaphore: Arc<Semaphore>` and spawn PRD background task with `CancellationToken` + shutdown timeout in continuous mode, keep inline PRD in single-iteration mode. (2) Refactor `kill_aborted_children()` to use `JoinSet` with `max_concurrent` semaphore for concurrent label queries. (3) Refactor `auto_rebase_phase()` into three-phase gather/execute/apply with `JoinSet` for parallel rebase execution, sequential metadata queries. (4) Refactor `poll_and_claim()` to collect claimed issues then dispatch concurrently via `JoinSet`, with per-issue error rollback. (5) Change `dispatch_task()` signature to return `Result<ChildHandle>`, accept `Arc<Semaphore>` for git ops. (6) Refactor `collect_children()` to run watcher teardown and `print_log_tail()` sequentially per-child, then `complete_task()` concurrently via `JoinSet`. |
| `src/daemon/mod.rs` | No structural changes needed; `ChildHandle` fields remain the same |

No new files or modules required. All changes are internal to `src/daemon/runtime.rs`.

## Testing Strategy

**Unit tests (in `src/daemon/runtime.rs:2611+`):**
- No existing `dispatch_task` unit tests exist (review issue #8 correction). The existing tests cover helper functions (`build_pr_title`, `build_pr_body`, `extract_original_title`, `should_retry_complete_task`, etc.) and do not call `dispatch_task` directly. These tests are unaffected by the signature change.
- Add unit test: verify `dispatch_task` returns `Result<ChildHandle>` with expected fields (mock git/process operations via test harness)
- Add unit test: verify `RebaseCandidate` / `RebaseOutcome` struct round-trip (if introduced as named types)

**Integration tests (in `src/validate/tests_daemon.rs`):**
- Existing `runtime_single_iteration_mode` test (line 1610) exercises the full single-iteration loop with mock `gh` and `ralph` scripts; must continue to pass with inline PRD and deterministic drain
- Existing daemon integration tests exercise claim → dispatch → collect → complete flow; must pass with concurrent dispatch
- Add integration test: concurrent dispatch of 2+ issues in a single cycle — verify both children are tracked and complete independently
- Add integration test: dispatch failure rollback — claim 2 issues, make one `dispatch_task` fail (e.g., via broken worktree setup mock), verify the failed issue gets `ralph:failed` label while the successful one gets `ralph:in-progress`
- Add integration test: verify PRD background task runs independently of main loop cadence in continuous mode (assert PRD issues advance even during long dispatch phases)

**PRD single-iteration determinism (review issue #8):**
- Existing `tests_interactive_prd.rs` conformance tests (39+ tests) run PRD logic via the harness, not through the daemon loop directly. They test `poll_and_advance_prd` in isolation and are unaffected by the background-task change.
- The `runtime_single_iteration_mode` integration test in `tests_daemon.rs` validates single-iteration behavior. Since single-iteration mode keeps inline PRD, no new PRD-specific single-iteration test is needed — the existing test provides coverage.

**Git lock contention (review issue #8):**
- Add integration test: concurrent dispatch + rebase in same cycle — two issues dispatching while a third is rebasing. Verify no git lock errors in stderr output and all operations complete.
- The `repo_git_semaphore` prevents contention at the application level; the test validates the semaphore is correctly plumbed.

**Concurrency correctness:**
- Verify via existing `single_iteration` mode that all phases complete deterministically before the loop exits (runtime.rs:876-880)
- The `drain_all_children()` path (line 879) remains sequential and unchanged
- The existing `collect_children` within `drain_all_children` will use the new concurrent completion logic, which is safe since `drain_all_children` already handles force-kill with its own sequential path

**Regression safety:**
- Run full `cargo test` suite — existing tests in `tests_daemon.rs`, `tests_daemon_rebase.rs`, `tests_interactive_prd.rs` cover the affected phases
- Run `cargo clippy` to catch any borrow-checker issues from the refactored signatures

## Out of Scope

- **`Arc<Mutex<HashMap>>` for children:** Not needed; the snapshot-execute-apply pattern avoids sharing mutable state across tasks
- **Concurrent GitHub label mutations:** Label swaps (`swap_lifecycle_label`) remain sequential per-issue to avoid races on the same issue's labels
- **Retry/backoff for `fetch_issue_labels` and `query_pr_merge_info`:** These read-only GitHub API calls do not have retry logic today. Adding retry/backoff is a separate concern from parallelization. The abort-check path is best-effort (transient failures skip that child for one cycle). The rebase metadata path uses break-on-first-failure, which is preserved by keeping queries sequential. A follow-up issue can add retry wrappers to these functions independently.
- **Parallel rebase metadata queries:** Kept sequential to preserve break-on-first-failure rate-limit safety (review issue #4). The latency cost is minimal since candidates are bounded by `max_rebases_per_cycle`.
- **Configurable concurrency limits per phase:** Rebase uses existing `max_rebases_per_cycle`; dispatch uses existing `max_concurrent` slots; abort-check reuses `max_concurrent` as its fan-out cap. No new configuration knobs.
- **Persisting PRD background task state:** PRD already persists per-issue state to JSON files (`InteractivePrdState`); no additional persistence needed
- **Refactoring `DaemonRuntimeConfig` to `Arc`:** Config is `Clone` (line 25) and cheap to clone for spawned tasks; no `Arc` wrapper needed
- **Moving `children` to a separate module or newtype:** The HashMap stays inline in `run()` as-is
- **Global centralized rate limiter:** Per-call retry with exponential backoff exists for label mutations (github.rs `add_label_with_retry`/`remove_label_with_retry`). Read APIs (`fetch_issue_labels`, `query_pr_merge_info`) lack retry but are bounded by per-phase concurrency caps. A centralized token-bucket rate limiter across all GitHub API calls is a larger effort orthogonal to this parallelization work.