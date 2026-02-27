## Summary

Refactor `poll_and_advance_prd()` in `src/daemon/interactive_prd.rs` to process PRD issues concurrently instead of sequentially. Today, the function iterates over polled issues in a `for` loop inside a single `spawn_blocking` task; a long-running backend call (question generation, draft generation, or revision — each up to 60 minutes) blocks every other PRD issue from advancing. Converting to concurrent per-issue processing lets quick state transitions (e.g., detecting an answer comment, reading an approval label) proceed without waiting for unrelated backend work to complete.

The function remains synchronous and continues to run inside `spawn_blocking`. Per-issue work is parallelized using a scoped thread pool (via `std::thread::scope` or `rayon`) bounded by `daemon_max_concurrent`, avoiding any async conversion of the heavily blocking call graph. The repo clone is refreshed once before spawning issue tasks, and backend subprocesses receive an explicit working directory instead of relying on process-global cwd.

## Acceptance Criteria

1. `poll_and_advance_prd()` processes multiple PRD issues concurrently rather than sequentially.
2. A long-running backend call on issue N does not prevent issue M from advancing during the same poll tick.
3. Maximum concurrent issue processing is bounded by the existing `daemon_max_concurrent` setting (default 5) from `.ralph/ralph.toml`. A config value of `0` is treated as `1` (serial) to prevent deadlock.
4. All existing state transitions (`Pending` → `AwaitingAnswers` → `AwaitingFeedback` → `Done`/`Failed`) work correctly.
5. An error (including panics) in one issue's processing does not affect other concurrent issues.
6. At most one transition per issue per tick: issue deduplication across the `ralph:prd` and `ralph:prd-active` poll passes is preserved. No issue is processed more than once per invocation of `poll_and_advance_prd`.
7. The PRD phase continues to block the main daemon poll loop — `run_prd_phase` waits for all spawned issue tasks to complete before returning. This preserves the existing invariant that PRD processing finishes before claim/dispatch, preventing dual-ownership of issues.
8. No regression in existing integration tests (`tests/daemon_interactive_prd.rs`) or conformance tests (`src/validate/tests_interactive_prd.rs`).

## Technical Approach

### Problem Analysis

`poll_and_advance_prd` (line 405) is a **synchronous** function called via `tokio::task::spawn_blocking` from the async daemon runtime (`runtime.rs:617`). Internally, per-issue transitions that invoke backends create their own throwaway `tokio::runtime::Builder::new_current_thread()` runtimes for async backend calls (`run_backend_sync` at line 1433, `run_review_with_retry_sync` at line 1352). The GitHub API calls (`github::poll_issues`) and all state file I/O are also synchronous blocking calls.

A `CwdGuard` RAII helper (line 282) mutates the **process-global** working directory before backend calls so that `CliBackend::execute_streaming` (which spawns `Command` without `.current_dir()`) inherits the correct cwd. Under concurrent execution, this is a data race.

### Approach: Keep synchronous, use thread-based concurrency with explicit cwd

Converting the entire call graph to async would require making all `github::*` shell calls, state file I/O, and `Command` spawns async — a large, risky refactor that risks stalling the tokio runtime with blocking code that's easy to miss. Instead, we keep `poll_and_advance_prd` synchronous and parallelize at the thread level.

**Step 1 — Add `max_concurrent: u32` to `PrdPollConfig`.** Plumb the `daemon_max_concurrent` value from `DaemonRuntimeConfig` into `PrdPollConfig` so the concurrency limit is available at the PRD layer. Populate it in `run_prd_phase` when constructing `PrdPollConfig` (line 603).

**Step 2 — Refresh the repo clone once per tick, before spawning issue tasks.** Currently, `refresh_repo_clone()` is called inside each `generate_*_with_timeout` function (lines 1019, 1266, 1393). Under concurrency, multiple tasks running `git fetch origin && git reset --hard origin/HEAD` on the same clone directory would race. Move the single `refresh_repo_clone()` call to the top of `poll_and_advance_prd`, after the poll queries complete and before issue processing begins. Remove the per-generation-function `refresh_repo_clone()` calls. This is safe because all backend calls in a single tick should see the same repo snapshot.

**Step 3 — Eliminate `CwdGuard`; pass explicit cwd to backend subprocess spawning.** The `CwdGuard` (line 282) calls `std::env::set_current_dir` which mutates process-global state and is unsafe under concurrency. The fix requires changes at two levels:

  - **Backend layer:** Add an optional `cwd: Option<PathBuf>` field to `CliBackend`. When set, `execute_streaming` calls `.current_dir(cwd)` on the `Command` before spawning. This field is set at backend creation time and does not change the `Backend` trait signature.
  - **Interactive PRD layer:** Thread the `repo_clone_path()` through the `generate_*_with_timeout` functions into `create_backend`, which sets the `cwd` field on the constructed `CliBackend`. Remove all `CwdGuard::set()` calls from `generate_questions_with_timeout` (line 1394), `generate_draft_from_answers_with_timeout` (line 1267), and `generate_revision_from_feedback_with_timeout` (line 1020).

This is the **critical safety change** for correctness under concurrency.

**Step 4 — Replace the sequential `for` loop with `std::thread::scope` bounded by a channel semaphore.** After collecting the deduplicated issue list from both poll passes (which remain sequential — they are GitHub API reads that produce the input set), spawn each issue into a scoped thread bounded by a counting semaphore:

```rust
let max = config.max_concurrent.max(1) as usize; // treat 0 as 1

// Refresh once before parallel processing
config.refresh_repo_clone()?;

let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());

std::thread::scope(|s| {
    let semaphore = Arc::new(std::sync::Semaphore::new(max));  // or channel-based
    let mut handles = Vec::new();

    for issue in &all_deduplicated_issues {
        let permit = semaphore.clone();
        permit.acquire();  // blocks until slot available
        let config = &config;
        let errors = &errors;
        handles.push(s.spawn(move || {
            let _guard = SemaphoreGuard(&permit); // release on drop
            let mut bot_login_cache: Option<String> = None;
            if let Err(err) = advance_issue(config, issue, &mut bot_login_cache) {
                errors.lock().unwrap().push(format!(
                    "prd: failed to advance {}/{}#{}: {err}",
                    config.owner, config.repo, issue.number
                ));
            }
        }));
    }
    // All handles joined automatically when scope exits
});

for msg in errors.into_inner().unwrap() {
    eprintln!("{msg}");
}
```

Since `std::sync::Semaphore` is unstable, use a channel-based semaphore (`std::sync::mpsc::sync_channel` with capacity `max`) or pull in the lightweight `crossbeam` crate's scoped threads if already available. The scoped threads approach avoids `'static` bounds, allows borrowing `config` and `all_deduplicated_issues` without cloning, and provides automatic panic isolation per thread.

**Step 5 — Make `bot_login_cache` per-thread.** Currently the cache is a `&mut Option<String>` shared across all issues in a tick (line 407). Under concurrency, each thread gets its own `Option<String>`. The worst case is N redundant `github::fetch_authenticated_login()` calls (one per issue, N ≤ 5 by default). This is simpler and avoids synchronization. If this becomes a concern later, an `Arc<OnceLock<String>>` shared across threads can be introduced, but it is not required for correctness.

**Step 6 — Update `finish_transition` signature.** `finish_transition` currently takes `&mut Option<String>` for the bot login cache (line 1097). Change it to take an owned `Option<String>` or `Option<&str>` since each thread owns its cache. This is a mechanical signature change with no behavioral impact.

### Key Design Decisions

- **Scoped threads over async `JoinSet`**: The entire `advance_issue` call graph is deeply synchronous: `github::poll_issues` shells out to `gh`, state I/O uses `std::fs`, and `run_backend_sync` creates throwaway tokio runtimes. Converting this to async would require wrapping every blocking call in `spawn_blocking`, or risk stalling the tokio worker pool. Scoped threads are the natural concurrency primitive for blocking work and match the existing execution model.
- **Single repo refresh before spawn** over per-issue refresh: Eliminates the `git fetch && git reset --hard` race condition where concurrent tasks mutate the same git worktree. All backend calls in a tick see the same code snapshot, which is the correct semantic — the repo doesn't change meaningfully within a single poll tick.
- **Per-thread `bot_login_cache`** instead of `Arc<Mutex<>>`: Saves one GitHub API call per tick per issue at most. Under the default `max_concurrent = 5`, this means at most 4 extra `gh api user` calls — negligible cost. Avoids lock contention and simplifies the implementation.
- **Explicit cwd on `CliBackend`** over passing cwd at call time: Setting cwd at construction time keeps the `Backend` trait unchanged and confines the API change to `CliBackend`. All callers that need a specific working directory create the backend with the cwd set.
- **`max(1)` guard on `max_concurrent`**: A semaphore of size 0 would block forever. Treating 0 as 1 (serial execution) is a safe default that matches the pre-parallelization behavior.
- **PRD phase continues to block the main loop**: The daemon's main poll cycle (runtime.rs line 544) runs `run_prd_phase()` before claim/dispatch to prevent dual-ownership. The scoped thread block naturally waits for all issue tasks to complete, preserving this ordering invariant. Making the PRD phase non-blocking would require a different lifecycle design and is out of scope.

## Files & Modules

| File | Change |
|---|---|
| `src/daemon/interactive_prd.rs` | Add `max_concurrent: u32` to `PrdPollConfig`. Replace sequential `for` loop in `poll_and_advance_prd` with `std::thread::scope` bounded by a channel-based semaphore. Move `refresh_repo_clone()` call from `generate_*_with_timeout` functions to top of `poll_and_advance_prd` (single call per tick). Remove `CwdGuard` struct entirely. Remove all `CwdGuard::set()` calls from `generate_questions_with_timeout`, `generate_draft_from_answers_with_timeout`, and `generate_revision_from_feedback_with_timeout`. Thread `repo_clone_path` into `create_backend` calls to set cwd on `CliBackend`. Make `bot_login_cache` per-thread (local `Option<String>` per spawned thread). Update `finish_transition` signature to take owned/ref cache instead of `&mut Option<String>`. |
| `src/backend/mod.rs` | Add `cwd: Option<PathBuf>` field to `CliBackend`. In `execute_streaming`, apply `.current_dir(path)` to the `Command` when `self.cwd` is `Some`. Update `CliBackend` constructors / builder to accept optional cwd. |
| `src/daemon/runtime.rs` | Add `max_concurrent` to the `PrdPollConfig` construction in `run_prd_phase` (line 603), sourced from `config.max_concurrent`. |
| `tests/daemon_interactive_prd.rs` | Add integration tests for concurrent processing (see Testing Strategy). Existing tests pass without modification. |
| `src/validate/tests_interactive_prd.rs` | Add conformance tests for bounded concurrency, panic/error isolation, dedup invariant under parallelism, and repo clone refresh ordering (see Testing Strategy). |

## Testing Strategy

1. **Existing tests pass unchanged.** The 21 integration tests in `tests/daemon_interactive_prd.rs` and 41 conformance tests in `src/validate/tests_interactive_prd.rs` exercise single-issue state machine transitions end-to-end. These must continue to pass without modification — they validate that the parallelization refactor preserves single-issue correctness.

2. **New integration test: concurrent issue advancement.** Create a test in `tests/daemon_interactive_prd.rs` that sets up two PRD issues at different states (e.g., one `Pending` needing backend work, one `AwaitingFeedback` with an approval comment ready). Use a slow mock backend for the `Pending` issue (e.g., `sleep 5 && echo output`). Run a single daemon iteration. Assert both issues advance in the same tick — the `AwaitingFeedback` issue transitions to `Done` without waiting for the slow `Pending` backend to finish.

3. **New integration test: error isolation.** Set up two issues where one has a failing backend (exit code 1) and the other has a working backend. Run a single iteration. Assert the healthy issue advances successfully while the failing issue records an error without affecting the other.

4. **New conformance test: bounded concurrency (`daemon_max_concurrent`).** Verify that with `max_concurrent = 2` and 4 issues, at most 2 are processed simultaneously. Use a mock backend that writes a timestamp file on entry/exit and assert no more than 2 overlap.

5. **New conformance test: panic isolation.** Inject a panic in one issue's processing path (e.g., via a mock backend that triggers a known panic path). Verify that other issues in the same tick advance successfully and the daemon does not crash.

6. **New conformance test: dedup invariant under parallel execution.** Create a scenario where the same issue appears in both `ralph:prd` and `ralph:prd-active` label queries. Verify it is processed exactly once per tick. This tests that the `HashSet<u32>` dedup runs before thread spawning (in the sequential collection phase).

7. **New conformance test: repo clone refresh ordering.** Verify that `refresh_repo_clone` is called exactly once per tick, before any `generate_*_with_timeout` function executes. Use a mock that records call ordering to confirm no per-issue refresh occurs.

8. **CwdGuard removal verification.** Verify that concurrent backend invocations with different repo clone paths do not interfere with each other. This is implicitly covered by the concurrent advancement test (item 2), since both issues would fail if cwd were mutated globally.

## Out of Scope

- Converting `poll_and_advance_prd` or `advance_issue` from sync to async. The call graph is deeply synchronous (GitHub CLI, state file I/O, subprocess spawning with throwaway runtimes). Thread-based concurrency matches the existing execution model without risking tokio runtime stalls.
- Converting `github::poll_issues` from sync to async.
- Changing the daemon's main poll loop structure. `run_prd_phase` continues to block the main loop until all PRD tasks complete, preserving the claim/dispatch ordering invariant.
- Changing `daemon_max_concurrent` semantics for non-PRD issue processing. The same config value is reused for PRD concurrency.
- Per-issue concurrency within a single transition (e.g., running Backend A and Backend B in parallel during question generation) — that is orthogonal and can be layered on later.
- Shared bot login cache optimization (e.g., `Arc<OnceLock<String>>` shared across threads) — per-thread caching is sufficient for the default max_concurrent of 5.
- Changes to the PRD state machine logic itself (states, transition rules, error retry thresholds).
- Making the `Backend` trait async-aware of working directory. The cwd is set as a field on `CliBackend` at construction time; the trait interface is unchanged.