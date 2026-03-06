### Objective
Fix backend selection during orchestrator resume so execution always uses the current config, while reconstructed backend values remain provenance-only.

### Problem Statement
On restart/resume, reconstructed state includes backend specs from artifact frontmatter (`loop_state.backends.*`, `completion.backends.*`). Current resume paths can reuse these stale values, so model/family changes in config may be ignored.

### Requirements
1. In feature-loop phases (`Implementing`, `QA`, `Reviewing`), always resolve effective backends via `assign_feature_backends(...)`.
2. In those feature-loop phases, never use `loop_state.backends.*` as execution source of truth; use them only for drift comparison/logging.
3. Remove `is_backend_available()` gating from these three phase paths.
4. In `Phase::Completing`, always resolve planner via `assign_completion_backends(...)` so loop parity is correct.
5. In `Phase::Completing`, re-resolve completer panel via `resolve_completion_panel(...)` only on resume entry, not on same-run entries.
6. Add `is_resumed_state: bool` in orchestrator `run()`.
7. Initialize it to `true` immediately after state reconstruction.
8. Set it to `false` after the first outer-loop iteration completes.
9. Use this flag to gate completion-panel re-resolution.
10. In same-run completion entries (`is_resumed_state == false`), use `completion.backends.completers` as already resolved in-process.
11. In resume completion entries (`is_resumed_state == true`), resolve completers from current config.
12. In `Phase::FinalReview`, resolve planner at the phase call site using `assign_completion_backends(...)` and pass it into `run_final_review_phase(..., planner_backend: &str)`.
13. `run_final_review_phase` must not read `completion.backends.planner` for execution decisions.
14. Keep `FeatureLoopBackends` and `CompletionLoopBackends` reconstruction unchanged for provenance/audit.
15. Do not change `state.json` schema.
16. Do not add session cleanup logic; `SessionStore::default()` on reconstruction is already correct.
17. Do not add empty `completion_backends` fallback logic; config validation already rejects empty values.

### Drift Logging Requirements
1. On each mismatch between reconstructed and re-resolved backend values, emit `warn!`.
2. Include `role`, `loop_number`, `original`, and `resolved` fields.
3. For completer-panel drift, log only when reconstructed list is non-empty and differs from re-resolved list.
4. Emit no drift warning when values match.

### Scope
1. Applies to restart/resume behavior.
2. No mid-run config hot-reload behavior.
3. Quick-dev orchestrator remains unchanged.
4. No changes to `register_feature_loop`, `register_completion_attempt`, or backend struct schemas.

### Files to Change
1. `src/workflow/orchestrator.rs` for backend re-resolution and resume-flag logic.
2. Any affected signature/call-site updates for `run_final_review_phase(..., planner_backend: &str)`.

### Required Tests
1. Add validate conformance tests in `src/validate/tests_resume_backend_resolution.rs`.
2. Register the new module in `src/validate/mod.rs`.
3. Add conformance case: feature implementing backend model drift on resume uses new model.
4. Add conformance case: feature QA backend drift on resume uses new backend.
5. Add conformance case: feature reviewer backend drift on resume uses new backend.
6. Add conformance case: completion planner drift on resume uses re-resolved planner and preserves alternation parity.
7. Add conformance case: completion completer panel drift on resume uses re-resolved panel.
8. Add conformance case: final-review planner drift when completion attempt is already completed uses re-resolved planner.
9. Add regression case: when no drift exists, execution is unchanged and no drift warning is emitted (if log capture is available; otherwise assert backend selection only).
10. Add focused test for resume gating: completer panel is not re-health-checked on same-run completion entry.

### Definition of Done
1. Resume paths no longer execute with reconstructed stale backend specs.
2. Reconstructed backend fields remain preserved for provenance.
3. New/updated tests pass with `nix develop -c cargo test` and `./result/bin/ralph validate --bin ./result/bin/ralph`.