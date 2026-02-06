# Ralph Loop Orchestration Tool

A Rust-based orchestration system for coordinating multiple AI backends (Claude CLI, Codex) in structured development workflows with alternating roles.

## Overview

Ralph Loop implements a multi-backend AI orchestration pattern where different AI systems take turns performing distinct roles (Planning, Implementation, Review) in a software development workflow. The key insight is that alternating backends between loops provides diverse perspectives and reduces single-model bias.

## Multi-Project Architecture

Ralph manages multiple projects within a single workspace. Each project represents a distinct development effort (PoC, alpha, beta, refactoring, production-ready, etc.).

### Directory Structure

```
.ralph/                              # Ralph workspace root
├── ralph.toml                       # Global configuration
├── index.json                       # Workspace index (all projects)
├── projects/
│   ├── 01-poc/
│   │   ├── prompt.md                # The single master prompt for this project
│   │   ├── state.json               # Project state (current loop, phase, etc.)
│   │   └── loops/
│   │       ├── 001-auth/
│   │       │   ├── spec.md                    # Planner → Implementer
│   │       │   ├── impl-notes.md              # Implementer → Reviewer (decisions, explanations)
│   │       │   ├── review-001-feedback.md     # Reviewer → Implementer (suggestions)
│   │       │   ├── impl-response-001.md       # Implementer → Reviewer (addressed feedback)
│   │       │   ├── review-002-feedback.md     # Reviewer → Implementer (more suggestions)
│   │       │   ├── impl-response-002.md       # Implementer → Reviewer (addressed feedback)
│   │       │   └── review-approved.md         # Reviewer final approval
│   │       │
│   │       ├── 002-database/
│   │       │   ├── spec.md
│   │       │   ├── impl-notes.md
│   │       │   └── review-approved.md         # Approved on first review
│   │       │
│   │       └── 003-completion/                # Special: completion attempt
│   │           ├── termination-request.md     # Planner → Completer
│   │           └── completer-verdict.md       # Completer response (continue/complete)
│   │
│   ├── 02-alpha/
│   │   ├── prompt.md
│   │   ├── state.json
│   │   └── loops/
│   │       └── ...
│   │
│   └── .../
│
└── templates/                       # Reusable prompt templates (optional)
    ├── planner.md
    ├── implementer.md
    ├── reviewer.md
    └── completer.md
```

### Project Lifecycle

```
┌─────────┐    ┌─────────┐    ┌─────────┐    ┌───────────┐    ┌─────────┐
│ 01-poc  │───▶│02-alpha │───▶│ 03-beta │───▶│04-refactor│───▶│ 05-prod │
│completed│    │completed│    │completed│    │ completed │    │completed│
└─────────┘    └─────────┘    └─────────┘    └───────────┘    └─────────┘
     │              │              │               │               │
     ▼              ▼              ▼               ▼               ▼
  learnings     learnings     learnings       learnings       SHIPPED!
  feed into     feed into     feed into       feed into
  next prompt   next prompt   next prompt     next prompt
```

Projects can:
- **Inherit** from previous projects (copy prompt as starting point)
- **Reference** previous project specs (for context)
- **Run independently** (no dependency required)

### Hierarchy Summary

| Level | Contains | Cardinality |
|-------|----------|-------------|
| Workspace | Projects | Many |
| Project | Prompt, State, Loops | 1 prompt, 1 state, many loops |
| Feature Loop | Artifacts | 1 spec + 1 impl-notes + N review cycles + 1 approval |
| Completion Loop | Artifacts | 1 termination-request + 1 completer-verdict |

## Canonical Conventions (Normative)

The following rules are normative and take precedence over examples elsewhere in this document:

1. **Canonical filenames and paths**
   - Project prompt file is always `prompt.md`.
   - Project state file is always `state.json`.
   - Feature-loop artifacts are stored only under `.ralph/projects/<project-id>/loops/<NNN>-<slug>/`.
   - Completion-loop artifacts are stored only under `.ralph/projects/<project-id>/loops/<NNN>-completion/`.
   - Canonical artifact names are `spec.md`, `impl-notes.md`, `review-<III>-feedback.md`, `impl-response-<III>.md`, `review-approved.md`, `termination-request.md`, and `completer-verdict.md`.
   - `<III>` is a zero-padded review iteration counter (`001`, `002`, ...), independent of loop number.
   - In `state.json`, all artifact paths are project-relative (for example `loops/001-user-auth/spec.md`), resolved from `.ralph/projects/<project-id>/`.

2. **Slug generation rules**
   - Slugs are derived from feature names by: lowercasing, replacing spaces/underscores with hyphens, removing non-alphanumeric characters (except hyphens), collapsing consecutive hyphens, trimming leading/trailing hyphens.
   - Maximum slug length is 50 characters (truncated at word boundary if possible).
   - Feature-loop slug generation applies only to `loop_type = "feature"`.
   - Completion-attempt loops always use the fixed slug `completion` (directory: `<NNN>-completion`).
   - Examples: `"User Authentication"` → `user-authentication`, `"REST API Endpoints (v2)"` → `rest-api-endpoints-v2`.

3. **Artifact authorship and frontmatter**
   - Backends produce artifact **body content** only (markdown, no frontmatter).
   - Orchestrator writes files to canonical paths and injects canonical YAML frontmatter.
   - Frontmatter fields are orchestrator-owned: `artifact`, `loop`, `iteration` (for review cycles), `iterations` (total feedback cycles recorded at approval), `project`, `backend`, `role`, `created_at`.
   - The `artifact` field contains the base type without iteration number (e.g., `review-feedback`, not `review-001-feedback`).
   - The `iteration` field (singular) appears on per-iteration artifacts (`review-feedback`, `impl-response`) to identify which cycle.
   - The `iterations` field (plural) appears only on `review-approved` to record total feedback/response cycles completed before approval.
   - If a backend returns frontmatter anyway, orchestrator ignores backend frontmatter and rewrites canonical frontmatter.
   - Invalid or unparsable backend output triggers retry/reformat flow; if retries are exhausted, the phase fails with explicit error and state is preserved.

4. **Loop numbering and backend alternation**
   - A single monotonic `loop_number` sequence is used per project.
   - Both feature loops and completion-attempt loops consume loop numbers.
   - Planner/Reviewer use parity-based alternation by loop number; Implementer uses the opposite backend; Completer must be opposite of Planner.

5. **Commit policy**
   - At most one orchestrator-managed commit is created per approved feature loop.
   - The orchestrator-managed commit occurs only after `review-approved.md`.
   - Review iterations may produce diffs and artifact updates, but no orchestrator-managed loop commit is finalized before approval.
   - Completion loops do not create code commits.
   - When `--skip-commit` or `auto_commit=false` is used, approved loops produce no commit or tag. Rollback to such loops falls back to the nearest prior tagged loop (see rollback behavior).

6. **Backend selection precedence**
   - Highest to lowest precedence:
     1. `ralph run --backend`
     2. Project config: `.ralph/projects/<id>/config.toml` `workflow.starting_backend`
     3. Global default: `.ralph/ralph.toml` `workspace.default_backend`
   - `ralph project new --backend` is persisted as `workflow.starting_backend` in project config.
   - `ralph run --backend` is invocation-scoped only and never writes config.
   - This precedence applies to selecting the Planner backend when a new loop starts.

7. **Prompt change detection**
   - `prompt_hash` is computed on every `ralph run` and compared to stored value.
   - `prompt_hash_at_loop_start` captures the hash when the current loop began.
   - If prompt changes mid-loop (between phases), behavior is controlled by `--on-prompt-change <continue|restart-loop|abort>`.
     - `continue`: Proceed with current loop using new prompt (may cause inconsistency)
     - `restart-loop`: Discard current loop progress and restart with new prompt
     - `abort`: Stop without further changes
   - If prompt changes between loops, no warning (expected workflow for iterative refinement).
   - State records both hashes for auditability.

8. **State field duplication policy**
   - `parent_project` appears in both `index.json` and `state.json` intentionally.
   - `index.json` is the source of truth for workspace-level queries.
   - `state.json` duplicates it for self-contained project state (useful for export/archive).
   - On load, orchestrator validates consistency; mismatch triggers warning.

9. **Canonical parser contracts**
   - Parsers key off the first markdown H1 line in backend body output.
   - Valid H1 values are:
     - Planner feature spec: `# Feature: <name>`
     - Planner completion request: `# Project Completion Request`
     - Implementer notes: `# Implementation Notes`
     - Implementer review response: `# Implementation Response (Iteration <N>)`
     - Reviewer approval: `# Review: APPROVED`
     - Reviewer feedback: `# Review: SUGGESTIONS`
     - Completer verdict: `# Verdict: COMPLETE` or `# Verdict: CONTINUE`
   - Section headings under each artifact must follow the role prompt templates in this document.

10. **State ordering model**
    - `loops[]` stores feature loops and `completion_attempts[]` stores completion loops.
    - Global chronological order is reconstructed by `loop_number` across both arrays.
    - `loop_number` values must be unique across both arrays.

11. **Template path resolution**
    - Global template paths in `.ralph/ralph.toml` are resolved relative to `.ralph/`.
    - Per-project template paths in `.ralph/projects/<id>/config.toml` are resolved relative to that project directory.

12. **Project inheritance and branch tracking**
    - `ralph project new --from <parent>` snapshots the parent branch tip at creation time.
    - Child project branches do not auto-track future parent commits.
    - Pulling later parent changes into a child project is a manual git operation (merge or cherry-pick), outside orchestrator automation.

13. **Loop status values**
    - Valid status values for both `loops[]` and `completion_attempts[]` entries are `pending`, `in_progress`, and `completed`.
    - `pending`: loop allocated but no role artifact has been written yet.
    - `in_progress`: at least one role artifact exists, but terminal artifact does not.
    - `completed`: terminal artifact exists (`review-approved.md` for feature loops, `completer-verdict.md` for completion loops).

14. **State locking and concurrent access**
    - Mutating commands (`run`, `rollback`, `config set`, project creation/inheritance) acquire an exclusive advisory lock at `.ralph/projects/<id>/.lock`.
    - Lock is acquired before reading mutable state (`state.json`) and held through all writes for that command.
    - If lock acquisition fails, command exits with a `StateLocked` error and performs no writes.
    - Read-only commands (`status`, `history`, `project list/show`, `config get/show`) do not require exclusive locks.

## Artifacts

Every interaction between roles produces an artifact that is persisted for traceability and context.

### Artifact Flow Diagram

```
FEATURE LOOP
  prompt.md
     -> Planner -> spec.md
     -> Implementer -> code diff + impl-notes.md
     -> Reviewer
          -> [if approved] review-approved.md -> commit + tag
          -> [if suggestions] review-III-feedback.md -> Implementer -> impl-response-III.md -> Reviewer (repeat)

COMPLETION LOOP
  prompt.md + state.json
     -> Planner -> termination-request.md
     -> Completer -> completer-verdict.md
          -> COMPLETE: project done
          -> CONTINUE: next feature loop
```

### Artifact Types

| Artifact | Producer | Consumer | Purpose |
|----------|----------|----------|---------|
| `prompt.md` | Human | Planner | Master project specification |
| `spec.md` | Planner | Implementer, Reviewer | Feature specification for this loop |
| `impl-notes.md` | Implementer | Reviewer | Explains implementation decisions, deviations, trade-offs |
| `review-III-feedback.md` | Reviewer | Implementer | Suggestions/required changes (iteration III) |
| `impl-response-III.md` | Implementer | Reviewer | Response to feedback: what was done, what couldn't be done and why |
| `review-approved.md` | Reviewer | Orchestrator | Final approval, ends the review cycle |
| `termination-request.md` | Planner | Completer | Request to end project, includes rationale |
| `completer-verdict.md` | Completer | Orchestrator, Planner | Approve completion or list remaining work |

### Artifact Naming Convention

```
loops/
└── {NNN}-{slug}/
    ├── spec.md                      # Always present
    ├── impl-notes.md                # Always present after implementation
    ├── review-{III}-feedback.md     # One per review iteration (001, 002, ...)
    ├── impl-response-{III}.md       # One per feedback response
    └── review-approved.md           # Present only when approved

# Special completion loop (no feature, just completion check)
└── {NNN}-completion/
    ├── termination-request.md
    └── completer-verdict.md
```

### Example: Loop with 2 Review Iterations

```
loops/001-user-auth/
├── spec.md                    # Planner wrote: "implement user auth with JWT..."
├── impl-notes.md              # Implementer wrote: "used bcrypt for passwords because..."
├── review-001-feedback.md     # Reviewer wrote: "missing rate limiting, tests incomplete"
├── impl-response-001.md       # Implementer wrote: "added rate limiting, here's why tests..."
├── review-002-feedback.md     # Reviewer wrote: "rate limiting good, but tests still need X"
├── impl-response-002.md       # Implementer wrote: "added X, couldn't do Y because..."
└── review-approved.md         # Reviewer wrote: "APPROVED - all criteria met"
```

### Artifact Content Structure

Each persisted artifact file follows a standard header format. The orchestrator injects this frontmatter; backend responses provide body content only.

```markdown
---
artifact: impl-notes
loop: 1
project: 03-beta
backend: codex
role: implementer
created_at: 2026-02-05T14:30:00Z
---

# Implementation Notes

[content here]
```

This YAML frontmatter allows programmatic parsing while keeping files human-readable.

## Core Concepts

### Backends

Two primary backends supported:

| Backend | CLI Tool | Invocation |
|---------|----------|------------|
| Claude | `claude` | Claude Code CLI |
| Codex | `codex` | OpenAI Codex CLI |

Backends are abstracted behind a common trait. **Version 1 is scoped to exactly two active backends.** The `opposite()` backend selection logic assumes a two-backend system. Future versions may support N-backend assignment strategies (see Phase 2).

### Roles

| Role | Responsibility |
|------|----------------|
| **Planner** | Reads `prompt.md`, analyzes `state.json`, generates next feature spec OR requests project termination |
| **Implementer** | Implements the specification, documents decisions, responds to reviewer feedback |
| **Reviewer** | Reviews implementation against spec and `prompt.md`, provides feedback or approval |
| **Completer** | Validates Planner's termination request, ensures all requirements are satisfied |

#### Role Inputs and Outputs

```
PLANNER
  Inputs:                         Outputs:
  ├── prompt.md                   ├── spec.md (normal loop)
  ├── state.json                  │   OR
  └── previous specs (context)    └── termination-request.md (completion)

IMPLEMENTER
  Inputs:                         Outputs:
  ├── spec.md                     ├── Code changes (git diff)
  ├── Current codebase            ├── impl-notes.md
  └── review-III-feedback.md *    └── impl-response-III.md *
      (* during review iterations)

REVIEWER
  Inputs:                         Outputs:
  ├── prompt.md                   ├── review-III-feedback.md
  ├── spec.md                     │   OR
  ├── git diff                    └── review-approved.md
  ├── impl-notes.md
  └── impl-response-III.md *
      (* during review iterations)

COMPLETER
  Inputs:                         Outputs:
  ├── prompt.md                   └── completer-verdict.md
  ├── state.json                      ├── COMPLETE: project done
  ├── All specs from project          └── CONTINUE: remaining items
  └── termination-request.md
```

### Loop Structure

```
┌─────────────────────────────────────────────────────────────────┐
│                        FEATURE LOOP N                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────┐    ┌─────────────┐    ┌──────────┐               │
│  │ Planner  │───▶│ Implementer │───▶│ Reviewer │               │
│  │(Backend A)│   │ (Backend B) │    │(Backend A)│               │
│  └──────────┘    └─────────────┘    └────┬─────┘               │
│                                          │                      │
│                         ┌────────────────┴────────────────┐    │
│                         │                                 │    │
│                         ▼                                 ▼    │
│                   [Suggestions]                     [Approved] │
│                         │                                 │    │
│                         ▼                                 ▼    │
│                  ┌─────────────┐                    ┌─────────┐│
│                  │ Implementer │                    │ COMMIT  ││
│                  │ (Backend B) │                    │  CODE   ││
│                  └──────┬──────┘                    └────┬────┘│
│                         │                                │     │
│                         ▼                                ▼     │
│                   ┌──────────┐                     ┌──────────┐│
│                   │ Reviewer │◀────────────────────│NEXT LOOP ││
│                   │(Backend A)│ (loop until approved)└──────────┘│
│                   └──────────┘                                 │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                        FEATURE LOOP N+1                        │
├─────────────────────────────────────────────────────────────────┤
│  Backends swap: Planner(B), Implementer(A), Reviewer(B)        │
└─────────────────────────────────────────────────────────────────┘
```

### Backend Alternation Pattern

| Loop | Planner | Implementer | Reviewer |
|------|---------|-------------|----------|
| 1 | Claude | Codex | Claude |
| 2 | Codex | Claude | Codex |
| 3 | Claude | Codex | Claude |
| N | (N%2==1 ? Claude : Codex) | (N%2==1 ? Codex : Claude) | (N%2==1 ? Claude : Codex) |

The Implementer always uses the opposite backend from Planner/Reviewer to maximize perspective diversity.  
Completion attempts also consume loop numbers, so backend parity continues monotonically across both feature and completion loops.
For completion loop `N`, Planner uses the parity-selected backend and Completer uses the opposite backend.

## Detailed Workflow

### Phase 1: Planning

```
Input:
  - prompt.md (the full project specification)
  - state.json (completed features, current status)

Planner Output (stored as `spec.md` by the orchestrator):
  - Feature name and description
  - Acceptance criteria
  - Files to create/modify
  - Dependencies on other features
  - OR: suggestion that project is complete (stored as `termination-request.md`)
```

### Phase 2: Implementation

```
Input:
  - spec.md
  - Current codebase

Implementer Output:
  - Code changes (files created/modified)
  - `impl-notes.md` body content explaining decisions
  - List of any spec items that couldn't be implemented (with reasons)
```

### Phase 3: Review Loop

```
Input:
  - prompt.md
  - spec.md
  - Git diff of implementation
  - impl-notes.md

Reviewer Output:
  - `# Review: APPROVED` -> proceed to commit (stored as `review-approved.md`)
  - `# Review: SUGGESTIONS` -> list of required changes (stored as `review-III-feedback.md`)
    - Each suggestion includes:
      - What needs to change
      - Why (referencing spec or master prompt)
```

If suggestions are returned, Implementer addresses them and Review repeats. Maximum feedback iterations configurable (default: 5).

### Phase 4: Commit

After approval:
1. Stage all changes
2. Generate commit message using the following precedence:
   - If `review-approved.md` contains a `## Commit Message` section, use that verbatim
   - Otherwise, generate from spec using `commit_message_style` from config:
     - `conventional`: `feat(ralph): {feature_name} [loop-{N}]`
     - `descriptive`: `{feature_name}\n\nImplemented via ralph loop {N}.\nBackends: planner={P}, implementer={I}, reviewer={R}`
     - `minimal`: `{feature_name}`
3. Create exactly one orchestrator-managed commit for the loop, with metadata tagging loop number and backends used
4. Tag that commit with `ralph/{project_id}/loop-{N}` for rollback reference

### Phase 5: Project Completion Check

When Planner suggests completion:

```
Input:
  - prompt.md
  - All `spec.md` artifacts generated in feature loops
  - Full project state
  - Planner's completion rationale

Completer (MUST be different backend than Planner):
  - COMPLETE: project finished (stored as `completer-verdict.md`)
  - CONTINUE: list of remaining work items (stored as `completer-verdict.md`)
```

## Data Structures

### Workspace Index (`.ralph/index.json`)

Tracks all projects in the workspace:

```json
{
  "workspace_version": "1.0",
  "created_at": "ISO8601",
  "active_project": "03-beta",
  "projects": [
    {
      "id": "01-poc",
      "name": "Proof of Concept",
      "status": "completed",
      "created_at": "ISO8601",
      "completed_at": "ISO8601",
      "total_feature_loops": 5,
      "total_completion_attempts": 1,
      "last_loop_number": 6,
      "parent_project": null
    },
    {
      "id": "02-alpha",
      "name": "Alpha Release",
      "status": "completed",
      "created_at": "ISO8601",
      "completed_at": "ISO8601",
      "total_feature_loops": 8,
      "total_completion_attempts": 2,
      "last_loop_number": 10,
      "parent_project": "01-poc"
    },
    {
      "id": "03-beta",
      "name": "Beta Release",
      "status": "in_progress",
      "created_at": "ISO8601",
      "completed_at": null,
      "total_feature_loops": 2,
      "total_completion_attempts": 0,
      "last_loop_number": 3,
      "parent_project": "02-alpha"
    }
  ]
}
```

Field notes:
- `total_feature_loops` counts approved feature loops.
- `total_completion_attempts` counts completed completion loops (`termination-request` + `completer-verdict`).
- `last_loop_number` is the highest allocated loop number across both loop types.

### Project State (`.ralph/projects/<id>/state.json`)

```json
{
  "project_id": "03-beta",
  "project_name": "Beta Release",
  "prompt_file": "prompt.md",
  "prompt_hash": "sha256-of-prompt-contents",
  "prompt_hash_at_loop_start": "sha256-when-current-loop-started",
  "parent_project": "02-alpha",  // Duplicated from index.json for self-contained state
  "current_loop": 3,
  "current_phase": "reviewing",  // planning | implementing | reviewing | committing | completing
  "phase_iteration": 2,          // Iteration counter for current phase; for reviewing, this is the next review iteration number
  "status": "in_progress",       // pending | in_progress | completed
  "loops": [
    {
      "loop_number": 1,
      "slug": "user-auth",
      "feature_name": "User Authentication",
      "loop_type": "feature",
      "status": "completed",
      "backends": {
        "planner": "claude",
        "implementer": "codex",
        "reviewer": "claude"
      },
      "artifacts": {
        "spec": "loops/001-user-auth/spec.md",
        "impl_notes": "loops/001-user-auth/impl-notes.md",
        "reviews": [  // Completed feedback/response cycles; phase_iteration is the next review iteration
          {
            "iteration": 1,
            "feedback": "loops/001-user-auth/review-001-feedback.md",
            "response": "loops/001-user-auth/impl-response-001.md"
          },
          {
            "iteration": 2,
            "feedback": "loops/001-user-auth/review-002-feedback.md",
            "response": "loops/001-user-auth/impl-response-002.md"
          }
        ],
        "approval": "loops/001-user-auth/review-approved.md"
      },
      "commit": "abc123",
      "started_at": "ISO8601",
      "completed_at": "ISO8601"
    },
    {
      "loop_number": 2,
      "slug": "database",
      "feature_name": "Database Schema",
      "loop_type": "feature",
      "status": "completed",
      "backends": {
        "planner": "codex",
        "implementer": "claude",
        "reviewer": "codex"
      },
      "artifacts": {
        "spec": "loops/002-database/spec.md",
        "impl_notes": "loops/002-database/impl-notes.md",
        "reviews": [],
        "approval": "loops/002-database/review-approved.md"
      },
      "commit": "ghi789",
      "started_at": "ISO8601",
      "completed_at": "ISO8601"
    },
    {
      "loop_number": 3,
      "slug": "api",
      "feature_name": "REST API Endpoints",
      "loop_type": "feature",
      "status": "in_progress",
      "backends": {
        "planner": "claude",
        "implementer": "codex",
        "reviewer": "claude"
      },
      "artifacts": {
        "spec": "loops/003-api/spec.md",
        "impl_notes": "loops/003-api/impl-notes.md",
        "reviews": [
          {
            "iteration": 1,
            "feedback": "loops/003-api/review-001-feedback.md",
            "response": "loops/003-api/impl-response-001.md"
          }
        ],
        "approval": null
      },
      "commit": null,
      "started_at": "ISO8601",
      "completed_at": null
    }
  ],
  "completion_attempts": []
}
```

Notes:
- This example shows an in-progress project before any completion attempt.
- `loops[]` and `completion_attempts[]` must use unique `loop_number` values within a project.
- To rebuild a full timeline, merge both arrays and sort by `loop_number`.
- Valid per-loop status values are `pending`, `in_progress`, `completed` for both arrays.
- Naming convention note: markdown frontmatter uses `loop`, while JSON state uses `loop_number`; these refer to the same logical loop index.
- `phase_iteration` semantics by phase:

| `current_phase` | `phase_iteration` meaning |
|-----------------|---------------------------|
| `planning` | Always `1` (single planner pass for the loop) |
| `implementing` | Always `1` on initial implementation pass; set to review iteration `N` when implementing response to `review-NNN-feedback.md` |
| `reviewing` | Next review iteration number to run (starts at `1`, increments after each feedback/response cycle) |
| `committing` | Always `1` (single commit finalization step) |
| `completing` | Always `1` (single completer verdict step per completion loop) |

Example `completion_attempts[]` entry:

```json
{
  "loop_number": 4,
  "slug": "completion",
  "loop_type": "completion",
  "status": "completed",
  "backends": {
    "planner": "codex",
    "completer": "claude"
  },
  "artifacts": {
    "termination_request": "loops/004-completion/termination-request.md",
    "verdict": "loops/004-completion/completer-verdict.md"
  },
  "verdict": "continue",
  "started_at": "ISO8601",
  "completed_at": "ISO8601"
}
```

`completion_attempts[]` field notes:
- `artifacts.verdict` and `completed_at` may be `null` while a completion loop is in progress.
- `verdict` is `continue` or `complete` once `completer-verdict.md` is written.
- `slug` is always the literal string `completion` for completion-attempt loops.

### Artifact Examples

All persisted artifact files use orchestrator-generated YAML frontmatter for metadata, followed by backend-generated markdown content.

#### `spec.md` (Planner → Implementer)

```markdown
---
artifact: spec
loop: 3
project: 03-beta
backend: claude
role: planner
created_at: 2026-02-05T14:30:00Z
---

# Feature: REST API Endpoints

## Description
Implement RESTful API endpoints for user management.

## Acceptance Criteria
- [ ] GET /api/users returns paginated user list
- [ ] POST /api/users creates a new user
- [ ] PUT /api/users/:id updates user
- [ ] DELETE /api/users/:id soft-deletes user

## Files to Modify/Create
- `src/api/mod.rs` - Add api module
- `src/api/users.rs` - User endpoints

## Dependencies
- Requires: loop-002-database
- Blocks: loop-004-frontend
```

#### `impl-notes.md` (Implementer → Reviewer)

```markdown
---
artifact: impl-notes
loop: 3
project: 03-beta
backend: codex
role: implementer
created_at: 2026-02-05T15:00:00Z
---

# Implementation Notes

## Decisions Made
- Used `axum` instead of `actix-web` because it's already in the project dependencies
- Pagination uses cursor-based approach for better performance with large datasets
- Soft-delete implemented via `deleted_at` timestamp column

## Spec Deviations
- Added `HEAD /api/users/:id` endpoint (not in spec) for existence checks - seemed useful
- Did NOT implement bulk delete - spec said "soft-deletes user" (singular)

## Testing
- Run `cargo test api::` to verify
- Manual testing: `curl http://localhost:8080/api/users`
```

#### `review-III-feedback.md` (Reviewer → Implementer)

```markdown
---
artifact: review-feedback
loop: 3
iteration: 1
project: 03-beta
backend: claude
role: reviewer
created_at: 2026-02-05T15:30:00Z
---

# Review: SUGGESTIONS

## Required Changes

1. **Missing Error Handling**
   - Current: Endpoints return 500 on any database error
   - Expected: Return appropriate 4xx/5xx codes with error messages
   - Reference: Master prompt section 3.2 "Error Handling Standards"

2. **Pagination Missing Total Count**
   - Current: Response only includes `items` array
   - Expected: Include `total_count` and `has_more` fields
   - Reference: Spec acceptance criterion 1

## Recommended Improvements
- Consider adding request validation middleware
- The `HEAD` endpoint addition is good, please add to spec
```

#### `impl-response-III.md` (Implementer → Reviewer)

```markdown
---
artifact: impl-response
loop: 3
iteration: 1
project: 03-beta
backend: codex
role: implementer
created_at: 2026-02-05T16:00:00Z
---

# Implementation Response (Iteration 1)

## Changes Made

### 1. Error Handling ✓
- Added custom error types in `src/api/errors.rs`
- All endpoints now return structured error responses
- Implemented error codes as per section 3.2

### 2. Pagination Total Count ✓
- Added `total_count` query with COUNT(*)
- Added `has_more` boolean field

## Could Not Address
- **Request validation middleware**: Would require significant refactoring of the
  router setup. Suggest deferring to a future loop focused on middleware.

## Pending Changes (Pre-Commit)
- Added structured error handling to API
- Added pagination metadata fields
- No commit was created yet; commit happens only after reviewer approval.
```

#### `review-approved.md` (Reviewer → Orchestrator)

```markdown
---
artifact: review-approved
loop: 3
project: 03-beta
backend: claude
role: reviewer
created_at: 2026-02-05T16:30:00Z
iterations: 2
---

# Review: APPROVED

## Acceptance Criteria Checklist
- [x] GET /api/users returns paginated user list (with total_count, has_more)
- [x] POST /api/users creates a new user
- [x] PUT /api/users/:id updates user
- [x] DELETE /api/users/:id soft-deletes user

## Notes
Error handling now follows project standards. Code is clean and well-tested.

Note: Request validation middleware deferred - acceptable for this loop.

## Commit Message
feat(api): complete REST user endpoints
```

#### `termination-request.md` (Planner → Completer)

```markdown
---
artifact: termination-request
loop: 5
project: 03-beta
backend: codex
role: planner
created_at: 2026-02-05T18:00:00Z
---

# Project Completion Request

## Rationale
All features specified in the master prompt have been implemented:

1. ✓ User Authentication (loop 1)
2. ✓ Database Schema (loop 2)
3. ✓ REST API Endpoints (loop 3)
4. ✓ Frontend Integration (loop 4)

## Summary of Work
- 4 feature loops completed
- 4 orchestrator loop commits
- All acceptance criteria met per review approvals

## Remaining Items
- Could add more comprehensive integration tests
- Documentation could be expanded

These are enhancements, not requirements from the master prompt.
```

#### `completer-verdict.md` (Completer → Orchestrator)

```markdown
---
artifact: completer-verdict
loop: 5
project: 03-beta
backend: claude
role: completer
created_at: 2026-02-05T18:30:00Z
---

# Verdict: CONTINUE

## Missing Requirements

### 1. Error Monitoring (Master Prompt Section 5)
> "The system must integrate with an error monitoring service"

This was not implemented in any loop. Need to add Sentry or similar.

### 2. Rate Limiting (Master Prompt Section 3.4)
> "All API endpoints must have rate limiting"

The API endpoints exist but rate limiting was never added.

## Recommended Next Features
1. **Loop 6**: Implement rate limiting middleware
2. **Loop 7**: Add error monitoring integration

After these, the project should meet all requirements.
```

### Global Configuration (`.ralph/ralph.toml`)

```toml
[workspace]
version = "1.0"
default_backend = "claude"  # Runtime fallback starting backend when run/project override is not set

[backends.claude]
command = "claude"
args = ["--dangerously-skip-permissions"]
timeout_seconds = 600
env = {}  # Optional environment variables

[backends.codex]
command = "codex"
args = []
timeout_seconds = 600
env = {}

[workflow]
max_review_iterations = 5          # Maximum reviewer feedback cycles per feature loop
auto_commit = true
commit_message_style = "conventional"  # conventional | descriptive | minimal
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"  # continue | restart-loop | abort

[templates]
# Global templates (can be overridden per-project)
planner = "templates/planner.md"          # Relative to .ralph/ (workspace root)
implementer = "templates/implementer.md"  # Relative to .ralph/
reviewer = "templates/reviewer.md"        # Relative to .ralph/
completer = "templates/completer.md"      # Relative to .ralph/

# Template variables available (use {{variable_name}} syntax):
#   {{project_id}}        - Current project ID (e.g., "03-beta")
#   {{project_name}}      - Human-readable project name
#   {{loop_number}}       - Current loop number (e.g., 3)
#   {{loop_slug}}         - Current loop slug (e.g., "user-auth")
#   {{feature_name}}      - Feature name from spec
#   {{phase}}             - Current phase (planning, implementing, reviewing)
#   {{iteration}}         - Review iteration number (1, 2, ...)
#   {{backend}}           - Backend executing this role
#   {{opposite_backend}}  - The other backend
#   {{prompt_content}}    - Full content of prompt.md
#   {{spec_content}}      - Full content of current spec.md
#   {{impl_notes_content}} - Full content of impl-notes.md
#   {{previous_specs}}    - Concatenated previous spec summaries
#   {{git_diff}}          - Current uncommitted changes
#   {{review_feedback_content}} - Current reviewer feedback content (for impl response)
#   {{feedback_content}}        - Alias of {{review_feedback_content}} (legacy template compatibility)
#   {{impl_response_content}}   - Current implementer response content (for reviewer follow-up iteration)
#   {{review_history}}          - Concatenated prior review feedback/response pairs for this loop
#   {{termination_request_content}} - Current termination-request content (for completer role)

[git]
# Git behavior
auto_branch = true                    # Create branch per project
branch_format = "ralph/{project_id}"  # Branch naming
sign_commits = false
base_branch = "master"                # Branch to create project branches from

# Git Branch Lifecycle:
# 1. On `ralph project new`: If auto_branch=true, create branch from base_branch
# 2. On `ralph project new --from <parent>`: Create branch from parent's branch tip
#    - This is a one-time snapshot; parent and child branches diverge naturally afterward.
#    - Ralph does not auto-sync child branches with future parent commits.
# 3. All feature-loop commits go to the project's branch
# 4. On project completion: Branch remains (user manually merges to master)
# 5. On `ralph rollback --hard`: Reset branch to specified loop's commit tag
#
# Branch naming: ralph/{project_id} (e.g., ralph/03-beta)
# Commit tags: ralph/{project_id}/loop-{N} (e.g., ralph/03-beta/loop-3)
```

### Per-Project Overrides (`.ralph/projects/<id>/config.toml`)

Optional file to override global settings for a specific project:

```toml
[workflow]
starting_backend = "codex"      # Override: start with codex as Planner
max_review_iterations = 3       # Stricter feedback-cycle limit for this project
auto_commit = false             # Optional project-specific commit behavior
commit_message_style = "minimal"
prompt_change_action = "restart-loop"

[templates]
# Use custom templates for this project
planner = "custom-planner.md"   # Relative to .ralph/projects/<id>/
implementer = "custom-implementer.md"
reviewer = "custom-reviewer.md"
completer = "custom-completer.md"
```

Backend resolution precedence is defined in **Canonical Conventions (Normative)** and is applied when selecting role backends during `ralph run`.

Per-project override schema (v1):

| Section | Keys supported in project config |
|---------|----------------------------------|
| `workflow` | `starting_backend`, `max_review_iterations`, `auto_commit`, `commit_message_style`, `prompt_change_action` |
| `templates` | `planner`, `implementer`, `reviewer`, `completer` |

Any unsupported key in project config is a validation error.

## CLI Interface

```
ralph - AI Backend Orchestration Tool

USAGE:
    ralph <COMMAND>

COMMANDS:
    # Workspace commands
    init          Initialize a new ralph workspace

    # Project commands
    project new   Create a new project
    project list  List all projects
    project use   Switch active project
    project show  Show project details

    # Orchestration commands
    run           Start or resume orchestration
    status        Show current project status
    history       Show loop history and decisions
    rollback      Rollback to a previous loop state

    # Configuration
    config        Manage configuration

EXAMPLES:
    # Initialize workspace
    ralph init

    # Create projects
    ralph project new --id 01-poc --name "Proof of Concept" --prompt ./poc-spec.md
    ralph project new --id 02-alpha --name "Alpha" --from 01-poc

    # Work on a project
    ralph project use 02-alpha
    ralph run
    ralph run --loops 3
    ralph status

    # View all projects
    ralph project list
```

### `ralph init`

Initialize a new workspace (run once per repo):

```
ralph init [OPTIONS]

OPTIONS:
    --dir <PATH>    Workspace directory (default: .ralph)

Creates:
    .ralph/
    ├── ralph.toml
    ├── index.json
    ├── projects/
    └── templates/
```

### `ralph project new`

Create a new project:

```
ralph project new [OPTIONS]

OPTIONS:
    --id <ID>           Project identifier (e.g., "01-poc", "02-alpha")
    --name <NAME>       Human-readable name
    --prompt <FILE>     Master prompt file to copy into project
    --from <PROJECT>    Inherit prompt from existing project (then edit it)
    --backend <BACKEND> Starting backend [claude|codex] (default: workspace.default_backend)

Behavior:
    - If `--backend` is provided, ralph persists it to project config as `workflow.starting_backend`.
    - If `--backend` is omitted, no project override is written and runtime falls back to precedence rules.

EXAMPLES:
    # New project with fresh prompt
    ralph project new --id 01-poc --name "Proof of Concept" --prompt PROMPT.md

    # Inherit from previous project
    ralph project new --id 02-alpha --name "Alpha Release" --from 01-poc
    # (copies 01-poc/prompt.md to 02-alpha/prompt.md for editing)
```

### `ralph project list`

```
$ ralph project list

PROJECTS IN WORKSPACE

  ID            NAME                STATUS        FEATURES  LAST_LOOP  ACTIVE
  ────────────────────────────────────────────────────────────────────────────
  01-poc        Proof of Concept    completed     5         6
  02-alpha      Alpha Release       completed     8         10
* 03-beta       Beta Release        in_progress   2         3          ◀
  04-refactor   (not started)       pending       -         -
  05-prod       (not started)       pending       -         -

* = currently active project
```

### `ralph project use`

Switch active project:

```
ralph project use <PROJECT_ID>

EXAMPLE:
    ralph project use 03-beta
```

### `ralph project show`

Show details for one project:

```
ralph project show [PROJECT_ID]

OPTIONS:
    --json          Output machine-readable JSON

BEHAVIOR:
    - If PROJECT_ID is omitted, shows the active project.
    - Includes prompt hash, current phase, loop summary, backend assignments, and parent project linkage.
```

### `ralph run`

```
ralph run [OPTIONS]

OPTIONS:
    --project <ID>        Run specific project (default: active project)
    --loops <N>           Maximum feature loops to complete in this invocation
    --until-review        Stop after next successful review (after writing review-approved.md, before commit phase)
    --until-complete      Run until Completer returns COMPLETE
    --dry-run             Show what would happen without executing
    --backend <BACKEND>   Override starting backend for this run only (highest precedence)
    --on-prompt-change <ACTION>
                         Behavior when prompt hash changes mid-loop:
                         continue | restart-loop | abort
    --skip-commit         Don't auto-commit (useful for testing)

NOTES:
    - `--loops`, `--until-review`, and `--until-complete` are mutually exclusive termination controls.
    - `--loops` counts completed feature loops only; completion attempts do not decrement this counter.
    - `--on-prompt-change` overrides `workflow.prompt_change_action` for this invocation.
    - `--backend` here is runtime-only; to persist a default backend for a project, use `ralph project new --backend` (or edit project config).
    - Preflight: `BackendRegistry::health_check_all()` runs once at the start of each `ralph run` invocation before phase execution; if any configured backend is unavailable, run exits before mutating state.
```

### `ralph status`

```
$ ralph status

WORKSPACE: /path/to/project/.ralph
ACTIVE PROJECT: 03-beta (Beta Release)

Project Status: in_progress
Current Loop: 3
Current Phase: reviewing (iteration 2)

┌─────────────────────────────────────────────────────────────┐
│ Loop 3: REST API Endpoints                                  │
├─────────────────────────────────────────────────────────────┤
│ Planner: claude    Implementer: codex    Reviewer: claude   │
│                                                             │
│ Latest Feedback (iteration 1):                              │
│   • Missing error handling in /api/users endpoint           │
│   • Tests needed for edge cases                             │
└─────────────────────────────────────────────────────────────┘

Previous Loops:
  [✓] Loop 1: User Authentication (2 feedback iterations)
  [✓] Loop 2: Database Schema (0 feedback iterations)

Loop artifacts: .ralph/projects/03-beta/loops/
  • 001-user-auth/spec.md
  • 002-database/spec.md
  • 003-api/spec.md (current)
```

### `ralph rollback`

Rollback to a previous loop state:

```
ralph rollback [OPTIONS] <LOOP_NUMBER>

ARGUMENTS:
    <LOOP_NUMBER>     The loop number to rollback to

OPTIONS:
    --project <ID>    Target project (default: active project)
    --hard            Also reset git to matching feature-loop commit tag (default: state only)
    --dry-run         Show what would be rolled back without doing it

BEHAVIOR:
    - Removes all loop directories after the specified loop number
    - Updates state.json to reflect the rollback
    - If a completion attempt is in progress (for example `termination-request.md` exists but `completer-verdict.md` does not), rollback removes that partial completion-loop directory and removes the corresponding `completion_attempts[]` entry when `verdict` is `null`.
    - With --hard:
      - If target is an approved feature loop with a tag, reset to `ralph/{project_id}/loop-{N}`
      - If target tag is missing (e.g., loop was approved with `--skip-commit`), fall back to the nearest prior tagged loop; if none exists, reset to project branch base commit
      - If target is a completion loop or an unapproved feature loop, reset to the most recent prior approved feature-loop tag
      - In-progress completion attempts have no commit tag; apply the same fallback rule above.
      - If no prior approved feature loop exists, reset to the project branch base commit
      - If the expected tag is missing and no fallback can be found, fail with a clear error message
    - Without --hard: preserves code changes, only resets orchestration state
    - Completion attempts after the target loop are also removed

EXAMPLES:
    # Rollback to loop 2 (state only, keep code)
    ralph rollback 2

    # Rollback to loop 2 including git reset
    ralph rollback 2 --hard

    # Preview rollback
    ralph rollback 2 --dry-run
```

### `ralph config`

Manage configuration:

```
ralph config <SUBCOMMAND>

SUBCOMMANDS:
    show              Show current configuration (merged global + project)
    get <KEY>         Get a specific config value
    set <KEY> <VALUE> Set a config value (default scope: active project if available, else global)
    edit              Open config file in $EDITOR

OPTIONS:
    --global          Target global config (.ralph/ralph.toml)
    --project <ID>    Target specific project config

SCOPE RULES:
    - `set/get/show` default to active project scope when an active project exists.
    - Without an active project, default scope is global.
    - `--global` forces global scope.
    - `--project <ID>` forces that project's scope.
    - `--global` and `--project` are mutually exclusive.

EXAMPLES:
    ralph config show
    ralph config get workflow.max_review_iterations
    ralph config set workflow.max_review_iterations 3
    ralph config edit --global
```

### `ralph history`

```
ralph history [OPTIONS]

OPTIONS:
    --project <ID>    Show history for specific project
    --verbose         Show detailed loop information
    --json            Output as JSON

$ ralph history --verbose

PROJECT: 03-beta (Beta Release)
PARENT: 02-alpha
PROMPT: prompt.md (sha256: abc123...)

LOOP HISTORY:

Loop 1: User Authentication
  Started:    2026-02-01T10:00:00Z
  Completed:  2026-02-01T11:30:00Z
  Backends:   planner=claude, implementer=codex, reviewer=claude
  Reviews:    2 feedback iterations
  Commit:     abc123
  Spec:       loops/001-user-auth/spec.md

Loop 2: Database Schema
  Started:    2026-02-01T11:35:00Z
  Completed:  2026-02-01T12:15:00Z
  Backends:   planner=codex, implementer=claude, reviewer=codex
  Reviews:    0 feedback iterations
  Commit:     ghi789
  Spec:       loops/002-database/spec.md

Loop 3: REST API Endpoints (IN PROGRESS)
  Started:    2026-02-01T12:20:00Z
  Phase:      reviewing (iteration 2)
  Backends:   planner=claude, implementer=codex, reviewer=claude
  Spec:       loops/003-api/spec.md
```

## Architecture

### Module Structure

```
src/
├── main.rs                     # CLI entry point
├── lib.rs                      # Library root
├── cli/
│   ├── mod.rs
│   ├── init.rs                 # Workspace initialization
│   ├── project.rs              # Project management (new, list, use, show)
│   ├── run.rs                  # Orchestration execution
│   ├── status.rs               # Status display
│   ├── history.rs              # History viewing
│   └── rollback.rs             # Rollback operations
├── backend/
│   ├── mod.rs                  # Backend trait definition
│   ├── claude.rs               # Claude CLI backend
│   ├── codex.rs                # Codex CLI backend
│   └── mock.rs                 # Mock backend for testing
├── workflow/
│   ├── mod.rs
│   ├── orchestrator.rs         # Main loop orchestration
│   ├── planner.rs              # Planner role logic
│   ├── implementer.rs          # Implementer role logic
│   ├── reviewer.rs             # Reviewer role logic
│   └── completer.rs            # Completer role logic
├── workspace/
│   ├── mod.rs
│   ├── index.rs                # Workspace index management
│   └── discovery.rs            # Find .ralph directory
├── project/
│   ├── mod.rs
│   ├── state.rs                # Project state (state.json)
│   ├── artifacts.rs            # Loop artifact file management
│   └── lifecycle.rs            # Project creation, inheritance
├── git/
│   ├── mod.rs
│   ├── commit.rs               # Commit operations
│   └── branch.rs               # Branch management per project
├── prompts/
│   ├── mod.rs
│   └── templates.rs            # Prompt templates
└── config/
    ├── mod.rs
    ├── global.rs               # Global ralph.toml
    └── project.rs              # Per-project config.toml
```

### Key Types and Traits

```rust
/// Workspace manages multiple projects
pub struct Workspace {
    pub root: PathBuf,              // .ralph directory
    pub config: GlobalConfig,
    pub index: WorkspaceIndex,
}

impl Workspace {
    pub fn discover() -> Result<Self>;           // Find .ralph from cwd
    pub fn init(path: &Path) -> Result<Self>;    // Create new workspace
    pub fn active_project(&self) -> Option<&ProjectRef>;
    pub fn list_projects(&self) -> Vec<ProjectRef>;
    pub fn get_project(&self, id: &str) -> Result<Project>;
}

/// A project within the workspace
pub struct Project {
    pub id: String,
    pub path: PathBuf,              // .ralph/projects/<id>
    pub prompt: Prompt,             // Loaded from prompt.md
    pub state: ProjectState,        // Loaded from state.json
    pub config: Option<ProjectConfig>,
}

impl Project {
    pub fn create(workspace: &Workspace, id: &str, name: &str, prompt: &Path) -> Result<Self>;
    pub fn inherit(workspace: &Workspace, id: &str, name: &str, from: &str) -> Result<Self>;
    pub fn current_loop(&self) -> Option<&Loop>;
    pub fn loop_specs(&self) -> Vec<ArtifactRef>;
    pub fn write_artifact(&mut self, loop_num: u32, artifact: ArtifactKind, body: &str) -> Result<PathBuf>;
}

/// A backend that can execute AI prompts
#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, prompt: &Prompt) -> Result<Response>;
    async fn health_check(&self) -> Result<()>;
}

/// A role in the workflow
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleKind {
    Planner,
    Implementer,
    Reviewer,
    Completer,
}

pub trait Role {
    fn kind(&self) -> RoleKind;
    fn system_prompt(&self) -> &str;
    fn build_prompt(&self, inputs: &RoleInputs) -> Prompt;
    fn parse_response(&self, response: &Response) -> Result<RoleOutput>;
}

/// Registry of available backends
pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn Backend>>,
    default_backend: String,
}

impl BackendRegistry {
    pub fn new(config: &GlobalConfig) -> Result<Self>;
    pub fn get(&self, name: &str) -> Option<Arc<dyn Backend>>;
    pub fn get_for_role(&self, role: RoleKind, loop_number: u32) -> Arc<dyn Backend>;
    pub fn opposite(&self, name: &str) -> &str;  // Returns the other backend
    pub fn health_check_all(&self) -> Result<()>;
}

/// Orchestrator runs the workflow for a project
pub struct Orchestrator {
    project: Project,
    backends: BackendRegistry,
}

impl Orchestrator {
    pub async fn run(&mut self, options: RunOptions) -> Result<OrchestrationResult>;
    pub async fn resume(&mut self) -> Result<OrchestrationResult>;
}
```

### Orchestrator State Machine

```
Init -> Planning

Planning -> Implementing    (planner produced feature spec)
Planning -> Completing      (planner suggested completion)

Implementing -> Reviewing
Reviewing -> Implementing   (review verdict: suggestions)
Reviewing -> Committing     (review verdict: approved)
Committing -> Planning      (next loop number)

Completing -> Complete      (completer verdict: COMPLETE)
Completing -> Planning      (completer verdict: CONTINUE, next loop number)
```

## Prompt Engineering

### Planner System Prompt

```markdown
You are a software architect planning features for a project.

Given `prompt.md` and `state.json`, you must:
1. Analyze what has been completed so far
2. Identify the next logical feature to implement
3. Write a detailed specification for that feature

Return markdown body only (no YAML frontmatter).  
Your output MUST be in this format:

# Feature: <name>

## Description
<what this feature does>

## Acceptance Criteria
- [ ] <criterion 1>
- [ ] <criterion 2>

## Files to Modify/Create
- `path/to/file.rs` - <what changes>

## Dependencies
- Requires: <previous feature or "none">
- Blocks: <future features or "none">

---

If the project is COMPLETE, output:

# Project Completion Request

## Rationale
<why all requirements are satisfied>

## Summary of Work
<what was built>

## Remaining Items
(optional)
- <non-blocking enhancements, or "None">
```

### Implementer System Prompt

```markdown
You are a software developer implementing a feature specification.

Given a feature spec, implement it by:
1. Creating/modifying the specified files
2. Following project conventions
3. Writing clean, tested code

Return markdown body only (no YAML frontmatter).

If this is the first implementation pass, output `impl-notes.md` in this format:

# Implementation Notes

## Decisions Made
- <decision and rationale>

## Spec Deviations
- <any items that couldn't be implemented exactly as specified, with explanation>

## Testing
- <how to verify the implementation>

If this is a review-response pass, output `impl-response-III.md` in this format:

# Implementation Response (Iteration <N>)

## Changes Made
1. <change tied to required feedback item>

## Could Not Address
- <feedback item not addressed and why> (or "None")

## Pending Changes (Pre-Commit)
(optional)
- <summary of uncommitted changes>
```

### Reviewer System Prompt

```markdown
You are a code reviewer ensuring implementations match specifications.

Given:
- `prompt.md`
- `spec.md`
- The implementation diff
- `impl-notes.md`

Review for:
1. Spec compliance - does it meet all acceptance criteria?
2. Code quality - is it clean, maintainable, secure?
3. Consistency - does it follow project patterns?

Return markdown body only (no YAML frontmatter).  
Your output MUST be:

# Review: APPROVED

## Acceptance Criteria Checklist
- [x] <criterion 1>
- [x] <criterion 2>

## Notes
(optional)
<approval rationale>

## Commit Message
(optional)
<single-line commit message suggestion>

---

OR:

# Review: SUGGESTIONS

## Required Changes
1. **<area>**: <what needs to change>
   - Current: <what it does now>
   - Expected: <what it should do>
   - Reference: <spec or prompt section>

## Recommended Improvements
(optional)
1. <suggestion>
```

### Completer System Prompt

```markdown
You are a project completion validator.

The Planner has suggested the project is complete. Your job is to:
1. Review requirements in `prompt.md`
2. Check all implemented features
3. Verify nothing is missing

You MUST use a DIFFERENT perspective than the Planner.

Return markdown body only (no YAML frontmatter).  
Output:

# Verdict: COMPLETE

The project satisfies all requirements:
- <requirement 1>: satisfied by <feature>
- ...

---

OR:

# Verdict: CONTINUE

## Missing Requirements
1. <requirement>: <why it's not satisfied>

## Recommended Next Features
1. <feature idea>
```

## Error Handling

### Recoverable Errors

| Error | Recovery |
|-------|----------|
| Backend timeout | Retry with exponential backoff (3 attempts) |
| Parse failure | Ask backend to reformat response (see below) |
| Git conflict | Pause and alert user |
| Review iteration limit | Pause and alert user |

Backend timeout retry policy:
1. First timeout: retry with backoff delay.
2. Second timeout: retry original request again with increased backoff delay.
3. Third timeout: fail phase with `BackendTimeoutExhausted`, persist state and raw backend output metadata, and exit non-zero.

#### Parse Failure Recovery (Reformat Flow)

When a backend response cannot be parsed (missing required sections, invalid format):

1. **First retry**: Send a reformat prompt to the SAME backend:
   ```
   Your previous response could not be parsed. The error was:
   {parse_error_message}

   Your original response was:
   ---
   {original_response}
   ---

   Please reformat your response following the required structure exactly:
   {expected_format_template}
   ```

2. **Second retry**: If reformat fails, retry the original prompt from scratch.

3. **Third retry**: If still failing, fail the phase with `ParseRetriesExhausted`, persist state and raw response, and exit non-zero.

### Fatal Errors

| Error | Action |
|-------|--------|
| Backend unavailable | Exit with error, preserve state |
| Backend timeout retries exhausted | Exit with `BackendTimeoutExhausted`, preserve state |
| Parse retries exhausted | Exit with `ParseRetriesExhausted`, preserve state |
| Invalid config | Exit with validation errors |
| Corrupted state | Attempt recovery from git, else exit |

### Orchestrator Error Types

```rust
pub enum OrchestratorError {
    BackendUnavailable { backend: String },
    BackendTimeoutExhausted { backend: String, phase: Phase, attempts: u8 },
    ParseRetriesExhausted { role: RoleKind, phase: Phase, attempts: u8 },
    StateLocked { project_id: String, lock_path: PathBuf },
    GitConflict { details: String },
    ReviewIterationLimitExceeded { loop_number: u32, max_iterations: u32 },
    InvalidConfig { key: String, reason: String },
    CorruptedState { path: PathBuf, reason: String },
}
```

### CLI Exit Codes

| Exit code | Meaning |
|-----------|---------|
| `0` | Success |
| `1` | Runtime orchestration failure (backend unavailable, timeout exhaustion, parse exhaustion, git conflict, corrupted state) |
| `2` | Usage/config validation error |
| `3` | Project lock contention (`StateLocked`) |

### State Recovery

On any interruption:
1. State is written after each phase transition
2. `ralph run` automatically resumes from last saved state
3. `ralph rollback` can revert to any previous loop

## Testing Strategy

### Unit Tests
- Prompt template rendering
- Response parsing
- State serialization
- Backend alternation logic

### Integration Tests
- Mock backend workflow execution
- Git operations
- State persistence/recovery

### E2E Tests
- Full workflow with mock backends
- Interrupt/resume scenarios
- Completion flow

## Future Enhancements

### Phase 2
- [ ] Parallel feature implementation (independent features within a loop)
- [ ] Custom backend plugins (Gemini, local LLMs, etc.)
- [ ] Web UI for monitoring orchestration
- [ ] Prompt optimization based on review patterns
- [ ] Cross-project spec search (find similar features from past projects)

### Phase 3
- [ ] Team collaboration (multiple humans + AIs)
- [ ] Cost tracking and optimization per project
- [ ] Learning from past projects (auto-suggest prompts based on history)
- [ ] Project templates (common patterns: API, CLI tool, web app, etc.)
- [ ] Diff between project prompts (track prompt evolution)

## Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
async-trait = "0.1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
git2 = "0.18"
sha2 = "0.10"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
```

## Getting Started

```bash
# 1. Initialize workspace (once per repo)
ralph init

# 2. Create your first project (PoC)
#    Write your master prompt describing what to build
ralph project new --id 01-poc --name "Proof of Concept" --prompt ./my-spec.md

# 3. Run the orchestration
ralph run

# 4. Monitor progress
ralph status

# 5. When PoC completes, create next project (inheriting the prompt)
ralph project new --id 02-alpha --name "Alpha Release" --from 01-poc

# 6. Edit the inherited prompt to add alpha requirements
#    .ralph/projects/02-alpha/prompt.md

# 7. Continue orchestration
ralph project use 02-alpha
ralph run

# View all projects
ralph project list

# View detailed history
ralph history --verbose
```

### Typical Project Progression

| Project | Purpose | Prompt Focus |
|---------|---------|--------------|
| `01-poc` | Prove core concept works | Minimal viable features |
| `02-alpha` | First usable version | Core features + basic UX |
| `03-beta` | Feature complete | All features + polish |
| `04-refactor` | Code quality | Architecture, tests, docs |
| `05-prod` | Production ready | Performance, security, deployment |

Each project's prompt evolves, building on learnings from the previous phase.

## License

MIT
