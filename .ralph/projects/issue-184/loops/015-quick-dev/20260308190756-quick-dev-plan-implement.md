---
artifact: quick-dev-plan-implement
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T19:07:56Z
---

# Implementation: Review Amendment Fixes

All 7 review amendments have been addressed. Here's a summary of changes:

## [P1] DRAIN-EXTERNAL-ABORT-FLAG-LOSS (`src/daemon/runtime.rs`)
The forced-drain path in `drain_all_children_with_deadline` was hardcoding `externally_aborted = false` when calling `complete_task`. Now reads `handle.aborted_externally.load(Ordering::Relaxed)` to propagate the actual flag, aligning with the normal `collect_children` path.

## [P2] Test Semantic Drift (A1) (`src/validate/tests_daemon_concurrency.rs`)
Updated docstrings on `partial_dispatch_rollback` and `dispatch_failure_explicit_markers` to accurately describe their semantics (drain cancels all tasks equally) and cross-reference `execution_failure_terminalization` and `mixed_outcome_claim_isolation` for the isolation invariants they don't cover.

## [P2] Concurrency Ordering Assertion (A2 + codex P2) (`src/validate/tests_daemon_concurrency.rs`)
Made the ordering assertion in `concurrent_dispatch_evidence` mandatory — changed `if let (Some, Some)` to `expect()` calls that fail explicitly if either position is missing, preventing the assertion from being silently skipped.

## [P3] Quick-PRD Cooperative Cancellation (A3) (`src/prd/quick.rs`, `src/daemon/tasks.rs`, `src/daemon/interactive_prd.rs`)
- Added `cancel: CancellationToken` field to `QuickPrdPipeline` with `with_cancel()` constructor
- Replaced all `backend.execute()` calls inside the pipeline with `backend.execute_with_cancel(_, None, &self.cancel)` — enabling SIGTERM→5s→SIGKILL cooperative shutdown instead of immediate SIGKILL via `KillOnDrop`
- Updated `run_review_with_retry` to accept a `&CancellationToken` parameter
- Daemon task entry points use `with_cancel(params.cancel.clone())`; CLI/interactive callers use the default `CancellationToken::new()`

## [P3] `preload_bare_default_backends` Documentation (A4) (`src/workflow/orchestrator.rs`)
Added doc comment documenting the hard-coded backend list limitation and suggesting future improvements.

## [P3] Project-Creation Assertion (`src/validate/tests_daemon.rs`)
Made the `project_state_dir.exists()` check unconditional (removed `if` guard) so the test fails explicitly if project creation regresses.

## Verification
- `cargo check`: clean build
- All 16 quick-prd unit tests: pass
- All daemon task tests: pass
- 967/970 library tests pass (2 pre-existing failures unrelated to changes)
