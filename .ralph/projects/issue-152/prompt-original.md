## Summary

When the orchestrator resumes an in-progress feature loop, completion loop, or final review phase, it reads backend specs from reconstructed state (`loop_state.backends.*` / `completion.backends.*`), which were populated from artifact frontmatter during `reconstruct_project_state`. The `is_backend_available()` check only validates the backend family (e.g. `codex`), not the specific model, so changing the model in `ralph.toml` between runs leaves stale models in use. The completion and final-review paths have no availability check at all — they read reconstructed values directly.

The fix: on every resume, unconditionally re-resolve all backends from the current config. The reconstructed values in `FeatureLoopBackends` / `CompletionLoopBackends` remain populated for provenance (they record what backends produced existing artifacts) but are never used to decide which backend to invoke. Session cleanup requires no changes — `SessionStore::default()` in `reconstruct_project_state` already gives an empty store on every restart.

## Acceptance Criteria

1. On resume of a feature loop, the orchestrator resolves backends from the current config via `assign_feature_backends()` instead of reading `loop_state.backends`. This covers all four feature-loop roles: `planner`, `implementer`, `reviewer`, and `qa`.
2. On resume of a completion loop (`Phase::Completing`), the orchestrator re-resolves the `planner` via `assign_completion_backends()` (for correct alternation parity) and the `completers` panel via `resolve_completion_panel()` (for optional-backend handling and `completion_min_completers` enforcement). `resolve_effective_config` rejects empty `completion_backends` at config validation time, so no empty-list fallback is needed.
3. On resume in `Phase::FinalReview`, the orchestrator re-resolves the planner via `assign_completion_backends()` instead of reading `completion.backends.planner`.
4. `FeatureLoopBackends` and `CompletionLoopBackends` remain populated during reconstruction for provenance/audit but are never used as the source of truth for backend selection on resume.
5. Session cleanup requires no code changes — `reconstruct_project_state` builds a fresh `ProjectState` with `SessionStore::default()` (empty records).
6. When a re-resolved backend differs from its reconstructed value, a `warn!` log is emitted showing old and new specs, role name, and loop number.
7. Resume continues from the current phase with the new model — no phase restart or artifact invalidation.
8. This applies only on restart/resume, not mid-run. The completion-phase re-resolution is gated on resume to avoid re-running health checks on same-run entries where availability was already validated at creation time.
9. The quick-dev orchestrator is unaffected — it already resolves backends fresh each run.

## Technical Approach

### Feature-loop backend re-resolution (3 phases)

In `src/workflow/orchestrator.rs`, three phase arms read backends from `loop_state.backends.*` then conditionally recalculate via `is_backend_available()`. Replace each with unconditional `assign_feature_backends()`.

`assign_feature_backends()` is a pure computation (loop number + config → backend names) with no I/O or health checks, so calling it unconditionally on every phase entry — including same-run re-entries — is safe and consistent with AC8's intent. The cost is negligible and the simplicity avoids the need for resume-detection logic in these phases.

**Implementing phase** (lines 740–777): Currently reads `loop_state.backends.planner` (line 752) and `loop_state.backends.implementer` (line 753), then conditionally recalculates at line 762. Replace with:

```rust
let recalc = registry.assign_feature_backends(
    loop_number,
    &effective.workflow.starting_backend,
    &role_overrides,
)?;
if recalc.implementer != implementer_backend_name {
    warn!(
        original = %implementer_backend_name,
        resolved = %recalc.implementer,
        loop_number,
        "implementer backend drift detected, using current config"
    );
}
if recalc.planner != planner_backend_name {
    warn!(
        original = %planner_backend_name,
        resolved = %recalc.planner,
        loop_number,
        "planner backend drift detected, using current config"
    );
}
let implementer_backend_name = recalc.implementer;
let planner_backend_name = recalc.planner;
```

Remove the `is_backend_available` conditional block (lines 761–777). The reconstructed values are still read for the drift comparison only.

**QA phase** (lines 1268–1326): Same pattern. Reads `loop_state.backends.qa` (line 1273) and `loop_state.backends.planner` (line 1272). Replace the `is_backend_available` block (lines 1310–1326) with unconditional `assign_feature_backends()`. Use `recalc.qa` and `recalc.planner`. Log drift warnings for both roles if values differ.

**Reviewing phase** (lines 1547–1575): Same pattern. Reads `loop_state.backends.reviewer` (line 1552) and `loop_state.backends.planner` (line 1551). Replace the `is_backend_available` block (lines 1559–1575) with unconditional `assign_feature_backends()`. Use `recalc.reviewer` and `recalc.planner`. Log drift warnings.

In all three phases, the re-resolved values from `assign_feature_backends()` are what get passed to `get_or_create_for_role()`.

### Completion loop backend re-resolution on resume

In `Phase::Completing` (lines 1828–1866), replace the planner and completers reads from `completion.backends.*` (lines 1845–1846) with re-resolution, **gated on resume** to avoid re-running `resolve_completion_panel()` health checks on same-run entries.

**Resume detection**: The completion phase is entered on the same run immediately after `register_completion_attempt()` (lines 684–701), which already calls `resolve_completion_panel()` and validates availability. On resume, the state is reconstructed from artifacts and the phase is entered without prior validation in the current process. The distinction is detectable by checking whether the completion attempt's backends were already resolved in this process — specifically, by tracking whether the current `Phase::Completing` entry is the first iteration after state reconstruction.

The simplest resume gate: compare `phase_at_step_start` (captured at line 549) against the current phase. On resume, `phase_at_step_start == Phase::Completing` because state reconstruction preserved that phase. On same-run transition, `phase_at_step_start == Phase::Planning` (or another prior phase) because the phase was just set to `Completing` by the planning arm's completion-request handling. However, this heuristic can fail if a review-cycle loops back. A more robust approach: always re-resolve, but for completers specifically, only call `resolve_completion_panel()` on the **first entry** into the completing phase per process — i.e., when the reconstructed completers list is non-empty (indicating it was populated from artifact frontmatter, not from a same-run `register_completion_attempt()` call which leaves completers populated from `resolve_completion_panel()` already).

Concretely:

**Planner**: Always call `registry.assign_completion_backends(completion.loop_number, &effective.workflow.starting_backend, &role_overrides)` to get the parity-correct planner. This is a pure computation like `assign_feature_backends()` — no health checks, safe to call unconditionally. If it differs from the reconstructed `completion.backends.planner`, log a drift warning.

**Completers**: Call `registry.resolve_completion_panel(&effective.workflow.completion_backends, effective.workflow.completion_min_completers)` **only when the reconstructed completers list is non-empty** — a non-empty list means it was populated from artifact frontmatter during reconstruction (i.e., this is a resume). When the list is empty, this is either a same-run entry (completers not yet populated) or a resume where the process stopped before any completer artifacts were written; in either case, the existing empty-completers fallback at lines 1855–1866 already calls `resolve_completion_panel()`, which is correct. This preserves the existing behavior for same-run entries (no redundant health checks) while ensuring resume entries get fresh resolution.

Replace the existing empty-completers-only fallback block (lines 1854–1866) with:

```rust
let (planner_backend_name, effective_completers) = {
    let completion = state.current_completion_attempt().ok_or_else(|| {
        RalphError::Orchestration(
            "current phase is completing but no completion attempt exists".to_owned(),
        )
    })?;

    let recalc = registry.assign_completion_backends(
        completion.loop_number,
        &effective.workflow.starting_backend,
        &role_overrides,
    )?;
    let reconstructed_planner = completion.backends.planner.clone();
    if recalc.planner != reconstructed_planner {
        warn!(
            original = %reconstructed_planner,
            resolved = %recalc.planner,
            loop_number = completion.loop_number,
            "completion planner backend drift detected, using current config"
        );
    }

    let reconstructed_completers = completion.backends.completers.clone();
    let effective_completers = if reconstructed_completers.is_empty() {
        // Empty list: either same-run entry (not yet populated) or resume
        // where process stopped before completer artifacts. In both cases,
        // resolve fresh — this matches the existing fallback behavior.
        registry
            .resolve_completion_panel(
                &effective.workflow.completion_backends,
                effective.workflow.completion_min_completers,
            )
            .await?
    } else {
        // Non-empty list from reconstruction: this is a resume with
        // prior completer artifacts. Re-resolve from current config.
        let re_resolved = registry
            .resolve_completion_panel(
                &effective.workflow.completion_backends,
                effective.workflow.completion_min_completers,
            )
            .await?;
        if re_resolved != reconstructed_completers {
            warn!(
                original = ?reconstructed_completers,
                resolved = ?re_resolved,
                loop_number = completion.loop_number,
                "completion completers drift detected, using current config"
            );
        }
        re_resolved
    };

    (recalc.planner, effective_completers)
};
```

Wait — on re-examination, the non-empty-completers case also calls `resolve_completion_panel()`, making the gate moot. The real concern from review issue #2 is that `resolve_completion_panel()` performs health checks that could cause hard failures on same-run entries where the backend was healthy at creation time but might transiently fail the check.

The correct approach: on same-run entries into `Phase::Completing`, the completers were already resolved by `register_completion_attempt()` moments earlier in the same process — they are stored in `completion.backends.completers` and are not stale. On resume, they may be stale. The distinction:

- **Same-run**: `completion.backends.completers` was populated by `register_completion_attempt()` in the current process. The list is non-empty and fresh.  
- **Resume with prior completers**: `completion.backends.completers` was populated from artifact frontmatter during reconstruction. The list is non-empty but potentially stale.
- **Resume without prior completers**: `completion.backends.completers` is empty (reconstructed from state with no completer artifacts yet). Needs resolution.

Cases 2 and 3 are indistinguishable from same-run entries by list content alone. The robust resume gate: track whether the orchestrator is on its **first outer-loop iteration** after state reconstruction. Add a `is_first_iteration` boolean, set to `true` before the loop, set to `false` at the end of each iteration. On the first iteration, the state was just reconstructed — this is the resume entry. On subsequent iterations, backends were resolved in-process.

Simplify: add a `let mut is_resumed_state = true;` before the outer loop (line ~520). At the bottom of the loop body (after the phase match), set `is_resumed_state = false;`. In the `Phase::Completing` arm, gate the `resolve_completion_panel()` call on `is_resumed_state`:

```rust
let effective_completers = if is_resumed_state {
    // Resume: re-resolve completers from current config
    let re_resolved = registry
        .resolve_completion_panel(
            &effective.workflow.completion_backends,
            effective.workflow.completion_min_completers,
        )
        .await?;
    let reconstructed_completers = completion.backends.completers.clone();
    if !reconstructed_completers.is_empty() && re_resolved != reconstructed_completers {
        warn!(
            original = ?reconstructed_completers,
            resolved = ?re_resolved,
            loop_number = completion.loop_number,
            "completion completers drift detected, using current config"
        );
    }
    re_resolved
} else {
    // Same-run: completers were resolved at creation time, use as-is
    completion.backends.completers.clone()
};
```

This cleanly separates resume from same-run and avoids redundant health checks. The planner re-resolution via `assign_completion_backends()` remains unconditional since it's a pure computation.

Remove the existing empty-completers-only fallback block (lines 1854–1866) and the `completion.backends.completers` persist-back (lines 1863–1864) since the re-resolved values are used directly and state mutation for completers is no longer needed.

`resolve_effective_config` rejects empty `completion_backends` at config validation time (src/config/mod.rs:793), so no empty-list fallback from `assign_completion_backends()` is needed. If `completion_backends` is empty, the orchestrator never reaches this point — config validation fails first.

### Final review planner re-resolution

In `run_final_review_phase` (line 3404), `planner_backend_name` is read from `completion.backends.planner.clone()`. This is a stale-backend path.

The simplest approach: resolve the planner at the **call site** (line 2303–2314 in the `Phase::FinalReview` match arm) before calling `run_final_review_phase`, then pass it as a parameter. At the call site, `effective`, `registry`, `role_overrides`, and `state` are all in scope. The completion's `loop_number` is available from `state.current_completion_attempt()`.

Add a `planner_backend: &str` parameter to `run_final_review_phase` (line 3381). At the call site:

```rust
Phase::FinalReview => {
    let completion = state.current_completion_attempt().ok_or_else(|| {
        RalphError::Orchestration(
            "current phase is final_review but no completion attempt exists".to_owned(),
        )
    })?;
    let recalc = registry.assign_completion_backends(
        completion.loop_number,
        &effective.workflow.starting_backend,
        &role_overrides,
    )?;
    let reconstructed_planner = completion.backends.planner.clone();
    if recalc.planner != reconstructed_planner {
        warn!(
            original = %reconstructed_planner,
            resolved = %recalc.planner,
            loop_number = completion.loop_number,
            "final review planner backend drift detected, using current config"
        );
    }
    let checkpoint = run_final_review_phase(
        &project_dir,
        &self.workspace.root,
        &effective,
        &mut registry,
        &mut state,
        &prompt_content,
        &mut logs,
        repo_root_ref,
        &recalc.planner,
    )
    .await?;
    pending_phase_checkpoint = checkpoint;
}
```

Inside `run_final_review_phase`, replace line 3404 (`let planner_backend_name = completion.backends.planner.clone()`) with the parameter value. Remove the borrow of `completion` for planner extraction (lines 3397–3404 can be simplified since `loop_number` and `loop_slug` are still read from `completion` but `planner` comes from the parameter).

This is critical because `Phase::FinalReview` can resume with `has_in_progress_loop() == false` — the guard at line 542 preserves FinalReview phase, but the completion attempt has `LoopStatus::Completed`, meaning any fix gated on `has_in_progress_loop()` would miss this path.

### Resume tracking flag

Add a `let mut is_resumed_state = true;` declaration before the outer loop (after state reconstruction, around line 520). At the bottom of each loop iteration (after the phase match block, around line 2317), set `is_resumed_state = false;`. This flag is used by:
- `Phase::Completing` to gate `resolve_completion_panel()` (re-run on resume, skip on same-run)
- Feature-loop phases and `Phase::FinalReview` do not need this flag because their re-resolution functions (`assign_feature_backends()`, `assign_completion_backends()`) are pure computations without health checks

### Session cleanup

No code changes. `reconstruct_project_state_internal` (lifecycle.rs:228) builds `ProjectState` via `ProjectState::new()` which initializes `session_store: SessionStore::default()` — empty `records` vec (state.rs:62–66). Sessions accumulate only during a single `run()` invocation.

### Drift logging

All drift comparisons follow the same pattern: compare reconstructed value (from artifact frontmatter) against re-resolved value (from current config), emit `warn!` with both values, role name, and loop number. No drift log is emitted when values match. For the completion-phase completers, drift is only logged when the reconstructed list is non-empty (otherwise there's nothing to compare against).

### No changes needed to

- `FeatureLoopBackends` or `CompletionLoopBackends` structs — still populated during reconstruction for audit/provenance
- `register_feature_loop()` — still stores backends at creation time
- `reconstruct_feature_loop()` or `reconstruct_completion_attempt()` in `lifecycle.rs` — still reconstruct from artifact frontmatter
- `is_backend_available()` — no longer on the critical resume path but remains available for other uses
- `state.json` schema — no structural changes
- `session_store` or `SessionStore` — no changes; already ephemeral per-run
- Quick-dev orchestrator — already resolves fresh

## Files & Modules

| File | Change |
|------|--------|
| `src/workflow/orchestrator.rs` | Add `is_resumed_state` flag before outer loop, set to `false` at end of each iteration. Replace 3 conditional `is_backend_available` blocks (implementing lines 761–777, QA lines 1310–1326, reviewing lines 1559–1575) with unconditional `assign_feature_backends()` calls + drift logs. Replace completion-phase cached reads (lines 1845–1866) with: unconditional `assign_completion_backends()` for planner, resume-gated `resolve_completion_panel()` for completers (called only when `is_resumed_state` is true; on same-run, use `completion.backends.completers` as-is). Add planner re-resolution at `Phase::FinalReview` call site (lines 2303–2314); add `planner_backend: &str` parameter to `run_final_review_phase` (line 3381); replace line 3404 with parameter usage. |
| `src/project/state.rs` | No changes |
| `src/backend/mod.rs` | No changes |
| `src/project/lifecycle.rs` | No changes |

## Testing Strategy

1. **Conformance test — feature-loop implementer model drift**: Set up a project with a mock backend, run until implementing-phase artifacts exist with `backend: mock(old-model)` in frontmatter. Change the mock backend's model config. Restart and verify the orchestrator invokes the implementer with the new model. Verify a drift warning is logged. Place in `src/validate/` following the `ConformanceTest` pattern in `tests_e2e_conformance.rs`.

2. **Conformance test — feature-loop family drift**: Start with `codex` as implementer, produce implementing-phase artifacts, then change config so the implementer role resolves to `claude`. Restart and verify `claude` is used. Verify drift warning shows family change (e.g. `codex(old-model)` → `claude(new-model)`).

3. **Conformance test — QA phase backend drift**: Run until QA-phase artifacts exist with `backend: mock(old-model)` in frontmatter for the `qa` role. Change the mock backend's model config. Restart and verify the orchestrator invokes the QA backend with the new model. Verify a drift warning is logged for the `qa` role.

4. **Conformance test — reviewing phase backend drift**: Run until reviewing-phase artifacts exist with `backend: mock(old-model)` in frontmatter for the `reviewer` role. Change the mock backend's model config. Restart and verify the orchestrator invokes the reviewer with the new model. Verify a drift warning is logged for the `reviewer` role.

5. **Conformance test — completion-loop planner drift**: Run until `termination-request.md` artifact exists with `backend: mock(old-model)`. Change config. Restart and verify the planner uses the new model. Verify that alternation parity is respected — if the completion loop number is even, the planner should be the opposite of `starting_backend`, not `starting_backend` itself.

6. **Conformance test — completion-loop completers drift on resume**: Run until at least one completer verdict artifact exists in an in-progress completion attempt. Change `completion_backends` config to use a different set of backends. Restart and verify the completers list is re-resolved from the updated config (not from the reconstructed artifact frontmatter). Verify a drift warning is logged showing old and new completer lists.

7. **Conformance test — final review planner drift with family change**: Run until `Phase::FinalReview` is reached (completion attempt is `Completed` but final review not yet done). Change the planner to a different backend family in config. Restart and verify the final review phase uses the re-resolved planner from the new family. This tests both family drift and the `has_in_progress_loop() == false` resume path.

8. **Unit test — `assign_feature_backends` / `assign_completion_backends` drift**: In `src/backend/mod.rs`, verify these functions return updated model names when `BackendRoleModels` config changes between calls. Verify alternation parity for even and odd loop numbers.

9. **Regression test — no drift, no warning**: When reconstructed backend matches config backend, verify no drift warning is logged and the orchestrator works correctly.

10. **Unit test — session store non-persistence**: In `src/project/lifecycle.rs` or `state.rs`, confirm `reconstruct_project_state` produces a `ProjectState` with empty `session_store.records` regardless of prior session activity.

11. **Unit test — `is_resumed_state` flag**: Verify that `is_resumed_state` is `true` on the first loop iteration (resume entry) and `false` on subsequent iterations (same-run entries). This can be a focused test ensuring the flag correctly gates `resolve_completion_panel()` — on same-run completion-phase entries, completers from `completion.backends.completers` are used without re-running health checks.

## Out of Scope

- Mid-run config change detection (only on restart)
- Non-alternation roles (`final_reviewer`, `arbiter`, `acceptance_qa`, `reformatter`) — resolved fresh at call sites, not cached in loop backends structs
- Changes to `is_backend_available()` semantics
- Changes to `reconstruct_feature_loop()` or `reconstruct_completion_attempt()` — still populate backends from frontmatter for provenance
- Quick-dev orchestrator (already resolves fresh)
- Schema changes to `state.json`, `FeatureLoopBackends`, or `CompletionLoopBackends`
- Artifact invalidation or phase restart on model drift
- Changes to `register_feature_loop()` or `register_completion_attempt()` — still record backends at creation time
- Explicit session cleanup code — `SessionStore` is ephemeral per-run by construction
- Removing `FeatureLoopBackends`/`CompletionLoopBackends` from state structs — they serve as provenance records
- Empty `completion_backends` fallback — `resolve_effective_config` rejects empty `completion_backends` at config validation time, so this case cannot occur in normal operation