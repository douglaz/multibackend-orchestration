---
artifact: prompt-review
project: task-implement-parallel-prd-issue-proces
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-24T20:28:38Z
---

# Prompt Review

## Issues Found
- The prompt proposes `std::sync::Semaphore`, which is not available on stable Rust. This creates implementation ambiguity and risk of dead-end design work.
- Panic isolation is underspecified and partially incorrect: panics in scoped threads can still unwind the parent scope unless explicitly caught. This can violate the requirement that one issue failure must not affect others.
- Acceptance criterion about “issue M advances while issue N is slow” conflicts with `daemon_max_concurrent = 0/1` serial behavior. The condition needs to be scoped to `max_concurrent > 1`.
- Error semantics are unclear (log-and-continue vs fail the whole tick). Without this, downstream loops may implement inconsistent behavior.
- The cwd fix is directionally correct but missing compatibility requirements for non-PRD callers. Default behavior must be explicitly preserved.
- Repo refresh timing is not fully specified for empty issue sets. Without this, tests and behavior can diverge on whether refresh runs unnecessarily.
- The testing plan relies on sleep/timing examples, which are prone to flakes. Deterministic synchronization requirements should be explicit.
- The prompt is over-constrained by line numbers and specific internals that may drift; this reduces maintainability for downstream implementation loops.

## Refined Prompt
### Goal
Refactor `poll_and_advance_prd()` in `src/daemon/interactive_prd.rs` so PRD issues are processed concurrently (bounded), while keeping the function synchronous and preserving existing daemon ordering guarantees.

### Current Problem
`poll_and_advance_prd()` currently processes issues sequentially inside one `spawn_blocking` call. A single long backend operation (question generation, draft generation, revision) can block all other issues in the same poll tick.

### Required Behavior
1. `poll_and_advance_prd()` must process multiple PRD issues concurrently within one invocation (“tick”).
2. When `max_concurrent > 1`, a slow issue must not block unrelated issues from advancing in that same tick.
3. Concurrency must be bounded by `daemon_max_concurrent` from config. Treat `0` as `1`.
4. Preserve state-machine correctness for all current transitions (`Pending` → `AwaitingAnswers` → `AwaitingFeedback` → `Done`/`Failed`).
5. Failure or panic while processing one issue must not stop other issues in the same tick.
6. Dedup across `ralph:prd` and `ralph:prd-active` poll passes must remain: each issue processed at most once per tick.
7. `run_prd_phase` must remain blocking: it waits until all per-issue work in that tick is complete before returning.
8. Existing integration and validate tests must continue to pass.

### Design Constraints
- Keep the PRD path synchronous (no async refactor of the call graph).
- Use thread-based concurrency with stable Rust primitives only.
- Do not mutate process-global cwd during per-issue work.

### Implementation Requirements

#### 1) Concurrency model in `poll_and_advance_prd`
- Keep poll queries sequential.
- Deduplicate issues before spawning workers.
- Compute `worker_count = max(1, config.max_concurrent)`.
- Use `std::thread::scope` with a bounded worker pool (for example: shared `Mutex<VecDeque<_>>` work queue with `worker_count` threads). Avoid unstable or non-existent std APIs.
- If no deduplicated issues exist, return early (no per-issue work).

#### 2) Repo refresh ordering
- Call `refresh_repo_clone()` once per non-empty tick, after poll/dedup and before worker processing.
- Remove per-issue/per-generation refresh calls from `generate_*_with_timeout` paths to avoid concurrent git operations on the same clone.

#### 3) CWD safety
- Remove `CwdGuard` usage from PRD processing.
- Add `cwd: Option<PathBuf>` to `CliBackend` (or equivalent constructor/builder field).
- In `CliBackend::execute_streaming`, set `Command::current_dir(cwd)` when provided.
- Preserve default behavior for all existing callers by using `None` when cwd is not set.
- In interactive PRD backend creation, pass the repo clone path explicitly as cwd.

#### 4) Per-issue isolation
- Each worker/thread owns its own `bot_login_cache: Option<String>`.
- Do not share mutable per-issue transition state across workers.
- If function signatures currently require shared mutable cache references, update them to per-thread-friendly forms.

#### 5) Error and panic handling
- Wrap each issue’s processing in `std::panic::catch_unwind`.
- Convert panic/failure into a per-issue error record; continue processing remaining issues.
- Aggregate per-issue errors thread-safely and emit them after workers join.
- Do not let a single issue panic unwind the entire tick.

### File-Level Change Targets
- `src/daemon/interactive_prd.rs`
  - Add `max_concurrent` to `PrdPollConfig`.
  - Replace sequential issue loop with bounded concurrent worker processing.
  - Dedup before spawning.
  - Move clone refresh to once-per-tick location.
  - Remove `CwdGuard` usage and adapt backend creation to explicit cwd.
  - Ensure per-thread bot login cache and panic isolation.
- `src/backend/mod.rs`
  - Extend `CliBackend` with optional cwd and apply it in process spawn.
  - Keep default behavior unchanged when cwd is unset.
- `src/daemon/runtime.rs`
  - Populate PRD poll config with `daemon_max_concurrent`.
- `tests/daemon_interactive_prd.rs`
  - Add deterministic integration tests for concurrency/isolation behavior.
- `src/validate/tests_interactive_prd.rs`
  - Add/extend conformance coverage for new concurrency guarantees.

### Testing Requirements
Use deterministic synchronization, not sleep-based timing assumptions.

1. Concurrent advancement test:
- One slow issue and one immediately-advanceable issue.
- With `max_concurrent >= 2`, assert both advance in one tick.

2. Bounded concurrency test:
- Configure `max_concurrent = 2`.
- Track active worker count via atomic counter in mock backend.
- Assert observed peak concurrency never exceeds 2.

3. Error isolation test:
- One issue fails backend call, one succeeds.
- Assert success path still advances.

4. Panic isolation test:
- Force panic in one issue path (test hook/mocked path).
- Assert daemon tick completes and other issues continue.

5. Dedup invariant test:
- Same issue appears in both label polls.
- Assert it is processed exactly once per tick.

6. Repo refresh ordering test:
- Assert refresh occurs once per non-empty tick and before any backend invocation.

7. Regression coverage:
- Existing `tests/daemon_interactive_prd.rs` and `src/validate/tests_interactive_prd.rs` suites remain green.

### Non-Goals
- No async conversion of PRD orchestration internals.
- No change to daemon phase ordering (PRD remains blocking before claim/dispatch).
- No change to PRD state-machine rules beyond concurrency-safe execution.
