---
artifact: prompt-review
project: add-a-dry-run-n-flag-to-the-ralph-init-c
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-17T21:14:15Z
---

# Prompt Review

## Issues Found
- The provided text is a changelog-style summary, not a standalone implementation prompt; downstream loops still need to infer the actual objective and scope.
- Output behavior is not fully specified (exact dry-run stdout contract and ordering), which weakens testability and can cause drift.
- Validation coverage is inconsistent: unreadable-target parity is required, but no concrete conformance test is named for it.
- Side-effect boundaries are underspecified; "dry-run" should explicitly guarantee zero filesystem mutations.
- Backward-compatibility boundaries are implicit; it should explicitly say real `init` behavior/output must remain unchanged except for shared planning internals.

## Refined Prompt
Implement dry-run support for `ralph init` with strong behavior parity, zero side effects, and conformance-test coverage.

### Goal
Add `--dry-run` (long) and `-n` (short) to `ralph init` so users can see planned workspace setup actions without creating or modifying files.

### Scope
- Update init command handling in the CLI flow.
- Refactor init internals to use one shared action plan for both dry-run printing and real execution.
- Add/extend validate conformance tests under `src/validate/`.
- Keep existing non-dry-run behavior compatible.

### Required Behavior
1. `ralph init <target> --dry-run` and `ralph init <target> -n` are equivalent.
2. Dry-run must run the same target validation as real init before branching into dry-run vs execute.
3. Validation parity must hold for both real and dry-run:
   - Non-empty directory target -> exit code `2`.
   - Target is a file/non-directory -> exit code `1`.
   - Unreadable/inaccessible target -> exit code `1`.
4. On valid dry-run:
   - Exit code `0`.
   - Print deterministic planned actions in execution order.
   - Do not print the real success line `initialized workspace at ...`.
   - Perform zero filesystem mutations (no files/dirs/symlinks created, removed, or changed).
5. On valid non-dry-run:
   - Behavior and output remain as before, except internals may now execute via shared action planning.

### Implementation Requirements
- Introduce shared constants for init content sources (including template files and legacy links), e.g. `TEMPLATE_FILES` and `LEGACY_LINKS`.
- Introduce a shared action representation (e.g. `InitAction` enum) and `plan_actions(...)`.
- Real workspace creation and dry-run output must both iterate the same planned action list.
- Extract and reuse `validate_target(...)` so both paths enforce identical checks and exit-code behavior.
- Keep action output deterministic and stable for conformance tests.

### Validate Conformance Tests
Add tests in `src/validate/tests_init.rs` (or extend existing init validate module) using existing harness patterns and assertion helpers:

- `dry_run_prints_actions`
  - Runs `init --dry-run` on an empty target.
  - Asserts full stdout action sequence and order.
  - Asserts no workspace artifacts are created.
- `dry_run_short_flag`
  - Verifies `-n` behaves exactly like `--dry-run`.
- `dry_run_rejects_nonempty_dir`
  - Asserts same failure behavior/exit code `2` as real init.
- `dry_run_rejects_file_target`
  - Asserts exit code `1`.
- `dry_run_rejects_unreadable_target`
  - Asserts exit code `1` (gate platform-specific setup if needed for determinism).

If a new validate test module is created, register it in `src/validate/mod.rs`.

### Acceptance Criteria
- `cargo test` passes.
- Validate suite passes for init-related coverage against built binary.
- Dry-run and real init share one source of truth for planned actions.
- Adding/removing template/link items updates both dry-run output and real execution without duplicate edits.
- No regressions in existing `init` success/failure behavior outside the new dry-run capability.
