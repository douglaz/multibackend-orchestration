---
artifact: termination-request
loop: 3
project: prompt-review
backend: claude(opus)
role: planner
created_at: 2026-02-13T05:20:23Z
---

All 14 acceptance criteria from the master prompt are fully implemented across the two completed loops. Every file listed in the spec has been modified/created, all conformance tests are present, and the feature is complete.

# Project Completion Request

## Rationale
All 14 acceptance criteria from the master prompt specification are fully satisfied by the two completed loops:

- **AC1-2**: Orchestrator executes prompt review before the first planning phase, using `state.prompt_file` and the `prompt_reviewer` template
- **AC3**: Parser extracts `## Refined Prompt` to EOF and the orchestrator rewrites the prompt file with updated hash
- **AC4**: Original prompt is backed up to `prompt-original.md` with existing-backup guard
- **AC5**: `state.prompt_review_completed` is persisted after successful review
- **AC6**: `--skip-prompt-review` on both `run` and `auto` bypasses the phase and marks state completed
- **AC7**: Two-tier config precedence (project > global) for `workflow.prompt_review_backend`
- **AC8**: `workflow.prompt_review_enabled = false` disables the phase
- **AC9**: `parse_prompt_reviewer_output()` validates H1, both required sections, non-empty refined prompt
- **AC10**: `prompt-review.md` artifact written with project-scoped frontmatter via `write_project_scoped_artifact()`
- **AC11**: Dry-run summary reports prompt review status (pending/completed/disabled/skipped)
- **AC12**: Migration guard silently marks `prompt_review_completed = true` for projects with existing loops
- **AC13**: `ralph config set/get/show` supports all three prompt review config keys at both scopes
- **AC14**: All 7 conformance tests implemented: run-and-rewrite, skip flag, auto skip, resume skip, disabled config, dry-run status, migration

## Summary of Work
- **Loop 1** (Foundation): Added `prompt_review_completed` state field, config plumbing (`WorkflowConfig`, `ProjectWorkflowOverrides`, `EffectiveWorkflowConfig`), template config and default template, `parse_prompt_reviewer_output()` parser with extract-to-EOF semantics, `write_project_scoped_artifact()` helper, config CLI match arms, and `ralph init` template writing
- **Loop 2** (Runtime + Tests): Wired the foundation into `Orchestrator::run()` as a pre-loop step with migration guard, skip-flag handling, dry-run reporting, and failure safety ordering; added `--skip-prompt-review` to `RunArgs` and `AutoArgs`; extended mock scripts; created all 7 conformance tests in `tests_prompt_review.rs`

## Remaining Items
- None
