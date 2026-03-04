---
artifact: quick-dev-plan-implement
loop: 15
project: issue-161
backend: claude
role: implementer
created_at: 2026-03-04T18:52:53Z
---

# Implementation: Guard against planner completion requests with unaddressed amendments

## Changes Made

### 1. `src/workflow/orchestrator.rs` — Guard in `PlannerDecision::CompletionRequest` branch (line 655)

Added a ~12-line guard at the top of the `PlannerDecision::CompletionRequest` arm that:
- Checks `state.completion_attempts.last()` for the most recent completion loop
- Uses `resolve_artifact_path_by_suffix()` to check for `final-review-exit-restart.md` in that loop's directory
- Returns `Err(RalphError::Orchestration("planner requested completion without addressing final review amendments"))` if the restart artifact exists
- Fires **before** any backend resolution or artifact writes

### 2. `src/validate/tests_final_review.rs` — Updated `restart_round_then_complete` mock script

The planner now uses a counter:
- **Call 1**: Returns `CompletionRequest` (first completion attempt → final review → AMENDMENTS → restart)
- **Call 2**: Returns `Feature: Address Amendments` (implementation loop to address amendments)
- **Call 3+**: Returns `CompletionRequest` (second completion attempt → final review → NO AMENDMENTS → approved)

Added implementer and code reviewer handlers to support the new implementation loop in the flow.

### 3. `src/validate/tests_final_review.rs` — New conformance test `planner_completion_after_amendments_fails`

Tests the guard by using a mock where the planner **always** returns `CompletionRequest` regardless of amendments. Asserts:
- Exit code 1
- Stderr contains the expected error message
- Only 1 completion attempt exists (guard fired before second was registered)

### 4. Regression coverage

The existing `completion_no_amendments` test already covers the valid path — planner returns `CompletionRequest` with no prior completion attempts, and the project completes successfully (exit code 0, status "completed").
