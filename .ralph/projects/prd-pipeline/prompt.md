# PRD Pipeline Command (`ralph prd`)

## Overview

Add a new `ralph prd` command — an interactive wizard that generates Product Requirements Documents through a multi-stage LLM pipeline: **Ideation → Research → Synthesis → PRD**. Between stages, it detects information gaps, asks the user targeted questions, and reruns stages as needed. This is a standalone workflow separate from the existing orchestrator.

## Architecture

New top-level `src/prd/` module (parallel to `src/workflow/`). The PRD pipeline has its own state machine and own artifact files. It reuses the existing `Backend` trait for LLM calls and `BackendRegistry` for backend management.

**Key design decisions:**
- Single backend per run (no alternation — linear document generation, not adversarial review)
- Does NOT require a ralph project — operates on CWD, only needs workspace for backend config
- In-process interactive mode (stdin/stdout) first; tmux as future optional work
- Add `serde_yaml` dependency for user-editable answers file
- Cache under `.ralph/prd/<idea_hash>/` — avoids interfering with ralph's clean-tree checks (which only exclude `.ralph/`), and namespaces by idea to prevent cross-contamination between runs
- Auto-detect non-TTY stdin — if stdin is not a terminal, auto-switch to non-interactive mode (unless `--interactive` is explicitly passed). Use `std::io::IsTerminal` (stable since Rust 1.70)
- Typed stage keys — use `BTreeMap<Stage, String>` (derive `Ord` on `Stage`) instead of stringly-typed maps
- Rerun targeting via `Question.impact_stage: Stage` — LLM gap report includes typed stage impact per question; no keyword heuristics
- V1 scope: defer RepairPrd — ship without auto-repair; on validation failure, return actionable missing-info report. Add repair in v2
- Lock file — reuse `fs2` lock pattern from `src/util/lock.rs` to prevent parallel `ralph prd` cache corruption

## New Files

```
src/prd/
  mod.rs              — module root, re-exports
  pipeline.rs         — PrdPipeline struct, state machine driver
  state.rs            — Stage enum, PrdPhase enum, PipelineContext, PrdMeta
  stages.rs           — prompt builders per stage, output parsers
  gaps.rs             — GapReport, Question types, deterministic + LLM gap checker
  interaction.rs      — UserInteraction trait, PlainInteraction (stdin), MockInteraction
  answers.rs          — AnswerStore: load/save YAML, merge, hash, category classification
  cache.rs            — CacheManager: read/write .ralph/prd/<idea_hash>/ artifacts, hash-based skip

src/cli/prd.rs        — PrdArgs, execute() function
tests/prd.rs          — Integration tests with MockBackend
```

## Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Add `serde_yaml = "0.9"` |
| `src/lib.rs` | Add `pub mod prd;` |
| `src/cli/mod.rs` | Add `mod prd;`, `Prd(PrdArgs)` to Commands, dispatch |
| `src/error.rs` | Add `PrdPipelineFailed`, `PrdValidationFailed`, `PrdMissingInfo`, `Yaml` error variants with exit codes 10/11/12 |

## Key Types

### Stage + State Machine (`src/prd/state.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Stage { Ideation, Research, Synthesis, Prd }

impl Stage {
    pub fn all() -> &'static [Stage];
    pub fn index(&self) -> usize;           // 0..3
    pub fn artifact_filename(&self) -> &str; // "01_ideation.md", etc.
}

pub enum PrdPhase {
    RunStage(Stage),
    CheckGaps(Stage),
    AskUser(Vec<Question>),
    ApplyAnswers,
    MaybeRerun(Stage),
    ValidatePrd,
    Done,
}

pub struct PipelineContext {
    pub idea: String,
    pub answers: BTreeMap<String, String>,
    pub stage_outputs: BTreeMap<Stage, String>,
    pub stage_input_hashes: BTreeMap<Stage, String>,
    pub answers_hash: String,
    pub question_rounds: u32,
}

pub struct PrdMeta {
    pub idea: String,
    pub idea_hash: String,
    pub backend: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub stage_timings: BTreeMap<Stage, f64>,
    pub question_rounds: u32,
    pub rerun_stages: Vec<Stage>,
}
```

### Gap Analysis (`src/prd/gaps.rs`)

```rust
pub struct GapReport {
    pub missing_fields: Vec<MissingField>,
    pub ambiguities: Vec<Ambiguity>,
    pub questions: Vec<Question>,
    pub suggested_defaults: Vec<SuggestedDefault>,
}

pub struct MissingField {
    pub field: String,
    pub description: String,
}

pub struct Ambiguity {
    pub area: String,
    pub description: String,
}

pub struct SuggestedDefault {
    pub key: String,
    pub value: String,
    pub rationale: String,
}

pub struct Question {
    pub key: String,
    pub prompt: String,
    pub kind: QuestionKind,
    pub suggested_default: Option<String>,
    pub impact_stage: Stage,  // typed — determines rerun scope directly
}

pub enum QuestionKind {
    FreeText,
    Choice(Vec<String>),
    YesNo,
}
```

Gap checking is two-pass:
1. **Deterministic** — verify expected section headings exist in stage output
2. **LLM gap analysis** — send output + gap-analysis prompt requesting fenced JSON (`\`\`\`json ... \`\`\``). Parse `GapReport` using extract-from-fence + `serde_json::from_str`. On parse failure, retry up to 3 attempts with reformat prompt (similar to `execute_with_parse_retries` pattern in the orchestrator).

### Interaction (`src/prd/interaction.rs`)

```rust
#[async_trait]
pub trait UserInteraction: Send + Sync {
    async fn ask_questions(&self, questions: &[Question], ctx: &InteractionContext)
        -> Result<Option<BTreeMap<String, String>>>;
    fn status(&self, message: &str);
    fn stage_complete(&self, stage: &Stage, summary: &str);
}

pub struct InteractionContext {
    pub stage: Stage,
    pub question_round: u32,
    pub max_rounds: u32,
}
```

Implementations:
- `PlainInteraction` — stdin/stdout with `:back`, `:edit`, `:show`, `:save`, `:quit` commands. Use `tokio::task::spawn_blocking` for synchronous stdin reads.
- `NonInteractiveInteraction` — always returns `None` (no answers).
- `MockInteraction` — canned answers for testing. Takes `Vec<Option<BTreeMap<String, String>>>` and pops from front on each call.

### Answers (`src/prd/answers.rs`)

`AnswerStore` — load/save YAML at `.ralph/prd/<idea_hash>/answers.yaml`, merge new answers, compute sha256 hash.

Rerun targeting: each `Question` carries a typed `impact_stage: Stage`. When answers change, the rerun start is `min(impact_stage)` across all answered questions in that round. No keyword heuristics needed.

### Cache (`src/prd/cache.rs`)

`CacheManager` at `.ralph/prd/<idea_hash>/`:
- `new(workspace_root, idea)` — computes `idea_hash = sha256_hex(&idea)[..12]`, creates dir
- `acquire_lock()` — `fs2` file lock on `.ralph/prd/<idea_hash>/.lock` (reuse `src/util/lock.rs` pattern). Returns a `PrdLock` RAII guard.
- `read_stage_output(stage)` / `write_stage_output(stage, content)` — files: `01_ideation.md`, `02_research.md`, `03_synthesis.md`, `04_prd.md`
- `read_meta()` / `write_meta(meta)` — `meta.json`
- `write_missing_info_report(report)` — `missing_info_report.md`
- `should_skip_stage(stage, context)` — compare `stage_input_hash` (sha256 of idea + relevant answers + prior stage outputs) against stored hash
- On `--resume`: validate that `meta.json` idea matches current `--idea`; reject if mismatched

### Pipeline Driver (`src/prd/pipeline.rs`)

```rust
pub struct PrdPipeline {
    backend: Arc<dyn Backend>,
    interaction: Box<dyn UserInteraction>,
    cache: CacheManager,
    answer_store: AnswerStore,
    context: PipelineContext,
    meta: PrdMeta,
    options: PrdOptions,
}
```

State machine loop in `run() -> Result<PrdResult>`:
- `RunStage(s)` — if resume & hash matches, skip; else build prompt via `stages.rs`, call `backend.execute()`, write cache → `CheckGaps(s)`
- `CheckGaps(s)` — run deterministic check then LLM gap checker; if empty, advance to next stage; if non-interactive or max rounds exceeded, write missing_info_report, return `PrdMissingInfo` (exit 12); else → `AskUser(questions)`
- `AskUser(qs)` — increment question_rounds, call `interaction.ask_questions()`; if user quits → return `PrdPipelineFailed` (exit 10); if returns None → advance (accept defaults); else → `ApplyAnswers`
- `ApplyAnswers` — merge answers into answer_store, save YAML, compute new hash, determine rerun stage as `min(q.impact_stage)` across answered questions → `MaybeRerun(stage)`
- `MaybeRerun(s)` — invalidate outputs from `s` onward in context, → `RunStage(s)`
- After `CheckGaps(Prd)` passes → `ValidatePrd`
- `ValidatePrd` — LLM validation call; if valid → `Done`; if missing info → convert to questions and loop back; else → return `PrdValidationFailed` (exit 11) with actionable report (no auto-repair in v1)
- `Done` — write final `meta.json`, copy `04_prd.md` to CWD as `PRD.md`, return `PrdResult`

```rust
pub struct PrdResult {
    pub prd_path: PathBuf,
    pub cache_dir: PathBuf,
    pub meta: PrdMeta,
    pub summary: String,
}
```

### CLI (`src/cli/prd.rs`)

```rust
#[derive(Debug, Args)]
pub struct PrdArgs {
    #[arg(long)]
    pub idea: String,
    #[arg(long, conflicts_with = "interactive")]
    pub non_interactive: bool,
    #[arg(long, conflicts_with = "non_interactive")]
    pub interactive: bool,
    #[arg(long, default_value_t = 3)]
    pub ask_max: u32,
    #[arg(long)]
    pub answers: Option<PathBuf>,
    #[arg(long)]
    pub resume: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub backend: Option<String>,
}
```

`execute()` pattern follows `src/cli/run.rs`:
1. `Workspace::discover()`
2. Create `BackendRegistry::new(&config, tmux_config)` with tmux disabled
3. Resolve backend spec: `args.backend.unwrap_or(config.workspace.default_backend)`, validate via `backend_spec::validate_backend_spec()` from `src/cli/backend_spec.rs`
4. Get backend via `registry.get_or_create_for_spec(&spec)` (not `.get()`)
5. Health check on the resolved backend
6. Auto-detect TTY: if `!std::io::stdin().is_terminal() && !args.interactive` → force non-interactive
7. Create `PlainInteraction` or `NonInteractiveInteraction`
8. If `args.answers` provided, pre-load into `AnswerStore`
9. Create `PrdPipeline`, call `.run().await`
10. Print result summary + path to output PRD

### Error Variants (`src/error.rs`)

Add these variants to `RalphError`:

```rust
#[error("PRD pipeline failed: {0}")]
PrdPipelineFailed(String),    // exit code 10

#[error("PRD validation failed: {0}")]
PrdValidationFailed(String),  // exit code 11

#[error("PRD missing information — see missing_info_report.md")]
PrdMissingInfo,               // exit code 12

#[error("yaml error: {0}")]
Yaml(#[from] serde_yaml::Error),

#[error("PRD cache mismatch: {0}")]
PrdCacheMismatch(String),     // exit code 2
```

Update `exit_code()`:
```rust
Self::PrdPipelineFailed(_) => 10,
Self::PrdValidationFailed(_) => 11,
Self::PrdMissingInfo => 12,
Self::PrdCacheMismatch(_) => 2,
```

## Stage Prompts (embedded in `src/prd/stages.rs`)

Each stage prompt is a `const &str` with `{{variable}}` placeholders. Render by simple string replacement (same pattern as `render_template` but inline, no file I/O needed). Each includes:
- Role assignment
- The idea + accumulated answers
- Prior stage outputs (Research gets Ideation, Synthesis gets both, PRD gets all three)
- Required output format (section headings)

### Ideation Stage Prompt
Role: Product ideation specialist. Input: idea + answers. Output sections: `## Core Concept`, `## Target Users`, `## Key Problems Solved`, `## Proposed Features`, `## Success Metrics`, `## Constraints & Assumptions`.

### Research Stage Prompt
Role: Technical research analyst. Input: idea + answers + ideation output. Output sections: `## Market Context`, `## Technical Landscape`, `## Comparable Solutions`, `## Technical Feasibility`, `## Risk Assessment`.

### Synthesis Stage Prompt
Role: Product strategist. Input: idea + answers + ideation + research outputs. Output sections: `## Product Vision`, `## User Stories`, `## Feature Prioritization`, `## Architecture Overview`, `## MVP Scope`, `## Open Questions`.

### PRD Stage Prompt
Role: Technical product manager. Input: idea + answers + all prior outputs. Output sections: `## Executive Summary`, `## Goals & Non-Goals`, `## User Stories`, `## Functional Requirements`, `## Non-Functional Requirements`, `## Technical Architecture`, `## Data Model`, `## API Design`, `## Security Considerations`, `## Testing Strategy`, `## Rollout Plan`, `## Success Metrics`, `## Open Questions`.

### Gap Analysis Prompt
Role: Requirements analyst. Input: stage output + stage name. Output: fenced JSON block with `GapReport` structure. Must include typed `impact_stage` per question.

### Validation Prompt
Role: PRD reviewer. Input: final PRD + idea + all answers. Output: fenced JSON with `{"valid": bool, "issues": [...]}`. Issues include field name and description of what's missing/unclear.

## Reusable Utilities

| Utility | Location | Usage |
|---------|----------|-------|
| `sha256_hex()` | `src/util/hash.rs` | Cache hash computation |
| `now_utc()`, `now_iso8601()` | `src/util/time.rs` | Meta timestamps |
| `strip_frontmatter()` | `src/workflow/parser.rs` | Stage output cleaning |
| `first_h1_line()` | `src/workflow/parser.rs` | Stage output validation |
| `Backend` trait | `src/backend/mod.rs` | LLM invocations |
| `BackendRegistry` | `src/backend/mod.rs` | Backend creation/config |
| `parse_backend_spec()` | `src/backend/mod.rs` | Backend spec parsing |
| `validate_backend_spec()` | `src/cli/backend_spec.rs` | Validate backend spec against config |
| `Workspace::discover()` | `src/workspace/mod.rs` | Workspace/config loading |
| `RalphError` + `exit_code()` | `src/error.rs` | Error handling pattern |
| `MockBackend` | `src/backend/mock.rs` | Testing |
| `BackendRegistryTmuxConfig` | `src/backend/mod.rs` | Registry constructor (use `enabled: false`) |

## Implementation Order

Tests should be written alongside each phase, not deferred to the end.

1. **Scaffolding + CLI wiring** — `Cargo.toml` deps (`serde_yaml`), `src/lib.rs` mod, `src/prd/mod.rs`, error variants in `src/error.rs`, `PrdArgs` in `src/cli/mod.rs`, `src/cli/prd.rs` with skeleton `execute()`. Verify: `nix build` compiles with `ralph prd --help`.

2. **Core types + unit tests** — `state.rs` (Stage with Ord, PrdPhase, PipelineContext, PrdMeta), `gaps.rs` (GapReport, Question with typed impact_stage). Unit tests for Stage ordering, serialization.

3. **Cache + lock + answers** — `cache.rs` (namespaced under `.ralph/prd/<idea_hash>/`, file I/O, lock via fs2, hash-based skip, resume validation), `answers.rs` (YAML load/save, merge, hash). Unit tests for cache roundtrip, answers merge, hash computation.

4. **Stages + deterministic parsers** — `stages.rs` (4 stage prompts, gap analysis prompt, validation prompt, output parsers). Unit tests for prompt construction, output parsing.

5. **Interaction layer** — `interaction.rs` (trait, PlainInteraction with :commands + TTY detection, NonInteractiveInteraction, MockInteraction). Unit tests for command parsing.

6. **Pipeline + non-interactive happy path** — `pipeline.rs` (state machine loop). Integration test in `tests/prd.rs` with MockBackend + MockInteraction: stages run sequentially, no gaps, produces PRD. Also test non-interactive mode (exit code 12 on gaps).

7. **Gap analysis + interactive flow** — Wire LLM gap analysis with fenced JSON + parse retries. Integration tests: gap detected → questions → answers → rerun from correct stage. Max question rounds exceeded → exit 12.

8. **Validation + resume** — Wire ValidatePrd phase. Integration tests: validation pass → Done, validation fail → exit 11 with report. Resume: cached outputs skipped correctly, idea mismatch rejected.

## Verification

The project builds and all tests pass via `nix build`. The `postPatch` in `flake.nix` already replaces `#!/usr/bin/env bash` in test files for sandbox compatibility — any new test files using bash scripts will need the same treatment (though this feature likely won't need bash in tests).

## Scope Boundaries

- Do NOT modify the existing orchestrator workflow
- Do NOT add tmux integration for the PRD pipeline (future work)
- Do NOT implement RepairPrd auto-repair (v2)
- Do NOT add new backend types
- Keep the interaction simple — no TUI frameworks, just stdin/stdout
