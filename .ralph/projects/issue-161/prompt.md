## Summary

When a final review produces `AMENDMENTS` and transitions back to Planning (`Phase::FinalReview → Phase::Planning`), the planner may erroneously respond with a `CompletionRequest` instead of scheduling a new implementation loop to address the amendments. The orchestrator currently accepts this and creates another completion loop, which silently wastes the `max_final_review_restarts` budget or force-completes with unactioned amendments. This feature adds a guard in the Planning phase handler that detects this condition — using loop-scoped artifact evidence from the most recent completion attempt — and immediately fails the task with a descriptive error.

## Acceptance Criteria

- [ ] When the planner emits `PlannerDecision::CompletionRequest` and the most recent completion attempt's loop directory contains a `final-review-exit-restart.md` artifact (indicating that completion loop ended with amendments), the orchestrator returns `Err(RalphError::Orchestration(...))` immediately
- [ ] The error message clearly identifies the condition: `"planner requested completion without addressing final review amendments"`
- [ ] The guard fires immediately upon parsing the planner's completion request — before any completion backends are resolved or artifacts written
- [ ] Valid planner behavior is unaffected: emitting `Feature` after amendments works normally; emitting `CompletionRequest` when no prior completion attempt exists (or the most recent completion loop ended with approval, not a restart) proceeds to completion normally
- [ ] The guard uses loop-scoped artifact evidence (most recent completion attempt's loop directory), not global file/count heuristics, ensuring it does not false-trigger on stale historical artifacts from earlier runs or unrelated completion loops
- [ ] A conformance test covers the failure path; a regression test confirms valid completion still works when no restart artifact exists in the most recent completion loop
- [ ] The existing `restart_round_then_complete` test is updated so the planner emits a `Feature` spec (scheduling implementation) after amendments, reflecting the correct post-amendments workflow

## Technical Approach

Insert a guard at the top of the `PlannerDecision::CompletionRequest` branch in the `Phase::Planning` match arm (`orchestrator.rs:655`), before any completion backend resolution or artifact writes.

**Detection logic:** Check whether the most recent completion attempt ended with a final-review restart. Use `state.completion_attempts.last()` to retrieve the previous completion loop's `loop_number`. Then use the existing `resolve_artifact_path_by_suffix()` helper to check whether a `final-review-exit-restart.md` artifact exists in that loop's directory (`loops/<N:03>-completion/`). If it does, the most recent completion loop ended with amendments that the planner was expected to address via implementation — not by requesting completion again.

This approach is loop-scoped: it only considers the most recent completion attempt, not global state. If a project has historical completion loops that ended with restarts but were subsequently resolved via approved final reviews, those don't trigger the guard because only the *last* completion attempt is checked.

**Guard placement** (inside `PlannerDecision::CompletionRequest { body }` at line 655):

```rust
PlannerDecision::CompletionRequest { body } => {
    // Guard: fail if planner requests completion with unaddressed final review amendments.
    // Check whether the most recent completion loop ended with a final-review restart.
    if let Some(last_attempt) = state.completion_attempts.last() {
        let has_restart = resolve_artifact_path_by_suffix(
            &project_dir,
            last_attempt.loop_number,
            "completion",
            "final-review-exit-restart.md",
        )?
        .is_some();
        if has_restart {
            return Err(RalphError::Orchestration(
                "planner requested completion without addressing final review amendments"
                    .to_owned(),
            ));
        }
    }
    // ... existing completion logic continues
```

**Why `state.completion_attempts.last()` is sufficient:** After a final-review restart, the orchestrator transitions to `Phase::Planning`. The completion attempt that triggered the restart is the last entry in `state.completion_attempts`. Its loop directory contains the `final-review-exit-restart.md` artifact written by `write_final_review_exit_artifact()` at line 3901. If the planner then emits another `CompletionRequest`, the guard detects the restart artifact in the previous loop and fails.

**Why no `ProjectStatus::Failed` assignment:** The `ProjectStatus::Failed` variant exists but is never set by orchestrator code, and `reconstruct_project_state` rebuilds status from artifacts — it never produces `Failed`. Returning `Err(RalphError::Orchestration(...))` is the standard orchestrator error path: it propagates to the CLI entry point, sets a non-zero exit code, and prints the error to stderr. This is the correct observable for tests and callers. No new status variant or persistence mechanism is needed.

**Update to `restart_round_then_complete` test:** The existing test uses a mock planner that always returns `CompletionRequest`, including after amendments. This is exactly the behavior the new guard detects and rejects. The test must be updated so the mock planner returns a `Feature` spec (triggering an implementation loop) when amendments are pending, and returns `CompletionRequest` only when no amendments are pending. The mock script can use a counter or check for the presence of the `final-review-amendments-applied.md` file to switch behavior.

## Files & Modules

| File | Change |
|---|---|
| `src/workflow/orchestrator.rs` | Add guard at top of `PlannerDecision::CompletionRequest` branch (~10 lines) |
| `src/validate/tests_final_review.rs` | Update `restart_round_then_complete` mock: planner returns `Feature` after amendments, `CompletionRequest` otherwise |
| `src/validate/tests_final_review.rs` | Add conformance test `planner_completion_after_amendments_fails` |
| `src/validate/tests_final_review.rs` | Add regression test `completion_without_prior_amendments_succeeds` (or verify existing coverage) |

## Testing Strategy

**Conformance test:** `final_review::planner_completion_after_amendments_fails`

1. Initialize workspace with mock backends using a script where:
   - **Planner** (system prompt contains `"You are a software architect"`): Always returns `# Project Completion Request` regardless of context. The script must also handle planner-position prompts (`"You are a technical evaluator"`) by returning ACCEPT for any amendment IDs.
   - **Completer** (system prompt contains `"You are a project completion validator."`): Returns `COMPLETE`.
   - **QA** (system prompt contains `"You are a QA engineer"`): Returns `PASS`.
   - **Final reviewer** (system prompt contains `"You are a final reviewer"`): Always returns `AMENDMENTS` with at least one amendment item.
   - **Vote prompts** (system prompt contains `"You are a reviewer voting"`): Returns ACCEPT for all amendment IDs.
2. Enable final review, set `max_final_review_restarts` to 3.
3. Run `ralph run --until-complete`.
4. Assert: exit code is non-zero (the `Err` propagated).
5. Assert: stderr contains `"planner requested completion without addressing final review amendments"`.
6. Assert: exactly 1 completion attempt exists (the guard fired before a second completion loop was registered, so only the first completion attempt that ended with a restart exists).
7. Assert: `max_final_review_restarts` budget was not consumed (only 1 restart artifact exists, regardless of the budget of 3).

**Updated existing test:** `restart_round_then_complete`

Update the mock planner to behave correctly after amendments:
- **Round 1:** Planner returns `CompletionRequest` → completion → final review returns `AMENDMENTS` → restart → back to planning.
- **Round 2:** Planner checks for amendments file (e.g., via a call counter or file existence check) and returns a `Feature` spec → implementation → QA → completion → final review returns `NO AMENDMENTS` → approved → project complete.
- Assertions remain: `status == "completed"`, `completion_attempts.len() == 2`, amendments file contains `## Round 1`.

**Regression test:** `completion_without_prior_amendments_succeeds` (or verify existing test coverage)

Confirm that a planner returning `CompletionRequest` on the first pass (no prior completion attempts, no amendments) proceeds to completion normally. This may already be covered by existing tests (e.g., basic completion tests). If covered, document the test name. If not, add a minimal test:
1. Mock planner returns `CompletionRequest`, completer returns `COMPLETE`, QA returns `PASS`, final reviewer returns `NO AMENDMENTS`.
2. Assert: `status == "completed"`, exit code 0.

## Out of Scope

- Cleaning up or deleting the `final-review-amendments-applied.md` file after successful completion (it is an audit artifact)
- Changing the `max_final_review_restarts` force-complete behavior — that remains as a separate backstop
- Modifying the planner prompt to more strongly discourage completion requests after amendments (prompt engineering is separate)
- Adding retry/recovery logic for this failure — it is a hard fail indicating a planner behavioral issue
- Introducing a new `ProjectStatus::Failed` persistence mechanism or failure artifact — the standard `Err` return and non-zero exit code are sufficient observables