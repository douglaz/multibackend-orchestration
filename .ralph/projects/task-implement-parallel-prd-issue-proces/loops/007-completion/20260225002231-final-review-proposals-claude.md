---
artifact: final-review-proposals
loop: 7
project: task-implement-parallel-prd-issue-proces
backend: claude
role: final_reviewer
created_at: 2026-02-25T00:22:31Z
---

All 760 unit tests pass. Let me now write the final review.

# Final Review: NO AMENDMENTS

## Summary

The implementation correctly fulfills all requirements from the master prompt. I verified by reading source code, tracing dependencies across files, running all test suites, and performing a thread-safety analysis.

### Core Changes Verified

**1. Concurrency model in `poll_and_advance_prd`** (`src/daemon/interactive_prd.rs:391-485`):
- Sequential poll queries (Phase 1), dedup across `ralph:prd` and `ralph:prd-active` (Phase 2), once-per-tick repo refresh (Phase 3), bounded `std::thread::scope` worker pool with `Mutex<VecDeque>` work queue (Phase 4), and aggregated error reporting (Phase 5).
- `worker_count = max(1, config.max_concurrent)` correctly handles the 0→1 case.
- Each worker has its own `bot_login_cache: Option<String>` (line 429).
- `std::panic::catch_unwind(AssertUnwindSafe(...))` wraps each issue's processing (line 440).

**2. Repo refresh ordering** (`src/daemon/interactive_prd.rs:413-419`):
- `refresh_repo_clone()` runs once after dedup and before workers start. Removed from `generate_questions_with_timeout`, `generate_draft_from_answers_with_timeout`, and `generate_revision_from_feedback_with_timeout`.

**3. CWD safety** (`src/backend/mod.rs:170, 195-197, 478-480`):
- `CwdGuard` fully removed from PRD paths. `CliBackend` has new `cwd: Option<PathBuf>` field with `with_cwd()` builder. `execute_streaming` applies `Command::current_dir(cwd)` when set. All existing callers pass `None`, preserving default behavior.

**4. Backend `cwd` propagation** (`src/backend/claude.rs`, `codex.rs`, `gemini.rs`):
- All `backend_from_config` functions accept `cwd: Option<PathBuf>` and chain `.with_cwd(cwd)`. All non-PRD call sites pass `None`.

**5. `PrdPollConfig.max_concurrent` wiring** (`src/daemon/runtime.rs:615`):
- `max_concurrent` populated from `DaemonRuntimeConfig.max_concurrent`.

**6. Thread-safety**: Confirmed via agent analysis — `PrdPollConfig` is read-only shared, `bot_login_cache` is per-thread, `InteractivePrdState` uses issue-specific file paths with atomic persist, GitHub functions are stateless subprocess calls, backends are fresh per call, no global CWD mutations.

### Testing Verified

All tests pass:
- **761 unit tests** (760 pass, 1 ignored) — `cargo test --lib`
- **35 integration tests** — `cargo test --test daemon_interactive_prd` — all pass, including 7 new concurrency tests:
  - `dedup_invariant_issue_processed_at_most_once`
  - `max_concurrent_zero_treated_as_one` / `max_concurrent_preserves_configured_value`
  - `error_isolation_tick_succeeds_despite_issue_error`
  - `empty_polls_return_early`
  - `concurrent_advancement_slow_and_fast` (barrier-based deterministic sync)
  - `bounded_concurrency_peak_never_exceeds_max` (flock-based atomic counter)
  - `panic_isolation_tick_completes_despite_panic` (env-var injection)
  - `refresh_repo_clone_once_before_processing` (event log ordering)
- **7 new conformance tests** registered in `src/validate/tests_interactive_prd.rs` covering all required scenarios.
- Conformance tests use `ENV_MUTEX` for process-global PATH serialization; integration tests use `#[serial]`.

### No Stray Files
`git status` shows only `.ralph/` (project metadata, not tracked).
