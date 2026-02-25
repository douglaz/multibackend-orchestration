Now I have all the context I need. Let me write the revised specification.

---

## Summary

Skip the entire final review deliberation pipeline (reviewer proposals, planner positions, voter rounds, arbiter) when the restart cap has already been reached. Currently, `restart_count` is computed at line 3329 but the cap check doesn't occur until line 3691—after up to 5 backend calls have already executed. These calls are wasted because their results are discarded when the cap triggers force-completion. The fix moves the cap check to immediately after line 3330, before any deliberation backend calls begin.

Note: the `resolve_effective_final_review_backends` call at lines 3309–3314 performs backend health checks (async I/O) and executes before `restart_count` is computed. Moving the cap check before backend resolution would require restructuring the function's control flow (reordering `restart_count` computation ahead of backend resolution and snapshot writes). This change intentionally does **not** move the cap check before backend resolution—the health checks are lightweight, and their results are needed if the cap hasn't been reached. The optimization targets the expensive deliberation calls (reviewer proposals, planner positions, voter rounds), not the setup preamble.

**Behavioral change**: When `restart_count == max_final_review_restarts`, the current code still runs the full deliberation pipeline and may exit via the APPROVED path (line 3671) if no amendments are accepted. After this change, the same scenario will always take the force-complete path. This is intentional—once the cap is reached, we should not spend backend calls on deliberation regardless of outcome. The resulting project state (`Completed` / `Completing`) is the same either way; only the exit artifact differs (`final-review-force-complete.md` vs `final-review-exit-approved.md`).

## Acceptance Criteria

- The restart cap check (`restart_count >= max_final_review_restarts`) executes immediately after `restart_count` and `round` are computed (after current line 3330), before the reviewer loop at line 3332.
- When the cap is reached, `write_force_complete_artifact` is called with an empty `BTreeSet` (no amendments were evaluated).
- State transitions to `ProjectStatus::Completed` / `Phase::Completing` / `phase_iteration = 1`.
- The function returns `Ok(Some((Phase::FinalReview, Phase::Completing)))`.
- Zero reviewer, planner, voter, or arbiter backend calls are made when the cap is reached.
- The existing late cap check at line 3691 is removed. After the early check, `restart_count < max_final_review_restarts` is guaranteed for all code that follows, making the late check dead code. The only remaining path to line 3691 is when `restart_count < cap`, where the check would never trigger.
- The log message clearly indicates the early force-completion path was taken (includes "skipping deliberation").
- When `restart_count == max_final_review_restarts` and deliberation would have produced no accepted amendments, the project now force-completes (writes `final-review-force-complete.md`) instead of exiting approved (writes `final-review-exit-approved.md`). Both lead to `Completed`/`Completing`; this is an intentional behavioral change that avoids wasted backend calls.

## Technical Approach

### 1. Insert early cap check (lines 3330–3332)

Insert a new block between lines 3330 and 3332 in `run_final_review_phase`:

```rust
// Line 3329-3330 (existing)
let restart_count = final_review_restart_count_from_artifacts(project_dir);
let round = restart_count.saturating_add(1);

// NEW: early cap check before any deliberation backend calls
if restart_count >= effective.workflow.max_final_review_restarts {
    write_force_complete_artifact(
        project_dir,
        round,
        restart_count,
        effective.workflow.max_final_review_restarts,
        &BTreeSet::new(),  // no amendments evaluated — deliberation skipped entirely
    )?;
    state.status = ProjectStatus::Completed;
    state.current_phase = Phase::Completing;
    state.phase_iteration = 1;
    logs.push(format!(
        "loop {loop_number}: final review restart cap already reached ({restart_count}/{}); skipping deliberation and force-completing",
        effective.workflow.max_final_review_restarts
    ));
    return Ok(Some((Phase::FinalReview, Phase::Completing)));
}

// Line 3332 (existing) — reviewer loop starts here
let mut reviewer_decisions: Vec<(String, FinalReviewerDecision)> = Vec::new();
```

### 2. Remove the late cap check (lines 3691–3707)

Delete the now-dead cap check block at lines 3691–3707:

```rust
// REMOVE this entire block — it is unreachable after the early check
if restart_count >= effective.workflow.max_final_review_restarts {
    write_force_complete_artifact(
        project_dir,
        round,
        restart_count,
        effective.workflow.max_final_review_restarts,
        &final_accepted,
    )?;
    state.status = ProjectStatus::Completed;
    state.current_phase = Phase::Completing;
    state.phase_iteration = 1;
    logs.push(format!(
        "loop {loop_number}: final review reached restart cap ({restart_count}/{}); force-completing project",
        effective.workflow.max_final_review_restarts
    ));
    return Ok(Some((Phase::FinalReview, Phase::Completing)));
}
```

After removal, the code at former line 3709 (`append_final_review_amendments_file`) follows directly from the `final_accepted.is_empty()` check at line 3671. At this point, `final_accepted` is non-empty (checked at 3671) and `restart_count < max` (guaranteed by the early check), so the restart path is always valid.

### Key design decisions

1. **Empty `BTreeSet` for accepted amendments**: Since the deliberation pipeline is skipped entirely, no amendments have been proposed or evaluated. The force-complete artifact will have an `## Accepted Amendments` section header with no list items beneath it, which accurately reflects that zero amendments were evaluated.

2. **Remove the late cap check**: `restart_count` is computed once at line 3329 via `final_review_restart_count_from_artifacts` (a filesystem scan) and is never modified within the function. After the early `if restart_count >= max` returns, all remaining code runs with `restart_count < max` guaranteed. Keeping the late check would be dead code that misleads readers into thinking `restart_count` could change mid-round.

3. **`BTreeSet::new()` requires no new import**: `BTreeSet` is already imported in this file (used at lines 3587, 3663).

4. **Log message distinguishes path**: The early path says "skipping deliberation and force-completing" to clearly distinguish it in logs from the normal completion paths.

5. **Intentional behavioral change at the boundary**: When `restart_count == max_final_review_restarts`, current code runs deliberation and may exit APPROVED if no amendments are accepted. The new code force-completes immediately. This is the correct trade-off: the cap exists precisely to limit backend calls, so spending 5 calls only to potentially discover "no amendments" defeats the purpose. The end state (`Completed`/`Completing`) is identical.

## Files & Modules

| File | Change |
|------|--------|
| `src/workflow/orchestrator.rs` | Insert early cap check block (~13 lines) between lines 3330 and 3332 in `run_final_review_phase`. Remove late cap check block (~17 lines) at lines 3691–3707. Net change: ~-4 lines. No changes to function signature, no new functions. |

No other files require changes. The `write_force_complete_artifact` function already accepts `&BTreeSet<String>` and handles the empty case correctly (writes the `## Accepted Amendments` header with no list items). No config, CLI, or state schema changes are needed.

## Testing Strategy

1. **New integration test: early cap force-completion** in `src/validate/tests_final_review.rs`. Create a mock backend script (can reuse the existing `restart_then_complete_script` pattern). Set `max_final_review_restarts = 1`. Pre-seed the project with one restart artifact file (`*-final-review-exit-restart.md`) in a loop directory so `restart_count` starts at 1. Run the orchestrator and assert:
   - Project completes (status = `completed`, phase = `completing`).
   - `final-review-force-complete.md` exists in the project directory.
   - The `## Accepted Amendments` section in `final-review-force-complete.md` contains no list items (no `- ` lines after the header), confirming zero amendments were evaluated.
   - **No deliberation artifacts exist**: assert absence of files matching `*-final-review-proposals-*`, `*-final-review-planner-positions*`, `*-final-review-votes-*`, and `*-final-review-arbiter-ruling*` in the loop directory for this round. This directly proves the deliberation pipeline was skipped.
   - The log/output contains "skipping deliberation and force-completing".

2. **New integration test: boundary behavioral change** in `src/validate/tests_final_review.rs`. Set `max_final_review_restarts = 1`. Pre-seed one restart artifact. Use a mock script that would return no amendments if called. Run the orchestrator and assert:
   - Project force-completes (not APPROVED) — `final-review-force-complete.md` exists.
   - `*-final-review-exit-approved.md` does **not** exist for this round.
   - This explicitly covers the behavioral edge case where the old code would have exited APPROVED but the new code force-completes.

3. **Existing tests remain passing**: The existing `restart_round_then_complete` test uses `max_final_review_restarts = 3` with a script that completes in round 2 (below the cap), so it exercises the normal deliberation path and is unaffected by this change.

## Out of Scope

- Changing the default value of `max_final_review_restarts` (remains 3)
- Modifying `write_force_complete_artifact` signature or behavior
- Moving the cap check before `resolve_effective_final_review_backends` (backend health checks are lightweight; restructuring control flow is not justified)
- Adding metrics/telemetry for skipped backend calls
- Changing how `final_review_restart_count_from_artifacts` counts restart artifacts
- UI or CLI changes to surface the early force-completion