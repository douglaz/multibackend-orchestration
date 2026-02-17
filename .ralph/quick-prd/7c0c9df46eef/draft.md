## Summary

Implement six areas of token-efficiency improvements and session reuse for the Ralph orchestration system. Today, every LLM call sends the full prompt from scratch — including the full `ProjectState` JSON (with all review/QA exchange records), the complete text of all prior specs, unbounded review/QA history, and content duplicated between template rendering and hardcoded appends. This work reduces input tokens through prompt de-duplication (A), planner prompt compression (B), and deterministic history capping (C), then adds per-(role, loop, backend) session persistence (D) so Claude and Codex receive only delta prompts on follow-up calls. Parse-retry optimization (E) leverages active sessions for cheaper retries, and token metrics logging (F) provides observability.

All reductions are deterministic (no LLM summarization). Session scoping rules prevent cross-role and cross-loop context contamination.

## Acceptance Criteria

### A – Prompt de-duplication
- `template_uses_var(template_source: &str, var_name: &str) -> bool` added to `src/prompts/template_introspection.rs`. Returns `true` if source contains `{{var_name}}`.
- `load_template_source(path: &Path, fallback: &str) -> String` loads raw template text (file or fallback) without rendering.
- Each prompt builder (`build_planner_prompt`, `build_implementer_prompt`, `build_reviewer_prompt`, `build_qa_prompt`, `build_completer_prompt`) loads the template source once, then conditionally appends post-render sections **only if** the template does not reference the corresponding placeholder. Specifically:
  - Planner: `## Master Prompt` appended only if `!template_uses_var(src, "prompt_content")`; `## Current State` appended only if `!template_uses_var(src, "state_content")`.
  - Implementer: `## Master Prompt` only if `!template_uses_var(src, "prompt_content")`; `## Feature Spec` only if `!template_uses_var(src, "spec_content")`; `## Review Feedback` only if `!template_uses_var(src, "review_feedback_content")`.
  - Reviewer: same pattern for `prompt_content`, `spec_content`, `impl_notes_content`, `impl_response_content`, `git_diff`.
  - QA: same pattern for `prompt_content`, `spec_content`, `impl_notes_content`, `git_diff`, `qa_history`.
  - Completer: same pattern for `prompt_content`, `state_content`, `previous_specs`, `termination_request_content`.
- Default templates (which do reference all placeholders) produce output with zero duplicated sections.
- Custom templates that omit a placeholder still receive the content appended once.
- Unit tests: `template_uses_var` correctness (present, absent, partial match, nested braces); rendered output of default templates has no duplicate `## Master Prompt` headings.

### B – Planner prompt compression
- `summarize_state_for_planner(state: &ProjectState, max_loops: usize) -> String` produces a compact multi-line summary: project ID/name, status, current loop/phase, then for the last N feature loops: loop number, feature name, status, iteration count, last review verdict (approved/suggestions), last QA verdict (pass/fail), spec artifact path. Excludes `review_feedback` text and `qa_report` text.
- `summarize_previous_specs_for_planner(state: &ProjectState, project_dir: &Path, mode: PreviousSpecsInPrompt, max_specs: usize) -> Result<String>` produces: `None` → empty; `Titles` → bullet list of `Loop N: feature_name (status) — spec_path`; `FullText` → current behavior.
- New config fields added to **all four config layers** (see B-Config below).
- Enum `PlannerStateInPrompt { FullJson, Summary }` and `PreviousSpecsInPrompt { None, Titles, FullText }` added to `src/config/global.rs`.
- `build_planner_prompt` and `build_completer_prompt` respect these config fields.
- Unit tests: `summarize_state_for_planner` with 5 completed loops + 1 in-progress omits review feedback text; snapshot test for summary format; `summarize_previous_specs_for_planner` with `Titles` mode returns expected bullet list.

#### B-Config – Config layer changes for planner compression

All new `WorkflowConfig` fields for area B use `#[serde(default)]` and follow the existing four-layer pattern:

1. **`src/config/global.rs` — `WorkflowConfig`**: Add `planner_state_in_prompt: PlannerStateInPrompt` (default `Summary`), `planner_previous_specs_in_prompt: PreviousSpecsInPrompt` (default `Titles`), `planner_max_prior_loops: Option<usize>` (default `Some(10)`).
2. **`src/config/project.rs` — `ProjectWorkflowOverrides`**: Add `planner_state_in_prompt: Option<PlannerStateInPrompt>`, `planner_previous_specs_in_prompt: Option<PreviousSpecsInPrompt>`, `planner_max_prior_loops: Option<Option<usize>>`.
3. **`src/config/mod.rs` — `EffectiveWorkflowConfig`**: Add `planner_state_in_prompt: PlannerStateInPrompt`, `planner_previous_specs_in_prompt: PreviousSpecsInPrompt`, `planner_max_prior_loops: Option<usize>`. Resolution: `project_ref.and_then(|p| p.workflow.planner_state_in_prompt.clone()).unwrap_or(global.workflow.planner_state_in_prompt.clone())` (same pattern for each field).
4. **`src/cli/config.rs`**: Add `workflow.planner_state_in_prompt`, `workflow.planner_previous_specs_in_prompt`, `workflow.planner_max_prior_loops` to both `set_global_value` and `set_project_value` match arms. Add parse functions: `parse_planner_state_in_prompt(raw) -> Result<PlannerStateInPrompt>` accepting `"full-json"` / `"summary"`; `parse_previous_specs_in_prompt(raw) -> Result<PreviousSpecsInPrompt>` accepting `"none"` / `"titles"` / `"full-text"`; corresponding `parse_optional_*` variants for project config.
5. **Unit test**: Global+project merge test: global sets `FullJson`, project overrides to `Summary` → effective is `Summary`. Project sets `None` → effective clears to `None`.

### C – Deterministic history capping
- New config fields added to **all four config layers** (see C-Config below).
- `collect_review_history` and `collect_qa_history` accept a `max_entries: usize` parameter. Implementation: **sort entries by `exchange.iteration` ascending** before capping, then take the last N entries via `.skip(entries.len().saturating_sub(max_entries))`. This guarantees highest iteration numbers win regardless of insertion order in the `Vec`.
- When `session_reuse_enabled && !include_history_when_session_reuse_enabled`, callers pass `max_entries = 0` (empty history).
- Callers in `build_implementer_prompt`, `build_reviewer_prompt`, `build_qa_prompt` thread the config values through.
- Unit tests:
  - 10 `ReviewExchange` entries with cap=3 yields exactly entries for iterations 8, 9, 10.
  - 10 entries inserted in **non-sequential order** (e.g., 3, 1, 7, 2, 10, 5, 4, 8, 6, 9) with cap=3 still yields iterations 8, 9, 10 (proving sort, not insertion-order, governs).
  - Cap=0 → empty string.

#### C-Config – Config layer changes for history capping

1. **`src/config/global.rs` — `WorkflowConfig`**: Add `max_review_history_entries_in_prompt: usize` (default `3`), `max_qa_history_entries_in_prompt: usize` (default `2`), `include_history_when_session_reuse_enabled: bool` (default `false`).
2. **`src/config/project.rs` — `ProjectWorkflowOverrides`**: Add `max_review_history_entries_in_prompt: Option<usize>`, `max_qa_history_entries_in_prompt: Option<usize>`, `include_history_when_session_reuse_enabled: Option<bool>`.
3. **`src/config/mod.rs` — `EffectiveWorkflowConfig`**: Add matching non-Option fields; resolve with project-then-global precedence.
4. **`src/cli/config.rs`**: Add `workflow.max_review_history_entries_in_prompt` and `workflow.max_qa_history_entries_in_prompt` (using `parse_usize` / `parse_optional_usize`), and `workflow.include_history_when_session_reuse_enabled` (using `parse_bool` / `parse_optional_bool`) to both global and project set handlers.

### D – Session reuse

#### D-State – Session state types
- `SessionRecord` struct: `session_id: String`, `backend_spec: String`, `role: String`, `loop_number: u32`, `bootstrap_hash: String`, `call_count: u32`, `created_at: DateTime<Utc>`, `last_used_at: DateTime<Utc>`.
- `SessionStore` struct: `records: Vec<SessionRecord>` with helper methods `lookup(loop_number, role, backend_spec) -> Option<&SessionRecord>`, `upsert(record)`, `remove_for_loop(loop_number)`.
- `ProjectState` gains `#[serde(default)] session_store: SessionStore`. `ProjectState::new()` initializes it as default. `ProjectState::remove_loop()` calls `session_store.remove_for_loop()`.

#### D-Config – Config layer changes for session reuse

1. **`src/config/global.rs` — `WorkflowConfig`**: Add `session_reuse_enabled: bool` (default `false`), `session_reuse_roles: Vec<String>` (default `["implementer","reviewer","qa"]`), `session_reuse_reset_on_prompt_change: bool` (default `true`), `session_reuse_reset_on_rollback: bool` (default `true`).
2. **`src/config/project.rs` — `ProjectWorkflowOverrides`**: Add `session_reuse_enabled: Option<bool>`, `session_reuse_roles: Option<Vec<String>>`, `session_reuse_reset_on_prompt_change: Option<bool>`, `session_reuse_reset_on_rollback: Option<bool>`.
3. **`src/config/mod.rs` — `EffectiveWorkflowConfig`**: Add matching non-Option fields; resolve with project-then-global precedence.
4. **`src/cli/config.rs`**: Add all four keys to both global and project set handlers. `session_reuse_roles` uses `parse_string_list` / `parse_optional_string_list`. Add `parse_session_reuse_roles` that validates each entry against the **known role set** `["planner", "implementer", "reviewer", "qa", "completer"]`: unknown roles produce a `RalphError::Validation` error with a message listing the valid roles. The orchestrator performs the same validation at startup when reading `effective.workflow.session_reuse_roles`, emitting a `tracing::warn!` for any unrecognized role and filtering it out (warn-and-skip policy at runtime, hard error only in `config set`).
5. **Unit test**: Global+project merge test; CLI `config set workflow.session_reuse_roles "planner,bogus"` returns validation error.

#### D-Backend – Backend invocation context and arg rewriting
- `BackendInvocationContext` struct in `src/backend/mod.rs`: `loop_dir: PathBuf`, `role: String`, `session_id: Option<String>`, `json_output_required: bool`.
- `CliBackend` gains `fn effective_args(&self, ctx: &BackendInvocationContext) -> Result<Vec<String>>`:
  - **Claude path** (when `session_id` is `Some`): Scan `self.args` for the `-p` flag. If found, remove it. Append `--resume <id>` and `--output-format json`. If `-p` is **not found** (custom arg layout), return `Err(RalphError::Validation("cannot rewrite Claude args for session resume: -p flag not found in base args"))`.
  - **Codex path** (when `session_id` is `Some`): Scan `self.args` for the `"-"` stdin marker (the last occurrence). If found, insert `resume <id>` before the sequence `exec ... -` (i.e., replace `["exec", ..., "-"]` with `["exec", "resume", "<id>", ..., "--json", "-"]`). If `"-"` marker is **not found**, return the same kind of `Err`.
  - When `session_id` is `None`, return `Ok(self.args.clone())` unchanged.
  - Both paths must be idempotent: calling `effective_args` twice with the same context produces identical output (no double-insertion).
- `TmuxBackend` delegates to `CliBackend::effective_args()` when building its shell command.
- Unit tests for `effective_args`:
  - Claude: default args + session → correct rewrite (no `-p`, has `--resume`, has `--output-format json`).
  - Claude: custom args missing `-p` + session → returns `Err`.
  - Claude: no session → args unchanged.
  - Codex: default args + session → correct rewrite (has `resume <id>`, has `--json`, `-` still last).
  - Codex: custom args missing `-` marker + session → returns `Err`.
  - Codex: no session → args unchanged.
  - Both: idempotency (calling twice gives same result).

#### D-Output – Output normalization
- New `src/backend/output_normalizer.rs`: `NormalizedOutput { text: String, session_id: Option<String>, tokens_in: Option<u64>, tokens_out: Option<u64>, cached_in: Option<u64> }`. `normalize_output(backend_name: &str, raw_stdout: &str) -> Result<NormalizedOutput>`:
  - Claude JSON path: parse top-level JSON object, extract `result` → text, `session_id`, usage fields. If JSON is present but `result` is missing, return `Err`.
  - Codex JSONL path: parse newline-delimited JSON, find `thread.started` → `thread_id`, last `item.completed` with `agent_message` → `text`, `turn.completed` → `usage`. If no `agent_message` event found, return `Err`.
  - Fallback: if `raw_stdout` does not start with `{` (Claude) or does not contain a JSONL `"type":` marker (Codex), return `text = raw_stdout`, all other fields `None`.
  - Malformed JSON → return `text = raw_stdout`, all other fields `None`, no panic.
- **Integration with parse-retry pipeline**: `normalize_output` is called **immediately** after each backend execution returns raw stdout, **before** the output is passed to `parse_fn`. This applies to every attempt (initial, reformatter, reminded). The `parse_fn` always receives `normalized.text`, never the raw JSON/JSONL wrapper. If normalization returns `Err`, the raw stdout is used as-is (graceful degradation).

#### D-Bootstrap – Bootstrap hash specification
- Hash formula: `sha256_hex(format!("{role}|{backend_spec}|{prompt_hash_at_loop_start}|{spec_hash}|{template_hashes}|sessions-v1"))`.
- **Template hashes**: For a given role, hash **only the template source used by that role**. The template source is the content returned by `load_template_source(effective_template_path, default_fallback)` — i.e., the custom file content if it exists, otherwise the hardcoded default. Compute `sha256_hex(template_source)` for the role's template. When the template file does not exist, the fallback content is hashed (ensuring the hash is stable and deterministic).
- **Canonical format**: The `template_hashes` component is a single hex string: `sha256_hex(role_template_source)`. Since each session record is per-role, only one template is relevant per record — no multi-template sorting is needed.
- **`spec_hash`**: `sha256_hex(spec_content)` where `spec_content` is the rendered spec file for the current loop. For the planner role (which runs before a spec exists), use the empty string hash `sha256_hex("")`.
- The hash does **not** cover git diff, review feedback, QA reports, or iteration numbers — these are ephemeral per-call inputs delivered via delta prompts.
- Unit tests: same inputs → same hash; changing role → different hash; changing template content → different hash; missing template file (uses fallback) → deterministic hash.

#### D-Isolation – Session isolation and invalidation
- Sessions are strictly isolated per `(project_id, loop_number, role, backend_spec)`. The `SessionStore::lookup` key is `(loop_number, role, backend_spec)`.
- **Loop-number reuse after rollback**: To prevent stale session reuse when a loop number is recycled after rollback, `remove_for_loop` is called for **all loops > target** during rollback (matching the existing `state.loops.retain(|l| l.loop_number <= args.loop_number)` pattern). This is **unconditional** — it always runs during rollback regardless of `session_reuse_reset_on_rollback`. The `session_reuse_reset_on_rollback` flag controls only whether the **target loop's own** sessions are also cleared (i.e., loop N when rolling back to N).
- **Loop restart** (prompt change with `prompt_change_action = RestartLoop`): When `session_reuse_reset_on_prompt_change=true`, clear session records for the current loop number before restarting.
- **Bootstrap hash mismatch**: Any change to bootstrap hash triggers a fresh session (no resume). The stale record is replaced by `upsert` after the fresh call completes.
- **Session invalidation is the union of**: (1) loop removal (unconditional for removed loops), (2) target-loop reset on rollback (controlled by `session_reuse_reset_on_rollback`), (3) loop restart on prompt change (controlled by `session_reuse_reset_on_prompt_change`), (4) bootstrap hash mismatch (always).

#### D-SessionID – Session ID lifecycle rules
- **First call**: Backend returns raw output. After normalization, if `normalized.session_id` is `Some`, store a new `SessionRecord` with that ID. If `normalized.session_id` is `None` (e.g., non-JSON backend, or fallback path), **do not** create a session record — the next call for this role will again be a fresh full prompt.
- **Resume call returning a new session ID**: If the resume response contains a `session_id` that differs from the stored one, update the record's `session_id` to the new value (backends may rotate IDs).
- **Resume call omitting session ID**: If the resume response's `normalized.session_id` is `None`, **retain** the prior stored `session_id` in the record. Increment `call_count` and update `last_used_at` normally.
- **Parse failure on first call with session ID**: If the first call's output fails parsing but normalization extracted a valid `session_id`, **still store** the session record. The parse-retry path (E) can then use the session for the follow-up correction attempt, avoiding a full re-prompt.
- **Parse failure on resume call**: The existing session record is retained (not cleared). The retry path in section E applies.

#### D-Orchestrator – Orchestrator session flow
- Before each `execute_with_parse_retries` call, the orchestrator: (1) checks if session reuse is enabled and the role is in `session_reuse_roles` (filtering out unrecognized roles with a warning), (2) computes the bootstrap hash, (3) looks up the session store, (4) decides fresh vs. resume based on hash match, (5) builds the prompt (full or delta), (6) creates the backend with effective args via `effective_args()`, (7) executes, (8) normalizes output, (9) updates session store with the returned `session_id`.
- Delta prompt builders: `build_implementer_delta_prompt`, `build_reviewer_delta_prompt`, `build_qa_delta_prompt` — contain only the new iteration's inputs (e.g., latest review feedback for implementer, latest diff for reviewer, latest diff + latest impl response for QA) plus a brief format reminder. No static context (system role, guardrails, master prompt, spec).
- **Working directory invariant**: The orchestrator must **never** change the process working directory to a loop subdirectory when invoking backends. All paths passed to backends must be relative to or absolute from the repo root. An `assert!` or `debug_assert!` verifies `std::env::current_dir()` matches the expected repo root before each backend invocation.

#### D-Tests
- Unit tests: `effective_args` for Claude with/without session; `effective_args` for Codex with/without session; `normalize_output` for Claude JSON, Codex JSONL, and raw text; bootstrap hash determinism; `SessionStore::lookup` and `upsert` correctness; session record round-trip serialization; backward-compat deserialization from `{}`.
- Integration test (mocked backend): first call stores session record; second call uses resume args and delta prompt; hash mismatch triggers fresh session.

### E – Parse-retry optimization (session-aware)

The existing parse-retry pipeline has a fixed 3-attempt structure: (1) initial backend, (2) reformatter on opposite backend, (3) reminded full prompt on original backend. With session awareness, the pipeline extends to **4 maximum attempts** when a session is active:

1. **Attempt 1**: Initial backend execution (full or delta prompt). Output is **normalized** before parsing. If parse succeeds → done.
2. **Attempt 2 (session follow-up)**: Only when `BackendInvocationContext.session_id.is_some()`. Send a short follow-up to the **same backend via session resume**: `"Your previous response could not be parsed. Error: {error}. Please reformat your last answer using this structure:\n{expected_format}\nRespond ONLY with the corrected markdown."` Output is normalized before parsing. If parse succeeds → done. If no active session, this attempt is **skipped** and the pipeline proceeds directly to attempt 3.
3. **Attempt 3 (reformatter)**: Existing behavior — send raw output + parse error to the opposite backend. Output is normalized before parsing.
4. **Attempt 4 (reminded full prompt)**: Existing behavior — send format reminder + original full prompt to original backend. Output is normalized before parsing.

If all applicable attempts fail, return `ParseRetriesExhausted { role, phase, attempts: N }` where `N` is the actual number of attempts executed (3 without session, 4 with session).

Token metrics (F) are logged for **every attempt**, not just the final successful one.

Unit test: mock backend with session returns reformatted output on follow-up; full re-prompt only fires when follow-up also fails.

### F – Token metrics logging
- After each `normalize_output` call (on **every attempt** including retries), emit a `tracing::info!` event with structured fields: `role`, `phase`, `loop_number`, `attempt` (1-based attempt number within the parse-retry sequence), `tokens_in`, `tokens_out`, `cached_in`, `session_reused: bool`, `backend`.
- When `NormalizedOutput` token fields are `None` (non-JSON backends or fallback path), the log entry omits those fields (uses `tracing::field::Empty`).
- The `attempt` field allows distinguishing initial calls from retries and correlating token spend per attempt for cost analysis.

### Cross-cutting – Conformance tests (`src/validate`)

The following conformance tests must be added to `src/validate/` following the existing `ConformanceTest` / `RalphHarness` pattern. Add a new module `src/validate/tests_sessions.rs` registered in `src/validate/mod.rs`:

- **`sessions::history_capping_limits_review_entries`**: Create a project, run through 5+ review iterations using a mock backend, then verify that the implementer prompt passed to the backend on iteration 6 contains only the last N review history entries (where N = `max_review_history_entries_in_prompt`). Verify via the agent-output log file content.
- **`sessions::history_capping_limits_qa_entries`**: Same pattern for QA history capping.
- **`sessions::session_lifecycle_stores_and_resumes`**: Enable `session_reuse_enabled=true`, configure a mock backend that returns Claude-style JSON with `session_id` in its output. Run two iterations. Verify: (a) state.json contains a session record after the first call, (b) the second call's agent-output log shows `--resume` args.
- **`sessions::session_invalidation_on_rollback`**: Create sessions across loops 1-3, rollback to loop 1, verify state.json has no session records for loops 2 or 3.
- **`sessions::session_invalidation_on_prompt_change`**: Enable `prompt_change_action=restart-loop` and `session_reuse_reset_on_prompt_change=true`. Modify prompt.md mid-run. Verify session records for the restarted loop are cleared.
- **`sessions::working_directory_stays_at_repo_root`**: Run a full loop with session reuse enabled, verify that the backend invocation's working directory (observable via mock script's `pwd` output captured to a sidecar file) is the repo root, not a loop subdirectory.

## Technical Approach

### A – Template introspection

Add `src/prompts/template_introspection.rs` with:

```rust
pub fn template_uses_var(template_source: &str, var_name: &str) -> bool {
    let needle = format!("{{{{{var_name}}}}}");
    template_source.contains(&needle)
}

pub fn load_template_source(path: &Path, fallback: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| fallback.to_owned())
}
```

In each `build_*_prompt` function in `orchestrator.rs`, call `load_template_source` once to get the raw template, then after `render_template_with_fallback`, gate each appended section with `if !template_uses_var(&source, "var_name")`. The `format!()` calls that build the post-template string must be decomposed into conditional `push_str` calls. This is a mechanical refactor — no logic change, just conditional gating.

### B – Planner prompt compression

Add `summarize_state_for_planner` and `summarize_previous_specs_for_planner` as functions in `orchestrator.rs` (or a new `src/workflow/prompt_compression.rs` extracted from `orchestrator.rs`). The summary format is a plain-text multi-line block:

```
Project: {id} ({name}) — {status}
Current: loop {N}, phase {phase}, iteration {iter}
Loops (last {max}):
  Loop 1: "feature-name" — Completed (3 review iters, last: approved; 1 QA iter, last: pass) — spec: loops/001-slug/...-spec.md
  Loop 2: "feature-name" — InProgress (1 review iter, last: suggestions; 0 QA iters) — spec: loops/002-slug/...-spec.md
```

`build_planner_prompt` switches between `state_json` and the summary based on `effective.workflow.planner_state_in_prompt`. Same for `collect_previous_specs` vs. `summarize_previous_specs_for_planner`.

The new `PlannerStateInPrompt` and `PreviousSpecsInPrompt` enums follow the existing enum pattern:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PlannerStateInPrompt {
    FullJson,
    #[default]
    Summary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PreviousSpecsInPrompt {
    None,
    #[default]
    Titles,
    FullText,
}
```

Config propagation follows the existing four-layer pattern (global → project override → effective → CLI). See B-Config acceptance criteria for exact field placements.

### C – History capping

Modify `collect_review_history` and `collect_qa_history` signatures to accept `max_entries: usize`. Implementation:

1. Collect all entries from the current loop's `artifacts.reviews` (or `artifacts.qa_results`).
2. **Sort by `exchange.iteration` ascending** (this is the critical change — the current code iterates by insertion order which happens to be sorted in practice, but the spec requires explicit sorting for correctness guarantees).
3. If `max_entries == 0`, return empty string immediately.
4. If `entries.len() > max_entries`, take the last N via `.skip(entries.len() - max_entries)`.
5. Format and return.

The callers in each `build_*_prompt` function compute `max_entries` from `EffectiveWorkflowConfig` fields and the `session_reuse_enabled` flag.

### D – Session reuse

**State layer:** `SessionStore` and `SessionRecord` in `state.rs`. Serde-compatible with existing state files via `#[serde(default)]`.

**Backend layer:** `effective_args` is a method on `CliBackend` that returns `Result<Vec<String>>`. It scans the existing args for known markers (`-p` for Claude, `"-"` for Codex) and rewrites them when a session ID is provided. The method returns `Err` if the expected markers are not found in custom arg layouts, allowing the caller to fall back to a non-session invocation with a warning rather than crashing. The orchestrator catches this error, logs a `tracing::warn!("session resume arg rewriting failed for {backend}: {err}, falling back to full prompt")`, clears the session context, and proceeds with a fresh full prompt.

**Output normalization pipeline integration:** `normalize_output` is called at a single integration point that wraps every `backend.execute_with_log()` return value. Concretely, the call chain becomes:

```
raw_stdout = backend.execute_with_log(prompt, log_writer).await?;
let normalized = normalize_output(backend.name(), &raw_stdout).unwrap_or_else(|_| {
    NormalizedOutput { text: raw_stdout.clone(), ..Default::default() }
});
// log token metrics (F)
// pass normalized.text to parse_fn
```

This ensures that `parse_fn` never sees Claude's JSON wrapper or Codex's JSONL envelope on any attempt.

**Session ID lifecycle:** See D-SessionID acceptance criteria. The key design principle is that session records are created/updated based on normalization output, not parse success. A failed parse with a valid session ID still stores the record, enabling the session-aware retry (E) to leverage the context.

**Bootstrap hash:** See D-Bootstrap acceptance criteria for the exact specification. The hash is computed by the orchestrator using the existing `sha256_hex` utility from `src/util/hash.rs`. Each role hashes only its own template source (resolved via `load_template_source` with the effective template path and the corresponding `default_*_template()` fallback).

**Working directory invariant:** The orchestrator already runs backends from the repo root. The spec adds a `debug_assert!` before each backend invocation confirming `std::env::current_dir()` matches the expected repo root. The conformance test `sessions::working_directory_stays_at_repo_root` validates this end-to-end via a mock script that captures `pwd`.

### E – Parse-retry optimization

Modify `execute_with_parse_retries` to accept an optional `BackendInvocationContext`. The function's internal attempt loop expands from 3 to 4 maximum attempts when a session is active. The new attempt (session follow-up) is inserted between the initial attempt and the reformatter attempt. The `LogWriter` attempt counter increments naturally — attempt separators in the log file will show `attempt=1` through `attempt=4` when all fire.

The `ParseRetriesExhausted.attempts` field reflects the actual number of attempts executed: 3 when no session was active (preserving existing behavior), 4 when a session follow-up was attempted. This is a semantic change but does not break existing error handling since `attempts` is used only for logging/display.

### F – Token metrics

After `normalize_output`, emit structured `tracing::info!` with the fields. This happens per-attempt (including retries), so a single phase may produce multiple log entries. Each entry includes the `attempt` number (1-based) to distinguish initial calls from retries. When multiple attempts occur, the consumer can sum `tokens_in` across attempts for total cost attribution, or filter to `attempt=1` for initial-call-only analysis.

### Rollout order
1. A + B + C — sessionless, backward-compatible, purely reduces prompt size.
2. D behind `session_reuse_enabled=false` — no behavioral change until opted in.
3. F — observability.
4. E — leverages sessions for cheaper retries.

## Files & Modules

| File | Change | Description |
|------|--------|-------------|
| `src/prompts/template_introspection.rs` | **New** | `template_uses_var`, `load_template_source` |
| `src/prompts/templates.rs` | Minor modify | Re-export or reference `template_introspection` |
| `src/prompts/mod.rs` | Modify | Add `pub mod template_introspection;` |
| `src/workflow/orchestrator.rs` | Major modify | De-dup logic in all `build_*_prompt` fns; planner compression calls; history capping params with explicit sort; session invocation flow; delta prompt builders; parse-retry session optimization with 4-attempt pipeline; token metrics logging; working-directory `debug_assert!` |
| `src/project/state.rs` | Modify | Add `SessionStore`, `SessionRecord`, `#[serde(default)] session_store` field, `remove_for_loop` called in `remove_loop` |
| `src/config/global.rs` | Modify | Add `PlannerStateInPrompt`, `PreviousSpecsInPrompt` enums; all new `WorkflowConfig` fields (10 fields: 3 for B, 3 for C, 4 for D) |
| `src/config/project.rs` | Modify | Add corresponding `Option<T>` fields to `ProjectWorkflowOverrides` (10 fields) |
| `src/config/mod.rs` | Modify | Add fields to `EffectiveWorkflowConfig` (10 fields); add resolution logic in `resolve_effective_config`; re-export new enum types |
| `src/cli/config.rs` | Modify | Add `set_global_value` / `set_project_value` match arms for all 10 new keys; add parse functions for new enums and `session_reuse_roles` validation |
| `src/backend/mod.rs` | Modify | `BackendInvocationContext` struct; `CliBackend::effective_args` method returning `Result<Vec<String>>` |
| `src/backend/claude.rs` | Minor modify | Helper constant or doc for `-p` flag expectation |
| `src/backend/codex.rs` | Minor modify | Helper constant or doc for `"-"` marker expectation |
| `src/backend/tmux_backend.rs` | Minor modify | Thread `BackendInvocationContext` through to `build_shell_command` via inner `CliBackend::effective_args` |
| `src/backend/output_normalizer.rs` | **New** | `NormalizedOutput`, `normalize_output` for Claude JSON / Codex JSONL / raw fallback |
| `src/cli/rollback.rs` | Modify | Call `state.session_store.remove_for_loop()` for all rolled-back loops (unconditional); additionally clear target loop sessions when `session_reuse_reset_on_rollback=true` |
| `src/validate/tests_sessions.rs` | **New** | Conformance tests for session lifecycle, invalidation, history capping, working directory |
| `src/validate/mod.rs` | Modify | Register `tests_sessions` module |
| `src/validate/mock_scripts.rs` | Modify | Add mock scripts that return Claude-style JSON with `session_id` and capture `pwd` |
| `src/lib.rs` | No change | `backend` mod already declared; `prompts` mod already declared |

## Testing Strategy

### Unit tests

**Template introspection** (`src/prompts/template_introspection.rs`):
- `template_uses_var("foo {{bar}} baz", "bar")` → `true`
- `template_uses_var("foo {{bar}} baz", "baz")` → `false`
- `template_uses_var("foo {{bar_extra}}", "bar")` → `false` (no partial match — exact `{{bar}}` not found)
- `template_uses_var("{{a}} {{b}} {{a}}", "a")` → `true`

**Prompt de-duplication** (`src/workflow/orchestrator.rs` or extracted module):
- Render `build_planner_prompt` with default template → count occurrences of `## Master Prompt` heading → exactly 1.
- Render with a custom template that omits `{{prompt_content}}` → `## Master Prompt` heading appears exactly 1 time (appended).
- Same pattern for each role's key sections.

**Planner compression** (`src/workflow/orchestrator.rs` or extracted):
- `summarize_state_for_planner` with a state containing 5 completed loops and 1 in-progress loop → output contains all 6 loop summaries; output does NOT contain any `feedback` text or `report` text from `ReviewExchange`/`QaExchange`.
- Snapshot test: exact format of summary for a known state fixture.
- `summarize_previous_specs_for_planner` with `Titles` mode → bullet list, no spec body content.

**History capping**:
- Build 10 `ReviewExchange` entries (iterations 1–10), call `collect_review_history(state, dir, max_entries=3)` → output contains only iterations 8, 9, 10.
- Same for `collect_qa_history` with 5 entries and cap=2 → only iterations 4, 5.
- Cap=0 → empty string.
- **Unsorted input**: Build 10 entries inserted in non-sequential order (e.g., 3, 1, 7, 2, 10, 5, 4, 8, 6, 9), cap=3 → still yields iterations 8, 9, 10 (proves explicit sort, not insertion order).

**Session store** (`src/project/state.rs`):
- `upsert` then `lookup` returns the record.
- `remove_for_loop(3)` removes loop 3 records but preserves loop 2 records.
- Round-trip serialization: `SessionStore` with records serializes/deserializes correctly; empty store deserializes from `{}` (backward compat).

**Effective args** (`src/backend/mod.rs`):
- Claude: base args `["-p", "--permission-mode", "acceptEdits", ...]` + session → args become `["--resume", "sess-123", "--output-format", "json", "--permission-mode", "acceptEdits", ...]` (no `-p`). Returns `Ok(...)`.
- Claude: custom args missing `-p` (e.g., `["--some-flag"]`) + session → returns `Err(...)` with descriptive message.
- Claude: no session → `Ok(args_unchanged)`.
- Codex: base args `["exec", "--dangerously-bypass-approvals-and-sandbox", "-"]` + session → args become `["exec", "resume", "thread-456", "--dangerously-bypass-approvals-and-sandbox", "--json", "-"]`. Returns `Ok(...)`.
- Codex: custom args missing `"-"` marker + session → returns `Err(...)`.
- Codex: no session → `Ok(args_unchanged)`.
- Both: idempotency test (calling `effective_args` twice with same ctx yields identical output).

**Output normalization** (`src/backend/output_normalizer.rs`):
- Claude JSON fixture → extracts `text`, `session_id`, usage.
- Codex JSONL fixture → extracts `thread_id`, last agent message text, usage.
- Raw non-JSON string → `text = raw`, everything else `None`.
- Malformed JSON → returns `text = raw`, no panic.

**Bootstrap hash**:
- Same inputs → same hash.
- Changing role → different hash.
- Changing template content → different hash.
- Missing template file (fallback used) → deterministic hash (same across calls).
- Version constant `sessions-v1` is included.

**Config merge** (`src/config/mod.rs`):
- Global `PlannerStateInPrompt::FullJson` + project override `Summary` → effective is `Summary`.
- Global `session_reuse_roles: ["implementer"]` + project override `["implementer","qa"]` → effective is `["implementer","qa"]`.
- All new fields with no project override → effective equals global default.

**Session role validation** (`src/cli/config.rs`):
- `parse_session_reuse_roles("implementer,reviewer,qa")` → `Ok(vec![...])`.
- `parse_session_reuse_roles("implementer,bogus")` → `Err(RalphError::Validation(...))` listing valid roles.
- At runtime (orchestrator), `session_reuse_roles: ["implementer","bogus"]` → `warn!` logged, `"bogus"` filtered out, only `"implementer"` used.

### Integration tests (mocked backend)

- **Session lifecycle**: mock backend returns `session_id` in JSON output. First orchestrator call stores record. Second call for same (loop, role, backend) uses `--resume` args and receives a delta prompt (verified via mock's received prompt). Third call with different bootstrap hash starts fresh.
- **Rollback clears sessions**: populate session records for loops 1–3, rollback to loop 1, verify records for loops 2 and 3 are gone (unconditional). Verify loop 1 records are cleared only when `session_reuse_reset_on_rollback=true`.
- **Parse failure with session ID**: mock backend returns valid JSON with `session_id` but unparseable `result` text. Verify session record is still created. Verify the session-aware retry (attempt 2) sends the short follow-up via resume.
- **Arg rewrite failure fallback**: configure a backend with custom args missing the expected marker. Verify the orchestrator logs a warning and falls back to full prompt (no crash).

### Conformance tests (`src/validate/tests_sessions.rs`)

See "Cross-cutting – Conformance tests" in acceptance criteria for the full list:
- `sessions::history_capping_limits_review_entries`
- `sessions::history_capping_limits_qa_entries`
- `sessions::session_lifecycle_stores_and_resumes`
- `sessions::session_invalidation_on_rollback`
- `sessions::session_invalidation_on_prompt_change`
- `sessions::working_directory_stays_at_repo_root`

### Build verification

All changes must pass `nix develop -c cargo check`, `nix develop -c cargo test`, and `nix develop -c cargo build`. The `#[serde(default)]` annotations ensure existing state files deserialize without error.

## Out of Scope

- **LLM-based summarization or compaction**: All reductions are deterministic string operations. No LLM calls for summarizing history or state.
- **Cross-role or cross-loop session sharing**: Sessions are strictly isolated per `(project_id, loop_number, role, backend_spec)`.
- **Prompt-reviewer or completer session reuse**: Only implementer, reviewer, and QA roles are eligible by default (configurable via `session_reuse_roles`).
- **Streaming output normalization**: The normalizer processes complete stdout after the backend process exits, not streamed chunks.
- **Automatic enablement of session reuse**: `session_reuse_enabled` defaults to `false`; opt-in only.
- **Changes to the `Backend` trait signature**: Session args are handled by `CliBackend::effective_args()`, not by changing the trait.
- **Token budget enforcement**: This spec adds token *logging*, not hard token limits or automatic prompt truncation.
- **Template engine migration**: The `{{var}}` replacement engine stays as-is. No move to Handlebars/Tera.
- **Changes to the daemon, MCP, PRD, or workspace modules**: Only the orchestration/backend/config/state paths are modified.
- **Acceptance QA session reuse**: The acceptance QA phase runs on both backends with a one-shot prompt; session reuse is not applied there.
- **RunWorkflowOverrides for session fields**: The new session/planner/history config fields are not exposed as CLI run-time overrides (no changes to `RunWorkflowOverrides`). They are set via `config set` or TOML files only.
