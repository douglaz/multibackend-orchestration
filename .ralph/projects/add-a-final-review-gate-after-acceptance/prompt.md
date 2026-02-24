### Feature
Add a `FinalReview` phase that gates project completion behind a multi-backend consensus review after acceptance QA passes in `Completing`.

### Objective
Before final completion, reviewer backends may propose amendments, planner takes positions, reviewers vote, and an arbiter resolves disputes. Accepted amendments restart planning; otherwise the project completes.

### Scope
- In scope: phase/state transitions, config, templates, parsers, orchestrator flow, artifact-based resume, status output, unit/integration/validate tests.
- Out of scope: new terminal `Phase::Completed`, new durable datastore, interactive UI approval, per-amendment backend routing.

### Deterministic Rules
1. Add `Phase::FinalReview` serialized as `final_review`.
2. Update `parse_phase` and all phase-label functions to support `final_review`.
3. `FinalReview` iteration in lifecycle is `1`.
4. Restart count is derived only from checkpoint commits `final_review -> planning` for the current project.
5. Final review round number is `restart_count + 1`.
6. All parsers are fail-closed.

### Required Config Changes
Add to `WorkflowConfig` with defaults:
- `final_review_enabled: bool = true`
- `final_review_backends: Vec<String> = ["claude", "codex"]`
- `final_review_arbiter_backend: String = "claude"`
- `final_review_min_reviewers: u32 = 2`
- `final_review_consensus_threshold: f64 = 1.0`
- `max_final_review_restarts: u32 = 3`

Add matching `Option<T>` fields to `ProjectWorkflowOverrides`. Resolve all in `EffectiveWorkflowConfig` using normal precedence (project > global > default).

Validation:
- Error if enabled and reviewer list is empty.
- Deduplicate reviewers by canonical backend spec; error if unique count `< final_review_min_reviewers`.
- Error if `final_review_consensus_threshold <= 0.0` or `> 1.0`.
- Error if arbiter backend family is unknown.
- Warn if arbiter backend family overlaps any reviewer family.
- Backend family = backend name before optional `(model)`.

### Role/Template Extensions
- Extend `BackendRoleModels` and `RoleTimeouts` with `final_reviewer` and `arbiter`.
- Add templates and effective path resolution for:
  - `default_final_reviewer_template()`
  - `default_planner_position_template()`
  - `default_vote_template()`
  - `default_arbiter_template()`

### Phase Transition Behavior
1. In `Completing`, when completion verdict is `Complete` and acceptance QA passed:
   - If `final_review_enabled`, checkpoint `completing -> final_review` and transition to `Phase::FinalReview`.
   - Else keep existing completion behavior.
2. In `FinalReview`:
   - If accepted amendments exist and restart limit not reached: checkpoint `final_review -> planning`, transition to `Planning`.
   - If no accepted amendments: checkpoint `final_review -> completing`, set `ProjectStatus::Completed`.
   - If accepted amendments exist but restart limit reached: write force-complete artifact, checkpoint `final_review -> completing`, set `ProjectStatus::Completed`.

### FinalReview Orchestrator Flow
Implement `Phase::FinalReview` as artifact-resumable steps:

1. Resolve effective final-review config and normalized reviewer list (deduped, stable order).
2. Manage `final-review-config.json` in current completion loop directory.
3. On resume, compare saved normalized config struct with current config.
4. If mismatch: log warning, delete only current-loop `final-review-*` artifacts and `final-review-config.json`, restart from step 1.
5. Invoke each reviewer with `final_reviewer` template unless proposal artifact exists.
6. Parse each proposal via `parse_final_reviewer_output`.
7. Merge amendments; amendment IDs must be globally unique across reviewers.
8. If no amendments: write approved-exit artifact and complete project.
9. Invoke planner with planner-position template unless positions artifact exists.
10. Parse planner positions via `parse_planner_position_output` (must include every amendment ID).
11. Invoke reviewer votes with vote template unless vote artifact exists per reviewer.
12. Parse votes via `parse_vote_output` (must include every amendment ID).
13. Compute per-amendment consensus:
- `ratio = accepts / total_voters`
- `accepted` if `ratio >= threshold`
- `rejected` if `accepts == 0`
- `disputed` otherwise
14. If disputed set non-empty, invoke arbiter unless arbiter artifact exists.
15. Parse arbiter output via `parse_arbiter_output` (must include every disputed ID).
16. Final accepted set = consensus-accepted + arbiter-accepted.
17. Exit:
- If final accepted set empty: write `final-review-exit-approved` artifact and complete.
- If non-empty and restart limit not reached: write/append `final-review-amendments-applied.md`, write restart-exit artifact, go to planning.
- If non-empty and restart limit reached: write `final-review-force-complete.md`, complete.

### Artifacts and Resume
Add `ArtifactKind` variants:
- `FinalReviewProposals` -> `final-review-proposals-{backend}.md`
- `FinalReviewPlannerPositions` -> `final-review-planner-positions.md`
- `FinalReviewVotes` -> `final-review-votes-{backend}.md`
- `FinalReviewArbiterRuling` -> `final-review-arbiter-ruling.md`
- `FinalReviewExit` -> `final-review-exit-{outcome}.md`

Rules:
- Use existing `write_artifact`.
- Resume probing must use artifact kind identity, not brittle filename assumptions.
- If an existing artifact fails parsing, fail the run (do not silently bypass).

### Parser Contracts
Add to `src/workflow/parser.rs`:
- `parse_final_reviewer_output`
- `parse_planner_position_output`
- `parse_vote_output`
- `parse_arbiter_output`

Formats:
- Reviewer: `# Final Review: AMENDMENTS` or `# Final Review: NO AMENDMENTS`
- Planner: `# Planner Positions`
- Vote: `# Vote Results`
- Arbiter: `# Arbiter Ruling`
- Amendment blocks: `## Amendment: {ID}` with required subsections.
- Fail-closed: missing required IDs/sections is an error.

### Planner Prompt Injection
In `build_planner_prompt`, read `final-review-amendments-applied.md` from project directory (if present) and inject via `append_section_if_missing` under heading `## Final Review Amendments` with alias `final_review_amendments`.

### CLI Status
When `Phase::FinalReview`, `ralph status` shows:
- `Final Review: round N`
- `Reviewers: M/N complete`
- `Disputed amendments: K`
- `Restart count: R / max_final_review_restarts`

### Files To Change
- `src/project/state.rs`
- `src/git/ralph_commit.rs`
- `src/project/lifecycle.rs`
- `src/project/artifacts.rs`
- `src/config/global.rs`
- `src/config/project.rs`
- `src/config/mod.rs`
- `src/workflow/parser.rs`
- `src/workflow/orchestrator.rs`
- `src/prompts/templates.rs`
- `src/cli/status.rs`
- `src/cli/project.rs`
- `src/cli/tail.rs`
- `src/cli/history.rs`
- `tests/orchestrator.rs`
- `src/validate/tests_final_review.rs` (new)
- `src/validate/mod.rs`

### Acceptance Criteria
- [ ] `FinalReview` phase added and parsed/labeled everywhere required.
- [ ] Config fields, overrides, effective resolution, and validation implemented exactly.
- [ ] `final_reviewer` and `arbiter` role model/timeout support added.
- [ ] `Completing -> FinalReview` transition implemented behind `final_review_enabled`.
- [ ] Full artifact-resumable `FinalReview` flow implemented.
- [ ] Config mismatch invalidation implemented with scoped deletion.
- [ ] Restart count derived from checkpoint history only.
- [ ] Force-complete behavior works at restart cap.
- [ ] Amendment injection into planner prompt implemented.
- [ ] Four templates and four parsers implemented and wired.
- [ ] Parser checks are fail-closed.
- [ ] `ralph status` final-review progress output implemented.
- [ ] Integration tests cover approve/restart/arbiter/resume/config-mismatch/force-complete.
- [ ] Validate tests added and registered (`src/validate/tests_final_review.rs`).

### Testing Requirements
Unit tests:
- Parser success/failure cases including fail-closed ID coverage.
- Consensus logic edge cases.
- Config validation including threshold bounds and min reviewers.
- Resume probing and parse-failure behavior.

Integration tests (`tests/orchestrator.rs`):
- No amendments -> complete.
- Accepted amendments -> planning restart.
- Disputed amendments -> arbiter invoked only for disputed IDs.
- Partial artifacts resume skips completed steps.
- Config mismatch invalidates and restarts round.
- Restart cap triggers force-complete.

Validate tests:
- New `src/validate/tests_final_review.rs` with:
  - Full completion flow with final review enabled.
  - Restart flow (round 1 amendments, round 2 none).
- Register module in `src/validate/mod.rs`.