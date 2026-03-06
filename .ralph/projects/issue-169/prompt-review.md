---
artifact: prompt-review
project: issue-169
backend: codex
role: prompt_reviewer
created_at: 2026-03-06T01:57:03Z
---

# Prompt Review

## Issues Found
- The prompt is overly tied to approximate line numbers and specific test names, which is brittle across rebases and can fail even when behavior is correct.
- It mixes desired outcomes with rigid implementation details, making it harder to optimize edits while still meeting the real goal.
- It says “no Gemini references remain,” but some retarget examples still include model IDs containing `gemini`, creating a contradictory pass condition.
- It relies on “no explicit test required” for legacy config tolerance, which weakens confidence for a schema-removal change.
- Verification commands are not aligned with this repo’s documented workflow (`nix develop -c ...`, `nix build`).
- Coverage expectations are spread across many bullets; preserved behaviors (optional skip, required failure, unknown backend rejection) should be centralized.
- Scope boundaries are not fully explicit, which can cause over-deletion (historical artifacts) or under-deletion (active source/config paths).
- User-facing behavior when `gemini` is still specified is implied but not explicitly stated as an acceptance behavior.

## Refined Prompt
### Title
Remove Gemini backend support from Ralph (deletion/simplification only)

### Objective
Delete direct Gemini backend support from the codebase and keep Ralph operating with exactly three backends: `claude`, `codex`, and `openrouter`.

### Constraints
- This is a removal task. Do not introduce new backend abstractions or migration machinery.
- Preserve existing behavior for supported backends.
- Keep regression coverage for optional-backend skip and required-backend failure by retargeting tests to a disabled supported backend (`openrouter`).

### In Scope
- Remove Gemini backend implementation, registration, config schema, CLI validation/execution paths, and Gemini-specific tests/fixtures.
- Retarget Gemini-dependent tests to backend-agnostic or `openrouter`-based equivalents.
- Update default config file content in repo to remove `[backends.gemini]`.
- Keep serde unknown-field tolerance for legacy user configs containing `[backends.gemini]`.

### Out of Scope
- Adding new backends.
- Refactoring backend architecture.
- Migrating or rewriting historical project artifacts under `.ralph/projects/...`.
- CI pipeline redesign.

### Required Code Changes
- Delete `src/backend/gemini.rs`.
- Delete `src/validate/tests_gemini_backend.rs`.
- Update `src/backend/mod.rs` to remove all Gemini module references, backend creation, spec routing, availability checks, argument rewriting, model defaults, and Gemini-only tests.
- Update `src/config/global.rs` to remove `backends.gemini` fields/defaults/deserializers/getters/setters and any default backend lists that include `?gemini`.
- Update `src/config/mod.rs` to remove Gemini-specific validation surface logic (`allows_gemini` and guards) and retarget/delete related tests.
- Update `src/cli/backend_spec.rs` to remove `gemini` from allowed backend names and update tests/docs accordingly.
- Update `src/cli/backend.rs` to remove Gemini backend construction path.
- Update `src/cli/config.rs` tests to stop using `?gemini`; use `?openrouter` or backend-agnostic values.
- Update `src/backend/output_normalizer.rs` to remove Gemini-specific branches/comments/tests while keeping generic multiline-JSON extraction utility.
- Update `src/validate/mod.rs` to unregister deleted Gemini validate suite.
- Update `src/validate/harness.rs` to remove all writes to `backends.gemini.enabled`.
- Update validate test modules that currently depend on Gemini to either delete Gemini-only cases or retarget to `openrouter` disabled-path behavior:
`tests_quick_dev.rs`, `tests_resume_backend_resolution.rs`, `tests_prompt_review_panel.rs`, `tests_completion_panel.rs`, `tests_stray_cleanup.rs`, `tests_e2e_conformance.rs`.
- Update `src/workflow/orchestrator.rs` tests to remove Gemini preload assertions/config.
- Update `src/daemon/refine.rs` unknown-backend test to use a truly unknown backend name (for example `badbackend(pro)`).
- Remove `[backends.gemini]` and child tables from `.ralph/ralph.toml`.

### Behavioral Requirements
- Backend spec `gemini` is treated as unknown/invalid everywhere user input is validated.
- Optional backend syntax behavior is still covered and working (for example `?openrouter` can be skipped when disabled/unavailable).
- Required backend behavior is still covered and working (required `openrouter` fails when disabled/unavailable).
- Loading a config file that still contains `[backends.gemini]` succeeds without migration code (unknown field ignored by serde in current schema).

### Acceptance Criteria
- No Gemini backend module, registration, or executable path remains in `src/`.
- No `backends.gemini.*` key is part of active config schema, defaults, getters, or setters.
- No Gemini-specific validation guard remains.
- No validate harness setup writes `backends.gemini.enabled`.
- Gemini-only tests are removed or retargeted with equivalent behavioral coverage.
- `.ralph/ralph.toml` no longer contains `[backends.gemini]`.
- Search check passes: `rg -n "\bgemini\b" src .ralph/ralph.toml` returns zero matches.
- Build and checks pass using project-standard commands:
`nix develop -c cargo check`
`nix develop -c cargo test`
`nix develop -c cargo clippy -- -D warnings`
`nix build -L`
- Conformance suite passes on built binary:
`./result/bin/ralph validate --bin ./result/bin/ralph`

### Implementation Notes
- Prefer behavior-based edits over line-number targeting.
- Where tests are retargeted, keep original intent and assertions (optional skip, required failure, unknown backend rejection).
- Avoid using model strings containing `gemini` in retargeted tests to keep the search-based acceptance criterion unambiguous.
