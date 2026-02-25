# Validate E2E Conformance for `ralph auto` Failure Semantics, Artifacts, and PR Metadata

## Goal
Implement and/or update conformance tests and docs so `ralph auto` behavior is unambiguous and regression-resistant across:
1. Auto-path execution feasibility
2. QA/review rollback behavior
3. Backend timeout handling
4. Reformatter fallback boundaries
5. Implementation-response artifact format
6. PR metadata generation

## In Scope
- Conformance tests under `src/validate/` (new or updated).
- Validate harness/script helpers needed by those tests.
- Documentation file `docs/validate-e2e.md`.
- Test registration wiring in `src/validate/mod.rs` if new test module(s) are added.

## Out of Scope
- Mid-phase state snapshot instrumentation inside runtime binaries.
- Any reformatter fallback beyond parse-error-driven flow.
- Changes to unrelated CLI/workflow behavior.

## Required Behavior (Normative)
1. Happy-path tests for `ralph auto` must use `auto_mock_script()` (not `standard_mock_script()`), so quick-PRD prompts and spec-writer/reviewer/reviser flow execute through the real `ralph auto`.
2. QA/review failure rollback must remove the loop from `state.json` via existing rollback semantics (`state.remove_loop()` behavior). With `--loops 1`, rollback leaves zero loops and task must end `failed` with non-zero exit.
3. Backend timeout must propagate as `BackendTimeoutExhausted` and mark the task `failed`.
4. Reformatter fallback rules:
   - Parse error: reformatter path allowed.
   - Backend non-zero exit: `BackendCommandFailed`, no reformatter.
   - Backend timeout: `BackendTimeoutExhausted`, no reformatter.
   - Empty output: retry same backend first, then reformatter attempt.
5. Review-feedback path must produce `*-impl-response-001.md` containing YAML frontmatter keys `artifact`, `iteration`, `role`, plus expected body content.
6. PR creation metadata must include:
   - `--title` with `ralph:` prefix
   - `--body-file` content containing `Closes #<N>`, diff stat, and project reference
   - `--head`
   - `--repo`

## Implementation Requirements

### 1. Mock Script Safety (No Recursion)
- Provide/use helper: `e2e_mock_ralph_script(ralph_bin: &Path)`.
- Script must embed absolute `h.ralph_bin` at generation time.
- Invocation pattern must be equivalent to:
  - `exec /absolute/path/to/ralph auto "$@"`
- Do not resolve `ralph` via `PATH` inside this mock.

### 2. Test Cases (Required)
Add/update tests with explicit setup/assertions equivalent to:

1. `backend_timeout_exhausted_fails_task`
- Mock backend sleeps (`sleep 30`).
- Set `backends.<name>.timeout_seconds = 2`.
- Assert surfaced error is `BackendTimeoutExhausted`.
- Assert daemon task state is `failed`.

2. QA/review rollback failure tests (rename legacy `*_new_loop` style names to `*_fails`)
- Run with `--loops 1`.
- Force QA failure and review failure separately.
- Assert:
  - non-zero orchestration exit
  - task state `failed`
  - `state.json` loops array is empty
  - loop directory is absent

3. `impl_response_artifact_on_review_feedback`
- Trigger review feedback iteration.
- Assert one artifact matching `*-impl-response-001.md`.
- Assert frontmatter includes `artifact`, `iteration`, `role`.
- Assert body contains expected implementation response content.

4. `pr_metadata_verification`
- Use `e2e_mock_gh_logging_script()` to capture full `gh pr create` args and `--body-file` content.
- Assert required flags/content listed in “Required Behavior”.

### 3. Reformatter Semantics Coverage
- Ensure tests explicitly encode:
  - non-zero exit => `BackendCommandFailed` and no reformatter
  - timeout => `BackendTimeoutExhausted` and no reformatter
  - empty output => same-backend retry, then reformatter
- If existing tests cover part of this, extend them rather than duplicating.

### 4. Documentation
Create/update `docs/validate-e2e.md` with exactly these sections:
1. Architecture overview
2. Test-to-requirement mapping
3. Running instructions
4. Troubleshooting
5. Adding-new-tests template

## File Targets
- `src/validate/tests_*.rs` (new and/or existing test modules)
- `src/validate/harness.rs` (mock helper functions as needed)
- `src/validate/mod.rs` (registration if new module)
- `docs/validate-e2e.md`

## Acceptance Criteria
1. All required tests exist, are registered, and pass locally.
2. Validate behavior matches all normative rules above.
3. No recursive mock `ralph` invocation is possible.
4. Docs file exists and includes all five required sections.
5. Commands succeed:
   - `nix develop -c cargo test`
   - `nix build -L`
   - `./result/bin/ralph validate --bin ./result/bin/ralph --filter mcp` (sanity; ensure no regressions)
   - `./result/bin/ralph validate --bin ./result/bin/ralph` (full suite expected green)

## Notes for Implementer
- Prefer extending existing validate patterns and helpers.
- Keep tests deterministic (no network dependency, controlled temp files/paths, bounded runtime).
- Preserve existing project conventions for naming and `run_case(|| { ... })` structure.