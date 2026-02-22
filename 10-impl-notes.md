# Implementation Notes

## Decisions Made
- **`run_draft_with_section_retry_sync` returns `InteractivePrdFailed` on final retry**: Instead of silently returning an incomplete spec, the function now returns an error with the specific missing section names. This ensures section-incomplete outputs never reach the draft comment posting logic.
- **Reviewer approval does not bypass section completeness**: In both `generate_draft_from_answers_with_timeout` and `generate_revision_from_feedback_with_timeout`, when the reviewer approves (`feedback.approved`) but `check_spec_sections()` still reports missing sections, the code falls through to the revision loop instead of returning `Ok(current_spec)`. This ensures section completeness is always enforced.
- **Revision acceptance requires `missing.is_empty()`**: Changed from `missing.len() < REQUIRED_SPEC_SECTION_COUNT` (which accepted any partial improvement) to `missing.is_empty()` (which requires all 6 sections). This is a strict enforcement that prevents any incomplete spec from being stored as `current_spec` during revision iterations.
- **Final section check after revision exhaustion**: Both `generate_*` functions now perform a final `check_spec_sections()` after the revision loop exhausts `max_revisions`. If sections are still missing, they return `InteractivePrdFailed` with the missing section names, which flows into the normal error accounting (`error_count`, `last_error`) via `finish_transition`.
- **`DRAFT_SECTION_RETRIES` and `REQUIRED_SPEC_SECTION_COUNT` made `pub`**: Needed for conformance tests and integration tests to reference these constants.
- **`ralph_bin_absolute()` multi-layout resolver**: The new resolver checks 6 sources in priority order: compile-time `CARGO_BIN_EXE_ralph`, runtime env, `RALPH_TEST_BIN`, then probes `target/{debug,release}/ralph` and `target/{triple}/{debug,release}/ralph` for detected and common triples. The panic message lists all searched paths to aid CI diagnosis.

## Spec Deviations
- None. All acceptance criteria are addressed as specified.

## Testing
- **Unit tests** (7 new in `src/daemon/interactive_prd.rs`): `section_complete_spec_passes_validation`, `section_incomplete_spec_reports_missing_sections`, `section_empty_spec_reports_all_missing`, `section_retry_constants_are_correct`, `section_incomplete_writer_output_would_fail_after_retries`, `section_reviewer_approval_does_not_bypass_completeness` — all pass (46/46 total).
- **Conformance tests** (4 new in `src/validate/tests_interactive_prd.rs`): `section_complete_spec_passes_validation`, `section_incomplete_draft_is_rejected`, `section_incomplete_revision_is_rejected`, `section_constants_are_correct` — registered in the `tests()` Vec.
- **Integration test binary resolver** (`tests/daemon_interactive_prd.rs`): `ralph_bin_absolute()` replaced with the multi-layout resolver supporting `option_env!`, runtime env, `RALPH_TEST_BIN`, `CARGO_TARGET_DIR`, triple-qualified paths, and diagnostic panic.
- **Build verification**: `nix develop -c cargo check` passes with zero warnings. `nix develop -c cargo test --lib -- interactive_prd` passes 46/46 tests. Integration test binary compiles successfully.
