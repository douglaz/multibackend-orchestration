Now I have a thorough understanding of all the codbase components relevant to the review issues. Let me produce the revised specification.

---

## Summary

Add a **prompt review phase** that runs once at the start of project execution (before the first planning loop). An AI agent evaluates the project's prompt file for completeness, ambiguity, missing acceptance criteria, and feasibility, then rewrites it in-place so all subsequent loops operate from the refined version. The phase is tracked in project state to prevent re-running on resume, controlled by configuration (`workflow.prompt_review_enabled`, `workflow.prompt_review_backend`), and skippable via `--skip-prompt-review` on the `run` and `auto` CLI commands.

## Acceptance Criteria

1. When `workflow.prompt_review_enabled` is `true` (default) and prompt review has not yet run for the project, executing `ralph run` or `ralph auto` invokes the prompt reviewer backend **before** the first planning phase.
2. The reviewer agent receives the full contents of the prompt file (resolved via `state.prompt_file`, not hardcoded `prompt.md`) via a template (`templates/prompt_reviewer.md`) and returns a structured response with `## Issues Found` and `## Refined Prompt` sections.
3. The `## Refined Prompt` section contents—extracted from the heading to **EOF** (not terminated by the next H2, since the refined prompt itself may contain H2 headings)—**replace** the prompt file on disk, and `state.prompt_hash` is updated to the new hash.
4. The original prompt file is preserved as `prompt-original.md` in the project directory before overwriting. If `prompt-original.md` already exists (e.g., from a manually aborted prior attempt), it is **not** overwritten—the phase fails with an explicit error directing the user to remove or rename the existing backup.
5. `state.prompt_review_completed` is set to `true` and persisted, ensuring the phase is skipped on resume.
6. `--skip-prompt-review` on `run` and `auto` commands bypasses the phase entirely (also sets `prompt_review_completed = true` in state so subsequent resumes don't trigger it).
7. `workflow.prompt_review_backend` defaults to `"codex(gpt-5.3-codex-xhigh)"` and follows **two-tier** precedence: project config > global config. There is no CLI flag for this backend (a `--prompt-review-backend` CLI flag is explicitly out of scope for this iteration).
8. `workflow.prompt_review_enabled` can be set to `false` in global or project config to disable the phase permanently.
9. The prompt reviewer output is parsed by a new `parse_prompt_reviewer_output()` function requiring H1 `# Prompt Review`, both `## Issues Found` and `## Refined Prompt` sections, and a **non-empty** refined prompt body.
10. The refined prompt artifact is written to the project directory (not a loop directory) as `prompt-review.md` with project-scoped frontmatter (fields: `artifact`, `project`, `backend`, `role`, `created_at`; no `loop`, `iteration`, or `iterations` fields).
11. `ralph run --dry-run` includes prompt review status in the dry-run summary (e.g., "prompt_review: pending" or "prompt_review: completed") but does not execute it.
12. **Migration safety**: For existing projects where `prompt_review_completed` defaults to `false`, the phase only runs if the project has **never started any loop** (`state.loops.is_empty() && state.completion_attempts.is_empty()`). Projects that have already begun looping are treated as if prompt review is completed—the flag is silently set to `true` and persisted.
13. `ralph config set/get/show` supports `workflow.prompt_review_enabled`, `workflow.prompt_review_backend`, and `templates.prompt_reviewer` at both global and project scope.
14. Validate conformance tests cover: prompt review runs and rewrites prompt, `--skip-prompt-review` bypasses the phase, resume skips already-completed review, config disabling works, `auto --skip-prompt-review` works, dry-run reports prompt review status, and parser handles refined prompts containing nested `##` headings.

## Technical Approach

### State tracking

Add a `prompt_review_completed: bool` field to `ProjectState` with `#[serde(default)]` for backward compatibility with existing serialized state. The orchestrator checks this field at the top of `Orchestrator::run()`, after loading state but before the main phase loop.

**Migration guard**: When `!state.prompt_review_completed` AND the project has already started looping (`!state.loops.is_empty() || !state.completion_attempts.is_empty()`), silently set `state.prompt_review_completed = true` and persist. This prevents existing in-progress projects from unexpectedly running prompt review on resume.

The full gate condition for executing prompt review is:
```
!state.prompt_review_completed
  && effective.workflow.prompt_review_enabled
  && !options.skip_prompt_review
  && state.loops.is_empty()
  && state.completion_attempts.is_empty()
```

### Orchestrator integration

Insert the prompt review as a **pre-loop step** in `Orchestrator::run()` (after the dry-run check at line 211 and before the `for _ in 0..MAX_PHASE_STEPS_PER_RUN` loop at line 217). This is intentionally **not** a new `Phase` enum variant—it is a one-shot pre-loop step, not a phase that participates in the loop state machine. This avoids disrupting the existing phase transition logic.

The implementation:
1. Read prompt file content via `project_dir.join(&state.prompt_file)`.
2. Render the `prompt_reviewer` template with `{{prompt_content}}` variable.
3. Resolve the backend from `workflow.prompt_review_backend` using `BackendRegistry::get_or_create_for_spec()`.
4. Execute the backend and parse the response via `parse_prompt_reviewer_output()`.
5. Validate that the `refined_prompt` body is non-empty (at least 10 characters after trimming); fail with `RalphError::ParseError` if empty.
6. Check that `prompt-original.md` does not already exist in the project directory; fail with `RalphError::Validation` if it does.
7. Copy current prompt file to `prompt-original.md`.
8. Write the refined prompt to the prompt file path (`state.prompt_file`).
9. Write the full reviewer response as `prompt-review.md` artifact in the project directory with project-scoped frontmatter (see Artifact section below).
10. Update `state.prompt_hash` and `state.prompt_hash_at_loop_start` to the new hash.
11. Set `state.prompt_review_completed = true`.
12. Save state.

**Failure safety**: If the parse fails, the refined prompt is empty, or a write error occurs at any step, no files are modified—the original prompt file remains intact. The backup-then-write ordering ensures: (a) backup is written first, (b) only if backup succeeds is the prompt file overwritten, (c) only if the prompt write succeeds is the artifact written and state updated. Any error at steps 4-9 causes the method to return `Err` without reaching the state-update steps 10-12. If step 8 fails after step 7 succeeds, `prompt-original.md` exists as a safety net for manual recovery.

**Skip-flag handling**: When `--skip-prompt-review` is set, `state.prompt_review_completed` is set to `true` and saved immediately (before the main loop) so that future resumes without the flag also skip the phase.

### Configuration

Add to `WorkflowConfig` in `src/config/global.rs`:
```rust
#[serde(default = "default_prompt_review_enabled")]
pub prompt_review_enabled: bool,          // default: true
#[serde(default = "default_prompt_review_backend")]
pub prompt_review_backend: String,        // default: "codex(gpt-5.3-codex-xhigh)"
```

Add to `ProjectWorkflowOverrides` in `src/config/project.rs`:
```rust
pub prompt_review_enabled: Option<bool>,
pub prompt_review_backend: Option<String>,
```

Add to `EffectiveWorkflowConfig` in `src/config/mod.rs`:
```rust
pub prompt_review_enabled: bool,
pub prompt_review_backend: String,
```

Wire through `resolve_effective_config()` with **two-tier** precedence (project > global):
```rust
prompt_review_enabled: project_ref
    .and_then(|p| p.workflow.prompt_review_enabled)
    .unwrap_or(global.workflow.prompt_review_enabled),
prompt_review_backend: project_ref
    .and_then(|p| p.workflow.prompt_review_backend.clone())
    .unwrap_or_else(|| global.workflow.prompt_review_backend.clone()),
```

Validate `prompt_review_backend` via `validate_backend_spec()` alongside the other backend specs.

Add to `RunOptions`:
```rust
pub skip_prompt_review: bool,
```

### CLI changes

Add `--skip-prompt-review` flag to both `RunArgs` and `AutoArgs`:
```rust
#[arg(long)]
pub skip_prompt_review: bool,
```

Pass through to `RunOptions::skip_prompt_review` in both `src/cli/run.rs` and `src/cli/auto.rs`.

### Config CLI support

Add match arms in `src/cli/config.rs`:

**`set_global_value()`** — add:
```rust
"workflow.prompt_review_enabled" => { cfg.workflow.prompt_review_enabled = parse_bool(val)?; }
"workflow.prompt_review_backend" => { cfg.workflow.prompt_review_backend = val.to_owned(); }
"templates.prompt_reviewer" => { cfg.templates.prompt_reviewer = val.to_owned(); }
```

**`set_project_value()`** — add:
```rust
"workflow.prompt_review_enabled" => { cfg.workflow.prompt_review_enabled = Some(parse_bool(val)?); }
"workflow.prompt_review_backend" => { cfg.workflow.prompt_review_backend = Some(val.to_owned()); }
"templates.prompt_reviewer" => { cfg.templates.prompt_reviewer = Some(val.to_owned()); }
```

The `config get` and `config show` commands work automatically via JSON serialization — no additional changes needed.

### Template

Create `templates/prompt_reviewer.md` (installed by `ralph init` alongside existing templates). Also add `default_prompt_reviewer_template()` in `src/prompts/templates.rs` as the fallback. The template opens with: `"You are a prompt reviewer"` (the string used by the mock script for detection).

Add to `TemplateConfig` in `src/config/global.rs`:
```rust
#[serde(default = "default_prompt_reviewer_template_path")]
pub prompt_reviewer: String,    // default: "templates/prompt_reviewer.md"
```

Add to `ProjectTemplateOverrides` in `src/config/project.rs`:
```rust
pub prompt_reviewer: Option<String>,
```

Add to `EffectiveTemplateConfig` in `src/config/mod.rs`:
```rust
pub prompt_reviewer: PathBuf,
```

Wire through `resolve_effective_config()` using the existing `resolve_template_path()` helper.

### Parser

Add `PromptReviewerDecision` struct and `parse_prompt_reviewer_output()` in `src/workflow/parser.rs`:

```rust
pub struct PromptReviewerDecision {
    pub body: String,
    pub refined_prompt: String,
}

pub fn parse_prompt_reviewer_output(raw: &str) -> Result<PromptReviewerDecision>
```

Validates:
- H1 is `# Prompt Review`
- Required sections: `## Issues Found`, `## Refined Prompt`
- Extracts everything after the `## Refined Prompt` heading **to EOF** (not until next H2), since the refined prompt itself is expected to contain H2 headings
- Validates that the extracted `refined_prompt` is non-empty after trimming (at least 10 characters); returns `RalphError::ParseError("refined prompt is empty or too short")` otherwise

The "extract to EOF" strategy is necessary because the refined prompt is a full project specification and will routinely contain `##` headings. This means `## Refined Prompt` **must be the last section** in the reviewer output. The parser enforces this by verifying that `## Issues Found` appears before `## Refined Prompt` in the output. The template instructions explicitly direct the reviewer to place `## Refined Prompt` last.

### Artifact

The prompt review artifact (`prompt-review.md`) is written to the **project directory** (not a loop directory) since it is project-scoped, not loop-scoped. It uses a **project-scoped frontmatter schema** distinct from the loop-scoped `ArtifactWriteInput`:

```yaml
---
artifact: prompt-review
project: <project_id>
backend: <backend_spec>
role: prompt_reviewer
created_at: <RFC3339 timestamp>
---
```

Fields intentionally **omitted** compared to loop-scoped artifacts: `loop`, `iteration`, `iterations`, `loop_slug`. This artifact is written directly via `fs::write()` rather than through `write_artifact()`, since `write_artifact()` requires a loop number and loop slug. A helper function `write_project_scoped_artifact()` is added to `src/project/artifacts.rs` to handle the frontmatter generation for this case, keeping the pattern extensible for future project-scoped artifacts.

### Dry-run summary

Extend `dry_run_summary()` in `src/workflow/orchestrator.rs` to include prompt review status. Before the existing summary logic, append a line indicating the prompt review state:
- If `state.prompt_review_completed`: `"prompt_review: completed"`
- Else if `!effective.workflow.prompt_review_enabled`: `"prompt_review: disabled"`
- Else if `options.skip_prompt_review`: `"prompt_review: will be skipped (--skip-prompt-review)"`
- Else: `"prompt_review: pending (backend: <backend>)"`

### Mock scripts

Extend `standard_mock_script()` to recognize the prompt reviewer template's system prompt prefix and return a well-formed `# Prompt Review` response. Detection matches the string `"You are a prompt reviewer"`:

```bash
elif echo "$INPUT" | grep -q "You are a prompt reviewer"; then
  # Extract the original prompt content for echo-back
  cat <<'MOCK_EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
MOCK_EOF
```

The mock returns a fixed refined prompt string rather than echoing the input, which gives tests a deterministic value to assert against.

## Files & Modules

| File | Change |
|------|--------|
| `src/project/state.rs` | Add `prompt_review_completed: bool` field (with `#[serde(default)]`) to `ProjectState` |
| `src/project/artifacts.rs` | Add `write_project_scoped_artifact()` helper for project-scoped artifacts (no loop number) |
| `src/config/global.rs` | Add `prompt_review_enabled`, `prompt_review_backend` to `WorkflowConfig`; add `prompt_reviewer` to `TemplateConfig`; add default fns |
| `src/config/project.rs` | Add `prompt_review_enabled`, `prompt_review_backend` to `ProjectWorkflowOverrides`; add `prompt_reviewer` to `ProjectTemplateOverrides` |
| `src/config/mod.rs` | Add `prompt_review_enabled`, `prompt_review_backend` to `EffectiveWorkflowConfig`; add `prompt_reviewer` to `EffectiveTemplateConfig`; wire through `resolve_effective_config()` with two-tier precedence and backend validation |
| `src/cli/mod.rs` | Add `--skip-prompt-review` to `RunArgs` |
| `src/cli/auto.rs` | Add `--skip-prompt-review` to `AutoArgs`; pass to `RunOptions` |
| `src/cli/run.rs` | Pass `skip_prompt_review` to `RunOptions` |
| `src/cli/config.rs` | Add `workflow.prompt_review_enabled`, `workflow.prompt_review_backend`, `templates.prompt_reviewer` match arms to both `set_global_value()` and `set_project_value()` |
| `src/workflow/orchestrator.rs` | Add `skip_prompt_review` to `RunOptions`; implement prompt review pre-loop step in `Orchestrator::run()` with migration guard; add `build_prompt_reviewer_prompt()` helper; extend `dry_run_summary()` to include prompt review status |
| `src/workflow/parser.rs` | Add `PromptReviewerDecision` struct and `parse_prompt_reviewer_output()` function with extract-to-EOF semantics and non-empty validation |
| `src/prompts/templates.rs` | Add `default_prompt_reviewer_template()` fallback (opening with `"You are a prompt reviewer"`) |
| `src/cli/init.rs` | Write `templates/prompt_reviewer.md` during `ralph init` using `default_prompt_reviewer_template()` |
| `src/validate/mock_scripts.rs` | Add prompt reviewer branch to `standard_mock_script()` matching `"You are a prompt reviewer"` |
| `src/validate/mod.rs` | Register `tests_prompt_review` module |
| `src/validate/tests_prompt_review.rs` | **New file** — conformance tests for the prompt review phase |

## Testing Strategy

### Unit tests (in-module `#[cfg(test)]`)

1. **Parser tests** (`src/workflow/parser.rs`):
   - Valid input with `# Prompt Review`, `## Issues Found`, `## Refined Prompt` → successful parse
   - Missing H1 → `ParseError`
   - Missing `## Issues Found` → `ParseError`
   - Missing `## Refined Prompt` → `ParseError`
   - Empty refined prompt (whitespace only after `## Refined Prompt`) → `ParseError`
   - Refined prompt too short (< 10 chars) → `ParseError`
   - **Refined prompt containing nested `##` headings** → successful parse, full content through EOF is captured
   - `## Issues Found` appearing after `## Refined Prompt` → `ParseError` (ordering enforced)
   - Extra sections before `## Refined Prompt` → successful parse (only ordering relative to `## Refined Prompt` matters)

2. **Config tests** (`src/config/global.rs`):
   - `prompt_review_enabled` defaults to `true`
   - `prompt_review_backend` defaults to `"codex(gpt-5.3-codex-xhigh)"`
   - Deserialization of TOML without new fields uses defaults (backward compat)
   - Deserialization with explicit `prompt_review_enabled = false` works

3. **State tests** (`src/project/state.rs`):
   - `prompt_review_completed` defaults to `false` for `ProjectState::new()`
   - Deserializing legacy state JSON (missing `prompt_review_completed`) defaults to `false`

4. **CLI parse tests** (`src/cli/mod.rs`):
   - `ralph run --skip-prompt-review` parses successfully
   - `ralph auto --idea "test" --skip-prompt-review` parses successfully

### Validate conformance tests (`src/validate/tests_prompt_review.rs`)

| Test name | Behavior |
|-----------|----------|
| `prompt_review::runs_and_rewrites_prompt` | With standard mock, `ralph run --loops 1` creates `prompt-original.md`, rewrites prompt file with mock's refined content, creates `prompt-review.md` artifact with project-scoped frontmatter (no `loop` field), and sets `prompt_review_completed: true` in state. |
| `prompt_review::skip_flag_bypasses` | `ralph run --skip-prompt-review --loops 1` does not create `prompt-review.md` or `prompt-original.md`; state shows `prompt_review_completed: true`. |
| `prompt_review::auto_skip_flag_bypasses` | `ralph auto --idea "test" --skip-prompt-review` does not create `prompt-review.md` or `prompt-original.md` in the generated project directory. |
| `prompt_review::resume_skips_completed` | After a run that completes review, a second `ralph run` does not produce a second `prompt-review.md` artifact or overwrite the existing `prompt-original.md`. |
| `prompt_review::disabled_via_config` | Set `workflow.prompt_review_enabled = false`, run; no `prompt-review.md` or `prompt-original.md` created; `prompt_review_completed` remains `false` in state. |
| `prompt_review::dry_run_reports_status` | `ralph run --dry-run` output includes `prompt_review: pending` for a new project, and `prompt_review: completed` after a prior run. |
| `prompt_review::existing_project_migration` | Create a project, manually run one loop (so `state.loops` is non-empty), then remove `prompt_review_completed` from state JSON. Next `ralph run` sets `prompt_review_completed = true` without running the reviewer (no `prompt-review.md` created). |

### Mock script extension

The mock script detects the prompt reviewer template by matching the string `"You are a prompt reviewer"` (from `default_prompt_reviewer_template()`) and returns:

```markdown
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
```

The fixed output gives tests a deterministic string to assert against for the rewritten prompt file content.

## Out of Scope

- **Interactive prompt review** — no user confirmation step before overwriting the prompt file. The phase is fully autonomous.
- **Multi-round prompt refinement** — the reviewer runs exactly once; no iterative back-and-forth.
- **Diff display** — no CLI output showing what changed between original and refined prompt (can be added later).
- **MCP tool exposure** — no new MCP tools for prompt review; this is orchestrator-internal.
- **Rollback of prompt review** — `ralph rollback` operates on loop-scoped state; prompt review is project-scoped and preserved via `prompt-original.md` for manual restoration.
- **`--prompt-review-backend` CLI flag** — backend override is config-only initially; a CLI flag can be added in a follow-up if needed. AC7 reflects two-tier (project > global) precedence only.
- **Per-project template override for prompt reviewer** — the template path follows the existing resolution pattern via `ProjectTemplateOverrides`, so this works automatically, but is not explicitly surfaced as a new feature.

---

### Review Issue Resolution Summary

| # | Issue | Resolution |
|---|-------|------------|
| 1 | **Parser truncation on nested H2s** | Changed extraction to **"from `## Refined Prompt` to EOF"** instead of "until next H2". Parser enforces that `## Refined Prompt` is the last section and `## Issues Found` must precede it. Template instructs the reviewer to place `## Refined Prompt` last. |
| 2 | **Migration / resume of existing projects** | Added a **migration guard**: prompt review only runs when `state.loops.is_empty() && state.completion_attempts.is_empty()`. Existing projects with loops get `prompt_review_completed` silently set to `true`. Added AC12 and a dedicated conformance test (`existing_project_migration`). |
| 3 | **Config precedence contradiction** | Corrected AC7 to **two-tier** precedence (project > global). Explicitly stated no CLI flag for `prompt_review_backend`. Removed the contradictory "CLI override" claim. |
| 4 | **Missing `config.rs` updates** | Added `src/cli/config.rs` to the files table with explicit match arms for `workflow.prompt_review_enabled`, `workflow.prompt_review_backend`, and `templates.prompt_reviewer` at both global and project scope. Added AC13. |
| 5 | **Hardcoded `prompt.md`** | All references now use `state.prompt_file` for reading, writing, and backing up the prompt. AC2 explicitly states "resolved via `state.prompt_file`". |
| 6 | **Failure safety** | Added explicit ordering guarantees: backup first, then overwrite, then artifact, then state. Non-empty validation (≥10 chars) on refined prompt. Existing `prompt-original.md` causes a hard error (not silent overwrite). Detailed failure-mode analysis in orchestrator integration section. |
| 7 | **Missing test coverage** | Added: `auto_skip_flag_bypasses` test, `dry_run_reports_status` test, parser test for nested `##` headings in refined prompt, `existing_project_migration` test. Added AC14 enumerating all required test coverage. |
| 8 | **Artifact frontmatter ambiguity** | Defined explicit project-scoped frontmatter schema: `artifact`, `project`, `backend`, `role`, `created_at` only — no `loop`, `iteration`, or `iterations`. Added `write_project_scoped_artifact()` helper to avoid misusing the loop-oriented `write_artifact()`. Updated AC10. |