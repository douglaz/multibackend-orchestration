# Ralph — Multi-Backend AI Orchestration Tool

A Rust-based orchestration system for coordinating multiple AI backends (Claude CLI, Codex CLI) in structured development workflows with alternating roles, per-role model selection, optional tmux execution, and an interactive PRD generation pipeline.

## Overview

Ralph implements a multi-backend AI orchestration pattern where different AI systems take turns performing distinct roles (Planning, Implementation, Review, Completion) in a software development workflow. Alternating backends between loops provides diverse perspectives and reduces single-model bias.

Key capabilities:
- **Plan/Implement/Review/Commit loops** with automatic backend alternation
- **Per-role model selection** — different models for different roles (e.g., opus for planning, sonnet for reformatting)
- **Codex reasoning effort decomposition** — suffixed model names (e.g., `gpt-5.3-codex-xhigh`) are automatically split into base model + effort CLI flag
- **Parse retry with reformatter agent** — failed output parsing triggers a reformatter (opposite backend) before full retry
- **Auto-rollback** on review iteration limit exceeded
- **Tmux execution mode** — visual backend execution in tmux windows with labeled panes
- **Auto-branch sync** — project branches are merged with base branch on `ralph run` to prevent stale checkouts
- **Interactive PRD pipeline** — `ralph prd` generates Product Requirements Documents through a multi-stage LLM pipeline with gap analysis

## Multi-Project Architecture

Ralph manages multiple projects within a single workspace. Each project represents a distinct development effort.

### Directory Structure

```
.ralph/                              # Ralph workspace root
├── ralph.toml                       # Global configuration
├── index.json                       # Workspace index (all projects)
├── prd/                             # PRD pipeline cache (per-idea)
│   └── <idea_hash>/
│       ├── .lock
│       ├── answers.yaml
│       ├── meta.json
│       ├── stage_hashes.json
│       ├── missing_info_report.md   # Written on exit code 12
│       ├── validation_report.md     # Written on exit code 11
│       ├── 01_ideation.md
│       ├── 02_research.md
│       ├── 03_synthesis.md
│       └── 04_prd.md
├── projects/
│   ├── <project-id>/
│   │   ├── .lock                    # Exclusive advisory lock (fs2)
│   │   ├── prompt.md                # Master prompt for this project
│   │   ├── state.json               # Project state
│   │   ├── config.toml              # Per-project config overrides (optional)
│   │   └── loops/
│   │       ├── 001-<slug>/
│   │       │   ├── {TS}-spec.md
│   │       │   ├── {TS}-impl-notes.md
│   │       │   ├── {TS}-qa-001-pass.md       # QA pass artifact
│   │       │   ├── {TS}-qa-001-fail.md       # QA fail artifact
│   │       │   ├── {TS}-impl-qa-response-001.md  # Implementer QA fix response
│   │       │   ├── {TS}-review-001-feedback.md
│   │       │   ├── {TS}-impl-response-001.md
│   │       │   └── {TS}-review-approved.md
│   │       └── 002-completion/
│   │           ├── {TS}-termination-request.md
│   │           ├── {TS}-completer-verdict.md
│   │           ├── {TS}-acceptance-pass.md    # QA acceptance pass
│   │           └── {TS}-acceptance-fail.md    # QA acceptance fail
│   └── .../
└── templates/                       # Prompt templates
    ├── spec.md                      # Planner template (legacy symlink: planner.md)
    ├── implementation.md            # Implementer template (legacy symlink: implementer.md)
    ├── review.md                    # Reviewer template (legacy symlink: reviewer.md)
    ├── qa.md                        # QA template
    └── completion.md                # Completer template (legacy symlink: completer.md)
```

### Hierarchy Summary

| Level | Contains | Cardinality |
|-------|----------|-------------|
| Workspace | Projects, PRD cache | Many |
| Project | Prompt, State, Config, Loops | 1 prompt, 1 state, optional config, many loops |
| Feature Loop | Artifacts | 1 spec + 1 impl-notes + N QA cycles + N review cycles + 1 approval |
| Completion Loop | Artifacts | 1 termination-request + 1 completer-verdict + optional acceptance-pass/fail |

## Backends

Two primary backends supported:

| Backend | CLI Tool | Invocation |
|---------|----------|------------|
| Claude | `claude` | Claude Code CLI with `--dangerously-skip-permissions` |
| Codex | `codex` | OpenAI Codex CLI with `exec --dangerously-bypass-approvals-and-sandbox -` |

Backends are abstracted behind the `Backend` trait:

```rust
#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, prompt: &str) -> Result<String>;
    async fn health_check(&self) -> Result<()>;
}
```

### Backend Spec Syntax

Backend references use the format `"backend"` or `"backend(model)"`:

```
claude              # Claude with default model
claude(opus)        # Claude with explicit model
codex               # Codex with default model
codex(gpt-5.3-codex-xhigh)  # Codex with suffixed model
```

The `parse_backend_spec()` function in `src/backend/mod.rs` parses these into `BackendSpec { name, model }`.

### Codex Reasoning Effort Decomposition

Codex model names with known effort suffixes (`-xhigh`, `-high`, `-medium`, `-low`) are automatically decomposed at invocation time:

```
Config model: gpt-5.3-codex-xhigh
CLI args:     codex -c model_reasoning_effort="xhigh" --model gpt-5.3-codex exec ...
Display name: codex(gpt-5.3-codex-xhigh)  (preserved for state.json/logs)
```

Suffix matching is longest-first (`-xhigh` before `-high`). Unknown suffixes pass through unchanged. This is codex-specific — Claude model names are not decomposed.

Implementation: `parse_codex_model_effort()` and `backend_from_config()` in `src/backend/codex.rs`.

### Per-Role Model Defaults

Each backend has per-role model defaults in `BackendRoleModels`:

```rust
pub struct BackendRoleModels {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub qa: Option<String>,
    pub completer: Option<String>,
    pub reformatter: Option<String>,
}
```

Code defaults (in `GlobalConfig::default()`):

| Role | Claude | Codex |
|------|--------|-------|
| planner | opus | gpt-5.3-codex-xhigh |
| implementer | opus | gpt-5.3-codex-high |
| reviewer | opus | gpt-5.3-codex-xhigh |
| qa | opus | gpt-5.3-codex-high |
| completer | opus | gpt-5.3-codex-xhigh |
| reformatter | sonnet | gpt-5.3-codex-medium |

When `GlobalConfig::load()` reads `ralph.toml`, any omitted model fields are filled from code defaults via `BackendRoleModels::fill_from()`. This means `ralph.toml` only needs to specify model overrides — omitted fields get the code defaults automatically.

### Model Resolution

`BackendRegistry::resolve_backend_for_role(base_backend, role)` injects the configured role-specific model into a bare backend spec. If the spec already has an explicit model (e.g., `claude(opus)`), it's left unchanged.

### Tmux Execution Mode

When tmux mode is enabled (`--tmux` flag or `workspace.tmux = true` in config), backends are wrapped in `TmuxBackend` which:

1. Creates a named tmux window per backend invocation (labeled `L{loop}-{role}-{backend}`)
2. Pipes the prompt via `cat prompt.tmp | command args | tee output.tmp`
3. Captures exit code via `${PIPESTATUS[1]}` through the pipe
4. Polls for completion at fixed 250ms interval
5. Enables `remain-on-exit` for configurable retention period (`tmux_window_keep_seconds`)
6. Distinguishes genuine timeout from external window disappearance

Config:
```toml
[workspace]
tmux = false                    # Enable tmux mode globally
tmux_session = "ralph"          # Tmux session name
tmux_window_keep_seconds = 5    # Window retention after completion
```

## Roles

| Role | Responsibility | Backend Selection |
|------|----------------|-------------------|
| **Planner** | Analyzes prompt.md + state.json, generates next feature spec or requests completion | Parity-based alternation by loop number |
| **Implementer** | Implements the specification, responds to reviewer feedback | Opposite of Planner (default) |
| **Reviewer** | Reviews implementation against spec, provides feedback or approval | Same as Planner (default) |
| **Completer** | Validates Planner's completion request | Opposite of Planner (default) |
| **QA** | Executes tests and verifies implementation before review | Planner-aligned (default) |
| **Reformatter** | Fixes unparseable backend output (automatic, not user-invoked) | Opposite of failing backend |

### Backend Alternation Pattern (Defaults Without Overrides)

| Loop | Planner | Implementer | Reviewer |
|------|---------|-------------|----------|
| 1 | Claude | Codex | Claude |
| 2 | Codex | Claude | Codex |
| 3 | Claude | Codex | Claude |
| N | (N%2==1 ? starting : opposite) | (opposite of Planner) | (same as Planner) |

Completion loops also consume loop numbers, maintaining monotonic parity. Completer defaults to the opposite backend from Planner.

**Note**: When per-role overrides are configured (via CLI flags, project config, or global config), they are applied independently of the alternation pattern. The alternation pattern only determines the planner backend; overrides can independently assign any backend to any role.

### Per-Role Backend Overrides

Role backends can be overridden at multiple levels:

1. **CLI flags** (highest precedence): `--planner-backend`, `--implementer-backend`, `--reviewer-backend`, `--qa-backend`, `--completer-backend`
2. **Workflow config**: `workflow.planner_backend`, `workflow.implementer_backend`, etc.
3. **Alternation pattern** (default): loop-number parity

## Workflow

### Loop Structure

```
┌──────────────────────────────────────────────────────────────────────┐
│                          FEATURE LOOP N                              │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────┐    ┌─────────────┐    ┌────┐    ┌──────────┐          │
│  │ Planner  │───▶│ Implementer │───▶│ QA │───▶│ Reviewer │          │
│  │(Backend A)│   │ (Backend B) │    └──┬─┘    │(Backend A)│          │
│  └──────────┘    └──────▲──────┘       │      └────┬─────┘          │
│                         │         [QA fail]        │                 │
│                         │              │    ┌──────┴─────────┐      │
│                         └──────────────┘    │                │      │
│                         │                   ▼                ▼      │
│                         │             [Suggestions]    [Approved]   │
│                         │                   │                │      │
│                         │                   ▼                ▼      │
│                         │            ┌─────────────┐   ┌─────────┐ │
│                         │            │ Implementer │   │ COMMIT  │ │
│                         │            │ (Backend B) │   │  CODE   │ │
│                         │            └──────┬──────┘   └────┬────┘ │
│                         │                   │               │      │
│                         │                   ▼               ▼      │
│                         │             ┌──────────┐   ┌──────────┐  │
│                         └─────────────│ Reviewer │   │NEXT LOOP │  │
│                     review feedback   │(Backend A)│   └──────────┘  │
│                                       └──────────┘                  │
└──────────────────────────────────────────────────────────────────────┘

(QA phase is skipped when qa_enabled=false)
```

### Orchestrator State Machine

```
Init -> Planning

Planning -> Implementing    (planner produced feature spec)
Planning -> Completing      (planner suggested completion)

Implementing -> QA          (when qa_enabled=true)
Implementing -> Reviewing   (when qa_enabled=false)
QA -> Reviewing             (QA pass)
QA -> Implementing          (QA fail → feedback loop)
QA -> [rollback + error]    (QA iteration limit hit → auto-rollback, or retry if --until-complete)
Reviewing -> Implementing   (review verdict: suggestions)
Reviewing -> Committing     (review verdict: approved)
Reviewing -> [rollback + error] (review iteration limit hit → auto-rollback, or retry if --until-complete)
Committing -> Planning      (next loop number)

Completing -> QA acceptance gate  (completer verdict: COMPLETE, qa_enabled=true)
Completing -> Complete            (completer verdict: COMPLETE, qa_enabled=false)
QA acceptance gate -> Complete    (acceptance pass)
QA acceptance gate -> Planning    (acceptance fail → force CONTINUE)
Completing -> Planning            (completer verdict: CONTINUE, next loop number)
```

### Parse Retry with Reformatter Agent

When a backend response cannot be parsed (missing H1, wrong format):

1. **Attempt 1**: Send original prompt to assigned backend
2. **Attempt 2 (reformatter)**: On parse failure, invoke the **reformatter** (opposite backend, with reformatter role model) with a reformat prompt containing the original response + expected format
3. **Attempt 3 (reminded original)**: If reformatter output also fails, retry with the **original backend** using the original prompt augmented with a format reminder preamble
4. If still failing, fail with `ParseRetriesExhausted`

The reformatter role uses a lighter model (e.g., `sonnet` for Claude, `gpt-5.3-codex-medium` for Codex) since it only needs to reformat, not generate.

### Auto-Rollback on Review Iteration Limit

When `phase_iteration > max_review_iterations` (default: 30):
- The orchestrator auto-rolls back the current loop
- If `--until-complete` is active: logs the event and continues to the next feature loop
- Otherwise: returns `ReviewIterationLimitExceeded` error after rollback
- This prevents infinite review cycles on complex changes

### Auto-Branch Sync

On `ralph run`, after checking out the project branch, the orchestrator calls `merge_base_branch()` to merge any new commits from the base branch (master). This fixes a race condition where:
1. `ralph project new` creates a branch at current master HEAD
2. Project state files are committed to master after branch creation
3. `ralph run` checks out the branch (which is behind master)
4. The merge brings the branch up to date

Implementation: `merge_base_branch()` in `src/git/branch.rs` uses `git rev-list --count HEAD..{base}` to check for divergence, then `git merge` if needed.

### Prompt Change Detection

Prompt hash is checked at the start of each loop. If changed mid-loop:
- `continue`: Proceed with new prompt (may cause inconsistency)
- `restart-loop`: Discard current loop progress, restart
- `abort`: Stop without changes

### Ensure Clean Start

`ensure_clean_start_for_new_loop()` validates the working tree is clean (excluding `.ralph/**`) before starting a new feature loop. This prevents unrelated changes from being swept into the loop's commit.

## Canonical Parser Contracts

Parsers key off the first markdown H1 line in backend body output:

| Role | H1 | Artifact |
|------|----|----|
| Planner (feature) | `# Feature: <name>` | spec |
| Planner (completion) | `# Project Completion Request` | termination-request |
| Implementer (initial) | `# Implementation Notes` | impl-notes |
| Implementer (feedback response) | `# Implementation Response (Iteration <N>)` | impl-response |
| Reviewer (approve) | `# Review: APPROVED` | review-approved |
| Reviewer (suggestions) | `# Review: SUGGESTIONS` | review-feedback |
| QA (pass) | `# QA: PASS` | qa-pass |
| QA (fail) | `# QA: FAIL` | qa-fail |
| Completer (done) | `# Verdict: COMPLETE` | completer-verdict |
| Completer (continue) | `# Verdict: CONTINUE` | completer-verdict |

## Artifact System

### Artifact Types

| Artifact | Producer | Consumer | Purpose |
|----------|----------|----------|---------|
| `prompt.md` | Human | Planner | Master project specification |
| `{TS}-spec.md` | Planner | Implementer, Reviewer | Feature specification |
| `{TS}-impl-notes.md` | Implementer | Reviewer | Implementation decisions |
| `{TS}-review-{III}-feedback.md` | Reviewer | Implementer | Required changes |
| `{TS}-impl-response-{III}.md` | Implementer | Reviewer | Addressed feedback |
| `{TS}-review-approved.md` | Reviewer | Orchestrator | Final approval |
| `{TS}-qa-{III}-pass.md` | QA | Orchestrator | QA verification passed |
| `{TS}-qa-{III}-fail.md` | QA | Implementer | QA failures requiring fixes |
| `{TS}-impl-qa-response-{III}.md` | Implementer | QA | Addressed QA feedback |
| `{TS}-acceptance-pass.md` | QA | Orchestrator | Completion acceptance passed |
| `{TS}-acceptance-fail.md` | QA | Planner | Completion acceptance failed |
| `{TS}-termination-request.md` | Planner | Completer | Completion rationale |
| `{TS}-completer-verdict.md` | Completer | Orchestrator | Continue/Complete |

`{TS}` = `YYYYMMDDHHMMSS` UTC timestamp. `{III}` = zero-padded review iteration (001, 002, ...).

### Artifact Frontmatter

The orchestrator injects YAML frontmatter; backend responses provide body content only:

```yaml
---
artifact: impl-notes
loop: 3
project: my-project
backend: codex(gpt-5.3-codex-high)
role: implementer
created_at: 2026-02-05T14:30:00Z
---
```

Frontmatter fields: `artifact`, `loop`, `iteration` (for review cycles), `iterations` (total cycles on approval), `project`, `backend`, `role`, `created_at`.

## Data Structures

### Workspace Index (`index.json`)

```json
{
  "workspace_version": "1.0",
  "created_at": "ISO8601",
  "active_project": "project-id",
  "projects": [
    {
      "id": "project-id",
      "name": "Project Name",
      "status": "in_progress",
      "created_at": "ISO8601",
      "completed_at": null,
      "total_feature_loops": 3,
      "total_completion_attempts": 0,
      "last_loop_number": 3,
      "parent_project": null
    }
  ]
}
```

### Project State (`state.json`)

```json
{
  "project_id": "project-id",
  "project_name": "Project Name",
  "prompt_file": "prompt.md",
  "prompt_hash": "sha256",
  "prompt_hash_at_loop_start": "sha256",
  "parent_project": null,
  "current_loop": 3,
  "current_phase": "reviewing",
  "phase_iteration": 2,
  "status": "in_progress",
  "loops": [ ... ],
  "completion_attempts": [ ... ]
}
```

`phase_iteration` semantics:

| Phase | Meaning |
|-------|---------|
| planning | Always 1 |
| implementing | 1 for initial; N when responding to review-N or QA-N feedback |
| qa | Next QA iteration to run |
| reviewing | Next review iteration to run |
| committing | Always 1 |
| completing | Always 1 |

## Configuration

### Global Configuration (`ralph.toml`)

```toml
[workspace]
version = "1.0"
default_backend = "claude"
tmux = false
tmux_session = "ralph"
tmux_window_keep_seconds = 5

[backends.claude]
command = "claude"
args = ["--dangerously-skip-permissions"]
timeout_seconds = 600
env = {}

[backends.claude.models]        # Optional — code defaults apply for omitted fields
planner = "opus"
implementer = "opus"
reviewer = "opus"
qa = "opus"
completer = "opus"
reformatter = "sonnet"

[backends.codex]
command = "codex"
args = ["exec", "--dangerously-bypass-approvals-and-sandbox", "-"]
timeout_seconds = 600
env = {}

[backends.codex.models]         # Optional — code defaults apply for omitted fields
planner = "gpt-5.3-codex-xhigh"
implementer = "gpt-5.3-codex-high"
reviewer = "gpt-5.3-codex-xhigh"
qa = "gpt-5.3-codex-high"
completer = "gpt-5.3-codex-xhigh"
reformatter = "gpt-5.3-codex-medium"

[workflow]
max_review_iterations = 30
auto_commit = true
commit_message_style = "conventional"   # conventional | descriptive | minimal
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"          # continue | restart-loop | abort
qa_enabled = false                      # Enable QA phase between implementing and reviewing
max_qa_iterations = 3                   # Maximum QA retry attempts before rollback
# qa_backend = "claude(opus)"           # Override QA backend (default: planner-aligned)
# Per-role backend overrides (optional):
# planner_backend = "claude(opus)"
# implementer_backend = "codex(gpt-5.3-codex-high)"
# reviewer_backend = "claude(opus)"
# completer_backend = "codex(gpt-5.3-codex-xhigh)"

[templates]
planner = "templates/spec.md"
implementer = "templates/implementation.md"
reviewer = "templates/review.md"
completer = "templates/completion.md"
qa = "templates/qa.md"

[git]
auto_branch = true
branch_format = "ralph/{project_id}"
sign_commits = false
base_branch = "master"
```

### Per-Project Overrides (`config.toml`)

Optional file at `.ralph/projects/<id>/config.toml`:

```toml
[workflow]
starting_backend = "codex"
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"   # or "descriptive"
prompt_change_action = "abort"          # or "continue" | "restart-loop"
qa_enabled = true
max_qa_iterations = 3
qa_backend = "claude(opus)"
planner_backend = "claude"
implementer_backend = "codex"
reviewer_backend = "claude"
completer_backend = "codex"

[templates]
planner = "custom/spec.md"
implementer = "custom/implementation.md"
reviewer = "custom/review.md"
qa = "custom/qa.md"
completer = "custom/completion.md"
```

### Backend Selection Precedence

Two independent resolution ladders:

**Starting backend** (determines planner alternation base):
1. `ralph run --backend` (CLI override)
2. Project config `workflow.starting_backend`
3. Global `workspace.default_backend`

**Per-role backend** (resolved independently per role):
1. CLI role flag (`--planner-backend`, `--implementer-backend`, etc.)
2. Project config role override (`workflow.planner_backend`, etc.)
3. Global config role override (`workflow.planner_backend`, etc.)
4. Alternation pattern (planner based on loop parity, others derived from planner)

## CLI Interface

```
ralph - AI Backend Orchestration Tool

COMMANDS:
    init                Initialize a new ralph workspace
    project new         Create a new project
    project list        List all projects
    project use         Switch active project
    project show        Show project details
    run                 Start or resume orchestration
    prd                 Generate a Product Requirements Document
    status              Show current project status
    history             Show loop history
    tail                Stream loop artifacts / attach to tmux
    rollback            Rollback to a previous loop state
    config              Manage configuration (show/get/set/edit)
    validate            Run conformance test suite against a ralph binary
```

### `ralph run`

```
ralph run [OPTIONS]

OPTIONS:
    --project <ID>              Run specific project (default: active project)
    --loops <N>                 Maximum feature loops to complete
    --until-review              Stop after next successful review
    --until-complete            Run until Completer returns COMPLETE
    --dry-run                   Show plan without executing
    --backend <SPEC>            Override starting backend for this run
    --planner-backend <SPEC>    Override planner backend
    --implementer-backend <SPEC> Override implementer backend
    --reviewer-backend <SPEC>   Override reviewer backend
    --qa-backend <SPEC>         Override QA backend
    --completer-backend <SPEC>  Override completer backend
    --on-prompt-change <ACTION> continue | restart-loop | abort
    --skip-commit               Don't auto-commit after approval
    --tmux                      Enable tmux execution mode
    --no-tmux                   Disable tmux execution mode
```

Constraints: `--loops` must be > 0; `--loops`, `--until-review`, `--until-complete` are mutually exclusive.

Side effect: `--project <ID>` updates the active project in `index.json`.

### `ralph init`

```
ralph init [OPTIONS]

OPTIONS:
    --dir <PATH>    Workspace directory (default: .ralph)
```

### `ralph project`

```
ralph project new --id <ID> --name <NAME> --prompt <FILE> [--backend <SPEC>]
ralph project new --id <ID> --name <NAME> --from <PARENT_PROJECT>
ralph project list
ralph project use <PROJECT_ID>
ralph project show [PROJECT_ID] [--json]
```

### `ralph history`

```
ralph history [OPTIONS]

OPTIONS:
    --project <ID>    Target project
    --verbose         Show detailed loop info
    --json            JSON output
```

### `ralph config`

```
ralph config show [--global | --project <ID>]
ralph config get <KEY> [--global | --project <ID>]
ralph config set <KEY> <VALUE> [--global | --project <ID>]
ralph config edit [--global | --project <ID>]
```

### `ralph prd`

Interactive PRD generation pipeline with 4 stages: Ideation, Research, Synthesis, PRD.

```
ralph prd [OPTIONS]

OPTIONS:
    --idea <TEXT>         The product/feature idea (required)
    --non-interactive     Skip interactive questions (exit 12 on gaps)
    --interactive         Force interactive mode even on non-TTY stdin
    --ask-max <N>         Maximum question rounds (default: 3)
    --answers <FILE>      Pre-load answers from YAML file
    --resume              Resume from cached state
    --dry-run             Reserved (currently no-op)
    --backend <SPEC>      Backend to use (default: workspace default)
```

Pipeline stages:
1. **Ideation** — brainstorm features, scope, user stories from the idea
2. **Research** — analyze technical approaches, constraints, prior art
3. **Synthesis** — consolidate ideation + research into structured requirements
4. **PRD** — generate the final Product Requirements Document

Between stages, the pipeline:
- Runs deterministic + LLM gap analysis
- Asks user targeted questions (interactive mode)
- Reruns affected stages when new answers change scope
- Validates the final PRD for completeness
- Caches all artifacts under `.ralph/prd/<idea_hash>/`
- Copies final PRD to `PRD.md` in the working directory

Exit codes: 10 (pipeline failed), 11 (validation failed), 12 (missing info — non-interactive mode, deterministic section check failure, or `ask_max` rounds exceeded).

Non-interactive mode is auto-enabled when stdin is not a TTY, unless `--interactive` is explicitly passed.

### `ralph tail`

```
ralph tail [OPTIONS]

OPTIONS:
    --project <ID>              Tail specific project
    -n, --last <N>              Show last N artifacts
    -F, --follow                Continuously stream new artifacts
    --poll-interval-ms <MS>     Rescan interval (default: 1000)
    --json                      JSON output per artifact
    --tmux                      Attach to ralph tmux session instead
```

### `ralph rollback`

```
ralph rollback <LOOP_NUMBER> [OPTIONS]

OPTIONS:
    --project <ID>    Target project
    --hard            Also reset git (resolves ref via: target tag → prior tag → merge-base/base branch)
    --dry-run         Preview without executing
```

### `ralph validate`

```
ralph validate [OPTIONS]

OPTIONS:
    --bin <PATH>      Path to the ralph binary under test (required)
    --filter <PATTERN> Only run tests matching pattern (e.g., "run::", "init::")
    --list            List available tests without running them
    --verbose         Show detailed output per test
```

Runs 40 black-box conformance tests against an arbitrary ralph binary. Tests cover init, project, run, and command behaviors. Used in `nix build` via `postCheck` to validate the built binary.

## Source Architecture

```
src/
├── main.rs                     # CLI entry point, tracing setup
├── lib.rs                      # Module declarations, Result type alias
├── error.rs                    # RalphError enum with exit codes
├── cli/
│   ├── mod.rs                  # Cli struct, Commands enum, arg parsing
│   ├── backend_spec.rs         # Backend spec validation helpers
│   ├── config.rs               # Config show/get/set/edit with scope resolution
│   ├── history.rs              # Loop history display
│   ├── init.rs                 # Workspace initialization
│   ├── prd.rs                  # PRD pipeline CLI entry point
│   ├── project.rs              # Project new/list/use/show
│   ├── rollback.rs             # Rollback operations
│   ├── run.rs                  # Orchestration dispatch
│   ├── status.rs               # Status display
│   └── tail.rs                 # Artifact streaming / tmux attach
├── backend/
│   ├── mod.rs                  # Backend trait, CliBackend, BackendRegistry,
│   │                           #   BackendSpec, parse_backend_spec(),
│   │                           #   resolve_backend_for_role(), assign_feature_backends()
│   ├── claude.rs               # Claude backend_from_config()
│   ├── codex.rs                # Codex backend_from_config(),
│   │                           #   parse_codex_model_effort() (suffix decomposition)
│   ├── mock.rs                 # MockBackend for testing
│   ├── tmux.rs                 # Tmux session/window management utilities
│   └── tmux_backend.rs         # TmuxBackend wrapper with RAII temp files
├── workflow/
│   ├── mod.rs
│   ├── orchestrator.rs         # Main 6-phase state machine loop (incl. QA),
│   │                           #   parse retry, auto-rollback, auto-branch sync,
│   │                           #   prompt change detection, tmux context,
│   │                           #   QA feedback loop, acceptance gate
│   └── parser.rs               # Agent output parsers (planner, implementer,
│                               #   reviewer, qa, completer), strip_frontmatter()
├── workspace/
│   ├── mod.rs                  # Workspace struct
│   ├── index.rs                # WorkspaceIndex (projects list, active project)
│   └── discovery.rs            # Find .ralph directory from cwd
├── project/
│   ├── mod.rs
│   ├── state.rs                # ProjectState, Phase, FeatureLoopState,
│   │                           #   CompletionLoopState, all status enums
│   ├── artifacts.rs            # Artifact file I/O with frontmatter injection
│   └── lifecycle.rs            # Project creation, inheritance, branch setup
├── prd/
│   ├── mod.rs                  # Module re-exports
│   ├── pipeline.rs             # PrdPipeline state machine driver
│   ├── state.rs                # Stage enum (Ideation/Research/Synthesis/Prd),
│   │                           #   PrdPhase, PipelineContext, PrdMeta
│   ├── stages.rs               # Stage prompt builders, output parsers
│   ├── gaps.rs                 # GapReport, Question, ValidationResult,
│   │                           #   deterministic + LLM gap analysis
│   ├── interaction.rs          # UserInteraction trait, PlainInteraction (stdin),
│   │                           #   NonInteractiveInteraction, MockInteraction
│   ├── answers.rs              # AnswerStore: YAML load/save/merge/hash
│   └── cache.rs                # CacheManager: .ralph/prd/<hash>/ file I/O,
│                               #   hash-based skip, lock, resume validation
├── prompts/
│   ├── mod.rs
│   └── templates.rs            # Template loading, variable substitution
├── config/
│   ├── mod.rs
│   ├── global.rs               # GlobalConfig, BackendConfig, BackendRoleModels,
│   │                           #   WorkflowConfig, GitConfig, fill_from() defaults
│   └── project.rs              # Per-project config (starting_backend, etc.)
├── validate/
│   ├── mod.rs                  # Conformance test suite module
│   ├── harness.rs              # Test harness (workspace setup, binary invocation)
│   ├── runner.rs               # Test runner and result reporting
│   ├── assertions.rs           # Custom assertion helpers
│   ├── mock_scripts.rs         # Mock backend scripts for testing
│   ├── tests_init.rs           # Init command conformance tests
│   ├── tests_project.rs        # Project command conformance tests
│   ├── tests_run.rs            # Run command conformance tests
│   └── tests_commands.rs       # Status/history/config/rollback conformance tests
├── git/
│   ├── mod.rs                  # Git utility functions (run_git, ensure_git_repo, etc.)
│   ├── branch.rs               # Branch create/checkout/exists, merge_base_branch()
│   └── commit.rs               # commit_feature_loop(), changed_paths(),
│                               #   read_porcelain_status(),
│                               #   stage_implementation_changes(),
│                               #   reset_and_clean_working_tree(),
│                               #   working_tree_diff_excluding_orchestration_state()
└── util/
    ├── mod.rs
    ├── hash.rs                 # sha256_hex()
    ├── lock.rs                 # ProjectLock (fs2 exclusive advisory lock, RAII)
    ├── slug.rs                 # slugify_feature_name() (50 char max)
    └── time.rs                 # now_utc(), now_iso8601(), format_timestamp_*()
```

## Error Handling

### Error Types and Exit Codes

| Error | Exit Code |
|-------|-----------|
| Validation, WorkspaceNotFound, ProjectNotFound, ActiveProjectNotSet, PrdCacheMismatch | 2 |
| StateLocked | 3 |
| PrdPipelineFailed | 10 |
| PrdValidationFailed | 11 |
| PrdMissingInfo | 12 |
| QaIterationLimitExceeded | 1 |
| All others (backend failures, parse errors, git conflicts, etc.) | 1 |

### State Recovery

- State is written after each phase transition
- `ralph run` automatically resumes from last saved state
- `ralph rollback` reverts to any previous loop
- On interruption, no data is lost — artifacts and state persist
- **Corruption recovery**: if `state.json` fails to parse, `load_project_state()` attempts auto-recovery from `git show HEAD:<state-path>` before returning an error

## Conventions

1. **Artifact filenames** are timestamp-prefixed: `{TS}-{type}.md` where `TS = YYYYMMDDHHMMSS` UTC
2. **Slug generation**: lowercase, replace spaces/underscores with hyphens, max 50 chars, truncated at word boundary
3. **Loop numbering**: single monotonic sequence shared by feature loops and completion attempts
4. **State locking**: mutating commands acquire exclusive fs2 lock at `.ralph/projects/<id>/.lock`
5. **Git commits**: at most one per approved feature loop, tagged with `ralph/{project_id}/loop-{N}`
6. **Prompt hash**: SHA-256 of prompt.md content, checked at loop boundaries
7. **Template resolution**: global paths relative to `.ralph/`, project paths relative to project dir

## Dependencies

```toml
[dependencies]
async-trait = "0.1"
chrono = { version = "0.4", features = ["clock", "serde"] }
clap = { version = "4", features = ["derive"] }
fs2 = "0.4"
regex = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sha2 = "0.10"
thiserror = "2"
tokio = { version = "1", features = ["full"] }
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
which = "7"
tempfile = "3"
```

## Build & Test

```bash
nix build     # Builds + runs all tests in sandbox (unit, integration, 40 conformance)
```

The `postPatch` in `flake.nix` replaces `#!/usr/bin/env bash` with nix store bash path in all test files for sandbox compatibility. Tests that modify PATH use a `lock_path()` mutex to avoid race conditions.

### Test Suite

| Test file | Coverage |
|-----------|----------|
| `tests/orchestrator.rs` | End-to-end orchestration: feature loops, review cycles, QA phase, rollback, resume, completion |
| `tests/backend.rs` | Backend spec parsing, role model injection, feature backend assignment |
| `tests/state.rs` | State serialization, deserialization, backward compatibility, QA fields |
| `tests/status_history.rs` | Status/history CLI output formatting including QA data |
| `tests/templates.rs` | Template loading, variable substitution |
| `tests/init_command.rs` | Workspace initialization, template creation |
| `tests/tail_tmux.rs` | Tail command parsing, tmux attach behavior |
| `tests/prd.rs` | PRD pipeline stages and gap analysis |
| `tests/recovery.rs` | State corruption auto-recovery |
| `tests/git.rs` | Git utility functions |
| `tests/backend_tmux.rs` | Tmux backend wrapper |
| `tests/validate_cli.rs` | Validate command argument parsing |
| `src/validate/` (40 tests) | Black-box conformance: init, project, run, commands |

## License

MIT
