# Plan: Add QA Phase to Ralph Orchestrator (Revised)

## Review Summary
The previous draft had the right direction but left critical integration and state details ambiguous. This revision closes those gaps so implementation can proceed without hidden regressions.

Key fixes in this revision:
1. Covers all integration surfaces, not only orchestrator internals (CLI, config, backend registry, status/history/tail, tests).
2. Defines deterministic iteration semantics for QA feedback vs reviewer feedback.
3. Defines artifact/state plumbing for acceptance-fail feedback to flow back into planning.
4. Preserves backward compatibility for existing `state.json`, `config.toml`, and `ralph.toml`.
5. Breaks work into execution phases with explicit exit criteria.

## Goals
1. Insert an optional QA gate between Implementing and Reviewing.
2. Make QA actively execute tests (build/test/manual checks), not static review.
3. Loop QA failures back to Implementing with structured, parseable feedback.
4. Add a final QA acceptance gate before honoring completer `COMPLETE`.
5. Keep current behavior unchanged when QA is disabled.

## Non-Goals
1. Replacing reviewer responsibilities.
2. Building full sandbox isolation for QA commands in this change.
3. Changing planner/implementer/reviewer/completer output contracts beyond required QA wiring.

## Final Workflow

### Feature loops

```text
Planning -> Implementing -> QA -> Reviewing -> Committing -> Planning
                          |      |
                          |      +--(suggestions)--> Implementing
                          +--(fail)-----------------> Implementing
```

### Completion loops

```text
Planning -> Completing --(CONTINUE)--> Planning
                    \
                     +--(COMPLETE)--> QA acceptance gate
                                        | pass -> Project Completed
                                        + fail -> force CONTINUE, back to Planning
```

## Core Design Decisions
1. `qa_enabled` defaults to `false` for backward compatibility.
2. `phase_iteration` remains the single runtime iteration counter; meaning is phase-specific.
3. QA and review feedback are stored separately to avoid artifact-name collisions and history confusion.
4. QA backend defaults to planner-aligned alternation unless explicitly overridden.
5. QA template includes a strict instruction to avoid source edits during testing.

## Data Model Changes

### `src/project/state.rs`

Add QA phase:
```rust
pub enum Phase {
    Planning,
    Implementing,
    QA,
    Reviewing,
    Committing,
    Completing,
}
```

Extend feature-loop backends:
```rust
pub struct FeatureLoopBackends {
    pub planner: String,
    pub implementer: String,
    pub reviewer: String,
    pub qa: String,
}
```

Extend feature-loop artifacts:
```rust
pub struct FeatureLoopArtifacts {
    pub spec: String,
    pub impl_notes: Option<String>,
    pub reviews: Vec<ReviewExchange>,
    pub approval: Option<String>,
    #[serde(default)]
    pub qa_results: Vec<QaExchange>,
    #[serde(default)]
    pub pending_qa_feedback: Option<String>,
}

pub struct QaExchange {
    pub iteration: u32,
    pub passed: bool,
    pub report: String,
    pub implementer_response: Option<String>,
}
```

Extend completion-loop artifacts:
```rust
pub struct CompletionLoopArtifacts {
    pub termination_request: String,
    pub verdict: Option<String>,
    #[serde(default)]
    pub acceptance_result: Option<String>,
    #[serde(default)]
    pub acceptance_passed: Option<bool>,
}
```

Compatibility requirements:
1. New fields must be `#[serde(default)]` where needed.
2. Existing states without QA fields must deserialize unchanged.
3. `validate_invariants()` must remain valid for legacy and QA-enabled states.

## Artifact Changes

### `src/project/artifacts.rs`

Add `ArtifactKind` variants:
```rust
QaPass { iteration: u32 },         // qa-001-pass.md
QaFail { iteration: u32 },         // qa-001-fail.md
ImplQaResponse { iteration: u32 }, // impl-qa-response-001.md
AcceptancePass,                    // acceptance-pass.md
AcceptanceFail,                    // acceptance-fail.md
```

Update `base_type()`, `file_name()`, and metadata helpers accordingly.

## Config + Effective Resolution Changes

### `src/config/global.rs`
Add workflow fields:
```rust
#[serde(default)]
pub qa_backend: Option<String>,
#[serde(default)]
pub qa_enabled: bool,
#[serde(default = "default_max_qa_iterations")]
pub max_qa_iterations: u32,
```

Add model field:
```rust
pub qa: Option<String>,
```

Add template field with default path to preserve old configs:
```rust
#[serde(default = "default_qa_template_path")]
pub qa: String, // templates/qa.md
```

Defaults:
1. `qa_enabled = false`
2. `max_qa_iterations = 3`
3. Claude `models.qa = Some("opus")`
4. Codex `models.qa = Some("gpt-5.3-codex-high")`

### `src/config/project.rs`
Add overrides:
```rust
pub qa_backend: Option<String>,
pub qa_enabled: Option<bool>,
pub max_qa_iterations: Option<u32>,
```

### `src/config/mod.rs`
Add to effective workflow/template structs and resolution logic:
1. `qa_backend`, `qa_enabled`, `max_qa_iterations`
2. `templates.qa`
3. CLI/project/global precedence identical to existing role overrides

### `src/cli/config.rs`
Update:
1. `config show/get` payloads include QA workflow/template fields.
2. `config set` supports `workflow.qa_backend`, `workflow.qa_enabled`, `workflow.max_qa_iterations`, `templates.qa`.
3. alias mapping includes `qa_backend -> workflow.qa_backend`.

## Backend Assignment + Role Model Changes

### `src/backend/mod.rs`
Add QA role support:
1. `RoleOverrides` gets `qa: Option<String>`.
2. `BackendRoleModels::for_role()` handles `"qa"`.
3. `BackendRoleModels::fill_from()` fills `qa`.
4. `backend_role_model_specs()` includes `qa`.
5. `assign_feature_backends()` resolves QA backend:
   - default alternating QA backend = planner backend
   - override from `role_overrides.qa` when present

## CLI Surface Changes

### `src/cli/mod.rs`, `src/cli/run.rs`, `src/workflow/orchestrator.rs`
Add `--qa-backend` path end-to-end:
1. `RunArgs.qa_backend`
2. `RunOptions.qa_backend`
3. `RunWorkflowOverrides.qa_backend`
4. plumb into `RoleOverrides.qa`

## Parser + Prompt Template Changes

### `src/workflow/parser.rs`
Add:
```rust
pub enum QaDecision {
    Pass { body: String },
    Fail { body: String },
}
```

Parser contract:
1. H1 must be `# QA: PASS` or `# QA: FAIL`.
2. PASS requires `## Tests Run` and `## Verification Summary`.
3. FAIL requires `## Failures` and `## Suggested Fixes`.

### `src/prompts/templates.rs`
Add `default_qa_template()` and include guardrails:
1. Must run build/tests/acceptance checks when available.
2. Must provide concrete commands/results.
3. Must not edit source files while testing.

### `src/cli/init.rs`
Write `templates/qa.md` during workspace initialization.

## Orchestrator Changes

### Implementing -> QA transition
In `src/workflow/orchestrator.rs`:
1. After `stage_changes_for_review()`, route to `Phase::QA` when `qa_enabled`.
2. Otherwise keep current `Phase::Reviewing` behavior.

### Implementing behavior when QA feedback is pending
If `loop_state.artifacts.pending_qa_feedback` is set:
1. Read QA feedback artifact.
2. Build implementer prompt as feedback response (same parser contract as review response).
3. Write `ImplQaResponse` artifact for current iteration.
4. Attach response path to latest `QaExchange.implementer_response`.
5. Clear `pending_qa_feedback`.
6. Transition back to `Phase::QA` with `phase_iteration + 1`.

### New `Phase::QA` arm
For each QA attempt:
1. If `phase_iteration > max_qa_iterations`, rollback loop and return `QaIterationLimitExceeded` (or continue when `--until-complete`, mirroring review-limit behavior).
2. Resolve QA backend using override precedence and role-model injection.
3. Build QA prompt with:
   - `prompt.md`
   - feature spec
   - impl-notes
   - current git diff
   - prior QA history
4. Execute QA backend via existing `execute_with_parse_retries()`.
5. Parse `QaDecision`.
6. Write `QaPass` or `QaFail` artifact.
7. Update `qa_results`.
8. PASS path: `Phase::Reviewing`, `phase_iteration = 1`.
9. FAIL path: set `pending_qa_feedback`, `Phase::Implementing`, keep iteration for response.

### Acceptance gate in `Phase::Completing`
When completer returns `Complete` and QA is enabled:
1. Run QA again with acceptance-oriented prompt.
2. Write `AcceptancePass` or `AcceptanceFail` artifact.
3. Store result on `CompletionLoopArtifacts`.
4. PASS: keep `ProjectStatus::Completed`.
5. FAIL: force `CompletionVerdict::Continue`, set `ProjectStatus::InProgress`, route to `Phase::Planning`.
6. Ensure planner receives acceptance-failure context on next planning prompt.

### Planner context update for acceptance failures
Update `build_planner_prompt()` to include latest completion feedback section containing:
1. completer verdict artifact (if exists)
2. acceptance-fail artifact (if exists)

This ensures failure details are not lost between completion and next planning loop.

### Supporting updates
1. `phase_label()` mappings in orchestrator + CLI modules include `qa`.
2. `dry_run_summary()` reflects QA backend when enabled.
3. `expected_format_template_for("qa", None)` returns QA format contract.
4. tmux context role label includes `qa`.

## Error Changes

### `src/error.rs`
Add:
```rust
#[error("QA iteration limit exceeded for loop {loop_number}, max={max_iterations}")]
QaIterationLimitExceeded { loop_number: u32, max_iterations: u32 },
```

## UX/Reporting Updates

### `src/cli/status.rs`
1. Show QA in phase label mapping.
2. Show QA backend for active feature loop.
3. Show latest QA result summary when available.

### `src/cli/history.rs`
1. Include QA backend in verbose feature loop output.
2. Include QA attempt count and last QA verdict path.

### `src/cli/project.rs` and `src/cli/tail.rs`
1. Update phase label handling for `Phase::QA`.
2. Ensure artifact parsing/output handles QA and acceptance artifact kinds.

## Files To Modify

| File | Change |
|------|--------|
| `src/project/state.rs` | Add `Phase::QA`, QA artifact fields, QA backend field, acceptance fields |
| `src/project/artifacts.rs` | Add QA/acceptance artifact kinds and filenames |
| `src/config/global.rs` | Add QA workflow/model/template defaults and serde defaults |
| `src/config/project.rs` | Add QA project overrides |
| `src/config/mod.rs` | Add QA fields to effective config and override resolution |
| `src/backend/mod.rs` | Add QA role override/model support and assignment |
| `src/cli/mod.rs` | Add `--qa-backend` |
| `src/cli/run.rs` | Pass QA backend override into run options |
| `src/cli/config.rs` | Expose/set/get QA config keys |
| `src/cli/status.rs` | Phase/backend/summary updates for QA |
| `src/cli/history.rs` | Verbose output includes QA details |
| `src/cli/project.rs` | Phase/backend display includes QA |
| `src/cli/tail.rs` | Phase labels + artifact role handling for QA |
| `src/workflow/parser.rs` | Add `QaDecision` + parser + tests |
| `src/workflow/orchestrator.rs` | QA phase logic, transitions, acceptance gate, planner context injection |
| `src/prompts/templates.rs` | Add `default_qa_template()` |
| `src/cli/init.rs` | Write `templates/qa.md` |
| `src/error.rs` | Add `QaIterationLimitExceeded` |
| `tests/backend.rs` | QA role model and backend assignment tests |
| `tests/state.rs` | Serialization/invariant tests for new QA fields |
| `tests/orchestrator.rs` | End-to-end QA phase and acceptance-gate tests |
| `src/cli/config.rs` tests | QA keys and alias coverage |

## Test Plan

### Parser/unit tests
1. `parse_qa_output_pass`
2. `parse_qa_output_fail`
3. malformed QA output triggers parse error and parse-retry path

### Backend/config tests
1. role-model injection supports `qa`
2. feature backend assignment includes QA default and override
3. effective config precedence for QA fields (CLI > project > global)
4. old TOML without QA keys deserializes with defaults

### State tests
1. old state JSON without QA fields deserializes cleanly
2. new state JSON round-trip preserves QA fields
3. `Phase::QA` snake_case serialization

### Orchestrator integration tests
1. `qa_disabled_skips_phase`
2. `qa_pass_proceeds_to_review`
3. `qa_fail_retries_implementer_then_passes`
4. `qa_limit_exceeded_rolls_back`
5. `resume_from_phase_qa`
6. `acceptance_gate_fail_overrides_complete_to_continue`
7. `acceptance_gate_pass_keeps_completed`
8. `planner_receives_acceptance_failure_context`

## Implementation Sequence

### Phase 1: Schema and plumbing
1. state/config/backend structs and defaults
2. artifact kinds
3. CLI/config key plumbing

Exit criteria:
1. `cargo test` passes without QA workflow enabled.
2. Existing projects run unchanged.

### Phase 2: QA parser/template + orchestrator loop gate
1. parser/template/init wiring
2. `Phase::QA` transitions and feedback loop
3. QA iteration limit and rollback behavior

Exit criteria:
1. QA pass/fail integration tests pass.
2. Parse-retry works for QA role.

### Phase 3: Acceptance gate
1. completion-time QA execution
2. acceptance artifact persistence
3. planner-context injection on acceptance fail

Exit criteria:
1. acceptance pass/fail tests pass.
2. failed acceptance reliably re-enters planning with context.

### Phase 4: UX and documentation
1. status/history/project/tail output updates
2. docs update (`PLAN.md`/workflow docs) with QA role and config keys

Exit criteria:
1. CLI output is coherent with QA-enabled runs.
2. new config keys are discoverable via `config show/get`.

## Verification Checklist
1. `cargo test`
2. `cargo test --test orchestrator`
3. `cargo test --test backend`
4. `cargo test --test state`
5. Optional: `nix build -L` if Nix workflow is active in CI.

## Definition of Done
1. QA can be enabled per project and globally.
2. QA executes between implementation and review, with retry loop on failures.
3. QA acceptance gate can veto completer `COMPLETE` and route back to planning.
4. All existing non-QA behavior remains unchanged when QA is disabled.
5. Added tests cover parser, config precedence, backend assignment, orchestrator transitions, and compatibility.
