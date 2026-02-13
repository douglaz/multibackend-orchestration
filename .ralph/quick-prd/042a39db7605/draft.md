I now have comprehensive understanding of the codebase. Let me write the specification.

---

## Summary

Extend the acceptance gate to run QA with **every registered backend** (currently two: `claude` and `codex`) before approving project completion. Today the acceptance gate invokes a single QA agent on one backend. This feature adds a serial loop over all backends so each independently evaluates the project. If **any** QA fails, the whole completion attempt is rejected: the verdict is forced to `Continue`, the planner re-enters the loop, implementation proceeds, and eventually all QAs must pass again on the next completion attempt.

## Acceptance Criteria

1. When the completer returns `Complete` and `qa_enabled` is true, the orchestrator runs acceptance QA on **both** backends (the completer's backend and its opposite), serially.
2. Each QA backend produces its own artifact (`AcceptancePass` or `AcceptanceFail`) stored in the completion loop directory.
3. Each QA result is recorded as a separate entry in `completion.artifacts.acceptance_results` (one `AcceptanceQaResult` per backend).
4. The second QA runs **regardless of whether the first passed or failed** — it must form its own independent opinion. Its prompt must **not** include the first QA's verdict or report.
5. The project is marked `Completed` only if **all** acceptance QA results passed. If **any** QA fails, the completion verdict is overridden to `Continue` and the orchestrator returns to `Phase::Planning`.
6. Log messages identify which backend(s) passed and which failed.
7. The existing `upsert_acceptance_result` and `acceptance_all_required_passed` / `acceptance_any_required_failed` helpers on `CompletionLoopArtifacts` are used for the pass/fail aggregation logic.
8. On retry (after a forced `Continue`), the next completion attempt starts fresh — all QAs run again from scratch on the new attempt.

## Technical Approach

### 1. Determine the list of acceptance QA backends

In the `CompletionVerdict::Complete` branch of `Phase::Completing` (orchestrator.rs ~line 1349), build a list of backend specs to run acceptance QA against:

```rust
let primary_qa_backend = /* existing logic: role_overrides.qa or completer_backend */;
let opposite_qa_backend = registry.opposite(&primary_qa_backend)?;
let acceptance_backends = vec![primary_qa_backend, opposite_qa_backend.to_owned()];
```

If a `qa` role override is set, it replaces the primary but the opposite is still derived from the completer's backend (not the override), preserving the intent of getting a second opinion from a different backend family.

### 2. Loop over backends serially

Replace the current single-QA block (lines ~1351–1486) with a `for` loop:

```rust
for acceptance_qa_backend_name in &acceptance_backends {
    let acceptance_qa_backend = registry.get_or_create_for_spec(acceptance_qa_backend_name)?;
    // ... set tmux context, build prompt, execute, store result (same as today per iteration)
}
```

Each iteration:
- Calls `build_acceptance_prompt` with the current backend's name (the `backend` parameter) and the planner backend as `opposite_backend`.
- Calls `execute_with_parse_retries` to get `QaDecision`.
- Writes the artifact (`AcceptancePass` or `AcceptanceFail`).
- Calls `completion.artifacts.upsert_acceptance_result(...)`.

The prompt given to each QA backend must **not** contain the other backend's result — each QA is independent.

### 3. Aggregate results after the loop

After all QAs run, determine the overall verdict:

```rust
let required_backends: Vec<&str> = acceptance_backends.iter().map(|s| s.as_str()).collect();
let completion = state.current_completion_attempt_mut()?;
if completion.artifacts.acceptance_all_required_passed(&required_backends) {
    state.status = ProjectStatus::Completed;
    // ... log "all acceptance QAs passed; project finished"
} else {
    completion.verdict = Some(CompletionVerdict::Continue);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::Planning;
    // ... log which backend(s) failed
}
```

This uses the existing `acceptance_all_required_passed` helper which is currently defined but unused.

### 4. No prompt contamination between QAs

The `build_acceptance_prompt` function already takes only the current QA's backend name and the planner backend name. Since each QA's artifact is written *after* its execution and the prompt is built *before*, there is no channel for the second QA to see the first QA's output. No changes to the prompt builder are needed.

### 5. Logging

Add per-backend log lines:
- `"loop {loop_number}: acceptance QA ({backend}): PASS"` or `"... FAIL"`
- Final aggregation log: `"loop {loop_number}: acceptance QA — all passed; project finished"` or `"loop {loop_number}: acceptance QA — {failed_list} failed; forcing CONTINUE"`.

## Files & Modules

| File | Change |
|------|--------|
| `src/workflow/orchestrator.rs` | **Primary change.** Replace single acceptance QA call (~lines 1349–1486) with a loop over both backends. Add aggregation logic using `acceptance_all_required_passed`. Update log messages. |
| `src/backend/mod.rs` | No code changes needed. `opposite()` and `resolve_backend_for_role()` already exist. |
| `src/project/state.rs` | No structural changes needed. `AcceptanceQaResult`, `upsert_acceptance_result`, `acceptance_all_required_passed`, and `acceptance_any_required_failed` already support multiple results. |
| `src/validate/tests_qa.rs` | **Update existing tests** and **add new test cases** (see Testing Strategy). |

## Testing Strategy

### Update existing conformance tests

1. **`qa::acceptance_gate_pass`** — Update assertion from `acceptance_results.len() == 1` to `acceptance_results.len() == 2`. Verify both entries have `passed: true` and different `backend` values.

2. **`qa::acceptance_gate_fail_forces_continue`** — Update to assert that the failed attempt has 2 acceptance results (one fail, one pass or both fail depending on mock script). Verify that a single failure among two QAs forces `Continue`.

### New conformance tests

3. **`qa::acceptance_gate_multi_backend_one_fails`** — Configure mock so backend A passes and backend B fails. Assert:
   - `acceptance_results.len() == 2`
   - One result has `passed: true`, the other `passed: false`
   - Completion verdict is forced to `continue`
   - Project returns to planning

4. **`qa::acceptance_gate_multi_backend_independent`** — Verify that both QA backends are invoked even when the first one fails. Both artifacts must exist. The second QA's artifact must not reference the first QA's decision (content check).

### Mock script changes

The mock scripts need to differentiate which backend is calling. Since acceptance QA artifacts include the backend name, the mock script can use the `$RALPH_BACKEND` environment variable (if available) or a counter-based approach where the script is invoked twice per completion attempt during the acceptance phase.

## Out of Scope

- **Parallel QA execution.** The spec explicitly states serial execution is acceptable for simplicity. Parallelism can be added later.
- **More than two backends.** While the loop naturally supports N backends, only the two currently registered backends (claude, codex) are used. No dynamic backend discovery or configuration for acceptance backend lists.
- **Configurable acceptance backend list.** No new config option to select which backends participate in acceptance QA. It always uses both.
- **Cross-QA feedback.** QAs do not see each other's results. No "second opinion aware" prompting.
- **Feature-loop (non-acceptance) QA changes.** The per-feature QA phase remains single-backend.
- **Acceptance QA retry/iteration limits.** The acceptance QA does not have its own iteration loop — if it fails, the entire completion attempt is rejected and the planner starts fresh.