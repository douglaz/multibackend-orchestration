Now I have everything I need. Here's the revised specification:

---

## Summary

Extend the acceptance gate to run QA with **both supported backend families** (`claude` and `codex`) before approving project completion. Today the acceptance gate invokes a single QA agent on one backend. This feature adds a serial loop over both backend families so each independently evaluates the project. If **any** QA fails, the whole completion attempt is rejected: the verdict is forced to `Continue`, the planner re-enters the loop, implementation proceeds, and eventually all QAs must pass again on the next completion attempt.

## Acceptance Criteria

1. When the completer returns `Complete` and `qa_enabled` is true, the orchestrator runs acceptance QA on **both backend families** (claude and codex), serially.
2. Each QA backend produces its own artifact (`AcceptancePass` or `AcceptanceFail`) stored in the completion loop directory.
3. Each QA result is recorded as a separate entry in `completion.artifacts.acceptance_results` (one `AcceptanceQaResult` per backend family). The two entries always have distinct `backend` values.
4. The second QA runs **regardless of whether the first passed or failed** — it must form its own independent opinion. Its prompt must **not** include the first QA's verdict, report, or acceptance result metadata.
5. The project is marked `Completed` only if **all** acceptance QA results passed. If **any** QA fails, the completion verdict is overridden to `Continue` and the orchestrator returns to `Phase::Planning`.
6. Log messages identify which backend(s) passed and which failed.
7. The existing `upsert_acceptance_result` and `acceptance_all_required_passed` / `acceptance_any_required_failed` helpers on `CompletionLoopArtifacts` are used for the pass/fail aggregation logic.
8. On retry (after a forced `Continue`), the next completion attempt starts fresh — all QAs run again from scratch on the new attempt.
9. When acceptance QA fails and the planner receives feedback, the feedback context includes **all** failing acceptance QA artifacts (not just the first one found).
10. Both acceptance QA backends use QA-role model resolution (`resolve_backend_for_role(backend, "qa")`) so that configured QA-specific models are applied consistently.

## Technical Approach

### 1. Determine the list of acceptance QA backends

In the `CompletionVerdict::Complete` branch of `Phase::Completing` (orchestrator.rs ~line 1349), build a deduplicated list of both backend families:

```rust
let completer_family = registry.opposite(
    &registry.opposite(&completer_backend_name)?  // identity round-trip to normalize
)?..; // or just parse the family directly
// Always run both families:
let acceptance_families: Vec<String> = vec!["claude".to_owned(), "codex".to_owned()];
let acceptance_backends: Vec<String> = acceptance_families
    .iter()
    .map(|family| registry.resolve_backend_for_role(family, "qa"))
    .collect();
```

**Why this approach (addresses Review Issue #1 — backend selection semantics):** The previous approach derived backends from `role_overrides.qa` and `registry.opposite()`, which could produce duplicates. For example, if the completer is `claude` and `workflow.qa_backend` is set to `codex`, then `opposite(completer) = codex`, yielding `[codex, codex]` — the `claude` family never runs QA. Instead, the list is always hardcoded to both families (`claude` and `codex`), and each family's QA-role model is resolved via `resolve_backend_for_role`. The `role_overrides.qa` field is **not consulted** for acceptance QA backend selection — it continues to apply only to feature-loop QA. This guarantees two distinct backend families always participate in acceptance QA.

### 2. Snapshot state before QA loop (addresses Review Issue #2 — prompt contamination)

The `build_acceptance_prompt` function serializes the full `ProjectState` as JSON into the prompt (line 2023). If the first QA's `AcceptanceQaResult` is upserted into `state` before the second QA's prompt is built, the second QA would see the first's verdict in the `acceptance_results` array within `state_json`, violating criterion #4.

**Solution:** Snapshot the state JSON **once** before the QA loop begins and pass it to all QA invocations:

```rust
let state_snapshot_json = serde_json::to_string_pretty(&state).unwrap_or_default();
for acceptance_qa_backend_name in &acceptance_backends {
    let acceptance_prompt = build_acceptance_prompt_with_snapshot(
        &state_snapshot_json,   // pre-loop snapshot, not live state
        &prompt_content,
        acceptance_qa_backend.name(),
        &planner_backend_name,
        &completed_feature_summary,
        &git_diff_against_base,
    );
    // ... execute, write artifact, upsert result into live state
}
```

Modify `build_acceptance_prompt` to accept a pre-serialized `state_json: &str` parameter instead of serializing `&ProjectState` internally. This is a minimal change: replace the `state: &ProjectState` parameter with `state_json: &str` and remove the internal `serde_json::to_string_pretty(state)` call.

### 3. QA-role model resolution for both backends (addresses Review Issue #3)

Both backends in `acceptance_backends` are resolved via `resolve_backend_for_role(family, "qa")`. This ensures that if the user configures `backends.claude.models.qa = "sonnet"` or `backends.codex.models.qa = "gpt-5.3-codex-medium"`, the acceptance QA correctly uses those models rather than the bare family defaults. The previous spec only applied `resolve_backend_for_role` to the primary QA backend (via the `role_overrides.qa` path) but not to the opposite backend.

### 4. Loop over backends serially

Replace the current single-QA block (lines ~1351–1486) with a `for` loop:

```rust
// Snapshot state before any QA mutations
let state_snapshot_json = serde_json::to_string_pretty(&state).unwrap_or_default();
let completed_feature_summary = collect_completed_feature_loop_summary(&state)?;
let git_diff_against_base = current_git_diff_against_base(
    &self.workspace.root,
    &effective.global.git.base_branch,
)?;

for acceptance_qa_backend_name in &acceptance_backends {
    let acceptance_qa_backend =
        registry.get_or_create_for_spec(acceptance_qa_backend_name)?;

    let acceptance_prompt = build_acceptance_prompt(
        &state_snapshot_json,   // frozen snapshot
        &prompt_content,
        acceptance_qa_backend.name(),
        &planner_backend_name,
        &completed_feature_summary,
        &git_diff_against_base,
    );

    registry.set_tmux_context(TmuxExecutionContext {
        loop_number: Some(loop_number),
        role: Some("qa".to_owned()),
    }).await;

    info!(loop = loop_number, backend = acceptance_qa_backend.name(),
          "invoking acceptance QA...");

    let acceptance_decision = execute_with_parse_retries(
        acceptance_qa_backend,
        &registry,
        "qa",
        "completing",
        &acceptance_prompt,
        parse_qa_output,
        &expected_format_template_for("qa", None),
    ).await?;

    // Write artifact and upsert result (same as today, per iteration)
    match acceptance_decision {
        QaDecision::Pass { body } => {
            let path = write_artifact(/* ... AcceptancePass ... */)?;
            let rel = artifact_relative_path(&project_dir, &path);
            let completion = state.current_completion_attempt_mut().ok_or_else(/* ... */)?;
            completion.artifacts.upsert_acceptance_result(AcceptanceQaResult {
                backend: acceptance_qa_backend_name.clone(),
                passed: true,
                artifact: rel,
            });
            info!(loop = loop_number, backend = acceptance_qa_backend_name,
                  "acceptance QA: PASS");
        }
        QaDecision::Fail { body } => {
            let path = write_artifact(/* ... AcceptanceFail ... */)?;
            let rel = artifact_relative_path(&project_dir, &path);
            let completion = state.current_completion_attempt_mut().ok_or_else(/* ... */)?;
            completion.artifacts.upsert_acceptance_result(AcceptanceQaResult {
                backend: acceptance_qa_backend_name.clone(),
                passed: false,
                artifact: rel,
            });
            info!(loop = loop_number, backend = acceptance_qa_backend_name,
                  "acceptance QA: FAIL");
        }
    }
}
```

### 5. Aggregate results after the loop

After all QAs run, determine the overall verdict:

```rust
let required_backends: Vec<&str> = acceptance_backends.iter().map(|s| s.as_str()).collect();
let completion = state.current_completion_attempt_mut()?;
if completion.artifacts.acceptance_all_required_passed(&required_backends) {
    state.status = ProjectStatus::Completed;
    state.current_phase = Phase::Completing;
    state.phase_iteration = 1;
    logs.push(format!(
        "loop {loop_number}: acceptance QA — all passed; project finished"
    ));
} else {
    let failed: Vec<_> = completion.artifacts.acceptance_results.iter()
        .filter(|r| !r.passed)
        .map(|r| r.backend.as_str())
        .collect();
    completion.verdict = Some(CompletionVerdict::Continue);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::Planning;
    state.phase_iteration = 1;
    logs.push(format!(
        "loop {loop_number}: acceptance QA — {} failed; forcing CONTINUE",
        failed.join(", ")
    ));
}
```

This uses the existing `acceptance_all_required_passed` helper which is currently defined but unused.

### 6. Planner feedback includes all failing QA artifacts (addresses Review Issue #4)

Update `latest_completion_feedback_context` to collect **all** failing acceptance results, not just the first one found via `.find()`:

```rust
// Before (only first failure):
let Some(acceptance_fail_rel) = completion.artifacts.acceptance_results.iter()
    .find(|result| !result.passed)
    .map(|result| result.artifact.as_str())

// After (all failures):
let failed_results: Vec<_> = completion.artifacts.acceptance_results.iter()
    .filter(|result| !result.passed)
    .collect();
if failed_results.is_empty() {
    return Ok(None);
}
let mut sections = vec![format!(
    "### Completer Verdict Artifact\n\n{completer_verdict_content}"
)];
for (i, result) in failed_results.iter().enumerate() {
    let fail_content = read_project_relative_file(project_dir, &result.artifact)?;
    sections.push(format!(
        "### Acceptance QA Failure — {} (backend: {})\n\n{fail_content}",
        i + 1, result.backend
    ));
}
Ok(Some(sections.join("\n\n")))
```

This ensures the planner sees feedback from every failing QA backend, not just whichever happens to be first in the vector.

### 7. No prompt contamination between QAs (reinforced)

Independence is guaranteed by two mechanisms:
1. **State snapshot** (section 2): The `state_json` embedded in the prompt is frozen before the loop, so upserted `AcceptanceQaResult` entries from earlier QAs are not visible to later ones.
2. **Prompt construction**: `build_acceptance_prompt` only includes the current QA's backend name and the planner backend name. No cross-reference to other QA results.

## Files & Modules

| File | Change |
|------|--------|
| `src/workflow/orchestrator.rs` | **Primary change.** Replace single acceptance QA call (~lines 1349–1486) with a loop over both backend families. Snapshot state before loop. Add aggregation logic using `acceptance_all_required_passed`. Update `build_acceptance_prompt` signature to accept `state_json: &str` instead of `state: &ProjectState`. Update `latest_completion_feedback_context` to collect all failing acceptance artifacts. Update log messages to per-backend and aggregate format. |
| `src/backend/mod.rs` | No code changes needed. `opposite()`, `resolve_backend_for_role()`, and `get_or_create_for_spec()` already exist and support this feature. |
| `src/project/state.rs` | No structural changes needed. `AcceptanceQaResult`, `upsert_acceptance_result`, `acceptance_all_required_passed`, and `acceptance_any_required_failed` already support multiple results with distinct backend keys. |
| `src/validate/tests_qa.rs` | **Update existing tests** and **add new test cases** (see Testing Strategy). |

## Testing Strategy

### Update existing conformance tests

1. **`qa::acceptance_gate_pass`** — Update assertion from `acceptance_results.len() == 1` to `acceptance_results.len() == 2`. Verify both entries have `passed: true` and different `backend` values (one containing "claude", the other "codex").

2. **`qa::acceptance_gate_fail_forces_continue`** — Update to assert that the failed attempt has 2 acceptance results. Verify that a single failure among two QAs forces `Continue`. Assert both backend families appear in results.

### New conformance tests

3. **`qa::acceptance_gate_multi_backend_one_fails`** — Configure mock so one backend family passes and the other fails. Assert:
   - `acceptance_results.len() == 2`
   - One result has `passed: true`, the other `passed: false`
   - The two results have different `backend` values
   - Completion verdict is forced to `Continue`
   - Project returns to planning

4. **`qa::acceptance_gate_multi_backend_independent`** — Verify that both QA backends are invoked even when the first one fails, and that the second QA's prompt does not contain the first QA's verdict data. **Implementation approach (addresses Review Issue #5):** The mock script should inspect the incoming prompt (via `$INPUT`) and **fail with a distinctive error** if the prompt contains acceptance result metadata (e.g., `grep -q '"acceptance_results"' <<< "$INPUT" && grep -q '"passed"' <<< "$INPUT"`). If the second QA's prompt were contaminated, the mock would detect this and the test would fail. Additionally, assert both artifacts exist and both acceptance results are recorded.

5. **`qa::acceptance_gate_qa_backend_override_no_duplicate` (addresses Review Issue #6)** — Set `workflow.qa_backend` to a same-family override (e.g., `codex`) when completer is also `codex`. Assert that acceptance QA still runs on both distinct families (`claude` and `codex`), producing `acceptance_results.len() == 2` with distinct backend values. This verifies that the `qa_backend` override does not influence acceptance QA backend selection and cannot cause duplicate entries.

6. **`qa::acceptance_gate_qa_backend_override_opposite_family` (addresses Review Issue #6)** — Set `workflow.qa_backend` to the opposite family. Assert acceptance QA still runs both families and produces two distinct results. Verifies the override is correctly scoped to feature-loop QA only.

7. **`qa::acceptance_gate_all_feedback_on_failure`** — Configure mock so both QA backends fail. After the forced `Continue`, trigger a new planning phase. Assert that the planner prompt feedback context includes **both** failing acceptance QA artifacts (not just one). Verify by checking the prompt content passed to the planner mock for both backend failure sections.

### Mock script changes

The mock scripts need to differentiate which backend is calling for tests where one backend passes and the other fails. Since both `claude` and `codex` backends are configured with the same mock script, the script can differentiate by inspecting the prompt content for the `QA Backend:` line that `build_acceptance_prompt` embeds (line 2073: `- QA Backend: {backend}`). Example:

```bash
if echo "$INPUT" | grep -q "QA Backend:.*claude"; then
    # Claude QA behavior (e.g., pass)
elif echo "$INPUT" | grep -q "QA Backend:.*codex"; then
    # Codex QA behavior (e.g., fail)
fi
```

This avoids relying on a `$RALPH_BACKEND` environment variable that does not currently exist in the test infrastructure.

## Out of Scope

- **Parallel QA execution.** The spec explicitly states serial execution is acceptable for simplicity. Parallelism can be added later.
- **More than two backend families.** While the loop naturally supports N backends, only the two currently supported backend families (`claude` and `codex`) are used. The hardcoded list `["claude", "codex"]` is intentional — there is no dynamic backend discovery or registry enumeration, as the registry may also contain model-injected backend specs (e.g., `claude(opus)`, `codex(gpt-5.3-codex-high)`) that are not distinct families.
- **Configurable acceptance backend list.** No new config option to select which families participate in acceptance QA. It always uses both families.
- **`workflow.qa_backend` affecting acceptance QA.** The QA role override continues to apply only to feature-loop QA. Acceptance QA always runs both families with QA-role model resolution. This is a deliberate scoping decision.
- **Cross-QA feedback.** QAs do not see each other's results. No "second opinion aware" prompting.
- **Feature-loop (non-acceptance) QA changes.** The per-feature QA phase remains single-backend.
- **Acceptance QA retry/iteration limits.** The acceptance QA does not have its own iteration loop — if it fails, the entire completion attempt is rejected and the planner starts fresh.