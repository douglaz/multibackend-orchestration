## Summary

Add a "quick-dev" orchestration mode — a leaner alternative to the full 7-phase pipeline for simpler tasks. Quick-dev collapses the workflow to 4 phases: Claude plans+implements in one pass, Codex reviews, Claude applies fixes in a review loop, then both backends do independent final reviews with fresh context. The flow is selected via a `ralph:quick` label on GitHub issues, and exposed through two new CLI commands (`quick-dev-run`, `quick-dev-auto`) that mirror the existing `run`/`auto` pattern.

**Key design decisions addressing review feedback:**

1. **Resume semantics**: A new `quick_dev_phase: Option<QuickDevPhase>` field in `ProjectState` provides durable phase tracking. The quick-dev orchestrator persists this field at each phase transition; `quick-dev-run` reads it to resume from the correct internal phase.
2. **Checkpoint reuse**: The orchestrator calls the public `commit_and_push_phase_transition()` from `git/commit.rs` and `commit_feature_loop()` directly, mapping quick-dev phases to `Phase` variants. The private wrappers in `orchestrator.rs` are not extracted.
3. **Backend assignment**: The orchestrator hardcodes role assignment — Claude for PlanAndImplement/ApplyFixes, Codex for CodexReview — using explicit `implementer_backend` and `reviewer_backend` fields in `QuickDevRunOptions`. If the reviewer backend is unavailable, the orchestrator fails with a clear error (two backends are required).
4. **Final review execution**: Runs sequentially (not `tokio::join!`), matching the existing `run_final_review_phase()` pattern. Each reviewer gets a fresh backend instance with no session reuse.
5. **Guard behavior**: `max_review_iterations` forces transition to FinalReview (loop guard). `max_final_review_retries` force-completes with `ProjectStatus::Completed` and writes a force-complete artifact, matching the existing cap behavior in `run_final_review_phase()`.
6. **PR lifecycle**: The orchestrator never calls `mark_pr_ready()`. PR readiness transitions remain daemon-owned via `handle_pr_flow()`.
7. **Template integration**: Four new template fields are added to `TemplateConfig`, `ProjectTemplateOverrides`, and `EffectiveTemplateConfig` with default paths and inline fallbacks, enabling user-override via `render_template_with_fallback()`.

## Acceptance Criteria

- [ ] `QuickDevPhase` enum in `src/project/state.rs` with variants: `PlanAndImplement`, `CodexReview`, `ApplyFixes`, `FinalReview`
- [ ] `quick_dev_phase: Option<QuickDevPhase>` field added to `ProjectState` with `#[serde(default)]`
- [ ] `QuickDevOrchestrator` at `src/workflow/quick_dev_orchestrator.rs` implementing the 4-phase state machine with review loop (CodexReview ↔ ApplyFixes) and sequential dual-backend final review
- [ ] Quick-dev orchestrator persists `quick_dev_phase` to `ProjectState` at every phase transition, enabling crash-safe resume
- [ ] `quick-dev-run` resumes from persisted `quick_dev_phase` (or starts from `PlanAndImplement` if `None`)
- [ ] `QuickDevRun` and `QuickDevAuto` commands registered in `src/cli/mod.rs` `Commands` enum
- [ ] `src/cli/quick_dev_run.rs` — analogous to `run.rs`, runs orchestration on an existing quick-dev project
- [ ] `src/cli/quick_dev_auto.rs` — analogous to `auto.rs`, generates spec via `QuickPrdPipeline` then runs quick-dev orchestration
- [ ] `dispatch_task()` in `src/daemon/runtime.rs` checks for `ralph:quick` label on the claimed issue and spawns `quick-dev-auto` instead of `auto` (or `quick-dev-run` instead of `run` for resumed projects)
- [ ] `spawn_ralph_quick_dev_auto()` and `spawn_ralph_quick_dev_run()` added to `src/daemon/process.rs` following the existing `spawn_ralph_auto`/`spawn_ralph_run` pattern
- [ ] `ralph:quick` added to `REQUIRED_LABELS` in `src/daemon/github.rs` so it is created during repo bootstrap via `ensure_labels_best_effort_with_gh_bin()`
- [ ] New parser functions in `src/workflow/parser.rs`: `parse_codex_review_output()` → `CodexReviewDecision { ReviewSatisfied | ChangesRequested { suggestions } }` and `parse_quick_final_review_output()` → `QuickFinalReviewDecision { Complete | IssuesFound { issues } }`
- [ ] Prompt builder functions for each phase (PlanAndImplement, CodexReview, ApplyFixes, dual FinalReview) using `render_template_with_fallback()` with configurable template paths
- [ ] Four new template fields (`quick_dev_plan_implement`, `quick_dev_codex_review`, `quick_dev_apply_fixes`, `quick_dev_final_review`) added to `TemplateConfig`, `ProjectTemplateOverrides`, and `EffectiveTemplateConfig`
- [ ] `src/workflow/mod.rs` exports `quick_dev_orchestrator`
- [ ] Quick-dev orchestrator never calls `github::mark_pr_ready()` — PR lifecycle remains daemon-owned
- [ ] Quick-dev orchestrator returns error if reviewer backend is unavailable (two backends required)
- [ ] Existing tests and daemon flows pass without modification

## Technical Approach

### 1. State enum and persistence (`src/project/state.rs`)

Add a new `QuickDevPhase` enum alongside the existing `Phase`. Add a persisted `quick_dev_phase` field to `ProjectState` for crash-safe resume.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuickDevPhase {
    PlanAndImplement,
    CodexReview,
    ApplyFixes,
    FinalReview,
}
```

**`ProjectState` changes:**

```rust
pub struct ProjectState {
    // ... existing fields ...

    /// Quick-dev orchestration phase. `None` for standard-flow projects.
    /// Persisted at every quick-dev phase transition for crash-safe resume.
    #[serde(default)]
    pub quick_dev_phase: Option<QuickDevPhase>,
}
```

**Resume semantics:** When `quick-dev-run` loads a project, it reads `quick_dev_phase`:
- `Some(phase)` → resume from that phase.
- `None` → start from `PlanAndImplement` (fresh run or legacy project).

The quick-dev orchestrator sets `current_phase` to match the phase-mapping table (see §2) at each transition. This keeps `validate_invariants()` satisfied: `current_phase` is always a valid `Phase` variant, and `phase_iteration` is always ≥ 1.

**Phase-iteration semantics:** `phase_iteration` is set to `1` on each quick-dev phase transition (since quick-dev phases are coarser than standard phases). The exception is the ApplyFixes ↔ CodexReview loop, where `phase_iteration` increments on each ApplyFixes re-entry (tracking review iteration count for the `max_review_iterations` guard).

### 2. Orchestrator (`src/workflow/quick_dev_orchestrator.rs`)

New struct `QuickDevOrchestrator` holding a `Workspace` reference, mirroring `Orchestrator` in `orchestrator.rs:144`. Exposes `pub async fn run(&mut self, options: QuickDevRunOptions) -> Result<OrchestrationResult>`.

**`QuickDevRunOptions`:**

```rust
pub struct QuickDevRunOptions {
    pub project: Option<String>,
    pub implementer_backend: Option<String>,  // Claude — plan+implement+fix role
    pub reviewer_backend: Option<String>,     // Codex — review role
    pub skip_commit: bool,
    pub pr_url: Option<String>,
    pub max_review_iterations: u32,           // default 5
    pub max_final_review_retries: u32,        // default 2
}
```

**Backend role enforcement:** The orchestrator resolves two backends at startup:
1. **Implementer** (Claude): `options.implementer_backend` → effective config `implementer_backend` → config `starting_backend` → error.
2. **Reviewer** (Codex): `options.reviewer_backend` → effective config `reviewer_backend` → error with message "quick-dev requires a second backend for review".

If the reviewer backend is the same as the implementer backend, log a warning but proceed (user may intentionally want single-backend mode). If the reviewer backend fails `registry.get_or_create_for_role()`, return `Err` immediately — quick-dev does not fall back to a single-backend flow.

**Phase-to-Phase mapping for git operations:**

Quick-dev phases map to `Phase` variants for `commit_and_push_phase_transition()`:

| Quick-dev transition | `from_phase` | `to_phase` |
|---|---|---|
| start → PlanAndImplement | `Phase::Planning` | `Phase::Implementing` |
| PlanAndImplement → CodexReview | `Phase::Implementing` | `Phase::Reviewing` |
| CodexReview → ApplyFixes | `Phase::Reviewing` | `Phase::Implementing` |
| ApplyFixes → CodexReview | `Phase::Implementing` | `Phase::Reviewing` |
| CodexReview → FinalReview | `Phase::Reviewing` | `Phase::FinalReview` |
| FinalReview → PlanAndImplement (re-loop) | `Phase::FinalReview` | `Phase::Implementing` |
| FinalReview → Complete | `Phase::FinalReview` | `Phase::Completing` |

This mapping ensures the draft-PR watcher (which watches for branch divergence, not specific phase labels) continues to work. The commit message format from `build_ralph_commit_message()` in `git/commit.rs` uses the `Phase` variants directly.

**State machine loop:**

```
PlanAndImplement
  → Set state.quick_dev_phase = Some(PlanAndImplement)
  → Set state.current_phase = Phase::Implementing, phase_iteration = 1
  → Persist state
  → Implementer backend (Claude): combined plan + implement prompt
  → Commit via commit_and_push_phase_transition() using public git::commit API
     (conditional on auto_commit && !skip_commit, same 3-line guard as orchestrator.rs:4354-4356)
  → transition → CodexReview

CodexReview
  → Set state.quick_dev_phase = Some(CodexReview)
  → Set state.current_phase = Phase::Reviewing, phase_iteration = 1
  → Persist state
  → Reviewer backend (Codex): review prompt with diff/code context
  → parse_codex_review_output()
  → if ReviewSatisfied → transition → FinalReview
  → if ChangesRequested → transition → ApplyFixes

ApplyFixes
  → Set state.quick_dev_phase = Some(ApplyFixes)
  → Set state.current_phase = Phase::Implementing, phase_iteration = review_iteration
  → Persist state
  → Implementer backend (Claude): apply suggestions from Codex review
  → Commit via commit_and_push_phase_transition()
  → transition → CodexReview (loops back)
  → Guard: review_iteration >= max_review_iterations (default 5) → force transition to FinalReview
    with log warning "review loop reached iteration cap ({n}/{max}); proceeding to final review"

FinalReview
  → Set state.quick_dev_phase = Some(FinalReview)
  → Set state.current_phase = Phase::FinalReview, phase_iteration = 1
  → Persist state
  → Run two independent backend calls SEQUENTIALLY (not tokio::join!)
    matching the existing run_final_review_phase() pattern at orchestrator.rs:3429
  → Each gets a fresh backend instance via registry.get_or_create_for_role()
    with role="quick_final_reviewer" (not reusing any existing session)
  → No session_store lookup — enforce fresh context by generating a unique session_id per call
  → Each receives full diff against base branch (not incremental)
  → parse_quick_final_review_output() on each
  → Both Complete → set ProjectStatus::Completed, state.current_phase = Phase::Completing
  → Either IssuesFound → transition back to PlanAndImplement (re-loop)
  → Guard: final_review_attempt >= max_final_review_retries (default 2)
    → write force-complete artifact (matching orchestrator.rs:3404 pattern)
    → set ProjectStatus::Completed, state.current_phase = Phase::Completing
    → log: "final review reached retry cap ({n}/{max}); force-completing project"
```

**Commit and push mechanics — no extraction needed:**

The orchestrator calls these public functions directly from `git/commit.rs`:
- `commit_and_push_phase_transition(repo_root, project_id, loop_number, from_phase, to_phase, branch, sign_commits)` — for phase transition commits with push
- `commit_feature_loop(repo_root, message, None, sign_commits)` — for intermediate commits without push

The 3-line auto-commit guard is replicated inline:
```rust
if !effective.workflow.auto_commit || options.skip_commit { return Ok(()); }
if !is_git_repo(repo_root) { return Ok(()); }
```

This avoids extracting or changing the private `commit_phase_checkpoint_if_enabled()` wrapper in `orchestrator.rs`.

**Reused infrastructure (no changes to these):**
- `commit_and_push_phase_transition()` from `git/commit.rs` (already public)
- `commit_feature_loop()` from `git/commit.rs` (already public)
- `stage_implementation_changes()` from `git/commit.rs`
- `BackendRegistry` for backend instantiation
- `ProjectState` / `FeatureLoopState` for persistence
- `github::push_branch()` at `github.rs:905` for explicit push operations
- Daemon `handle_pr_flow()` at `runtime.rs:2892` for PR creation and mark-ready transitions

**Not called by the orchestrator (daemon-owned):**
- `github::mark_pr_ready()` — the daemon calls this in `handle_pr_flow()` when the child process completes with `ralph:completed` label. The quick-dev orchestrator sets `ProjectStatus::Completed` and exits; the daemon handles the rest.

### 3. CLI commands

**`src/cli/quick_dev_auto.rs`** — follows `auto.rs` pattern exactly:
1. Parse `QuickDevAutoArgs` (same shape as `AutoArgs`: `--idea`, `--implementer-backend`, `--reviewer-backend`, `--project-id`, `--pr-url`, `--workspace-root`, `--skip-commit`, `--max-review-iterations`, `--max-final-review-retries`)
2. Run `QuickPrdPipeline` to generate spec (reuses existing `src/prd/quick.rs`)
3. Call `create_project()` from `src/project/lifecycle.rs`
4. Set `state.quick_dev_phase = Some(QuickDevPhase::PlanAndImplement)` and persist
5. Instantiate `QuickDevOrchestrator` and call `.run()`

**`src/cli/quick_dev_run.rs`** — follows `run.rs` pattern:
1. Parse `QuickDevRunArgs` (`--project`, `--implementer-backend`, `--reviewer-backend`, `--pr-url`, `--workspace-root`, `--skip-commit`, `--max-review-iterations`, `--max-final-review-retries`)
2. Load existing project from disk
3. Read `state.quick_dev_phase` to determine resume point
4. Instantiate `QuickDevOrchestrator`, call `.run()`

**Registration in `src/cli/mod.rs`:**
```rust
mod quick_dev_auto;
mod quick_dev_run;

pub enum Commands {
    // ... existing variants ...
    QuickDevRun(quick_dev_run::QuickDevRunArgs),
    QuickDevAuto(quick_dev_auto::QuickDevAutoArgs),
}
```

Add match arms in `pub async fn run(cli: Cli)` at `mod.rs:286`.

### 4. Daemon integration

**`src/daemon/runtime.rs` — `dispatch_task()`:**

After claiming the issue (line ~1159) and before spawning the child process (line ~1604), check if `issue.labels` contains `"ralph:quick"`. The `GhIssue` struct at `github.rs:48` already carries `labels: Vec<String>`, and the issue is available in `poll_and_claim()`. Thread the label list into `dispatch_task()` by adding an `issue_labels: &[String]` parameter.

```rust
// In dispatch_task(), replacing lines 1604-1628:
let has_quick_label = issue_labels.iter().any(|l| l == "ralph:quick");
let spawned = if has_quick_label {
    if resume_existing_project {
        process::spawn_ralph_quick_dev_run(
            &ralph_bin, &wt, &project_id, &log_path, pr_url.as_deref(),
        ).await?
    } else {
        process::spawn_ralph_quick_dev_auto(
            &ralph_bin, &wt, &idea_clone, &log_path,
            Some(&project_id), pr_url.as_deref(),
        ).await?
    }
} else {
    if resume_existing_project {
        process::spawn_ralph_run(...).await?
    } else {
        process::spawn_ralph_auto(...).await?
    }
};
```

The `ralph:quick` label is a flow-type marker — it is never removed or swapped by the lifecycle label machinery. `poll_and_claim()` continues to filter on `ralph:ready`; `ralph:quick` is simply an additional label present on the issue.

**`src/daemon/process.rs`:**

Add `spawn_ralph_quick_dev_auto()` and `spawn_ralph_quick_dev_run()` functions following the exact same pattern as `spawn_ralph_auto()` (line 27) and `spawn_ralph_run()` (line 73). The only difference is the subcommand arg: `"quick-dev-auto"` / `"quick-dev-run"` instead of `"auto"` / `"run"`.

Also add corresponding `build_ralph_quick_dev_auto_command()` and `build_ralph_quick_dev_run_command()` helpers following `build_ralph_auto_command()` (line 113) and `build_ralph_run_command()` (line 148).

**`src/daemon/github.rs`:**

Add `ralph:quick` to `REQUIRED_LABELS` (line 28):
```rust
("ralph:quick", "#5319e7", "Use quick-dev orchestration flow"),
```

This is not a lifecycle label — it does NOT appear in `LIFECYCLE_LABELS` and is never swapped. It is purely a flow-type marker that `dispatch_task()` reads to decide which orchestrator to spawn.

### 5. Parsers (`src/workflow/parser.rs`)

Two new parse functions, following the existing H1-header-based pattern (matching `parse_reviewer_output` at line 143 and `parse_final_reviewer_output` at line 411):

```rust
#[derive(Debug, Clone)]
pub enum CodexReviewDecision {
    ReviewSatisfied { body: String },
    ChangesRequested { suggestions: String },
}

pub fn parse_codex_review_output(raw: &str) -> Result<CodexReviewDecision> {
    // 1. strip_frontmatter(raw)
    // 2. first_h1_line(&body)
    // 3. Match "# Review: SATISFIED" → ReviewSatisfied { body }
    //    Match "# Review: CHANGES REQUESTED" → ChangesRequested { suggestions: body }
    // 4. Else → Err with descriptive parse error
}

#[derive(Debug, Clone)]
pub enum QuickFinalReviewDecision {
    Complete { body: String },
    IssuesFound { issues: String },
}

pub fn parse_quick_final_review_output(raw: &str) -> Result<QuickFinalReviewDecision> {
    // 1. strip_frontmatter(raw)
    // 2. first_h1_line(&body)
    // 3. Match "# Final Review: COMPLETE" → Complete { body }
    //    Match "# Final Review: ISSUES FOUND" → IssuesFound { issues: body }
    // 4. Else → Err with descriptive parse error
}
```

Both parsers use case-insensitive matching on the H1 keyword (SATISFIED/CHANGES REQUESTED/COMPLETE/ISSUES FOUND) but require the exact `# Review:` or `# Final Review:` prefix. This matches the strictness level of existing parsers.

### 6. Prompt builders and template integration

**Config changes — four new template fields:**

`src/config/global.rs` — `TemplateConfig`:
```rust
pub struct TemplateConfig {
    // ... existing fields ...
    #[serde(default = "default_quick_dev_plan_implement_template_path")]
    pub quick_dev_plan_implement: String,
    #[serde(default = "default_quick_dev_codex_review_template_path")]
    pub quick_dev_codex_review: String,
    #[serde(default = "default_quick_dev_apply_fixes_template_path")]
    pub quick_dev_apply_fixes: String,
    #[serde(default = "default_quick_dev_final_review_template_path")]
    pub quick_dev_final_review: String,
}
```

Default path functions follow the existing pattern (e.g., `fn default_quick_dev_plan_implement_template_path() -> String { "quick_dev_plan_implement.md".to_owned() }`).

`src/config/project.rs` — `ProjectTemplateOverrides`:
```rust
pub struct ProjectTemplateOverrides {
    // ... existing fields ...
    pub quick_dev_plan_implement: Option<String>,
    pub quick_dev_codex_review: Option<String>,
    pub quick_dev_apply_fixes: Option<String>,
    pub quick_dev_final_review: Option<String>,
}
```

`src/config/mod.rs` — `EffectiveTemplateConfig`:
```rust
pub struct EffectiveTemplateConfig {
    // ... existing fields ...
    pub quick_dev_plan_implement: PathBuf,
    pub quick_dev_codex_review: PathBuf,
    pub quick_dev_apply_fixes: PathBuf,
    pub quick_dev_final_review: PathBuf,
}
```

Resolution follows the existing pattern: project override → global config → default path, resolved relative to workspace templates directory.

**Prompt builder functions in `src/prompts/quick_dev.rs`** (new file):

Each function calls `render_template_with_fallback(path, vars, fallback)`:

- **`build_quick_dev_plan_implement_prompt(effective, state, prompt_content) -> Result<String>`**: Combined planning + implementation prompt. Receives the project spec/prompt. Instructs Claude to plan and implement in a single pass, outputting `# Implementation Notes`. Template vars: `{{spec}}`, `{{project_id}}`, `{{backend}}`.
- **`build_quick_dev_codex_review_prompt(effective, state, diff, spec_content) -> Result<String>`**: Review prompt for Codex. Receives the diff and project spec. Instructs Codex to review and output `# Review: SATISFIED` or `# Review: CHANGES REQUESTED`. Template vars: `{{spec}}`, `{{diff}}`, `{{project_id}}`, `{{backend}}`, `{{iteration}}`.
- **`build_quick_dev_apply_fixes_prompt(effective, state, suggestions) -> Result<String>`**: Fix prompt for Claude. Receives Codex's suggestions. Instructs Claude to apply fixes and output `# Implementation Response`. Template vars: `{{suggestions}}`, `{{project_id}}`, `{{backend}}`, `{{iteration}}`.
- **`build_quick_dev_final_review_prompt(effective, state, diff, spec_content, backend, opposite_backend) -> Result<String>`**: Fresh-context final review. Receives full diff against base. Outputs `# Final Review: COMPLETE` or `# Final Review: ISSUES FOUND`. Template vars: `{{spec}}`, `{{diff}}`, `{{project_id}}`, `{{backend}}`, `{{opposite_backend}}`. Same template used for both Claude and Codex calls (backend name injected via vars).

All prompts include `CRITICAL FORMAT REQUIREMENTS` sections matching the existing template pattern, specifying the exact H1 header format expected by the corresponding parser.

### 7. `src/prompts/mod.rs`

Add `pub mod quick_dev;` to re-export the new prompts submodule.

## Files & Modules

| File | Action | Description |
|------|--------|-------------|
| `src/project/state.rs` | Edit | Add `QuickDevPhase` enum and `quick_dev_phase: Option<QuickDevPhase>` field to `ProjectState` |
| `src/workflow/quick_dev_orchestrator.rs` | New | 4-phase state machine orchestrator with resume support |
| `src/workflow/mod.rs` | Edit | Add `pub mod quick_dev_orchestrator;` |
| `src/workflow/parser.rs` | Edit | Add `CodexReviewDecision`, `QuickFinalReviewDecision`, and their parse functions |
| `src/prompts/quick_dev.rs` | New | Prompt builder functions for all 4 phases |
| `src/prompts/mod.rs` | Edit | Add `pub mod quick_dev;` |
| `src/config/global.rs` | Edit | Add 4 quick-dev template fields to `TemplateConfig` with default path functions |
| `src/config/project.rs` | Edit | Add 4 quick-dev template fields to `ProjectTemplateOverrides` |
| `src/config/mod.rs` | Edit | Add 4 quick-dev template fields to `EffectiveTemplateConfig`; add resolution logic in `resolve_effective_config()` |
| `src/cli/quick_dev_auto.rs` | New | `QuickDevAutoArgs` + `execute()` (mirrors `auto.rs`) |
| `src/cli/quick_dev_run.rs` | New | `QuickDevRunArgs` + `execute()` (mirrors `run.rs`) with resume from `quick_dev_phase` |
| `src/cli/mod.rs` | Edit | Register `QuickDevRun`, `QuickDevAuto` in `Commands` enum and `run()` dispatcher |
| `src/daemon/runtime.rs` | Edit | Add `issue_labels` param to `dispatch_task()`; branch on `ralph:quick` label to spawn quick-dev vs. standard flow |
| `src/daemon/process.rs` | Edit | Add `spawn_ralph_quick_dev_auto()`, `spawn_ralph_quick_dev_run()`, and their `build_*_command()` helpers |
| `src/daemon/github.rs` | Edit | Add `ralph:quick` to `REQUIRED_LABELS` |

## Testing Strategy

### Unit tests

1. **Parser tests in `src/workflow/parser.rs`** — following the existing `parse_reviewer_output` test pattern:
   - `parse_codex_review_output`: valid SATISFIED, valid CHANGES REQUESTED, missing H1, wrong H1 prefix, empty body, frontmatter stripping
   - `parse_quick_final_review_output`: valid COMPLETE, valid ISSUES FOUND, missing H1, wrong H1 prefix, empty body, frontmatter stripping
   - Edge cases: mixed case keywords (should reject — exact match), trailing whitespace after keyword, body with multiple H1s (only first counts)

2. **Process command tests in `src/daemon/process.rs`** — following the existing `spawn_command_uses_long_idea_flag` pattern at `process.rs:317`:
   - `build_ralph_quick_dev_auto_command()`: verify subcommand is `quick-dev-auto`, `--idea` flag present, `--workspace-root` flag present, optional `--project-id` and `--pr-url` flags
   - `build_ralph_quick_dev_run_command()`: verify subcommand is `quick-dev-run`, `--project` flag present, optional `--pr-url` flag

3. **CLI parse tests in `src/cli/mod.rs`** — following the existing `parses_auto_command_with_defaults` pattern at `mod.rs:359`:
   - `quick-dev-auto --idea "test"` parses to `Commands::QuickDevAuto`
   - `quick-dev-run --project issue-1` parses to `Commands::QuickDevRun`
   - Verify `--implementer-backend`, `--reviewer-backend`, `--max-review-iterations`, `--max-final-review-retries` flags parse correctly

4. **Label tests in `src/daemon/github.rs`** — extend `required_labels_are_unique_and_include_lifecycle_labels` test:
   - Verify `ralph:quick` is present in `REQUIRED_LABELS`
   - Verify `ralph:quick` is NOT present in `LIFECYCLE_LABELS`

5. **State persistence tests in `src/project/state.rs`**:
   - Serialize/deserialize `ProjectState` with `quick_dev_phase: Some(CodexReview)` — verify round-trip
   - Deserialize legacy JSON without `quick_dev_phase` — verify defaults to `None`
   - `validate_invariants()` passes with `quick_dev_phase: Some(FinalReview)` and `current_phase: Phase::FinalReview`

6. **Template config tests in `src/config/mod.rs`**:
   - `resolve_effective_config()` resolves quick-dev template paths from global config defaults
   - Project-level overrides take precedence over global defaults

### Integration / conformance tests

7. **Quick-dev e2e test in `src/validate/tests_quick_dev.rs`** — new file, following the existing e2e pattern in `tests_e2e_conformance.rs`:
   - **Happy path**: PlanAndImplement → CodexReview(satisfied) → FinalReview(both complete) → Completed. Uses mock backends returning canned outputs.
   - **Review loop**: PlanAndImplement → CodexReview(changes requested) → ApplyFixes → CodexReview(satisfied) → FinalReview(both complete) → Completed.
   - **Final review re-loop**: FinalReview(Claude finds issues) → PlanAndImplement → ... → FinalReview(both complete) → Completed.
   - **Max review iterations guard**: ApplyFixes/CodexReview loops `max_review_iterations` times → forced transition to FinalReview.
   - **Max final review retries guard**: FinalReview re-loops `max_final_review_retries` times → force-complete with artifact.

8. **Resume mid-quick-dev tests in `src/validate/tests_quick_dev.rs`**:
   - Set `quick_dev_phase = Some(CodexReview)` in persisted state → run `quick-dev-run` → verify orchestrator resumes at CodexReview (skips PlanAndImplement).
   - Set `quick_dev_phase = Some(FinalReview)` → run `quick-dev-run` → verify orchestrator resumes at FinalReview.
   - Set `quick_dev_phase = None` → run `quick-dev-run` → verify orchestrator starts from PlanAndImplement.

9. **Daemon dispatch branching test in `src/validate/tests_daemon.rs`**:
   - Issue with labels `["ralph:ready", "ralph:quick"]` → verify `dispatch_task()` spawns `quick-dev-auto` (check spawned command args).
   - Issue with labels `["ralph:ready"]` (no `ralph:quick`) → verify `dispatch_task()` spawns standard `auto`.
   - Resumed project with `ralph:quick` → verify `dispatch_task()` spawns `quick-dev-run`.

10. **Backend-unavailable test in `src/validate/tests_quick_dev.rs`**:
    - Configure reviewer backend as an invalid/unavailable backend spec → verify `QuickDevOrchestrator::run()` returns `Err` with message containing "quick-dev requires a second backend".

11. **Regression**: Run the full existing test suite (`cargo test`) to confirm no breakage. The `#[serde(default)]` attribute on `quick_dev_phase` ensures existing serialized state deserializes without error.

## Out of Scope

- Modifying the existing `Phase` enum or `Orchestrator` — the quick-dev flow is a parallel code path
- Extracting or changing visibility of `commit_phase_checkpoint_if_enabled()` or `checkpoint_phase_transition()` — the quick-dev orchestrator uses the public `git/commit.rs` functions directly
- New git plumbing — reuses `commit_and_push_phase_transition()`, `commit_feature_loop()`, `push_branch()` unchanged
- Changes to `poll_and_claim()` logic — label filtering stays the same; `ralph:quick` is orthogonal to lifecycle labels
- Changes to `handle_pr_flow()` — PR creation/readiness transitions remain daemon-owned; the orchestrator does not call `mark_pr_ready()`
- Interactive PRD integration for quick-dev — uses `QuickPrdPipeline` only
- Configurable phase skipping (e.g., skip final review) — can be added later as flags on `QuickDevRunOptions`
- Parallel final review execution (`tokio::join!`) — sequential execution avoids shared mutable state issues with `BackendRegistry`; can be revisited if performance becomes a concern
- Dashboard or UI changes
- Single-backend fallback mode — quick-dev requires two backends by design; single-backend users should use the standard flow