I now have comprehensive understanding of the codebase. Let me write the specification.

## Summary

Add a `FinalReview` phase that gates project completion behind a multi-backend consensus review. After acceptance QA passes in the `Completing` phase, the orchestrator transitions to `FinalReview` instead of immediately marking the project `Completed`. In this phase, multiple reviewer backends propose structured amendments to the project specification, a planner evaluates each amendment, the reviewers vote on the planner's positions, and an arbiter resolves disputes when consensus is not reached. Accepted amendments restart the planning phase with augmented requirements; if all amendments are rejected (or none are proposed), the project completes. The feature is enabled by default (`final_review_enabled = true`) and is opt-out. All state is derived from artifacts and checkpoint commits (git-first), with no new durable state store.

## Acceptance Criteria

- [ ] `Phase::FinalReview` variant added to the `Phase` enum in `src/project/state.rs`
- [ ] `parse_phase` in `src/git/ralph_commit.rs` handles `"final_review"` string
- [ ] All 6 `phase_label` functions (in `ralph_commit.rs`, `orchestrator.rs`, `status.rs`, `project.rs`, `tail.rs`, `history.rs`) return `"final_review"` for the new variant
- [ ] Phase iteration calculation in `src/project/lifecycle.rs` handles `Phase::FinalReview` (returns 1)
- [ ] 6 new config fields added to `WorkflowConfig` with defaults: `final_review_enabled` (bool, default `true`), `final_review_backends` (Vec\<String\>, default `["claude", "codex"]`), `final_review_arbiter_backend` (String, default `"claude"`), `final_review_min_reviewers` (u32, default 2), `final_review_consensus_threshold` (f64, default 1.0), `max_final_review_restarts` (u32, default 3)
- [ ] Corresponding `Option<T>` fields added to `ProjectWorkflowOverrides` in `src/config/project.rs`
- [ ] All 6 fields resolved in `EffectiveWorkflowConfig` via `resolve_effective_config` with standard 3-tier precedence
- [ ] Config validation: warn (log) if arbiter backend family overlaps with any `final_review_backends` entry; error if `final_review_backends` is empty when enabled; error if arbiter backend family is unknown
- [ ] `BackendRoleModels` and `RoleTimeouts` extended with `final_reviewer`, `arbiter` role entries; `for_role` and `fill_from` updated
- [ ] 8-step orchestrator flow implemented in the `Phase::FinalReview` match arm: (1) collect reviewer backends, (2) invoke each reviewer with `final_reviewer` template to produce proposals, (3) parse proposals into amendments via `parse_final_reviewer_output`, (4) build planner position prompt with amendments, (5) invoke planner to produce positions, (6) parse positions via `parse_planner_position_output`, (7) invoke each reviewer with vote template, (8) parse votes via `parse_vote_output`, compute consensus per amendment, invoke arbiter via `parse_arbiter_output` for disputed amendments, write exit decision
- [ ] Artifact-based resume probing: each step writes an artifact; on resume the orchestrator probes for existing artifacts and skips completed steps
- [ ] `final-review-config.json` written to the completion loop directory at the start of each round; on resume, if the file exists and differs from current effective config, the in-progress round is invalidated (artifacts deleted, round restarted)
- [ ] Checkpoint commits: `completing -> final_review` (when acceptance QA passes and final review enabled), `final_review -> planning` (when amendments accepted), `final_review -> completing` (when no amendments accepted; project marked `Completed`)
- [ ] Amendment integration: `build_planner_prompt` reads `final-review-amendments-applied.md` from the project directory (if it exists) and injects its content as a `## Final Review Amendments` section via `append_section_if_missing`
- [ ] 4 new template files with hardcoded defaults: `default_final_reviewer_template()`, `default_planner_position_template()`, `default_vote_template()`, `default_arbiter_template()`; paths added to `TemplateConfig` and `EffectiveTemplateConfig`
- [ ] 4 new parser functions in `src/workflow/parser.rs`: `parse_final_reviewer_output` (extracts amendment IDs + descriptions), `parse_planner_position_output` (extracts positions keyed by amendment ID), `parse_vote_output` (extracts accept/reject keyed by amendment ID), `parse_arbiter_output` (extracts ruling keyed by amendment ID). All parsers fail-closed: if an amendment ID referenced in input is missing from output, the parser returns an error
- [ ] Restart count derived from checkpoint commit messages matching `final_review -> planning` for the current project; no new state field
- [ ] Force-complete after `max_final_review_restarts` reached: write `final-review-force-complete.md` artifact, skip to `Completed`
- [ ] CLI `ralph status` shows: current final review round, reviewer count and progress, disputed amendment count, restart count
- [ ] Unit tests for all 4 parsers, consensus computation logic, config validation (arbiter overlap warning, empty backends error), and resume probing logic
- [ ] Integration tests (`tests/orchestrator.rs`): approve path (no amendments -> complete), restart path (amendments accepted -> planning), arbiter path (disputed amendments -> arbiter invoked), resume (partial artifacts -> skip completed steps), config mismatch (invalidate round)
- [ ] Validate tests (`src/validate/tests_final_review.rs`): full completion flow with final review enabled via mock backends

## Technical Approach

### Phase Model

Add `FinalReview` to the `Phase` enum in `src/project/state.rs`:

```rust
pub enum Phase {
    Planning,
    Implementing,
    QA,
    Reviewing,
    Committing,
    Completing,
    FinalReview,
}
```

The serde rename for `FinalReview` follows the existing `snake_case` convention, producing `"final_review"`. Update `parse_phase` and all 6 `phase_label` functions to handle the new variant. Update the `Phase` match in `lifecycle.rs` to return iteration 1 for `FinalReview`.

### Configuration

Add 6 fields to `WorkflowConfig` in `src/config/global.rs`:

```rust
pub final_review_enabled: bool,           // default true
pub final_review_backends: Vec<String>,    // default ["claude", "codex"]
pub final_review_arbiter_backend: String,  // default "claude"
pub final_review_min_reviewers: u32,       // default 2
pub final_review_consensus_threshold: f64, // default 1.0 (unanimity)
pub max_final_review_restarts: u32,        // default 3
```

Mirror as `Option<T>` fields in `ProjectWorkflowOverrides`. Add corresponding resolution in `resolve_effective_config` and `EffectiveWorkflowConfig`. Add validation: (1) error if `final_review_backends` is empty when `final_review_enabled`; (2) error if arbiter backend family is unknown; (3) warn if arbiter backend family appears in `final_review_backends`.

Add `final_reviewer` and `arbiter` entries to `BackendRoleModels` and `RoleTimeouts`, with `for_role` and `fill_from` updates. Default models: same as `reviewer` defaults for each backend family.

### Orchestrator Flow

In `src/workflow/orchestrator.rs`, modify the `Completing` phase: when `completer_decision.verdict == Complete` and acceptance QA passes and `final_review_enabled`, transition to `Phase::FinalReview` instead of setting `ProjectStatus::Completed`.

Add a new `Phase::FinalReview` match arm implementing the 8-step flow:

**Step 1 - Collect reviewers**: Resolve backend instances from `final_review_backends` config for the `final_reviewer` role.

**Step 2 - Invoke reviewers**: For each reviewer backend, build a prompt using `default_final_reviewer_template()` containing the master prompt, full git diff against base, completed feature summary, and the completer's verdict. Invoke via `execute_with_parse_retries`. Write each response as a `final-review-proposals-{backend}.md` artifact in the completion loop directory.

**Step 3 - Parse proposals**: Parse each reviewer's output with `parse_final_reviewer_output`. Collect all amendments with unique IDs. If no amendments are proposed by any reviewer, skip directly to completion (write `final-review-exit-approved.md`, set `ProjectStatus::Completed`, phase stays `Completing`).

**Step 4 - Build planner position prompt**: Combine all amendments into a single prompt using `default_planner_position_template()`. Include the master prompt, amendments keyed by ID, and the original project spec summary.

**Step 5 - Invoke planner**: Use the project's planner backend. Parse response with `parse_planner_position_output` to get accept/reject positions per amendment ID.

**Step 6 - Build vote prompt & invoke voters**: For each reviewer backend, build a vote prompt using `default_vote_template()` containing the amendments and the planner's positions. Parse responses with `parse_vote_output`.

**Step 7 - Compute consensus**: For each amendment, count accept votes. If `accepts / total_voters >= consensus_threshold`, the amendment is accepted. Amendments below threshold with at least one accept vote are "disputed" and forwarded to the arbiter.

**Step 8 - Arbiter (if needed)**: If disputed amendments exist, invoke the arbiter backend with `default_arbiter_template()` containing the disputed amendments, planner positions, and vote tallies. Parse with `parse_arbiter_output`. Merge arbiter rulings into final amendment decisions.

**Exit decision**: If any amendments are accepted (by consensus or arbiter), write `final-review-amendments-applied.md` to the project directory (not the loop directory) containing the accepted amendment descriptions. Derive restart count from checkpoint history. If restart count >= `max_final_review_restarts`, write `final-review-force-complete.md` and set `ProjectStatus::Completed`. Otherwise, transition to `Phase::Planning` (checkpoint: `final_review -> planning`).

If no amendments are accepted, write `final-review-exit-approved.md`, set `ProjectStatus::Completed`, checkpoint `final_review -> completing`.

### Resume Safety

Each of the 8 steps writes a distinct artifact. On entry to `Phase::FinalReview`, probe the completion loop directory for existing artifacts:

- `final-review-proposals-{backend}.md` exists → skip that reviewer invocation
- `final-review-planner-positions.md` exists → skip planner invocation
- `final-review-votes-{backend}.md` exists → skip that voter invocation
- `final-review-arbiter-ruling.md` exists → skip arbiter invocation
- `final-review-exit-*.md` exists → skip to exit decision

Parse existing artifacts to reconstruct intermediate state. This follows the same pattern as the existing acceptance QA resume logic (checking `has_acceptance_result_for`).

### Config Mismatch Detection

At the start of each final review round, serialize the relevant config fields to `final-review-config.json` in the completion loop directory. On resume, read the file and compare against current effective config. If they differ, log a warning, delete all `final-review-*` artifacts from the loop directory, and restart the round. This ensures config changes (e.g., adding/removing a reviewer backend) don't produce inconsistent state.

### Amendment Injection

In `build_planner_prompt`, after computing `completion_feedback`, check for `final-review-amendments-applied.md` in the project directory. If it exists, read its contents and insert into the template vars as `final_review_amendments`. Use `append_section_if_missing` with aliases `["final_review_amendments"]` and heading `"## Final Review Amendments"` to inject into the prompt.

### New Parsers

All 4 parsers follow the existing pattern: strip frontmatter, find first H1, match heading, validate required sections, extract structured data.

**`parse_final_reviewer_output`**: Expects H1 `# Final Review: AMENDMENTS` or `# Final Review: NO AMENDMENTS`. For AMENDMENTS, requires `## Amendment` subsections with format `## Amendment: {ID}` followed by `### Description` and `### Rationale`. Returns `FinalReviewerDecision::Amendments(Vec<Amendment>)` or `NoAmendments`.

**`parse_planner_position_output`**: Expects H1 `# Planner Positions`. Requires `## Amendment: {ID}` subsections each containing `### Position: ACCEPT` or `### Position: REJECT` and `### Rationale`. Validates all input amendment IDs are present (fail-closed).

**`parse_vote_output`**: Expects H1 `# Vote Results`. Requires `## Amendment: {ID}` subsections with `### Vote: ACCEPT` or `### Vote: REJECT`. Validates all input amendment IDs are present.

**`parse_arbiter_output`**: Expects H1 `# Arbiter Ruling`. Requires `## Amendment: {ID}` subsections with `### Ruling: ACCEPT` or `### Ruling: REJECT` and `### Rationale`. Validates all input amendment IDs are present.

### New Artifact Kinds

Add to `ArtifactKind` in `src/project/artifacts.rs`:
- `FinalReviewProposals` (file: `final-review-proposals-{backend}.md`)
- `FinalReviewPlannerPositions` (file: `final-review-planner-positions.md`)
- `FinalReviewVotes` (file: `final-review-votes-{backend}.md`)
- `FinalReviewArbiterRuling` (file: `final-review-arbiter-ruling.md`)
- `FinalReviewExit` (file: `final-review-exit-{outcome}.md`)

These use `write_artifact` with the completion loop's slug and loop number, following the existing timestamp-prefixed artifact pattern.

### Restart Count Derivation

Count checkpoint commits matching `final_review -> planning` for the current project ID using `list_ralph_commits`. This follows the git-first principle and requires no new state field.

### CLI Status Display

In `src/cli/status.rs`, when `state.current_phase == Phase::FinalReview`, display:
- "Final Review: round N" (derived from checkpoint history)
- "Reviewers: M/N complete" (derived from presence of proposal artifacts)
- "Disputed amendments: K" (derived from vote artifacts if they exist)
- "Restart count: R / max" (derived from checkpoint commits)

## Files & Modules

| File | Change |
|------|--------|
| `src/project/state.rs` | Add `Phase::FinalReview` variant |
| `src/project/artifacts.rs` | Add 5 new `ArtifactKind` variants, `base_type`, `file_name` arms |
| `src/project/lifecycle.rs` | Add `Phase::FinalReview` to iteration match (returns 1) |
| `src/config/global.rs` | Add 6 `WorkflowConfig` fields with defaults; extend `BackendRoleModels` + `RoleTimeouts` with `final_reviewer`, `arbiter`; add `TemplateConfig` entries for 4 new templates |
| `src/config/project.rs` | Add 6 `Option<T>` fields to `ProjectWorkflowOverrides`; add 4 template overrides to `ProjectTemplateOverrides` |
| `src/config/mod.rs` | Resolve 6 new fields in `EffectiveWorkflowConfig`; resolve 4 new template paths in `EffectiveTemplateConfig`; add config validation logic |
| `src/workflow/parser.rs` | Add 4 new parser functions + decision types (`FinalReviewerDecision`, `PlannerPositionDecision`, `VoteDecision`, `ArbiterDecision`) |
| `src/workflow/orchestrator.rs` | Add `Phase::FinalReview` match arm (8-step flow); modify `Completing` arm to transition to `FinalReview` when enabled; inject amendments in `build_planner_prompt`; add `expected_format_template_for` entries for new roles |
| `src/prompts/templates.rs` | Add 4 `default_*_template()` functions |
| `src/git/ralph_commit.rs` | Add `"final_review"` to `parse_phase` and `phase_label` |
| `src/cli/status.rs` | Add `FinalReview` to `phase_label`; add final review progress display |
| `src/cli/project.rs` | Add `FinalReview` to `phase_label` |
| `src/cli/tail.rs` | Add `FinalReview` to `phase_label` |
| `src/cli/history.rs` | Add `FinalReview` to `phase_label` |
| `tests/orchestrator.rs` | Add integration tests for approve/restart/arbiter/resume/mismatch paths |
| `src/validate/tests_final_review.rs` (new) | Validate tests for full completion flow with final review |
| `src/validate/mod.rs` | Register `tests_final_review` module |

## Testing Strategy

### Unit Tests (embedded `#[cfg(test)]` modules)

**Parser tests** (`src/workflow/parser.rs`):
- `parse_final_reviewer_output`: valid amendments, no amendments, missing required sections, empty amendment ID
- `parse_planner_position_output`: valid positions, missing amendment ID (fail-closed), invalid position value
- `parse_vote_output`: valid votes, missing amendment ID (fail-closed), invalid vote value
- `parse_arbiter_output`: valid ruling, missing amendment ID (fail-closed), invalid ruling value
- All parsers: frontmatter stripping, missing H1, unsupported H1

**Consensus logic** (`src/workflow/orchestrator.rs`):
- Unanimity threshold (1.0): all accept → accepted, one reject → disputed
- Majority threshold (0.5): majority accept → accepted, minority accept → disputed
- Zero accepts → rejected (not sent to arbiter)
- Empty amendment list → immediate completion

**Config validation** (`src/config/mod.rs`):
- Arbiter backend family overlapping `final_review_backends` → warning logged
- Empty `final_review_backends` when enabled → error
- Unknown arbiter backend family → error
- `final_review_enabled = false` → validation skipped

**Resume probing** (`src/workflow/orchestrator.rs`):
- Existing proposal artifacts → corresponding reviewers skipped
- Existing position artifact → planner skipped
- Existing vote artifacts → corresponding voters skipped
- Existing exit artifact → entire flow skipped

### Integration Tests (`tests/orchestrator.rs`)

- **Approve path**: Mock all reviewers to return `NO AMENDMENTS` → project completes directly
- **Restart path**: Mock reviewers to propose amendments, planner/voters to accept → verify transition to Planning with amendments injected
- **Arbiter path**: Mock split votes → verify arbiter invoked for disputed amendments only
- **Resume test**: Write partial artifacts, re-run orchestrator → verify skipped steps and correct final state
- **Config mismatch test**: Write `final-review-config.json` with stale config, re-run → verify round invalidated and restarted

### Validate Tests (`src/validate/tests_final_review.rs`)

- **Full completion flow with final review**: Mock backends producing the full sequence (planner → feature → implement → review → commit → completion → acceptance QA pass → final review NO AMENDMENTS → completed). Verify exit code 0 and `ProjectStatus::Completed`.
- **Restart flow**: Mock final reviewers to produce amendments on first round, no amendments on second → verify two planning phases occurred and project completed.

## Out of Scope

- **New `Phase::Completed`**: No new terminal phase; `ProjectStatus::Completed` remains the terminal signal
- **Persistent state store**: No database, no JSON state file beyond artifacts and git commits
- **UI/dashboard for amendment review**: CLI status display only; no interactive approval workflow
- **Per-amendment backend routing**: All reviewer backends see all amendments; no selective routing
- **Amendment priority or ordering**: Amendments are unordered; all accepted amendments are injected together
- **Backward-compatible checkpoint parsing for old binaries**: Old binaries encountering `final_review` in a checkpoint commit will hard-fail per standard phase incompatibility behavior (documented, not mitigated)
- **Template customization docs**: Template files are created with hardcoded defaults; documentation of template variables is deferred
- **Final review for non-completion flows**: Final review only triggers after acceptance QA passes in the `Completing` phase; it does not apply to individual feature loops