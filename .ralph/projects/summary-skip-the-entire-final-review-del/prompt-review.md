---
artifact: prompt-review
project: summary-skip-the-entire-final-review-del
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-24T07:25:51Z
---

# Prompt Review

## Issues Found
- Uses hardcoded line numbers (for example `3330`, `3691`). This is brittle and can cause incorrect edits after routine refactors.
- “Zero backend calls” is ambiguous because backend health checks run before `restart_count`; the requirement should explicitly scope this to deliberation calls.
- The boundary behavior change (`restart_count == max`) is described but not consistently enforced as a strict artifact-level acceptance condition.
- Test instructions mention a new validate module but omit mandatory registration in `src/validate/mod.rs`, so tests could be added but never executed.
- The empty-amendments check is defined via fragile markdown text shape (“no `- ` lines”) rather than a stable section-level assertion.
- Late-cap-check removal is specified by line range instead of semantic location, increasing risk of partial/incorrect deletion.
- Logging expectations are underspecified (exact substring/location), which makes test assertions inconsistent.
- The prompt relies on `restart_count` immutability for dead-code removal but does not state that invariant explicitly.

## Refined Prompt
### Objective
Optimize final-review execution by skipping the deliberation pipeline when the restart cap has already been reached, preventing wasted deliberation backend calls.

### Target
- File: `src/workflow/orchestrator.rs`
- Function: `run_final_review_phase`

### Required Behavior
1. Keep existing computation of:
   - `restart_count = final_review_restart_count_from_artifacts(project_dir)`
   - `round = restart_count.saturating_add(1)`
2. Immediately after those computations, add an early guard:
   - Condition: `restart_count >= effective.workflow.max_final_review_restarts`
   - Actions:
     - Call `write_force_complete_artifact(`  
       `project_dir, round, restart_count, effective.workflow.max_final_review_restarts, &BTreeSet::new()`  
       `)?;`
     - Set:
       - `state.status = ProjectStatus::Completed`
       - `state.current_phase = Phase::Completing`
       - `state.phase_iteration = 1`
     - Push a log line containing the substring: `"skipping deliberation"`
     - Return `Ok(Some((Phase::FinalReview, Phase::Completing)))`
3. Remove the existing late restart-cap guard in the post-deliberation region (the one that force-completes using `final_accepted`).
4. Preserve existing behavior for all paths where `restart_count < effective.workflow.max_final_review_restarts`.

### Constraint
- Do **not** move this cap check ahead of `resolve_effective_final_review_backends(...)`.
- Health-check/setup work may still occur before `restart_count`; the optimization is specifically to skip deliberation calls.

### Intentional Behavioral Change
- At `restart_count == max_final_review_restarts`, final review must now force-complete immediately.
- It must no longer run deliberation and no longer produce `final-review-exit-approved.md` for that boundary case.

### Acceptance Criteria
- Early cap guard runs immediately after `restart_count`/`round` computation and before reviewer proposals begin.
- When cap is reached:
  - `write_force_complete_artifact` is called with `&BTreeSet::new()`
  - state becomes `Completed` + `Completing` + `phase_iteration = 1`
  - function returns `Ok(Some((Phase::FinalReview, Phase::Completing)))`
  - logs include `"skipping deliberation"`
  - no reviewer/planner/voter/arbiter deliberation backend calls are made
- Late cap guard is removed from the post-deliberation path.
- For `restart_count < cap`, deliberation flow remains unchanged.

### Test Requirements
Add validate conformance coverage and register it in `src/validate/mod.rs`.

1. `cap_reached_skips_deliberation_and_force_completes`
- Setup:
  - `max_final_review_restarts = 1`
  - Pre-seed one restart artifact so `restart_count == 1`
- Assert:
  - final state is completed/completing
  - force-complete artifact exists
  - approved-exit artifact for the round does not exist
  - no round artifacts for proposals/planner-positions/votes/arbiter exist
  - output/log contains `"skipping deliberation"`

2. `cap_boundary_force_completes_even_if_no_amendments_would_be_found`
- Setup:
  - same cap and pre-seeded restart count at boundary
  - backend script that would yield no accepted amendments if called
- Assert:
  - force-complete artifact exists
  - approved-exit artifact does not exist
  - confirms boundary behavior change

### Non-Goals
- No config default changes
- No CLI/UI changes
- No `write_force_complete_artifact` API changes
- No changes to restart counting semantics
- No preamble/control-flow restructuring before backend-resolution setup
