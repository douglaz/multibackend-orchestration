---
artifact: final-review-planner-positions
loop: 3
project: task-implement-parallel-prd-issue-proces
backend: claude(opus)
role: planner
created_at: 2026-02-25T15:53:13Z
---

Now I have everything I need. Let me compile my findings.

# Planner Positions

## Amendment: SLOW-FAST-WATCHDOG

### Position
ACCEPT

### Rationale
Verified at `tests/daemon_interactive_prd.rs:3239-3242`: the integration test `concurrent_advancement_slow_and_fast()` calls `poll_and_advance_prd(&config)` directly on the test thread with no timeout protection. The test uses a FIFO (`mkfifo` at line 3086) where issue #80 blocks on a `read` until issue #90 writes — if concurrency regresses to sequential processing, this hangs indefinitely.

The conformance counterpart at `src/validate/tests_interactive_prd.rs:4121-4134` correctly uses `std::thread::spawn` + `std::sync::mpsc::channel` + `recv_timeout(30s)` to bound the same call. The integration test lacks this pattern. This is a real CI/test-suite reliability issue — a regression would hang `cargo test` forever with no diagnostic output.

## Amendment: OUT-OF-SCOPE-LEGACY-REMOVAL

### Position
REJECT

### Rationale
The amendment's framing is about scope/process ("should be in a separate PR"), which is not a technical correctness issue. The commit `7cdda5b` genuinely removed `LEGACY_LINKS`, `CreateLegacyLink`, and symlink logic from `src/cli/init.rs` — I confirmed the diff. However, the legacy links (`planner.md`, `implementer.md`, `reviewer.md`, `completer.md`) are convenience symlinks to canonical template names that still exist. Removing these symlinks is a deliberate cleanup, not a bug. The amendment does not identify any code incorrectness, safety issue, or robustness problem — it is purely an organizational/process concern about PR scope, which is not grounds for a technical amendment.

## Amendment: FR-PRD-001

### Position
ACCEPT

### Rationale
Verified at `src/daemon/interactive_prd.rs:421-465`: concurrent worker threads all call `advance_issue(config, ...)` which in turn calls `generate_questions_with_timeout`, `generate_draft_from_answers_with_timeout`, and `generate_revision_from_feedback_with_timeout`. Each of these functions calls `create_backend(..., Some(config.repo_clone_path()))` (lines 1081, 1335, 1479), meaning every concurrent backend gets the same `cwd`. The backends are configured with write-capable tools (`Edit`, `Write` at `src/config/global.rs:701,730`), and `CliBackend::execute` sets `current_dir(cwd)` at `src/backend/mod.rs:478-479`. Multiple concurrent backend processes writing to the same filesystem directory is a real shared-mutable-state defect. Even if current test scenarios don't trigger visible corruption, this is architecturally unsound for write-capable concurrent backends.

## Amendment: FR-PRD-002

### Position
ACCEPT

### Rationale
Verified at `src/daemon/interactive_prd.rs:440-461`: when `advance_issue` panics inside a worker, `catch_unwind` catches it and the error is pushed to the shared `errors` vec (line 459). However, the state for that issue is never updated — `advance_issue` loads its own `InteractivePrdState` on line 493-495 and calls `finish_transition` (which increments `error_count` and persists state) only on the normal error path. A panic bypasses `finish_transition` entirely, so the issue's persisted state retains `error_count=0` and stays in a non-terminal state. On the next daemon tick, the same issue will panic again, forever. This is a genuine infinite-retry bug for repeatable panics — the durable failure accounting at `apply_transition_result` (line 1221) and `transition_to_failed` (line 1566) is completely bypassed.

## Amendment: FR-PRD-003

### Position
ACCEPT

### Rationale
Verified that `tests/daemon_interactive_prd.rs:3502` (bounded-worker test) also calls `poll_and_advance_prd` directly without a watchdog, and `src/validate/tests_interactive_prd.rs:3839` does the same for its bounded-worker test. The validate runner at `src/validate/runner.rs:117` (`run_parallel`) has no per-test timeout — it dispatches tests to worker threads with no deadline. This amendment correctly identifies that FIFO-based tests at lines 3241 and 3502 in the integration tests, plus line 3839 in the conformance tests, all lack watchdog timeouts. This overlaps with SLOW-FAST-WATCHDOG (which covers one of the same sites) but is broader in scope, covering additional FIFO and barrier tests. The RAII env restoration improvement is also technically sound — a timeout/panic before `set_var("PATH", &old_path)` will leak the mutated PATH.

## Amendment: PRD-CONCURRENCY-STATE-LOSS

### Position
REJECT

### Rationale
The amendment claims each worker thread "creates its own `prd_state` by calling `PrdState::from_root(root)`" in a `std::thread::scope` block at lines 207-259. This is factually wrong. No such code exists. The actual implementation at lines 426-465 uses `std::thread::scope` where each worker calls `advance_issue(config, &issue, ...)` (line 441), and `advance_issue` loads state via `InteractivePrdState::load(&config.data_dir, ...)` from a JSON file on disk (line 493-494). Each issue has its own separate state file (keyed by issue number at `state_path()` line 195-202). State is persisted to disk via `state.save()` inside `finish_transition` (line 1186). There is no "PrdState" type, no `sled` database, no `PrdState::from_root`. Each issue's state is independent by design (separate files), so there is no cross-issue state loss. The amendment fundamentally mischaracterizes the implementation.

## Amendment: PRD-INCORRECT-ERROR-HANDLING

### Position
REJECT

### Rationale
The amendment references functions `generate_answers_with_timeout` and `generate_feedback_with_timeout` and methods `record_gaps_failed` and `record_revision_failed`. None of these exist in the codebase. I searched for all four names via `grep` and found zero matches. The actual function names are `generate_questions_with_timeout`, `generate_draft_from_answers_with_timeout`, and `generate_revision_from_feedback_with_timeout`. Error handling is done through `finish_transition` which calls `apply_transition_result` (line 1221) — incrementing `error_count` on `Err` and transitioning to `Failed` at threshold 3. The amendment describes code that does not exist and proposes changes to a non-existent error handling pattern.

## Amendment: PRD-DEAD-CODE-NORMALIZER

### Position
REJECT

### Rationale
The amendment claims that `poll_and_advance_prd` constructs an `Arc<Mutex<BackendOutputNormalizer>>` on lines 164-167. I searched the entire `src/daemon/interactive_prd.rs` file for `BackendOutputNormalizer` and `output_normalizer` and found zero matches. The `BackendOutputNormalizer` exists in the codebase (`src/backend/output_normalizer.rs`, used in `src/backend/mod.rs:692` and `src/workflow/orchestrator.rs`), but it is not used anywhere in the interactive PRD module. The code described by this amendment simply does not exist in `interactive_prd.rs`. There is nothing to remove.
