I now have comprehensive understanding of the entire codebase. Let me write the specification.

---

## Summary

Add a **prompt review phase** that runs once at the start of project execution (before the first planning loop). An AI agent evaluates the project's `prompt.md` for completeness, ambiguity, missing acceptance criteria, and feasibility, then rewrites it in-place so all subsequent loops operate from the refined version. The phase is tracked in project state to prevent re-running on resume, controlled by configuration (`workflow.prompt_review_enabled`, `workflow.prompt_review_backend`), and skippable via `--skip-prompt-review` on the `run` and `auto` CLI commands.

## Acceptance Criteria

1. When `workflow.prompt_review_enabled` is `true` (default) and prompt review has not yet run for the project, executing `ralph run` or `ralph auto` invokes the prompt reviewer backend **before** the first planning phase.
2. The reviewer agent receives the full contents of `prompt.md` via a template (`templates/prompt_reviewer.md`) and returns a structured response with `## Issues Found` and `## Refined Prompt` sections.
3. The `## Refined Prompt` section contents **replace** `prompt.md` on disk, and `state.prompt_hash` is updated to the new hash.
4. The original `prompt.md` is preserved as `prompt-original.md` in the project directory before overwriting.
5. `state.prompt_review_completed` is set to `true` and persisted, ensuring the phase is skipped on resume.
6. `--skip-prompt-review` on `run` and `auto` commands bypasses the phase entirely (also sets `prompt_review_completed = true` in state so subsequent resumes don't trigger it).
7. `workflow.prompt_review_backend` defaults to `"codex(gpt-5.3-codex-xhigh)"` and follows the same three-tier precedence (CLI override > project config > global config).
8. `workflow.prompt_review_enabled` can be set to `false` in global or project config to disable the phase permanently.
9. The prompt reviewer output is parsed by a new `parse_prompt_reviewer_output()` function requiring H1 `# Prompt Review` and both `## Issues Found` and `## Refined Prompt` sections.
10. The refined prompt artifact is written to the project directory (not a loop directory) as `prompt-review.md` with standard frontmatter.
11. `ralph run --dry-run` prints the prompt review configuration but does not execute it.
12. Validate conformance tests cover: prompt review runs and rewrites prompt.md, `--skip-prompt-review` bypasses the phase, resume skips already-completed review, config disabling works.

## Technical Approach

### State tracking

Add a `prompt_review_completed: bool` field to `ProjectState` (with `#[serde(default)]` for backward compatibility). The orchestrator checks this field at the top of `Orchestrator::run()`, after loading state but before the main phase loop. If `!state.prompt_review_completed && prompt_review_enabled && !skip_prompt_review`, execute the review phase.

### Orchestrator integration

Insert the prompt review as a **pre-loop step** in `Orchestrator::run()` (around line 213, after dry-run check and before the `for _ in 0..MAX_PHASE_STEPS_PER_RUN` loop). This is intentionally **not** a new `Phase` enum variant—it is a one-shot pre-loop step, not a phase that participates in the loop state machine. This avoids disrupting the existing phase transition logic.

The implementation:
1. Read `prompt.md` content.
2. Render the `prompt_reviewer` template with `{{prompt_content}}` variable.
3. Resolve the backend from `workflow.prompt_review_backend` using `BackendRegistry::get_or_create_for_spec()`.
4. Execute the backend and parse the response via `parse_prompt_reviewer_output()`.
5. Extract the `## Refined Prompt` section body.
6. Copy current `prompt.md` to `prompt-original.md`.
7. Write the refined prompt to `prompt.md`.
8. Write the full reviewer response as `prompt-review.md` artifact in the project directory (with frontmatter).
9. Update `state.prompt_hash` to the new hash.
10. Set `state.prompt_review_completed = true`.
11. Save state.

### Configuration

Add to `WorkflowConfig`:
```rust
#[serde(default = "default_prompt_review_enabled")]
pub prompt_review_enabled: bool,          // default: true
#[serde(default = "default_prompt_review_backend")]
pub prompt_review_backend: String,        // default: "codex(gpt-5.3-codex-xhigh)"
```

Add to `ProjectWorkflowOverrides`:
```rust
pub prompt_review_enabled: Option<bool>,
pub prompt_review_backend: Option<String>,
```

Add to `EffectiveWorkflowConfig`:
```rust
pub prompt_review_enabled: bool,
pub prompt_review_backend: String,
```

Wire through `resolve_effective_config()` with standard three-tier precedence.

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

Pass through to `RunOptions::skip_prompt_review`.

### Template

Create `templates/prompt_reviewer.md` (installed by `ralph init` alongside existing templates). Also add `default_prompt_reviewer_template()` in `src/prompts/templates.rs` as the fallback.

Add to `TemplateConfig`:
```rust
#[serde(default = "default_prompt_reviewer_template_path")]
pub prompt_reviewer: String,    // default: "templates/prompt_reviewer.md"
```

Add to `EffectiveTemplateConfig`:
```rust
pub prompt_reviewer: PathBuf,
```

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
- Extracts everything after `## Refined Prompt` heading (until next H2 or EOF) as `refined_prompt`

### Artifact

The prompt review artifact is written to the **project directory** (not a loop directory) as `prompt-review.md` since it is project-scoped, not loop-scoped. Use standard frontmatter with `artifact: prompt-review`, `role: prompt_reviewer`, `backend: <backend>`.

### Mock scripts

Extend `standard_mock_script()` to recognize the prompt reviewer template's system prompt prefix and return a well-formed `# Prompt Review` response.

## Files & Modules

| File | Change |
|------|--------|
| `src/project/state.rs` | Add `prompt_review_completed: bool` field to `ProjectState` |
| `src/config/global.rs` | Add `prompt_review_enabled`, `prompt_review_backend` to `WorkflowConfig`; add `prompt_reviewer` to `TemplateConfig`; add default fns |
| `src/config/project.rs` | Add `prompt_review_enabled`, `prompt_review_backend` to `ProjectWorkflowOverrides`; add `prompt_reviewer` to `ProjectTemplateOverrides` |
| `src/config/mod.rs` | Add `prompt_review_enabled`, `prompt_review_backend` to `EffectiveWorkflowConfig`; add `prompt_reviewer` to `EffectiveTemplateConfig`; wire through `resolve_effective_config()` |
| `src/cli/mod.rs` | Add `--skip-prompt-review` to `RunArgs` |
| `src/cli/auto.rs` | Add `--skip-prompt-review` to `AutoArgs`; pass to `RunOptions` |
| `src/cli/run.rs` | Pass `skip_prompt_review` to `RunOptions` |
| `src/workflow/orchestrator.rs` | Add `skip_prompt_review` to `RunOptions`; implement prompt review pre-loop step in `Orchestrator::run()`; add `build_prompt_reviewer_prompt()` helper |
| `src/workflow/parser.rs` | Add `PromptReviewerDecision` struct and `parse_prompt_reviewer_output()` function |
| `src/prompts/templates.rs` | Add `default_prompt_reviewer_template()` fallback |
| `src/cli/init.rs` | Write `templates/prompt_reviewer.md` during `ralph init` |
| `src/validate/mock_scripts.rs` | Add prompt reviewer branch to `standard_mock_script()` |
| `src/validate/mod.rs` | Register `tests_prompt_review` module |
| `src/validate/tests_prompt_review.rs` | **New file** — conformance tests for the prompt review phase |

## Testing Strategy

### Unit tests (in-module `#[cfg(test)]`)

1. **Parser tests** (`src/workflow/parser.rs`): Test `parse_prompt_reviewer_output()` with valid input, missing H1, missing sections, and edge cases (empty refined prompt, extra sections).
2. **Config tests** (`src/config/global.rs`): Verify `prompt_review_enabled` defaults to `true`, `prompt_review_backend` defaults to `"codex(gpt-5.3-codex-xhigh)"`, and deserialization with/without the new fields.
3. **State tests** (`src/project/state.rs`): Verify `prompt_review_completed` defaults to `false` for both `new()` and deserialized legacy state JSON.
4. **CLI parse tests** (`src/cli/mod.rs`): Verify `--skip-prompt-review` parses on both `run` and `auto` commands.

### Validate conformance tests (`src/validate/tests_prompt_review.rs`)

| Test name | Behavior |
|-----------|----------|
| `prompt_review::runs_and_rewrites_prompt` | With standard mock, `ralph run --loops 1` creates `prompt-original.md`, rewrites `prompt.md`, creates `prompt-review.md` artifact, and sets `prompt_review_completed: true` in state. |
| `prompt_review::skip_flag_bypasses` | `ralph run --skip-prompt-review --loops 1` does not create `prompt-review.md` or `prompt-original.md`; state still shows `prompt_review_completed: true`. |
| `prompt_review::resume_skips_completed` | After a run that completes review, a second `ralph run` does not produce a second `prompt-review.md` artifact. |
| `prompt_review::disabled_via_config` | Set `workflow.prompt_review_enabled = false`, run; no `prompt-review.md` or `prompt-original.md` created. |

### Mock script extension

The mock script detects the prompt reviewer template by matching the string `"You are a prompt reviewer"` (from `default_prompt_reviewer_template()`) and returns:

```markdown
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
<echoes back the original prompt content unchanged for predictable test assertions>
```

## Out of Scope

- **Interactive prompt review** — no user confirmation step before overwriting `prompt.md`. The phase is fully autonomous.
- **Multi-round prompt refinement** — the reviewer runs exactly once; no iterative back-and-forth.
- **Diff display** — no CLI output showing what changed between original and refined prompt (can be added later).
- **MCP tool exposure** — no new MCP tools for prompt review; this is orchestrator-internal.
- **Rollback of prompt review** — `ralph rollback` operates on loop-scoped state; prompt review is project-scoped and preserved via `prompt-original.md` for manual restoration.
- **`--prompt-review-backend` CLI flag** — backend override is config-only initially; a CLI flag can be added in a follow-up if needed.
- **Per-project template override for prompt reviewer** — the template path follows the existing resolution pattern via `ProjectTemplateOverrides`, so this works automatically, but is not explicitly surfaced as a new feature.