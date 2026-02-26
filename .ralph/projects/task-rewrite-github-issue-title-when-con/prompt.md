Implement structured refinement output for daemon issue refinement so the system can produce and persist a rewritten title plus refined body, and use that title for PR creation when available.

### Objective
Change refinement from `String` output to structured output:
- `title: Option<String>`
- `body: String`

Use the refined title for PR titles and refined-prompt issue comments, while preserving existing body behavior and fallback safety.

### Scope
- Modify daemon refinement parsing and return type.
- Persist optional refined title on task state.
- Update PR title derivation precedence.
- Update refined-prompt GitHub comment formatting.
- Add unit tests and validate conformance tests.

### Required Behavior

#### 1) Structured refinement result
- Add `RefinedPrompt` in `src/daemon/refine.rs`:
  - `pub title: Option<String>`
  - `pub body: String`
- Change `refine_prompt()` return type from `Result<String>` to `Result<RefinedPrompt>`.

#### 2) Refinement system prompt format
- Update `REFINEMENT_SYSTEM_PROMPT` to require:
  - First line: `TITLE: <concise title, max 80 chars>`
  - Next section separator: `---` on its own line
  - Remaining text: refined body
- Keep existing body-quality instructions intact.

#### 3) Deterministic parser contract
Implement `parse_refined_output()` with this exact behavior:
1. Find first non-empty line.
2. Structured parsing is attempted only if that line starts with `TITLE:`.
3. Structured parsing succeeds only if a delimiter line exists after title where `line.trim() == "---"`.
4. On structured success:
- `title` is text after `TITLE:`, trimmed.
- `body` is all content after first delimiter line, trimmed.
- Validate title: non-empty and `<= 80` chars; if invalid, return error.
- Validate body with existing `validate_output()` rules; if invalid, return error.
5. If structured format is not satisfied (missing `TITLE:` first non-empty line or missing delimiter), return:
- `RefinedPrompt { title: None, body: full_output }`
- Then validate body with existing `validate_output()` rules.
6. Do not truncate overlong titles; return error.

This preserves legacy behavior when model output is unstructured.

#### 4) Task model changes
In `src/daemon/mod.rs`, add to `DaemonTask`:
- `#[serde(default)] pub refined_title: Option<String>`

Backwards compatibility requirement:
- Legacy serialized tasks without this field must deserialize with `None`.

#### 5) Runtime refinement flow
In `dispatch_task()` (`src/daemon/runtime.rs`):
- On refinement success:
  - Use `RefinedPrompt.body` as `idea` passed to `spawn_ralph_auto()`.
  - Set in-memory task `refined_title` from `RefinedPrompt.title`.
  - Persist `refined_title` best-effort via store update; log warning on failure; do not abort dispatch.
- On refinement disabled:
  - Do not set `refined_title`.
  - Use raw idea unchanged.
- On refinement failure (backend/parse/validation):
  - Do not set `refined_title`.
  - Use raw idea unchanged.

Also ensure new tasks created in polling/claim paths initialize `refined_title: None`.

#### 6) Original title extraction
Add helper in `src/daemon/runtime.rs`:
- `extract_original_title(raw_idea: &str) -> Option<String>`
- Rule:
  - Take segment before first `\n\n`.
  - Trim it.
  - Return `None` if empty; otherwise `Some(title)`.

#### 7) PR title precedence
In `handle_pr_flow()` replace hardcoded PR title logic with:
1. `task.refined_title` if present.
2. Else `extract_original_title(task.raw_idea.as_deref().unwrap_or_default())`.
3. Else `format!("ralph: {}", task.task_id)`.

#### 8) Refined-prompt comment formatting
When posting refined-prompt comment:
- If refined title exists, comment body must be exactly:
  - `**{title}**\n\n{body}`
- If no refined title, comment body remains body only.

### Files To Modify
- `src/daemon/refine.rs`
- `src/daemon/mod.rs`
- `src/daemon/runtime.rs`
- `src/validate/tests_daemon.rs` (or create it if absent)
- `src/validate/mod.rs` (register validate test module if newly added)

### Acceptance Criteria
- `refine_prompt()` returns `Result<RefinedPrompt>`.
- Prompt instructions include `TITLE:` and `---` output contract.
- Parser fallback preserves full body when structured delimiter is absent.
- Title validation enforces non-empty and max 80 chars with error on violation.
- `DaemonTask` includes `refined_title` with serde default and legacy compatibility.
- PR title precedence is `refined_title` -> original raw title -> `ralph: {task_id}`.
- Refined-prompt issue comment includes bold title header when present.
- Disabled or failed refinement never sets `refined_title` and uses raw idea path.
- Existing body refinement behavior remains compatible.
- Existing refine tests continue passing after updates.

### Required Tests

#### Unit tests (`src/daemon/refine.rs`)
- Structured parse success (`TITLE` + delimiter + body).
- Missing delimiter returns `title: None`, full output as body.
- Empty title with structured format returns error.
- Title > 80 chars returns error.
- Body validation still enforced.
- Prompt builder/system prompt includes required format contract.

#### Unit tests (`src/daemon/mod.rs` and/or relevant serde test module)
- Legacy `DaemonTask` deserialization without `refined_title` yields `None`.
- Round-trip serialization preserves `refined_title: Some(...)`.

#### Unit tests (`src/daemon/runtime.rs`)
- `extract_original_title("Fix bug\n\nDetails") == Some("Fix bug")`
- `extract_original_title("Fix bug") == Some("Fix bug")`
- `extract_original_title("") == None`
- `extract_original_title("\n\nBody only") == None`

#### Validate conformance (`src/validate/tests_daemon.rs`)
- `refinement_title_in_pr`: refined title used by `gh pr create`.
- `refinement_disabled_pr_uses_original_title`: original title fallback.
- `refinement_failure_pr_uses_original_title`: original title fallback on refine error.
- `legacy_task_without_raw_idea_pr_uses_fallback`: falls back to `ralph: {task_id}`.
- `refined_prompt_comment_includes_title`: bold title header appears in issue comment when present.
- Register test module in `src/validate/mod.rs`.

### Out of Scope
- Updating GitHub issue titles via API.
- Separate LLM call for title refinement.
- Configurable title length.
- Migration/backfill of old tasks.
- PR body format changes.
- `ralph auto` prompt format changes beyond using refined body text.