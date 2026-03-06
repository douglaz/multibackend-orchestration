## Summary

Remove the Gemini CLI backend from the Ralph codebase. OpenRouter (via Goose CLI) already provides access to Google models, making the direct Gemini CLI integration redundant. The project will support three backends: Claude (Anthropic CLI), Codex (OpenAI CLI), and OpenRouter (Goose CLI). This is a pure deletion/simplification task — no new code or abstractions are introduced.

## Acceptance Criteria

- [ ] No Gemini backend module, struct, or registration exists in source code
- [ ] No Gemini-related config keys (`backends.gemini.*`) are parsed, defaulted, or documented
- [ ] No `allows_gemini()` method or Gemini-specific validation guard exists in `src/config/mod.rs`
- [ ] All Gemini-specific tests and test fixtures are deleted across all modules (backend, config, validate, cli, workflow, daemon)
- [ ] No Gemini references remain in output normalizer comments or event-type lists
- [ ] The `[backends.gemini]` section is removed from `.ralph/ralph.toml`
- [ ] All validate harness and test setup functions no longer set `backends.gemini.enabled=false` (the key no longer exists)
- [ ] Optional-backend-skip and required-backend-failure panel tests are re-targeted to use a disabled supported backend (e.g., `openrouter` with `enabled=false`) so regression coverage is preserved
- [ ] `cargo build` succeeds with no warnings related to unused code from Gemini removal
- [ ] `cargo test` passes — all remaining Claude, Codex, and OpenRouter tests continue to pass
- [ ] `cargo clippy` passes
- [ ] A `ralph.toml` containing a leftover `[backends.gemini]` section is silently accepted at load time (serde ignores unknown fields since `deny_unknown_fields` is not set); no migration code is needed

## Technical Approach

### 1. Delete the Gemini backend module

Delete `src/backend/gemini.rs` entirely. Remove `pub mod gemini;` from `src/backend/mod.rs`.

### 2. Remove Gemini from backend routing and registry

In `src/backend/mod.rs`:
- Remove the `effective_args_gemini()` method and its match arm.
- Remove the Gemini match arm in the no-session JSON output rewriting block.
- Remove Gemini backend construction in the `BackendRegistry` constructor: the `gemini::backend_from_config()` call and `backends.insert("gemini", ...)` block.
- Remove the `"gemini"` entry from the `model_defaults` iterator and the `is_backend_available` enabled-mode list.
- Remove the `"gemini" =>` match arm in `create_backend_for_spec()`.
- Remove all Gemini-related unit tests: `parse_backend_spec_accepts_optional_bare_name` (~line 1420), `parse_backend_spec_accepts_optional_name_with_model` (~line 1432), `backend_registry_creates_gemini_backend_for_modeled_spec` (~line 1515), `backend_registry_rejects_disabled_backend` (~line 1526), `effective_args_no_session_gemini_rewrites_output_format_to_json` (~line 1727), `effective_args_gemini_rewrites_for_resume_and_keeps_print_flag` (~line 2058), `effective_args_gemini_resume_rewrite_is_idempotent` (~line 2097), and `is_backend_available_returns_false_for_disabled_backend` (~line 2270).

### 3. Remove Gemini from config parsing

In `src/config/global.rs`:
- Remove the `gemini: BackendConfig` field from `BackendConfigs` struct (~line 100).
- Remove `deserialize_gemini_backend_config()` (~line 613) and `default_gemini_backend_config()` (~lines 790–815).
- Remove config getters for `backends.gemini.*` (~lines 1152, 1162).
- Remove config setters for `backends.gemini.*` (~lines 1572–1640).
- Remove `"?gemini"` from any default `final_review_backends` or `completion_backends` lists (~lines 1012, 1036).

**Legacy config handling:** Because `BackendConfigs` does **not** use `#[serde(deny_unknown_fields)]`, a user's existing `ralph.toml` containing `[backends.gemini]` will be silently ignored by serde during deserialization. No migration or cleanup code is needed.

### 4. Remove Gemini validation surface logic

In `src/config/mod.rs`:
- Delete the `allows_gemini()` method from `ValidationSurface` (~lines 33–38).
- Remove the `parsed.name == "gemini"` guard clause and its error branch in `validate_backend_spec()` (~lines 546–550).
- Update the `validate_required_backend_spec()` doc-comment (~line 556) to remove the "and gemini backends" clause.
- Delete the following Gemini-specific unit tests:
  - `resolve_effective_config_rejects_gemini_on_required_surfaces` (~lines 972–1001)
  - `validate_prd_config_rejects_gemini_backend_specs` (~lines 1232–1247)
  - `validate_daemon_workspace_config_rejects_gemini_refinement_backend` (~lines 1250–1262)
  - `validate_effective_daemon_config_rejects_project_gemini_refinement_backend` (~lines 1265–1282)
  - `resolve_effective_config_accepts_optional_syntax_in_final_review_list` (~lines 1716–1734) — retarget this test to use `"?openrouter"` instead of `"?gemini"` so optional-syntax coverage is preserved
  - `prompt_review_alias_rejects_optional_global_singular_backend` (~lines 1817–1833) — retarget to use `"?openrouter"` instead of `"?gemini"` (this tests optional-syntax rejection on singular aliases, which is gemini-independent behavior)
  - `prompt_review_panel_accepts_optional_gemini_backend` (~lines 2001–2014) — retarget to use `"?openrouter"` instead of `"?gemini"`

### 5. Remove Gemini from CLI backend spec validation

In `src/cli/backend_spec.rs`:
- Remove `"gemini"` from the hardcoded known-backend list in `validate_backend_spec_name()` (~line 27). Update the doc-comment (~line 23) to list `claude, codex, openrouter`.
- Delete the `validate_name_only_accepts_optional_gemini_with_model` test (~lines 128–131).

### 6. Remove Gemini from CLI backend execution

In `src/cli/backend.rs`:
- Remove `gemini` from the `use crate::backend::{claude, codex, gemini, openrouter, ...}` import (~line 6).
- Remove the `"gemini" =>` match arm (~line 57).

### 7. Remove Gemini from CLI config tests

In `src/cli/config.rs`:
- `ensure_required_backend_rejects_optional_syntax` (~line 889): retarget from `"?gemini"` to `"?openrouter"` — this tests optional-syntax rejection, not gemini-specific behavior.
- `parse_optional_backend_accepts_optional_gemini` (~line 910): retarget to `"?openrouter(google/gemini-3-pro)"` or delete — the parsing logic is backend-name-agnostic.
- `set_global_value_rejects_optional_prompt_review_backend_alias` (~line 933): retarget from `"?gemini(gemini-3-pro-preview)"` to `"?openrouter(some-model)"`.

### 8. Clean up output normalizer comments

In `src/backend/output_normalizer.rs`:
- Remove the `// Gemini CLI stream-json` comment (~line 36). Keep `"init"`, `"message"`, `"tool_use"`, `"tool_result"` in the recognized event types list — these may be used by other backends or are harmless to keep.
- Remove the Gemini-specific branch in the `"message"` event handler: the `else if event.get("role") == Some("assistant")` block with the `// Gemini format: role at top level` comment (~lines 240–249). The Goose/OpenRouter `if let Some(inner) = event.get("message")` branch remains.
- Update or remove Gemini-referencing comments at lines 55, 201, 435.
- Delete Gemini-specific unit tests: `normalize_output_gemini_stream_extracts_session_and_text` (~line 934), `normalize_output_gemini_message_requires_assistant_role_for_text` (~line 954), `normalize_output_gemini_pipe_multiline_json_with_preamble` (~line 970), `normalize_output_gemini_429_error_before_response_json` (~line 1013).
- **Keep** `try_extract_multiline_json_after_preamble()` — it handles a general pattern (non-JSON preamble before JSON body) that may benefit other backends. Update its doc-comment to remove the Gemini reference.

### 9. Delete Gemini-specific test suite

Delete `src/validate/tests_gemini_backend.rs` entirely. In `src/validate/mod.rs`:
- Remove the `mod tests_gemini_backend;` declaration.
- Remove `tests_gemini_backend::tests()` from the test list.

### 10. Remove Gemini references from validate harness

In `src/validate/harness.rs`:
- Remove the three blocks that set `backends.gemini.enabled=false` (~lines 235–238, 274–277, 306–309).
- In `setup_mock_backends_fast()` (~lines 388–395): remove the `self.set_config_fast("backends.gemini.enabled", "false")` call. The function should return the result of the last codex config call instead. Update the doc-comment to remove the "and disables gemini" clause.

### 11. Remove Gemini references from validate test files

**`src/validate/tests_quick_dev.rs`:**
- Delete the `auto_gemini_reviewer_fails_fast` test function (~lines 1178–1235) and its `ConformanceTest` entry (~lines 83–85).
- Remove all `backends.gemini.enabled=false` config blocks (~lines 169–177, 786–790, 960–964, 1026–1030, 1146–1150, 1319–1323). After gemini removal, the key no longer exists and these calls would fail.

**`src/validate/tests_resume_backend_resolution.rs`:**
- Retarget the optional-completer test (~lines 1443–1486) from `"?gemini"` to `"?openrouter"` with openrouter disabled via `backends.openrouter.enabled=false`. Update associated comments. This preserves coverage for optional-backend-skip behavior in completion panel resolution.

**`src/validate/tests_prompt_review_panel.rs`:**
- Delete `singular_alias_rejects_optional_global_gemini` test (~lines 593–610) and its `ConformanceTest` entry (~lines 55–57). This tested gemini-specific validation that no longer exists.
- Delete `singular_alias_rejects_optional_project_gemini` test (~lines 614–640) and its `ConformanceTest` entry (~lines 59–61). Same reason.
- Remove the gemini-disabling block in `setup_panel_mocks()` (~lines 236–243).
- Delete `configure_gemini_mock()` helper (~lines 252–276).
- In `mixed_accept_reject_aggregation` (~line 349): remove the `configure_gemini_mock()` call, remove `"gemini"` from the `prompt_review_backends` list, and remove the gemini-specific assertions (~lines 371, 375). The test still covers mixed accept/reject with claude and codex.
- In `optional_validator_skipping` (~line 389): replace `"?gemini"` with `"?openrouter"` (openrouter is not mocked and thus unavailable, preserving optional-skip behavior). Update the assertion at ~line 400.
- In `optional_first_backend_falls_through` (~line 413): replace `"?gemini"` with `"?openrouter"`. Update the assertion at ~line 428.
- In `prompt_original_guard_prevents_artifact_writes` (~line 494): replace `"?gemini"` with `"?openrouter"`.

**`src/validate/tests_completion_panel.rs`:**
- Retarget `optional_backend_skip` (~lines 578–637): replace `"?gemini"` with `"?openrouter"` and add `backends.openrouter.enabled=false` config (or rely on openrouter not being mocked). Update comments referencing gemini.
- Retarget `required_backend_failure` (~lines 643–670): replace `"gemini"` with `"openrouter"` and ensure openrouter is disabled. Update comments.

**`src/validate/tests_stray_cleanup.rs`:**
- Remove the `backends.gemini.enabled=false` config block (~lines 89–97).

**`src/validate/tests_e2e_conformance.rs`:**
- Remove the `h.set_config_fast("backends.gemini.enabled", "false")` line (~line 616) from `setup_panel_mock`.

### 12. Remove Gemini from workflow orchestrator tests

In `src/workflow/orchestrator.rs`:
- In `preload_role_model_backends_creates_expected_entries_for_default_config` (~line 6110): remove the `assert!(registry.get("gemini(gemini-3-pro-preview)").is_some())` line (~line 6122).
- In `preload_role_model_backends_is_noop_when_models_are_unset` (~line 6126): remove `config.backends.gemini.models = BackendRoleModels::default()` (~line 6130) and `assert!(registry.get("gemini(gemini-3-pro-preview)").is_none())` (~line 6138).
- In `preload_role_model_backends_covers_all_roles_for_all_backends` (~line 6142): remove the entire `config.backends.gemini.models = BackendRoleModels { ... }` block (~lines 6166–6176) and remove all nine `"gemini(...)"` entries from the expected-spec assertion list (~lines 6201–6209).

### 13. Remove Gemini from daemon refine tests

In `src/daemon/refine.rs`:
- In `create_backend_rejects_unknown` (~line 365): retarget the test input from `"gemini(pro)"` to `"badbackend(pro)"` — this test validates the unknown-backend rejection path, which is gemini-independent behavior. The test remains valuable.

### 14. Remove Gemini from config file

In `.ralph/ralph.toml`: Delete the entire `[backends.gemini]` section (lines 104–122), including sub-tables `[backends.gemini.env]`, `[backends.gemini.models]`, and `[backends.gemini.role_timeouts]`.

## Files & Modules

| Action | File |
|--------|------|
| **Delete** | `src/backend/gemini.rs` |
| **Delete** | `src/validate/tests_gemini_backend.rs` |
| **Edit** | `src/backend/mod.rs` — remove module decl, routing, registry entries, `effective_args_gemini()`, tests |
| **Edit** | `src/backend/output_normalizer.rs` — remove Gemini message branch, comments, tests |
| **Edit** | `src/config/global.rs` — remove `BackendConfigs.gemini` field, deserializer, defaults, getters, setters |
| **Edit** | `src/config/mod.rs` — remove `allows_gemini()`, Gemini validation guard, doc-comment update, delete/retarget 7 tests |
| **Edit** | `src/cli/backend.rs` — remove import and match arm |
| **Edit** | `src/cli/backend_spec.rs` — remove from known-backend list, update doc-comment, delete test |
| **Edit** | `src/cli/config.rs` — retarget 3 tests from gemini to openrouter/other backend names |
| **Edit** | `src/workflow/orchestrator.rs` — remove gemini lines from 3 preload tests |
| **Edit** | `src/daemon/refine.rs` — retarget `create_backend_rejects_unknown` test from `"gemini(pro)"` to `"badbackend(pro)"` |
| **Edit** | `src/validate/mod.rs` — remove module decl and test registration |
| **Edit** | `src/validate/harness.rs` — remove gemini-disabling blocks, update `setup_mock_backends_fast` |
| **Edit** | `src/validate/tests_quick_dev.rs` — delete gemini test, remove all gemini-disabling config blocks (6 occurrences) |
| **Edit** | `src/validate/tests_resume_backend_resolution.rs` — retarget `?gemini` to `?openrouter` |
| **Edit** | `src/validate/tests_prompt_review_panel.rs` — delete 2 gemini rejection tests and mock helper, retarget 3 tests, trim mixed test |
| **Edit** | `src/validate/tests_completion_panel.rs` — retarget `optional_backend_skip` and `required_backend_failure` to openrouter |
| **Edit** | `src/validate/tests_stray_cleanup.rs` — remove gemini-disabling block |
| **Edit** | `src/validate/tests_e2e_conformance.rs` — remove gemini-disabling line |
| **Edit** | `.ralph/ralph.toml` — delete `[backends.gemini]` section |

## Testing Strategy

1. **Compile check**: `cargo build` must succeed with no dead-code warnings from removed Gemini paths.
2. **Unit tests**: `cargo test` — all existing Claude, Codex, and OpenRouter tests pass. Deleted Gemini tests no longer exist. Retargeted tests (optional-backend-skip, required-backend-failure, unknown-backend-rejection) pass with their new backend targets.
3. **Lint**: `cargo clippy` passes cleanly.
4. **Validate harness**: `ralph validate` passes with Gemini fully removed. The harness no longer sets `backends.gemini.enabled=false` since the key doesn't exist. Retargeted panel tests use disabled `openrouter` to exercise the same optional-skip and required-failure code paths.
5. **Regression check for output normalizer**: Verify that Goose/OpenRouter stream-json parsing still works correctly — the `"message"` event handler's Goose branch (`event.get("message")`) is unaffected by removing the Gemini branch (`event.get("role")`).
6. **Legacy config tolerance**: Verify that loading a `ralph.toml` containing a leftover `[backends.gemini]` section succeeds silently. Since `BackendConfigs` does not use `#[serde(deny_unknown_fields)]`, serde ignores the unknown `gemini` field after the struct field is removed. No explicit test is required — this is a serde guarantee — but a manual smoke test is prudent.
7. **Retargeted test coverage**: Confirm that the following behaviors are still covered after retargeting:
   - Optional backend skip on panel surfaces (`?openrouter` with openrouter disabled)
   - Required backend failure on panel surfaces (`openrouter` required but disabled)
   - Unknown backend rejection in daemon refinement (`"badbackend(pro)"`)
   - Optional syntax acceptance/rejection on various config surfaces (using `?openrouter`)

## Out of Scope

- Adding new backends or restructuring the backend abstraction
- Refactoring `ValidationSurface` enum variants (keep `RequiredPanel` even if it was primarily for Gemini — it serves the arbiter surface)
- Removing or modifying the `try_extract_multiline_json_after_preamble()` utility (it's general-purpose; only its doc-comment is updated)
- Updating OpenRouter/Goose to explicitly advertise Google model support
- Cleaning up historical `.ralph/projects/` artifacts that reference Gemini in filenames or content — these are immutable project history records, not source code or active configuration
- Changes to CI pipeline configuration
- Adding config migration or deprecation warnings for leftover `[backends.gemini]` sections in user configs (serde silently ignores them, which is acceptable)