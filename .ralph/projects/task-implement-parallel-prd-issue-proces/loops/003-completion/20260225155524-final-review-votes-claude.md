---
artifact: final-review-votes
loop: 3
project: task-implement-parallel-prd-issue-proces
backend: claude
role: final_reviewer
created_at: 2026-02-25T15:55:24Z
---

# Vote Results

## Amendment: SLOW-FAST-WATCHDOG

### Vote
ACCEPT

### Rationale
Verified: `tests/daemon_interactive_prd.rs:3241` calls `poll_and_advance_prd(&config)` directly on the test thread with no timeout. The test uses FIFOs (mkfifo at lines 3086-3090), creating a deadlock scenario if concurrency regresses to sequential processing. The conformance test at `src/validate/tests_interactive_prd.rs:4125-4133` correctly uses `std::thread::spawn` + `recv_timeout(30s)`. The integration test should match. This is a real CI reliability issue — a concurrency regression would hang `cargo test` indefinitely with no diagnostic. The proposed watchdog pattern is minimal, proven (already used in conformance), and directly addresses the hang risk.

## Amendment: OUT-OF-SCOPE-LEGACY-REMOVAL

### Vote
REJECT

### Rationale
I agree with the planner's position. The amendment raises a process/scope concern ("should be in a separate PR"), not a technical correctness issue. The legacy symlinks (`planner.md`, `implementer.md`, etc.) are convenience aliases to canonical templates that still exist. Their removal is a deliberate cleanup, not an accidental deletion or bug. No user-facing functionality is broken — the canonical template paths remain. Organizational concerns about PR scope are valid in a team context but do not constitute a technical amendment warranting code changes.

## Amendment: FR-PRD-001

### Vote
ACCEPT

### Rationale
Verified: all concurrent workers call `create_backend` with `Some(config.repo_clone_path())` at lines 1081, 1335, and 1479 in `interactive_prd.rs`. The backends are write-capable (Edit, Write tools confirmed at `src/config/global.rs:701`; Codex uses `--dangerously-bypass-approvals-and-sandbox` at line 730). `CliBackend::execute` sets `cmd.current_dir(cwd)` at `src/backend/mod.rs:478-479`. Multiple concurrent backend processes with write access to the same filesystem directory is a genuine shared-mutable-state defect. Even if current tests don't expose corruption, this is architecturally unsound for any production use with concurrent write-capable backends. Per-issue isolated working directories are the correct fix.

## Amendment: FR-PRD-002

### Vote
ACCEPT

### Rationale
Verified: at `src/daemon/interactive_prd.rs:440-460`, when `advance_issue` panics, `catch_unwind` catches it and pushes the error to a shared `errors` vec, but the issue's persisted state is never updated. `advance_issue` loads `InteractivePrdState` at line 494 and calls `finish_transition` only on the normal error path. A panic bypasses `finish_transition` entirely, meaning `error_count` is never incremented and the issue stays in a non-terminal state. On the next tick, the same panic recurs — forever. This is a genuine infinite-retry bug for repeatable panics. Converting panics to `Err` values and routing through the existing `finish_transition` / failure-threshold path is the correct fix.

## Amendment: FR-PRD-003

### Vote
ACCEPT

### Rationale
Verified: `tests/daemon_interactive_prd.rs:3502` (bounded-worker test `bounded_concurrency_peak_never_exceeds_max`) and `src/validate/tests_interactive_prd.rs:3839` (bounded-worker conformance test) both call `poll_and_advance_prd` directly without watchdog timeouts. The validate runner at `src/validate/runner.rs:117` (`run_parallel`) has no per-test timeout. This amendment is broader than SLOW-FAST-WATCHDOG, covering additional FIFO/barrier test sites. The RAII env restoration proposal is also sound — a timeout or panic before the `set_var("PATH", &old_path)` cleanup will leak a mutated PATH into subsequent tests. Both the watchdog addition and the RAII cleanup are warranted.

## Amendment: PRD-CONCURRENCY-STATE-LOSS

### Vote
REJECT

### Rationale
The amendment's description of the code is factually incorrect. It claims workers create their own `prd_state` via `PrdState::from_root(root)` in a `std::thread::scope` block at lines 207-259, and references `sled` databases. None of this exists. The actual implementation at lines 426-465 uses `advance_issue(config, &issue, ...)`, which loads per-issue state via `InteractivePrdState::load` from individual JSON files on disk (line 494). Each issue has its own state file keyed by issue number. There is no `PrdState` type, no `sled`, no `PrdState::from_root`. The per-issue state isolation is by design. The amendment fundamentally mischaracterizes the implementation and proposes a fix for a non-existent problem.

## Amendment: PRD-INCORRECT-ERROR-HANDLING

### Vote
REJECT

### Rationale
The amendment references functions and methods that do not exist in the codebase: `generate_answers_with_timeout`, `generate_feedback_with_timeout`, `record_gaps_failed`, and `record_revision_failed`. The actual functions are `generate_questions_with_timeout`, `generate_draft_from_answers_with_timeout`, and `generate_revision_from_feedback_with_timeout`. Error handling flows through `finish_transition` which calls `apply_transition_result`, incrementing `error_count` on errors and transitioning to `Failed` at the threshold. The amendment describes non-existent code and proposes changes to a non-existent error handling pattern. The actual error handling works as designed.

## Amendment: PRD-DEAD-CODE-NORMALIZER

### Vote
REJECT

### Rationale
The amendment claims `poll_and_advance_prd` constructs an `Arc<Mutex<BackendOutputNormalizer>>` at lines 164-167 of `interactive_prd.rs`. I verified that neither `BackendOutputNormalizer` nor `output_normalizer` appear anywhere in `src/daemon/interactive_prd.rs`. While `BackendOutputNormalizer` does exist in the codebase (in `src/backend/output_normalizer.rs`, used in `src/backend/mod.rs` and `src/workflow/orchestrator.rs`), it is not present in the interactive PRD module. The code described by this amendment simply does not exist. There is nothing to remove.
