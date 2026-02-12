# Feature Spec: Add `auto` command — idea-to-implementation in one shot

## Overview

Add a `ralph auto` command that chains three existing operations into a single workflow:
1. Generate an engineering spec via the quick-prd pipeline (claude writes, codex reviews)
2. Create a new ralph project using the spec as the prompt
3. Run the project until complete

This gives users a one-command path from idea to implemented, tested, committed code.

## CLI Interface

```
ralph auto --idea "add exponential backoff retry to Backend::execute()" \
    [--project-id retry-backoff] \
    [--writer-backend claude] \
    [--reviewer-backend codex] \
    [--max-revisions 2] \
    [--backend claude] \
    [--planner-backend ...] \
    [--implementer-backend ...] \
    [--reviewer-backend ...] \
    [--qa-backend ...] \
    [--completer-backend ...] \
    [--skip-commit] \
    [--tmux] \
    [--no-tmux]
```

### Args

**Quick-PRD phase args:**
- `--idea` (required): Feature description (validated non-empty)
- `--writer-backend`: Backend for spec drafting (default: `claude`)
- `--reviewer-backend`: Backend for spec review (default: `codex`). Note: this is the spec reviewer, distinct from `--reviewer-backend` for the run phase. To disambiguate, use `--spec-writer` and `--spec-reviewer` for the quick-prd phase.
- `--max-revisions`: Max spec review cycles (default: 2)

Actually, to avoid confusion between the spec reviewer and the run reviewer, rename the quick-prd flags:
- `--spec-writer` (default: `claude`): Backend that writes the spec
- `--spec-reviewer` (default: `codex`): Backend that reviews the spec
- `--max-spec-revisions` (default: 2): Max write→review cycles for the spec

**Project creation args:**
- `--project-id` (optional): Project ID. If omitted, auto-derive from the idea by slugifying: lowercase, replace non-alphanumeric with `-`, collapse consecutive dashes, trim dashes, truncate to 40 chars.
- `--name` is auto-derived from the idea (first 60 chars).

**Run phase args (pass-through to `ralph run`):**
- `--backend`: Starting backend for orchestration
- `--planner-backend`, `--implementer-backend`, `--reviewer-backend`, `--qa-backend`, `--completer-backend`: Role overrides
- `--skip-commit`: Skip git commit phase
- `--tmux` / `--no-tmux`: Tmux mode control
- `--until-complete` is always implied (hardcoded true)
- `--dry-run`: If set, only run the quick-prd phase (print spec), do NOT create project or run

## Implementation

### New file: `src/cli/auto.rs`

Contains `AutoArgs` struct and `pub async fn execute(args: AutoArgs) -> Result<()>`.

The execute function does three steps sequentially:

**Step 1: Generate spec via quick-prd**
- Create `BackendRegistry` (tmux disabled for the spec phase)
- Validate and create writer + reviewer backends
- Health-check both
- Create `QuickPrdOptions` and `QuickPrdPipeline`
- Run pipeline → get `QuickPrdResult` with `spec_path`
- Print spec phase summary

**Step 2: Create project**
- Call `create_project()` from `src/project/lifecycle.rs` with:
  - `id`: from `--project-id` or slugified idea
  - `name`: idea text (truncated to 60 chars)
  - `source`: `PromptSource::File(result.spec_path)` pointing to the SPEC.md written by quick-prd
  - `starting_backend`: from `--backend` arg if provided
- Print project creation confirmation

**Step 3: Run project**
- Create a new `Workspace::discover()` (to pick up the newly created project)
- Create `Orchestrator::new(workspace)`
- Call `orchestrator.run()` with `RunOptions`:
  - `project: Some(project_id)`
  - `until_complete: true`
  - All other flags passed through from args
- Print run result summary

### Slugify helper

Add a `fn slugify_idea(idea: &str) -> String` function in `src/cli/auto.rs`:
- Lowercase the input
- Replace any run of non-alphanumeric, non-hyphen characters with a single `-`
- Trim leading/trailing dashes
- Truncate to 40 characters
- Trim trailing dash after truncation

### Modified file: `src/cli/mod.rs`

- Add `mod auto;`
- Add `Auto(auto::AutoArgs)` variant to `Commands` enum
- Add dispatch: `Commands::Auto(args) => auto::execute(args).await`

## Acceptance Criteria

1. `src/cli/auto.rs` exists with `AutoArgs` struct and `execute()` function
2. `Commands::Auto` registered in `src/cli/mod.rs`
3. `ralph auto --idea "test feature" --dry-run` runs the quick-prd phase only and prints the spec
4. `ralph auto --idea "test feature"` runs all 3 phases: spec → project → run
5. `--project-id` overrides auto-slugification
6. Auto-slugified IDs are lowercase, hyphenated, max 40 chars
7. `cargo check` compiles with zero errors
8. `cargo test` passes all existing + new tests
9. `nix build -L` clean release build succeeds

## Tests Required

Unit tests in `src/cli/auto.rs` or `src/cli/mod.rs`:
1. `test_slugify_idea_basic` — "add retry logic" → "add-retry-logic"
2. `test_slugify_idea_special_chars` — "fix bug #123 (urgent!)" → "fix-bug-123-urgent"
3. `test_slugify_idea_truncation` — long idea truncated to 40 chars, no trailing dash
4. `test_slugify_idea_consecutive_dashes` — "hello   world---test" → "hello-world-test"
5. `parses_auto_command_with_defaults` — clap parsing test for default values
6. `parses_auto_command_with_all_args` — clap parsing test with all flags
7. `rejects_auto_with_empty_idea` — `--idea ""` rejected

## Important notes

- The `auto` command reuses existing library functions (`QuickPrdPipeline`, `create_project`, `Orchestrator`). It does NOT duplicate their logic — it calls them directly.
- The workspace is re-discovered between step 2 and step 3 because `create_project` modifies the index on disk.
- Error handling: if quick-prd fails, exit with error (no project created). If project creation fails, exit with error (no run). If run fails partway, the project and commits up to that point are preserved.
