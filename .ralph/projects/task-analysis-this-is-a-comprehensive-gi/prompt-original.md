Now I have a complete understanding of both the codebase and the issue requirements. Let me write the engineering specification.

## Summary

Add Gemini CLI (`gemini`) as a third backend to the Ralph orchestration system. Phase 1 introduces Gemini as a recognized backend in the config, registry, spec parser, and output normalizer — scoped exclusively to final-review, completion-panel, and prompt-review-panel roles. Phase 2 replaces single-completer completion with a multi-backend completion panel. Phase 3 replaces single-refiner prompt-review with a serial refine-then-validate panel. The standard planner/implementer/reviewer/QA alternation loop remains strictly claude-codex; Gemini is rejected for those surfaces. Optional `?backend` spec syntax enables graceful degradation when Gemini is unavailable.

## Acceptance Criteria

1. **`gemini` recognized as a backend**: `parse_backend_spec("gemini")` and `parse_backend_spec("gemini(gemini-3-pro)")` succeed. `BackendRegistry::create_cli_backend_for_spec` dispatches to `gemini::backend_from_config`. `GlobalConfig::backend_config("gemini")` returns the gemini config.
2. **Gemini config with defaults**: `BackendConfigs` has a `gemini` field defaulting to `command="gemini"`, `args=["-p","--yolo","--output-format","stream-json"]`, and models only for `final_reviewer`, `arbiter`, `completer` (all `"gemini-3-pro"`). Other role models are `None`.
3. **Backend enable mode**: Each `BackendConfig` gains an `enabled` field (`auto|true|false`). `auto` (default) checks availability only when the backend is selected; `true` forces startup health-check; `false` treats the backend as unavailable.
4. **Optional backend specs**: `?backend` and `?backend(model)` syntax supported in panel list surfaces (`final_review_backends`, `completion_backends`, `prompt_review_backends`). Optional specs are skipped with warning when unavailable; rejected on single-backend required surfaces.
5. **Scope guardrails**: Gemini is rejected for `starting_backend`, `planner_backend`, `implementer_backend`, `reviewer_backend`, `qa_backend`, and daemon refinement backends.
6. **Output normalizer handles Gemini events**: `init`, `message` (assistant role), `result`, `tool_use`, and `tool_result` stream-json events are recognized. Session ID and response text are correctly extracted.
7. **Session resume for Gemini**: `effective_args_gemini` strips old `--resume`/`--output-format`, adds new `--resume <id>` and `--output-format json`, keeps `-p` (Strategy A).
8. **Multi-completer panel (Phase 2)**: `CompletionLoopBackends` stores `completers: Vec<String>`. Per-backend verdict artifacts (`completer-verdict-<backend>.md`). Consensus aggregation: `complete_votes >= min_completers AND ratio >= threshold`. Acceptance QA runs once after panel `Complete`. Backward-compatible reconstruction from old single `completer-verdict.md`.
9. **Multi-reviewer prompt-review panel (Phase 3)**: Serial refine-then-validate flow. Primary refiner produces refined prompt; validator(s) return `ACCEPT`/`REJECT(reason)`. `prompt_review_backends` list; `prompt_review_backend` (singular) is a compatibility alias. Single-entry list degrades to current behavior.
10. **All existing tests pass** and new tests cover every change area.

## Technical Approach

### Phase 1: Backend Plumbing

**New file: `src/backend/gemini.rs`** — Follows the exact pattern of `claude.rs` and `codex.rs`:
- `backend_from_config(config, model, role) -> CliBackend`: reads `config.backends.gemini`, injects `--model` if specified, applies role-specific timeout.
- `ensure_gemini_stream_json_args(args) -> Vec<String>`: strips any existing `--output-format` variants (both `--output-format <val>` and `--output-format=<val>` forms), appends canonical `--output-format stream-json`. Same idx-skip loop as claude.rs's `ensure_stream_json_args`.
- No effort-suffix parsing needed (unlike codex). Model names used as-is.

**`src/backend/mod.rs` changes:**
- Add `pub mod gemini;` declaration.
- `parse_backend_spec`: Extend to handle `?`-prefixed optional specs. Add `optional: bool` field to `BackendSpec`. Strip leading `?` before parsing name/model. Callers in panel resolution use `optional` field for skip-vs-fail behavior.
- `CliBackend::effective_args()`: Add `n if n.starts_with("gemini")` match arm dispatching to `effective_args_gemini()`.
- `CliBackend::ensure_json_output_args()`: Add gemini arm that strips existing `--output-format` before adding `--output-format json`.
- `CliBackend::effective_args_gemini()`: Strategy A — keep `-p`, strip `--resume`/`--output-format`, add `--resume <id>` and `--output-format json`.
- `BackendRegistry::new()`: Add lazy gemini backend creation (same pattern as claude/codex).
- `BackendRegistry::create_cli_backend_for_spec()`: Add `"gemini"` arm.
- `BackendRegistry::backend_role_model_specs()`: Add gemini to the iteration loop.
- `BackendRegistry::opposite()`: No change. Add comment clarifying gemini doesn't participate in alternation.
- Add panel backend resolution method that handles `?`-prefixed specs: resolves availability, filters optional unavailable backends with warning, enforces min-reviewer thresholds after filtering.

**`src/config/global.rs` changes:**
- Add `BackendEnabled` enum (`Auto`, `Enabled`, `Disabled`) with custom serde (accepts TOML `true`/`false`/`"auto"`).
- Add `enabled: BackendEnabled` field to `BackendConfig` (default: `Auto`).
- Add `gemini: BackendConfig` to `BackendConfigs` with `default_gemini_backend_config()` and `deserialize_gemini_backend_config()`.
- `default_gemini_backend_config()`: command=`"gemini"`, args=`["-p","--yolo","--output-format","stream-json"]`, models for final_reviewer/arbiter/completer = `"gemini-3-pro"`, all others `None`.
- `GlobalConfig::load()`: Add `fill_from` calls for gemini models and role_timeouts.
- `GlobalConfig::backend_config()`: Add `"gemini"` arm.
- Update `default_final_review_backends()` to include `"?gemini"`.

**`src/config/mod.rs` changes:**
- `validate_backend_spec()`: Add surface-context parameter. Reject gemini for starting-backend/feature-loop/daemon surfaces. Accept for panel surfaces.
- Reject `?`-prefixed specs on single-backend required surfaces.
- Add optional filtering for panel list validation that evaluates availability before min-reviewer checks.

**`src/backend/output_normalizer.rs` changes:**
- Add `"init"` to `STREAM_EVENT_TYPES`.
- Add Gemini event handlers in `normalize_claude_stream_json()` (consider renaming to `normalize_stream_json()`):
  - `"init"`: extract `session_id`.
  - `"message"`: extract text from assistant-role messages.
  - `"tool_use"` / `"tool_result"`: skip (no text extraction).
  - `"result"`: already handled — verify Gemini's `result` event matches existing handler shape.

### Phase 2: Multi-Completer Completion Panel

**`src/config/global.rs`**: Add `completion_backends: Vec<String>`, `completion_min_completers: u32`, `completion_consensus_threshold: f64` to `WorkflowConfig` with defaults `["claude","codex","?gemini"]`, `2`, `1.0`.

**`src/project/state.rs`**: Change `CompletionLoopBackends` from `{ planner, completer }` to `{ planner, completers: Vec<String> }`.

**`src/project/artifacts.rs`**: Add `CompleterVerdictForBackend { backend: String }` variant. File name: `completer-verdict-<backend>.md`.

**`src/project/lifecycle.rs`**: Backward-compatible reconstruction: detect old `completer-verdict.md` → single-entry; detect new `completer-verdict-*.md` → multi-entry.

**`src/workflow/orchestrator.rs`**: Invoke all configured completers, write per-backend verdict artifacts, aggregate using `complete_votes >= min_completers && ratio >= threshold`. Acceptance QA runs once after panel Complete.

### Phase 3: Multi-Reviewer Prompt-Review Panel

**`src/config/global.rs`**: Add `prompt_review_backends: Vec<String>`, `prompt_review_min_reviewers: u32`. Compatibility: if `prompt_review_backends` unset, synthesize from `prompt_review_backend`.

**`src/workflow/orchestrator.rs`**: Serial refine-then-validate: primary refiner produces refined prompt, validators return `ACCEPT`/`REJECT(reason)`. Accept if `accept_count >= min_reviewers - 1`. Single-entry list degrades to current behavior.

**Parser/templates**: Add validator verdict parser (`ACCEPT`/`REJECT`), validator template.

## Files & Modules

### Phase 1 (New/Modified)
| File | Action | Summary |
|------|--------|---------|
| `src/backend/gemini.rs` | **NEW** | `backend_from_config()`, `ensure_gemini_stream_json_args()` |
| `src/backend/mod.rs` | Modify | Add `mod gemini`, optional spec parsing, gemini arms in registry/effective_args/ensure_json |
| `src/config/global.rs` | Modify | Add `BackendEnabled` enum, `enabled` field, gemini to `BackendConfigs`, defaults |
| `src/config/mod.rs` | Modify | Surface-specific validation, optional `?` spec handling in panel lists |
| `src/config/project.rs` | Modify | Add new workflow override fields as they arrive |
| `src/backend/output_normalizer.rs` | Modify | Add Gemini stream event types and handlers |

### Phase 2 (New/Modified)
| File | Action | Summary |
|------|--------|---------|
| `src/config/global.rs` | Modify | Add `completion_backends`, `completion_min_completers`, `completion_consensus_threshold` |
| `src/project/state.rs` | Modify | `CompletionLoopBackends.completers: Vec<String>` |
| `src/project/artifacts.rs` | Modify | `CompleterVerdictForBackend` artifact kind |
| `src/project/lifecycle.rs` | Modify | Backward-compat reconstruction for completion artifacts |
| `src/workflow/orchestrator.rs` | Modify | Multi-completer invocation, aggregation, acceptance QA |

### Phase 3 (New/Modified)
| File | Action | Summary |
|------|--------|---------|
| `src/config/global.rs` | Modify | Add `prompt_review_backends`, `prompt_review_min_reviewers`, alias resolution |
| `src/workflow/orchestrator.rs` | Modify | Serial refine-then-validate prompt-review flow |
| `src/workflow/parser.rs` | Modify | Validator verdict parser |
| Templates directory | Modify | Validator template for prompt-review |

## Testing Strategy

### Phase 1 Tests

**`src/backend/gemini.rs`**: Unit tests for `backend_from_config` with/without model, `ensure_gemini_stream_json_args` stripping and idempotency.

**`src/backend/mod.rs`**: Tests for `parse_backend_spec` with `"gemini"`, `"gemini(gemini-3-pro)"`, `"?gemini"`, `"?gemini(gemini-3-pro)"`. Tests for `create_cli_backend_for_spec` gemini dispatch. Tests for `effective_args_gemini` session resume (strips old, adds new, keeps `-p`, idempotent). Tests for `ensure_json_output_args` gemini arm (strips `--output-format stream-json`, adds `--output-format json`). Tests for `backend_role_model_specs` including gemini. Tests for optional panel backend filtering (skip unavailable optional, fail unavailable required).

**`src/config/global.rs`**: Default config includes gemini with correct defaults. TOML deserialization with/without gemini section. `enabled` deserializes from `true`/`false`/`"auto"`. `backend_config("gemini")` returns correct config. `fill_from` works for gemini models/timeouts. `final_review_backends` default includes `"?gemini"`.

**`src/config/mod.rs`**: Gemini rejected for `starting_backend` and feature-loop overrides. Gemini accepted for `final_review_backends`, `final_review_arbiter_backend`. `?gemini` accepted for panel list surfaces, rejected for single-backend surfaces. `enabled=false` backend fails on required surfaces, skips on optional. `?gemini` + `enabled=false` skipped with warning.

**`src/backend/output_normalizer.rs`**: Gemini `init` extracts session_id. Gemini `message` extracts assistant text. Gemini `result` extracts final text. `tool_use`/`tool_result` skipped. Full end-to-end stream normalization. Mixed event streams don't cross-contaminate.

### Phase 2 Tests
Completion state stores multiple backends. Per-backend verdict artifacts. Aggregation: all Complete + threshold 1.0 → Complete; 2/3 + threshold 0.67 → Complete; 1/3 + threshold 1.0 → Continue. Acceptance QA once after Complete. Backward-compat: old `completer-verdict.md` reconstructs to single-entry panel. Optional completer unavailable → skipped, panel runs with required completers. Threshold guard fails when effective set < `min_completers`.

### Phase 3 Tests
Serial refine-then-validate flow. Validator ACCEPT → refined prompt written. Validator REJECT → prompt-review fails with reasons. Single-entry list degrades to current behavior. Optional validator unavailable degrades to single-refiner. `prompt_review_backend` (singular) compatibility alias. Per-backend artifacts written. Canonical `prompt-review.md` still written. `state_has_prompt_review()` recognizes both layouts.

### Integration Testing
All existing tests must pass unmodified (except those testing default values that change, e.g., `default_final_review_backends`). Run `cargo test` after each phase. Run `cargo clippy` for linting. Manual validation of Gemini stream-json output (empirical validation for normalizer accuracy).

## Out of Scope

- **Gemini for planner/implementer/reviewer/QA roles**: Not in this issue. Gemini is restricted to final-review, completion-panel, and prompt-review-panel surfaces only.
- **3-way alternation logic**: The `opposite()` and `planner_for_loop()` functions remain binary (claude ↔ codex). Gemini does not participate in alternation.
- **Automatic npx fallback**: Users must configure `command = "npx"` and adjust args manually if gemini is not in PATH.
- **Gemini-specific prompt optimizations**: Templates remain backend-agnostic. Gemini-specific tuning is a future concern.
- **Parallel prompt-review validation**: The design uses serial refine-then-validate, not parallel agreement (which is fragile across different models).
- **Daemon PRD backend changes**: `daemon_prd_question_backends`, `daemon_prd_writer_backend`, `daemon_prd_reviewer_backend` are not updated to support gemini in this issue.
- **Empirical Gemini CLI validation**: The exact stream-json event schema and session resume behavior must be validated by running `gemini` locally. The spec documents Strategy A (keep `-p` during resume) as default, with Strategy B as fallback if empirical testing reveals conflicts. This validation happens during implementation, not as part of spec approval.