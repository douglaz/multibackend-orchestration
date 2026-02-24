Implement token-efficiency and session reuse v1 for Ralph orchestration, with deterministic behavior and strong conformance coverage.

### Goal
Reduce prompt tokens and retry costs without changing workflow semantics:
- A: prompt de-duplication
- B: planner/completer prompt compression
- C: deterministic history capping
- D: per-loop per-role session reuse
- E: session-aware parse retries
- F: per-attempt token metrics

All reductions must be deterministic string/data operations. No LLM summarization.

### Hard Constraints
- Backward compatibility for existing `.ralph/projects/<id>/state.json` and config files.
- No Backend trait signature change.
- Session isolation is strict per `(loop_number, role, backend_spec)` within a project state file.
- Conformance tests are required for new user-visible behavior.

### A. Prompt De-duplication
Add `src/prompts/template_introspection.rs`:
```rust
pub fn template_uses_var(template_source: &str, var_name: &str) -> bool;
pub fn load_template_source(path: &Path, fallback: &str) -> String;
```

Update prompt builders:
- `build_planner_prompt`
- `build_implementer_prompt`
- `build_reviewer_prompt`
- `build_qa_prompt`
- `build_completer_prompt`

Rules:
- Load template source once.
- Append hardcoded sections only if the matching placeholder is absent.
- Default templates (with placeholders) must not duplicate sections.
- Custom templates that omit placeholders must still receive content exactly once.

Required unit tests:
- `template_uses_var`: present, absent, exact-match vs partial, repeated placeholders.
- Default-template rendering shows exactly one `## Master Prompt` section.
- Custom-template omission causes exactly one appended section.

### B. Planner/Completer Prompt Compression
Add:
```rust
summarize_state_for_planner(state: &ProjectState, max_loops: Option<usize>) -> String
summarize_previous_specs_for_planner(
    state: &ProjectState,
    project_dir: &Path,
    mode: PreviousSpecsInPrompt,
    max_specs: Option<usize>
) -> Result<String>
```

Add enums in `src/config/global.rs`:
- `PlannerStateInPrompt { FullJson, Summary }` default `Summary`
- `PreviousSpecsInPrompt { None, Titles, FullText }` default `Titles`

Semantics:
- `max_loops = None` means unlimited loops.
- `max_loops = Some(0)` means include no loop summaries/spec titles.
- Loop selection is by loop number ascending, then take latest N when capped.
- Summary must exclude raw review feedback text and QA report text.
- `build_planner_prompt` and `build_completer_prompt` must honor these modes.

Config surface (all four layers):
- `src/config/global.rs` `WorkflowConfig`:
  - `planner_state_in_prompt: PlannerStateInPrompt` default `Summary`
  - `planner_previous_specs_in_prompt: PreviousSpecsInPrompt` default `Titles`
  - `planner_max_prior_loops: Option<usize>` default `Some(10)`
- `src/config/project.rs` `ProjectWorkflowOverrides`:
  - `planner_state_in_prompt: Option<PlannerStateInPrompt>`
  - `planner_previous_specs_in_prompt: Option<PreviousSpecsInPrompt>`
  - `planner_max_prior_loops: Option<Option<usize>>`
- `src/config/mod.rs` `EffectiveWorkflowConfig` with resolved non-override values.
- `src/cli/config.rs` global/project set handlers + parse helpers.

CLI parse semantics for `planner_max_prior_loops`:
- Integer => capped mode.
- `"none"` => `None` (unlimited).
- For project overrides: absent key = inherit; explicit `"none"` = override to unlimited.

Required unit tests:
- Summary includes loop status/iteration/verdict/spec path but excludes feedback/report bodies.
- `Titles` mode produces bullet titles only.
- Global/project merge precedence for all new B fields.

### C. Deterministic History Capping
Update:
- `collect_review_history(state, project_dir, max_entries)`
- `collect_qa_history(state, project_dir, max_entries)`

Rules:
- Sort exchanges by `iteration` ascending before capping.
- Use last N by iteration.
- `max_entries = 0` returns empty string.
- History omission condition is per invocation: omit only when `session_reused_this_call == true` and `include_history_when_session_reuse_enabled == false`.
- If resume arg rewriting fails and the call falls back to fresh full prompt, use configured caps (do not force empty history).

Config surface (all four layers):
- `max_review_history_entries_in_prompt` default `3`
- `max_qa_history_entries_in_prompt` default `2`
- `include_history_when_session_reuse_enabled` default `false`

Required unit tests:
- Sequential and non-sequential insertion both cap by highest iteration numbers.
- `cap=0` yields empty.
- Per-invocation fallback case keeps history when resume is not actually used.

### D. Session Reuse

#### D1. State Model
Add to `src/project/state.rs`:
- `SessionRecord` fields:
  - `session_id: String`
  - `backend_spec: String`
  - `role: String`
  - `loop_number: u32`
  - `bootstrap_hash: String`
  - `call_count: u32`
  - `created_at: DateTime<Utc>`
  - `last_used_at: DateTime<Utc>`
- `SessionStore { records: Vec<SessionRecord> }`
- Methods:
  - `lookup(loop_number, role, backend_spec)`
  - `upsert(record)`
  - `remove_for_loop(loop_number)`
- `ProjectState` gets `#[serde(default)] session_store: SessionStore`.
- `ProjectState::new()` initializes default.
- `ProjectState::remove_loop()` removes matching session records.

#### D2. Session Config
Add fields to all four config layers:
- `session_reuse_enabled` default `false`
- `session_reuse_roles` default `["implementer","reviewer","qa"]`
- `session_reuse_reset_on_prompt_change` default `true`
- `session_reuse_reset_on_rollback` default `true`

Role validation policy:
- `config set` validates against known roles:
  - `planner`, `implementer`, `reviewer`, `qa`, `completer`
- Runtime orchestrator validates two sets:
  - Unknown role => warn and skip.
  - Known but unsupported for session reuse v1 (`planner`, `completer`) => warn and skip reuse for that role.
- Supported reuse roles in v1 are only `implementer`, `reviewer`, `qa`.

#### D3. Backend Invocation Context + Arg Rewriting
Add in `src/backend/mod.rs`:
```rust
pub struct BackendInvocationContext {
    pub loop_dir: PathBuf,
    pub role: String,
    pub session_id: Option<String>,
    pub json_output_required: bool,
}
```

Add `CliBackend::effective_args(&self, ctx: &BackendInvocationContext) -> Result<Vec<String>>`.

Claude resume rules:
- Require `-p` marker in base args; if missing => `Err(Validation(...))`.
- Remove `-p`.
- Ensure exactly one `--resume <id>` (replace existing if different).
- Ensure output format is JSON with exactly one `--output-format json` (replace conflicting existing value).
- Idempotent across repeated calls.

Codex resume rules:
- Require final stdin marker `"-"` in `exec ... -` form; if missing => `Err(Validation(...))`.
- Produce `exec resume <id> ... --json -`.
- Ensure exactly one `--json`.
- Preserve `-` as last token.
- Idempotent across repeated calls.

No-session rule:
- Return unchanged args.

Orchestrator handling:
- If `effective_args` returns `Err`, log warning, disable session for this invocation, rebuild prompt as fresh full prompt, continue without crash.

Tmux backend:
- Delegate to `CliBackend::effective_args()` for command construction.

#### D4. Output Normalization
Add `src/backend/output_normalizer.rs`:
```rust
#[derive(Default)]
pub struct NormalizedOutput {
    pub text: String,
    pub session_id: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub cached_in: Option<u64>,
}
pub fn normalize_output(backend_name: &str, raw_stdout: &str) -> Result<NormalizedOutput>;
```

Rules:
- Claude JSON: extract response text, `session_id`, usage.
- Codex JSONL: extract thread/session id, last assistant message text, usage.
- Non-JSON fallback: `text=raw`, metadata `None`.
- Malformed JSON/JSONL must not panic; fall back to raw text.
- If structured JSON exists but required message payload is missing, return `Err` so caller can degrade gracefully.

Integration point:
- Run normalization immediately after each `backend.execute_with_log()` on every attempt.
- Parse functions always receive `normalized.text`.
- On normalization error, use raw stdout as text.

#### D5. Bootstrap Hash
Formula:
`sha256_hex("{role}|{backend_spec}|{prompt_hash_at_loop_start}|{spec_hash}|{role_template_hash}|sessions-v1")`

Definitions:
- `prompt_hash_at_loop_start` comes from `ProjectState.prompt_hash_at_loop_start`.
- Legacy fallback: if empty, use `ProjectState.prompt_hash`, persist repaired value.
- `spec_hash = sha256(spec_content)`; planner uses `sha256("")`.
- `role_template_hash = sha256(load_template_source(effective_role_template_path, default_role_template_fallback))`.
- Hash excludes diff/review/qa iteration artifacts.

#### D6. Invalidation + Isolation
- Rollback must always remove session records for loops `> target` unconditionally.
- `session_reuse_reset_on_rollback` controls only clearing sessions for the target loop.
- Prompt-change restart with `session_reuse_reset_on_prompt_change=true` clears current loop sessions.
- Bootstrap hash mismatch forces fresh call and record replacement.
- Session ID lifecycle:
  - First call stores record only if normalized `session_id` exists.
  - Resume response with new `session_id` updates stored ID.
  - Resume response without `session_id` keeps previous stored ID.
  - Parse failure after normalization still updates/stores session record.

Working directory invariant:
- Keep process cwd at repo root for all backend invocations.
- Add `debug_assert_eq!(current_dir, repo_root)` before invocation.
- Add conformance test that captures `pwd` in mock backend and verifies repo root.

### E. Parse-Retry Optimization (Session Aware)
Modify retry pipeline to max 4 attempts:

1. Attempt 1: initial execution (fresh or resumed), then normalize, then parse.
2. Attempt 2: session follow-up only if a session id is active after attempt 1 normalization. Use same backend via resume with short correction prompt.
3. Attempt 3: opposite-backend reformatter (existing behavior), normalize before parse.
4. Attempt 4: reminded full prompt on original backend as a fresh call (`session_id=None`) to avoid compounding bad session state.

Rules:
- Without session, execute attempts 1/3/4 only.
- `ParseRetriesExhausted.attempts` must equal actual attempts executed (3 or 4).
- If attempt 1 parse fails but yields a new session ID, attempt 2 must use that new ID immediately.

### F. Token Metrics
After every normalization call (all attempts), log `tracing::info!` with:
- `role`
- `phase`
- `loop_number`
- `attempt` (1-based within parse-retry sequence)
- `backend`
- `session_reused` (true only when resume args were actually used)
- `tokens_in`
- `tokens_out`
- `cached_in`

If token fields are unavailable, omit via `tracing::field::Empty`.

### Required Conformance Tests (`src/validate/tests_sessions.rs`)
Register in `src/validate/mod.rs`.

Implement exactly these six tests:
- `sessions::history_capping_limits_review_entries`
- `sessions::history_capping_limits_qa_entries`
- `sessions::session_lifecycle_stores_and_resumes`
- `sessions::session_invalidation_on_rollback`
- `sessions::session_invalidation_on_prompt_change`
- `sessions::working_directory_stays_at_repo_root`

### Required Unit/Integration Tests
Add tests for:
- Arg rewrite success/failure/idempotency for Claude and Codex.
- Output normalization (Claude JSON, Codex JSONL, malformed input fallback).
- Bootstrap hash determinism and invalidation sensitivity.
- Session store serde compatibility and lookup/upsert/remove behavior.
- Parse retry attempt-count semantics (`attempts=3` without session, `attempts=4` with session).
- Token logging includes correct `attempt` and `session_reused` semantics.

### Files to Modify
- `src/prompts/template_introspection.rs` (new)
- `src/prompts/mod.rs`
- `src/workflow/orchestrator.rs`
- `src/project/state.rs`
- `src/config/global.rs`
- `src/config/project.rs`
- `src/config/mod.rs`
- `src/cli/config.rs`
- `src/backend/mod.rs`
- `src/backend/tmux_backend.rs`
- `src/backend/output_normalizer.rs` (new)
- `src/cli/rollback.rs`
- `src/validate/tests_sessions.rs` (new)
- `src/validate/mod.rs`
- `src/validate/mock_scripts.rs` (if needed for fixtures)

### Done Criteria
- `nix develop -c cargo check` passes.
- `nix develop -c cargo test` passes.
- `nix build -L` succeeds.
- `./result/bin/ralph validate --bin ./result/bin/ralph --filter sessions` passes.
- Existing validate suites remain green.

### Out of Scope
- LLM summarization of history/state.
- Cross-role or cross-loop shared sessions.
- Acceptance QA session reuse.
- Backend trait signature changes.
- Streaming normalization.
- Token-budget enforcement.