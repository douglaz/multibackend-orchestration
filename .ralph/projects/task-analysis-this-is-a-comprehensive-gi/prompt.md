### Title
Add Gemini backend support and panelized completion/prompt-review flows with optional backend specs.

### Goal
Implement Gemini CLI (`gemini`) as a third backend in Ralph, but only for:
1. Final review panel.
2. Completion panel.
3. Prompt-review panel.

Keep the planner/implementer/reviewer/QA alternation loop strictly `claude`/`codex`.

### Hard Constraints
1. Gemini must be rejected for `starting_backend`, `planner_backend`, `implementer_backend`, `reviewer_backend`, `qa_backend`, and daemon PRD/refinement backends.
2. `?backend` optional syntax is allowed only in backend-list panel surfaces:
   1. `final_review_backends`
   2. `completion_backends`
   3. `prompt_review_backends`
3. `?backend` is invalid on single-backend required surfaces.
4. Existing behavior for non-Gemini paths must remain unchanged.
5. Backward compatibility for old completion artifacts and old prompt-review config must be preserved.

### Definitions
1. Backend spec grammar: `[?]<name>(<model>)?`
2. `BackendSpec` must include `optional: bool`, `name: String`, `model: Option<String>`.
3. Backend availability policy:
   1. `enabled = "auto"`: lazy check only when backend is selected.
   2. `enabled = true`: eager startup health-check.
   3. `enabled = false`: always unavailable.
4. For optional specs on panel lists: unavailable backends are skipped with warning.
5. For required specs: unavailable backend is an error.

### Phase 1: Gemini Plumbing
1. Add `src/backend/gemini.rs` modeled after existing CLI backends.
2. Implement `backend_from_config(config, model, role) -> CliBackend`.
3. Implement `ensure_gemini_stream_json_args(args) -> Vec<String>`:
   1. Remove existing `--output-format` forms.
   2. Append canonical `--output-format stream-json`.
4. Update `src/backend/mod.rs`:
   1. Add `pub mod gemini;`.
   2. Extend `parse_backend_spec` for optional `?`.
   3. Add Gemini arms in registry creation and CLI dispatch.
   4. Add Gemini handling in `effective_args()` and JSON-output normalization path.
   5. Add `effective_args_gemini()` resume strategy:
      1. Strip old `--resume` and `--output-format`.
      2. Keep `-p`.
      3. Add `--resume <id>` and `--output-format json`.
6. Update `src/config/global.rs`:
   1. Add `BackendEnabled` serde for `true`/`false`/`"auto"`.
   2. Add `enabled` to `BackendConfig` with default `auto`.
   3. Add `backends.gemini` defaults:
      1. `command = "gemini"`
      2. `args = ["-p","--yolo","--output-format","stream-json"]`
      3. Models set only for `final_reviewer`, `arbiter`, `completer` to `"gemini-3-pro"`
      4. All other role models `None`
   4. Add `GlobalConfig::backend_config("gemini")`.
   5. Default `final_review_backends` must include `"?gemini"`.
7. Update `src/config/mod.rs`:
   1. Add surface-aware backend validation.
   2. Enforce Gemini guardrails.
   3. Enforce optional syntax restrictions by surface.
8. Update `src/backend/output_normalizer.rs`:
   1. Recognize Gemini events: `init`, `message`, `result`, `tool_use`, `tool_result`.
   2. Extract session id from `init`.
   3. Extract assistant text from `message`.
   4. Ignore `tool_use` and `tool_result` for text output.
   5. Keep existing Claude/Codex behavior intact.

### Phase 2: Multi-Completer Completion Panel
1. Add workflow config fields in `src/config/global.rs`:
   1. `completion_backends: Vec<String>` default `["claude","codex","?gemini"]`
   2. `completion_min_completers: u32` default `2`
   3. `completion_consensus_threshold: f64` default `1.0`
2. Change `CompletionLoopBackends` in `src/project/state.rs` to `completers: Vec<String>`.
3. Add per-backend completion artifacts in `src/project/artifacts.rs`.
4. Prevent filename collision by requiring deduplicated resolved completer specs during config validation.
5. Add backward reconstruction in `src/project/lifecycle.rs`:
   1. Old `completer-verdict.md` maps to single completer.
   2. New per-backend verdict files map to panel layout.
6. Update `src/workflow/orchestrator.rs`:
   1. Run all effective completers.
   2. Write one verdict artifact per completer.
   3. Decision rule:
      1. `complete_votes >= completion_min_completers`
      2. `complete_votes / total_effective_completers >= completion_consensus_threshold`
      3. Threshold is inclusive.
   4. Run acceptance QA exactly once after panel reaches `Complete`.

### Phase 3: Serial Prompt-Review Panel
1. Add in `src/config/global.rs`:
   1. `prompt_review_backends: Vec<String>`
   2. `prompt_review_min_reviewers: u32`
   3. Compatibility alias: `prompt_review_backend` (singular)
2. Alias precedence:
   1. If `prompt_review_backends` is set, use it.
   2. Else synthesize list from singular alias.
3. Orchestrator flow in `src/workflow/orchestrator.rs`:
   1. First backend is refiner.
   2. Remaining backends are validators, executed serially.
   3. Validator output grammar: `ACCEPT` or `REJECT(<reason>)`.
   4. If any validator rejects, fail prompt-review and aggregate reasons.
   5. If no validators exist (single-entry list), degrade to current behavior.
4. Add parser/template support for validator verdicts.
5. Preserve canonical `prompt-review.md` artifact output.

### Acceptance Criteria
1. `parse_backend_spec("gemini")`, `parse_backend_spec("gemini(gemini-3-pro)")`, and optional forms succeed.
2. Gemini backend can be created from registry and config lookup.
3. Gemini defaults are present exactly as specified.
4. `enabled` supports TOML `true`/`false`/`"auto"` and enforces availability semantics.
5. Optional specs are accepted only on panel lists.
6. Gemini is rejected on all guardrail surfaces.
7. Stream normalizer extracts Gemini session id and assistant text correctly.
8. Gemini resume args are rewritten as specified and are idempotent.
9. Completion panel stores multiple completers and writes per-backend verdicts.
10. Completion consensus rule works for 100%, partial-threshold, and insufficient-vote cases.
11. Acceptance QA is invoked once after completion panel success.
12. Prompt-review serial refine-then-validate works with accept and reject paths.
13. Singular prompt-review alias remains compatible.
14. Old completion artifact layout reconstructs correctly.
15. All existing tests pass after updates to expected defaults.

### Required Tests
1. Unit tests for new/changed backend parsing, config defaults, enable modes, arg rewriting, normalizer events, and panel resolution.
2. Unit tests for completion consensus math and prompt-review decision logic.
3. Add conformance tests under `src/validate/` for:
   1. Optional backend skipping and required-backend failure.
   2. Gemini scope guardrails.
   3. Multi-completer artifact/state behavior and consensus outcomes.
   4. Prompt-review serial validator accept/reject behavior and alias compatibility.
4. Register new validate modules in `src/validate/mod.rs`.
5. Run and pass:
   1. `nix develop -c cargo check`
   2. `nix develop -c cargo test`
   3. `nix build -L`
   4. `./result/bin/ralph validate --bin ./result/bin/ralph`

### Out of Scope
1. Gemini as planner/implementer/reviewer/QA backend.
2. 3-way alternation logic changes.
3. Automatic `npx` fallback for Gemini CLI.
4. Gemini-specific prompt tuning.
5. Parallel validator execution for prompt-review.
6. Daemon PRD backend surface changes.