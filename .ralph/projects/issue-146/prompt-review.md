---
artifact: prompt-review
project: issue-146
backend: codex
role: prompt_reviewer
created_at: 2026-03-04T01:58:31Z
---

# Prompt Review

## Issues Found
- Backend requirement is contradictory: one section allows implementer and reviewer to be the same backend, while another says quick-dev requires two backends by design. This blocks deterministic implementation and testing.
- Parser behavior is contradictory: technical approach says case-insensitive keyword matching, but tests say mixed-case headers should be rejected. This creates incompatible parser/test expectations.
- Commit/checkpoint behavior is underspecified for review-only transitions and clean working trees, which can lead to empty-commit edge cases and flaky behavior.
- Resume semantics are partially duplicated (`quick-dev-auto` seeds phase and orchestrator also sets phase) without a single authoritative rule for when phase is persisted.
- Loop counter semantics are ambiguous (when to increment/reset `phase_iteration`, `review_iteration`, and `final_review_attempt`), which affects guard behavior.
- Daemon integration change is incomplete as written: adding `issue_labels` to `dispatch_task()` requires explicit signature/callsite updates to avoid compile breaks.
- “Existing tests pass without modification” conflicts with introducing new commands, parser behavior, and daemon branching; expected outcome should be “existing behavior preserved” rather than “no test updates.”
- “Fresh backend instance with unique session id” needs explicit, testable contract (no session reuse for final reviews) rather than implementation hints that may vary by registry internals.

## Refined Prompt
Implement a new **quick-dev orchestration mode** in `ralph` as a parallel path to the existing `run/auto` flow. Quick-dev is for simpler tasks and uses 4 phases:

`PlanAndImplement -> CodexReview -> ApplyFixes (loop) -> FinalReview`

### Objective
Add a crash-safe, resumable quick-dev flow that can be triggered by CLI commands and GitHub issue label (`ralph:quick`), with deterministic parser contracts, template-driven prompts, and conformance coverage.

### Non-Negotiable Behavior
- Quick-dev uses exactly 4 internal phases: `PlanAndImplement`, `CodexReview`, `ApplyFixes`, `FinalReview`.
- `quick-dev-run` resumes from persisted `quick_dev_phase`; if `None`, start from `PlanAndImplement`.
- `quick-dev-auto` creates/specs a project then runs quick-dev.
- Daemon dispatch uses issue label `ralph:quick` to choose quick-dev commands (`quick-dev-auto` for new, `quick-dev-run` for resumed).
- Quick-dev orchestrator must never call `github::mark_pr_ready()`; PR lifecycle remains daemon-owned.
- Final reviews are sequential (no parallel join), and each final review call must use fresh context with no session reuse.

### Backend Requirements
- Quick-dev requires two configured backends: implementer and reviewer.
- Implementer resolves from: CLI `--implementer-backend` -> effective config `implementer_backend` -> config `starting_backend`.
- Reviewer resolves from: CLI `--reviewer-backend` -> effective config `reviewer_backend`.
- If reviewer backend is missing/unresolvable, return error: `"quick-dev requires a second backend for review"`.
- Implementer and reviewer must be distinct backend specs; if equal, return a clear error (no single-backend fallback in quick-dev).

### State and Resume
- Add `QuickDevPhase` enum in `src/project/state.rs`:
  - `PlanAndImplement`
  - `CodexReview`
  - `ApplyFixes`
  - `FinalReview`
- Add `quick_dev_phase: Option<QuickDevPhase>` to `ProjectState` with `#[serde(default)]`.
- Persist `quick_dev_phase` before executing each phase action.
- Keep `current_phase` mapped to existing `Phase` for compatibility and git checkpoint messaging.
- `phase_iteration` semantics:
  - Set to `1` on entry to `PlanAndImplement`, `CodexReview`, and `FinalReview`.
  - In `ApplyFixes`, set to the current review-loop iteration count (1-based), incrementing each time `ApplyFixes` is re-entered.

### Phase Machine
- `PlanAndImplement`:
  - Implementer runs combined plan+implement prompt.
  - Transition to `CodexReview`.
- `CodexReview`:
  - Reviewer runs review prompt.
  - Parse with `parse_codex_review_output`.
  - `ReviewSatisfied` -> `FinalReview`.
  - `ChangesRequested` -> `ApplyFixes`.
- `ApplyFixes`:
  - Implementer applies reviewer suggestions.
  - Transition back to `CodexReview`.
  - Guard: if review iterations reach `max_review_iterations` (default 5), skip to `FinalReview` with warning log.
- `FinalReview`:
  - Run two sequential independent final-review calls (implementer then reviewer, both fresh context).
  - Parse each with `parse_quick_final_review_output`.
  - If both `Complete`, set `ProjectStatus::Completed` and `current_phase = Phase::Completing`.
  - If either `IssuesFound`, transition to `PlanAndImplement` and increment final-review attempt counter.
  - Guard: if attempts reach `max_final_review_retries` (default 2), write force-complete artifact and mark `Completed`.

### Git/Checkpoint Rules
- Reuse public git APIs from `git/commit.rs` directly; do not extract private orchestrator wrappers.
- Use existing auto-commit guard logic:
  - skip if `!effective.workflow.auto_commit`
  - skip if `options.skip_commit`
  - skip if not a git repo
- Do not create empty commits.
- Use phase mapping for transition checkpoints:

| Quick-dev transition | from `Phase` | to `Phase` |
|---|---|---|
| start -> PlanAndImplement | `Planning` | `Implementing` |
| PlanAndImplement -> CodexReview | `Implementing` | `Reviewing` |
| CodexReview -> ApplyFixes | `Reviewing` | `Implementing` |
| ApplyFixes -> CodexReview | `Implementing` | `Reviewing` |
| CodexReview -> FinalReview | `Reviewing` | `FinalReview` |
| FinalReview -> PlanAndImplement | `FinalReview` | `Implementing` |
| FinalReview -> Complete | `FinalReview` | `Completing` |

### CLI Deliverables
- Add commands in `src/cli/mod.rs`:
  - `QuickDevRun(quick_dev_run::QuickDevRunArgs)`
  - `QuickDevAuto(quick_dev_auto::QuickDevAutoArgs)`
- Add files:
  - `src/cli/quick_dev_run.rs`
  - `src/cli/quick_dev_auto.rs`
- `quick-dev-run` args:
  - `--project`, `--implementer-backend`, `--reviewer-backend`, `--pr-url`, `--workspace-root`, `--skip-commit`, `--max-review-iterations`, `--max-final-review-retries`
- `quick-dev-auto` args:
  - `--idea`, `--implementer-backend`, `--reviewer-backend`, `--project-id`, `--pr-url`, `--workspace-root`, `--skip-commit`, `--max-review-iterations`, `--max-final-review-retries`
- `quick-dev-auto` flow:
  - run `QuickPrdPipeline`
  - create project
  - run `QuickDevOrchestrator`

### Daemon Deliverables
- In `src/daemon/github.rs`, add required label:
  - `("ralph:quick", "#5319e7", "Use quick-dev orchestration flow")`
- `ralph:quick` is a flow-type label only:
  - included in `REQUIRED_LABELS`
  - excluded from `LIFECYCLE_LABELS`
- In `src/daemon/process.rs`, add:
  - `spawn_ralph_quick_dev_auto()`
  - `spawn_ralph_quick_dev_run()`
  - matching `build_*_command()` helpers
- In `src/daemon/runtime.rs`, update dispatch to branch by `issue_labels.contains("ralph:quick")`:
  - quick label + new project -> `quick-dev-auto`
  - quick label + resumed project -> `quick-dev-run`
  - else existing `auto/run`
- Update all affected signatures/call sites to thread `issue_labels` cleanly.

### Parser Contracts (Strict)
- Add in `src/workflow/parser.rs`:
  - `parse_codex_review_output(raw) -> CodexReviewDecision`
  - `parse_quick_final_review_output(raw) -> QuickFinalReviewDecision`
- Strip frontmatter before parsing.
- Use first H1 only.
- Exact required H1 values (case-sensitive):
  - `# Review: SATISFIED`
  - `# Review: CHANGES REQUESTED`
  - `# Final Review: COMPLETE`
  - `# Final Review: ISSUES FOUND`
- Allow trailing whitespace after header text.
- Any other header format returns descriptive parse error.

### Prompt Templates and Builders
- Add 4 template fields to:
  - `TemplateConfig` (`src/config/global.rs`)
  - `ProjectTemplateOverrides` (`src/config/project.rs`)
  - `EffectiveTemplateConfig` (`src/config/mod.rs`)
- Field names:
  - `quick_dev_plan_implement`
  - `quick_dev_codex_review`
  - `quick_dev_apply_fixes`
  - `quick_dev_final_review`
- Add resolution logic in effective config merge.
- Add prompt builder module `src/prompts/quick_dev.rs` and export in `src/prompts/mod.rs`.
- All builders must use `render_template_with_fallback()`.
- Templates must include explicit `CRITICAL FORMAT REQUIREMENTS` matching parser contracts.

### Required Files
- Edit: `src/project/state.rs`
- New: `src/workflow/quick_dev_orchestrator.rs`
- Edit: `src/workflow/mod.rs`
- Edit: `src/workflow/parser.rs`
- New: `src/prompts/quick_dev.rs`
- Edit: `src/prompts/mod.rs`
- Edit: `src/config/global.rs`
- Edit: `src/config/project.rs`
- Edit: `src/config/mod.rs`
- New: `src/cli/quick_dev_auto.rs`
- New: `src/cli/quick_dev_run.rs`
- Edit: `src/cli/mod.rs`
- Edit: `src/daemon/runtime.rs`
- Edit: `src/daemon/process.rs`
- Edit: `src/daemon/github.rs`
- New: `src/validate/tests_quick_dev.rs`
- Edit: `src/validate/mod.rs`
- Edit or add validate tests in daemon-focused module as needed for label-based dispatch assertions

### Acceptance Criteria
- [ ] Quick-dev state enum and persisted `quick_dev_phase` field implemented with backward-compatible serde default.
- [ ] `QuickDevOrchestrator` implements full 4-phase machine, review loop, final-review reloop, and both guards.
- [ ] `quick-dev-run` and `quick-dev-auto` commands are wired and executable from CLI.
- [ ] Daemon dispatch selects quick-dev commands when `ralph:quick` is present.
- [ ] `ralph:quick` is created during label bootstrap and treated as non-lifecycle label.
- [ ] New strict parser functions and enums are implemented and covered by unit tests.
- [ ] Quick-dev templates/config fields resolve correctly via global/project override flow.
- [ ] Orchestrator never calls `mark_pr_ready`.
- [ ] Reviewer backend missing or equal to implementer backend fails fast with clear error.
- [ ] Existing non-quick-dev behavior remains intact.

### Testing Requirements
- Add/extend unit tests for parser, CLI arg parsing, daemon process command builders, state serde/invariants, label tables, and template config resolution.
- Add validate conformance tests covering:
  - happy path
  - review loop
  - final-review reloop
  - max-review-iterations guard
  - max-final-review-retries force-complete
  - resume from `CodexReview`, `FinalReview`, and `None`
  - daemon branching by `ralph:quick`
  - reviewer backend unavailable/equal backend failure
- Run:
  - `nix develop -c cargo test`
  - `nix build -L`
  - `./result/bin/ralph validate --bin ./result/bin/ralph`

### Out of Scope
- Changes to existing standard orchestrator flow semantics.
- PR readiness transitions inside quick-dev orchestrator.
- Parallel final-review execution.
- New git plumbing APIs beyond reusing existing public commit functions.
- UI/dashboard changes.
