Now I have all the information needed. Let me write the revised specification.

Here is the revised specification addressing all five review issues:

---

## Summary

Extend the daemon's GitHub issue refinement flow to produce a **rewritten title** alongside the existing rewritten body. Currently, `refine::refine_prompt()` receives the combined `title\n\nbody` as a single string and returns a single refined text block. The refinement system prompt does not instruct the LLM to produce a separate title. This spec adds structured output to the refinement step so the LLM returns both a concise rewritten title and a refined body. The rewritten title will be used for the PR title (replacing the current hardcoded `"ralph: {task_id}"` pattern at `runtime.rs:878`) and persisted on the `DaemonTask` for downstream consumers.

## Acceptance Criteria

- [ ] `refine::refine_prompt()` returns a structured `RefinedPrompt { title, body }` instead of a single `String`
- [ ] The refinement system prompt instructs the LLM to output a `TITLE: ...` line (max 80 chars) followed by a `---` delimiter and the refined body
- [ ] `parse_refined_output()` parses the structured output; when the delimiter is missing, returns the **entire output as the body** with `title = None`, preserving existing body behavior
- [ ] Title validation rejects titles longer than 80 characters (returns an error, does not truncate)
- [ ] The rewritten title is stored on `DaemonTask` (new `refined_title: Option<String>` field with `#[serde(default)]`)
- [ ] PR title resolution follows this precedence: `refined_title` → original issue title extracted from `raw_idea` → `"ralph: {task_id}"`
- [ ] The refined-prompt comment posted to the GitHub issue includes the refined title as a bold header when present
- [ ] When refinement is **disabled** (`config.refinement_enabled == false`): `refined_title` is not set; PR title falls back to original issue title from `raw_idea`
- [ ] When refinement **fails** (backend error): `refined_title` is not set; PR title falls back to original issue title from `raw_idea`
- [ ] Legacy tasks (no `refined_title` field in serialized data) deserialize with `refined_title: None`; PR title for legacy tasks uses original issue title from `raw_idea` if available, otherwise `"ralph: {task_id}"`
- [ ] Existing body refinement output quality is unaffected
- [ ] All existing tests in `refine.rs` continue to pass
- [ ] New unit tests cover title extraction, delimiter parsing, and edge cases
- [ ] New validate-conformance tests cover PR title derivation and refined-prompt comment formatting

## Technical Approach

### 1. Introduce `RefinedPrompt` struct (`src/daemon/refine.rs`)

```rust
pub struct RefinedPrompt {
    pub title: Option<String>,
    pub body: String,
}
```

The `title` is `Option<String>` because the LLM may not produce the expected delimiter format. Change `refine_prompt()` return type from `Result<String>` to `Result<RefinedPrompt>`.

### 2. Update the refinement system prompt (`src/daemon/refine.rs:6-17`)

Add a title instruction and a parseable output format to `REFINEMENT_SYSTEM_PROMPT`:

```
Output format:
TITLE: <concise title summarizing the task, max 80 characters>
---
<refined task description body>
```

The delimiter `TITLE: ...\n---\n` is simple, unambiguous, and easy to parse. The LLM is already producing structured output (checklists, sections); adding a title line is a natural extension. The max length in the prompt is 80 characters; validation enforces the same 80-character limit.

### 3. Add title parsing logic (`src/daemon/refine.rs`)

Add a `parse_refined_output()` function:

- Looks for the `TITLE: ` prefix on the first non-empty line
- Splits on the first `---` line separator after the title line
- Returns `RefinedPrompt { title: Some(extracted_title), body }`
- **Fallback when delimiter is missing**: returns `RefinedPrompt { title: None, body: full_output }` — the entire LLM output becomes the body, preserving existing body behavior exactly. This ensures that if the LLM ignores the title format instruction, no body content is lost or misclassified.

Validation rules:
- Title (when present) must be non-empty after trimming and <= 80 characters; if it exceeds 80 characters, `parse_refined_output()` returns an error (not a truncation). The caller (`refine_prompt()`) will then fall back to the raw idea via the existing error path.
- Body must pass existing `validate_output()` checks (non-empty, >= 20 chars).

### 4. Add `extract_original_title()` helper (`src/daemon/runtime.rs`)

Add a helper to extract the original issue title from `raw_idea`:

```rust
fn extract_original_title(raw_idea: &str) -> Option<String> {
    let title = raw_idea.split("\n\n").next()?.trim();
    if title.is_empty() { None } else { Some(title.to_owned()) }
}
```

This exploits the `{title}\n\n{body}` format established by `compose_raw_idea()` at `runtime.rs:493`. Returns `None` if `raw_idea` is empty or the first segment is blank.

### 5. Update `dispatch_task()` in `runtime.rs`

At lines 339-354, where refinement occurs:
- Destructure `RefinedPrompt { title, body }` from `refine_prompt()`
- Store `title` on the task via `refined_title` if `Some`
- Pass `body` to `spawn_ralph_auto()` (same as current `idea`)
- On refinement failure fallback: `refined_title` is not set (remains `None`), raw idea passed through as-is
- When refinement is disabled: `refined_title` is not set (remains `None`), raw idea passed through as-is

### 6. Add `refined_title` field to `DaemonTask` (`src/daemon/mod.rs:54-69`)

```rust
#[serde(default)]
pub refined_title: Option<String>,
```

Using `#[serde(default)]` maintains backwards compatibility with existing serialized tasks (same pattern as `raw_idea`). Legacy tasks without this field deserialize with `refined_title: None`.

### 7. Update PR creation in `handle_pr_flow()` (`runtime.rs:877-878`)

Replace:
```rust
let title = format!("ralph: {}", task.task_id);
```
With:
```rust
let title = task.refined_title.clone()
    .or_else(|| extract_original_title(task.raw_idea.as_deref().unwrap_or_default()))
    .unwrap_or_else(|| format!("ralph: {}", task.task_id));
```

This establishes a three-tier precedence:
1. **`refined_title`** — set when refinement succeeds and the LLM produced a valid title
2. **Original issue title** — extracted from `raw_idea`'s `{title}\n\n{body}` format; covers refinement-disabled, refinement-failed, and legacy tasks that have `raw_idea` populated
3. **`"ralph: {task_id}"`** — ultimate fallback for legacy tasks with no `raw_idea`

### 8. Update refined-prompt comment (`runtime.rs:357-380`)

When the refined title is present, include it as a bold header in the comment:
```rust
let comment_body = match &refined_title {
    Some(t) => format!("**{}**\n\n{}", t, idea),
    None => idea.clone(),
};
```

Pass `comment_body` to `post_idempotent_comment()` instead of `idea_clone`.

### 9. Persist refined title after refinement (`runtime.rs`)

After successful refinement, update the task record:
```rust
if let Some(ref t) = refined_title {
    let store_clone = store.clone();
    let tid = task.task_id.clone();
    let title_clone = t.clone();
    if let Err(err) = spawn_blocking_op(move || {
        store_clone.update_task(&tid, |task| {
            task.refined_title = Some(title_clone.clone());
            Ok(())
        })
    }).await {
        eprintln!("warning: failed to persist refined_title for {}: {err}", task.task_id);
    }
}
```

This follows the existing best-effort pattern — persistence failure logs a warning but does not abort dispatch.

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/refine.rs` | Add `RefinedPrompt` struct (with `title: Option<String>`), update `REFINEMENT_SYSTEM_PROMPT` with title format (80-char prompt + 80-char validation), add `parse_refined_output()` with body-preserving fallback, change `refine_prompt()` return type, add new unit tests |
| `src/daemon/mod.rs` | Add `refined_title: Option<String>` field to `DaemonTask` (with `#[serde(default)]`) |
| `src/daemon/runtime.rs` | Add `extract_original_title()` helper, update `dispatch_task()` to destructure `RefinedPrompt` and persist `refined_title`, update `handle_pr_flow()` PR title with three-tier precedence, update refined-prompt comment to include title header, update `poll_and_claim()` `DaemonTask` construction to include `refined_title: None` |
| `src/validate/tests_daemon.rs` | Add validate-conformance tests for PR title derivation (refined title, original-title fallback, `ralph:` fallback) and refined-prompt comment formatting |

No new files are needed. No changes to `github.rs`, `process.rs`, `worktree.rs`, or configuration modules.

## Testing Strategy

**Unit tests in `refine.rs`:**
- `parse_refined_output()` correctly splits `TITLE: ...\n---\nbody` into `RefinedPrompt { title: Some(...), body }`
- `parse_refined_output()` handles missing delimiter: returns `RefinedPrompt { title: None, body: full_output }` — body content preserved exactly
- `parse_refined_output()` handles empty title after `TITLE: ` prefix: returns error
- `parse_refined_output()` handles title exceeding 80 characters: returns error
- `parse_refined_output()` trims whitespace from both title and body
- `parse_refined_output()` handles body-only content below `MIN_OUTPUT_LENGTH`: returns error
- `build_refinement_prompt()` output contains the `TITLE:` format instructions and `---` delimiter
- Existing `validate_output` tests remain unchanged and pass

**Unit tests in `mod.rs`:**
- `DaemonTask` deserialization without `refined_title` field produces `None` (backwards compat)
- `DaemonTask` round-trip with `refined_title: Some(...)` populated

**Unit tests in `runtime.rs`:**
- `extract_original_title("Fix bug\n\nDetails")` returns `Some("Fix bug")`
- `extract_original_title("Fix bug")` returns `Some("Fix bug")` (no body)
- `extract_original_title("")` returns `None`
- `extract_original_title("\n\nBody only")` returns `None` (empty title segment)

**Validate-conformance tests in `tests_daemon.rs`:**
- **`refinement_title_in_pr`**: Mock refinement backend returns `TITLE: Fix login SSO\n---\nRefined body...`. Assert `gh pr create` receives the refined title `"Fix login SSO"` (not `"ralph: {task_id}"`). Assert persisted `refined_title` on the task JSON matches.
- **`refinement_disabled_pr_uses_original_title`**: Disable refinement. Seed task with `raw_idea = "Original Title\n\nBody"`. Assert `gh pr create` receives `"Original Title"` as PR title.
- **`refinement_failure_pr_uses_original_title`**: Mock refinement backend that fails. Seed task with `raw_idea = "Original Title\n\nBody"`. Assert `gh pr create` receives `"Original Title"` as PR title.
- **`legacy_task_without_raw_idea_pr_uses_fallback`**: Seed task without `raw_idea` or `refined_title`. Assert `gh pr create` receives `"ralph: {task_id}"` as PR title.
- **`refined_prompt_comment_includes_title`**: Mock refinement backend returns `TITLE: ...\n---\nbody`. Assert the `gh issue comment` call body contains `**...**\n\n` bold title header followed by body.
- Update existing `refinement_happy_path` test to verify the mock output still works (no delimiter → body-only passthrough, `refined_title` remains `None`); or update the mock to emit the new format and assert both title and body.

**Integration-level verification:**
- Existing daemon single-iteration integration tests continue to pass (refinement is mocked/disabled in those tests, so the fallback path is exercised and PR title falls through to original-title or `ralph:` fallback)

## Out of Scope

- **Updating the GitHub issue title via the API** — this spec only rewrites the title for internal use (PR title, task metadata); it does not mutate the original GitHub issue title
- **Separate API call for title refinement** — the title is generated in the same LLM call as the body to maintain coherence and avoid extra latency/cost
- **Configurable title max length** — hardcoded at 80 chars for both the prompt instruction and validation; can be made configurable later if needed
- **Retroactive refinement of existing tasks** — only newly dispatched tasks get refined titles; existing tasks use the `extract_original_title` → `"ralph: {task_id}"` fallback chain
- **PR body refinement** — the PR body remains the current format (`"Automated PR for task...\nCloses #..."`)
- **Changes to `ralph auto` prompt format** — only the body (refined task description) is passed to the child process, same as today
- **Backfilling `refined_title` on legacy tasks** — legacy tasks without `refined_title` rely on the three-tier PR title fallback at PR creation time; no migration or backfill step is added