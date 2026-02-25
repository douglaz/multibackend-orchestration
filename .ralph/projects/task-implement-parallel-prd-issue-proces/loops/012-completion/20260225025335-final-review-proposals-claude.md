---
artifact: final-review-proposals
loop: 12
project: task-implement-parallel-prd-issue-proces
backend: claude
role: final_reviewer
created_at: 2026-02-25T02:53:35Z
---

Let me do a final summary review of all requirements against the implementation:

**Requirement checklist:**

1. **Concurrent PRD issue processing** — `std::thread::scope` with `Mutex<VecDeque>` work queue (line 422-465). ✅
2. **Slow issue doesn't block others** — Each worker pops independently from queue. ✅ (verified by FIFO-based `concurrent_advancement_slow_and_fast` test)
3. **Bounded by max_concurrent** — `worker_count = max(1, config.max_concurrent)` (line 422). ✅
4. **State-machine correctness preserved** — `advance_issue` unchanged; same transitions. ✅
5. **Failure/panic isolation** — `catch_unwind(AssertUnwindSafe(...))` around each issue (line 440). Errors collected thread-safely. ✅
6. **Dedup across poll passes** — `HashSet<u32>` dedup before spawning (line 400-406). ✅
7. **Blocking: waits until complete** — `std::thread::scope` blocks until all threads join. ✅
8. **Existing tests pass** — 760 unit tests, 35 integration tests all pass. ✅

**Design constraint checklist:**

1. **Keep PRD path synchronous** — No async refactor. `run_backend_sync` creates per-call tokio runtime. ✅
2. **Thread-based concurrency with stable Rust** — `std::thread::scope` (stable since Rust 1.63). ✅
3. **No process-global cwd mutation** — CwdGuard removed from PRD. `CliBackend::cwd` field with `Command::current_dir()`. ✅

**Implementation requirements:**

1. **Concurrency model** — Sequential polls → dedup → worker pool with `Mutex<VecDeque>`. ✅
2. **Repo refresh ordering** — `refresh_repo_clone()` called once after dedup, before workers (line 414). ✅
3. **CWD safety** — `cwd: Option<PathBuf>` field on `CliBackend` (line 170), `with_cwd` builder (line 195), applied in `execute_streaming` (line 478-479). All backends updated. ✅
4. **Per-issue isolation** — Per-thread `bot_login_cache` (line 429). No shared mutable per-issue state. ✅
5. **Error and panic handling** — `catch_unwind` + error aggregation. ✅

**Test requirements:**

1. **Concurrent advancement** — `concurrent_advancement_slow_and_fast` (FIFO-based deterministic). ✅
2. **Bounded concurrency** — `bounded_concurrency_peak_never_exceeds_max` (atomic counter in mock). ✅
3. **Error isolation** — `error_isolation_tick_succeeds_despite_issue_error`. ✅
4. **Panic isolation** — `panic_isolation_tick_completes_despite_panic` (uses `RALPH_TEST_INJECT_PANIC`). ✅
5. **Dedup invariant** — `dedup_invariant_issue_processed_at_most_once`. ✅
6. **Repo refresh ordering** — `refresh_repo_clone_once_before_processing`. ✅
7. **Regression coverage** — All existing tests pass. ✅

# Final Review: NO AMENDMENTS

## Summary

The implementation correctly fulfills all requirements of the master prompt. I verified by reading source code across 9 modified files:

**Core concurrency** (`src/daemon/interactive_prd.rs:391-485`): `poll_and_advance_prd` uses sequential polls → `HashSet`-based dedup → single `refresh_repo_clone()` call → `std::thread::scope` with `Mutex<VecDeque>` bounded worker pool (`max(1, max_concurrent)` threads). Each worker owns its own `bot_login_cache` and wraps `advance_issue` in `catch_unwind(AssertUnwindSafe(...))`. Errors are collected thread-safely and emitted after all workers join. The function always returns `Ok(())`.

**CWD safety** (`src/backend/mod.rs:170,195-198,478-479`): `CliBackend` gained `cwd: Option<PathBuf>` field with `with_cwd` builder. Applied via `cmd.current_dir(cwd)` in `execute_streaming`. All 3 backend factories (`claude.rs`, `codex.rs`, `gemini.rs`) accept and forward the new parameter. Non-PRD callers pass `None`, preserving default behavior. PRD `create_backend` passes `Some(repo_clone_path())`.

**Config plumbing** (`src/daemon/runtime.rs:615`): `max_concurrent` properly propagated from `DaemonRuntimeConfig` to `PrdPollConfig`.

**Thread safety**: No shared mutable state across workers. State files are keyed by issue number (no write contention). `bot_login_cache` is per-thread. GitHub API calls are stateless. `run_backend_sync` creates per-call tokio runtime. `AssertUnwindSafe` wrapping is safe since `bot_login_cache` is per-thread-owned `Option<String>`.

**Tests**: 9 new integration tests and 8 new conformance tests cover all specified scenarios using deterministic FIFO-based synchronization (no sleep-based timing). All 760 unit tests and 35 integration tests pass. No stray files.
