I now have all the information needed to write a focused engineering specification. Here's the document:

---

## Summary

Extend the daemon's GitHub issue refinement flow to produce a **rewritten title** alongside the existing rewritten body. Currently, `refine::refine_prompt()` receives the combined `title\n\nbody` as a single string and returns a single refined text block. The refinement system prompt does not instruct the LLM to produce a separate title. This spec adds structured output to the refinement step so the LLM returns both a concise rewritten title and a refined body. The rewritten title will be used for the PR title (replacing the current hardcoded `"ralph: {task_id}"` pattern at `runtime.rs:878`) and persisted on the `DaemonTask` for downstream consumers.

## Acceptance Criteria

- [ ] `refine::refine_prompt()` returns a structured result containing both a `title` (String) and `body` (String), instead of a single String
- [ ] The refinement system prompt instructs the LLM to output a concise title line (max ~80 chars) followed by the refined body, using a parseable delimiter
- [ ] The rewritten title is stored on `DaemonTask` (new `refined_title: Option<String>` field)
- [ ] PR creation at `runtime.rs:878` uses the refined title instead of `"ralph: {task_id}"`
- [ ] The refined-prompt comment posted to the GitHub issue includes the refined title
- [ ] When refinement is disabled or fails (fallback to raw idea), the original issue title is preserved and used
- [ ] Existing body refinement output quality is unaffected
- [ ] All existing tests in `refine.rs` continue to pass
- [ ] New unit tests cover title extraction, delimiter parsing, and edge cases

## Technical Approach

### 1. Introduce `RefinedPrompt` struct (`src/daemon/refine.rs`)

```rust
pub struct RefinedPrompt {
    pub title: String,
    pub body: String,
}
```

Change `refine_prompt()` return type from `Result<String>` to `Result<RefinedPrompt>`.

### 2. Update the refinement system prompt (`src/daemon/refine.rs:6-17`)

Add a title instruction and a parseable output format to `REFINEMENT_SYSTEM_PROMPT`:

```
Output format:
TITLE: <concise title summarizing the task, max 80 characters>
---
<refined task description body>
```

The delimiter `TITLE: ...\n---\n` is simple, unambiguous, and easy to parse. The LLM is already producing structured output (checklists, sections); adding a title line is a natural extension.

### 3. Add title parsing logic (`src/daemon/refine.rs`)

Add a `parse_refined_output()` function that:
- Looks for the `TITLE: ` prefix on the first non-empty line
- Splits on the first `---` line separator
- Returns `RefinedPrompt { title, body }`
- Falls back gracefully: if the delimiter is missing, uses the first line as the title and the rest as the body (defensive parsing)

Validation: title must be non-empty and <= 120 characters; body must pass existing `validate_output()` checks.

### 4. Update `dispatch_task()` in `runtime.rs`

At line 339-354, where refinement occurs:
- Destructure `RefinedPrompt { title, body }` from `refine_prompt()`
- Store `title` on the task via a new `refined_title` field
- Pass `body` to `spawn_ralph_auto()` (same as current `idea`)
- On refinement failure fallback, extract the original issue title from `raw_idea` (it's the first line before `\n\n`)

### 5. Add `refined_title` field to `DaemonTask` (`src/daemon/mod.rs:54-69`)

```rust
#[serde(default)]
pub refined_title: Option<String>,
```

Using `#[serde(default)]` maintains backwards compatibility with existing serialized tasks (same pattern as `raw_idea`).

### 6. Update PR creation in `handle_pr_flow()` (`runtime.rs:877-878`)

Replace:
```rust
let title = format!("ralph: {}", task.task_id);
```
With:
```rust
let title = task.refined_title.clone()
    .unwrap_or_else(|| format!("ralph: {}", task.task_id));
```

### 7. Update refined-prompt comment (`runtime.rs:357-380`)

Include the refined title in the comment body posted to the GitHub issue:
```rust
let comment = format!("**{}**\n\n{}", refined.title, refined.body);
```

### 8. Persist refined title after refinement (`runtime.rs`)

After successful refinement, update the task record:
```rust
store.update_task(&task.task_id, |t| {
    t.refined_title = Some(refined.title.clone());
    Ok(())
})?;
```

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/refine.rs` | Add `RefinedPrompt` struct, update `REFINEMENT_SYSTEM_PROMPT` with title format, add `parse_refined_output()`, change `refine_prompt()` return type, update `validate_output()` to validate body portion, add new tests |
| `src/daemon/mod.rs` | Add `refined_title: Option<String>` field to `DaemonTask` (with `#[serde(default)]`) |
| `src/daemon/runtime.rs` | Update `dispatch_task()` to destructure `RefinedPrompt`, persist `refined_title`, update `handle_pr_flow()` PR title, update refined-prompt comment format, update fallback path to extract original title |

No new files are needed. No changes to `github.rs`, `process.rs`, `worktree.rs`, or configuration modules.

## Testing Strategy

**Unit tests in `refine.rs`:**
- `parse_refined_output()` correctly splits `TITLE: ...\n---\nbody` into title and body
- `parse_refined_output()` handles missing delimiter (falls back to first-line-as-title)
- `parse_refined_output()` handles empty title after `TITLE: ` prefix (error)
- `parse_refined_output()` trims whitespace from both title and body
- `parse_refined_output()` handles title exceeding max length (truncates or errors)
- `build_refinement_prompt()` output contains the title format instructions
- Existing `validate_output` tests remain unchanged and pass

**Unit tests in `mod.rs`:**
- `DaemonTask` deserialization without `refined_title` field (backwards compat, `None`)
- `DaemonTask` round-trip with `refined_title` populated

**Integration-level verification:**
- Existing daemon single-iteration integration tests continue to pass (refinement is mocked/disabled in those tests, so the fallback path is exercised)

## Out of Scope

- **Updating the GitHub issue title via the API** — this spec only rewrites the title for internal use (PR title, task metadata); it does not mutate the original GitHub issue title
- **Separate API call for title refinement** — the title is generated in the same LLM call as the body to maintain coherence and avoid extra latency/cost
- **Configurable title max length** — hardcoded at 120 chars; can be made configurable later if needed
- **Retroactive refinement of existing tasks** — only newly dispatched tasks get refined titles; existing tasks use the `unwrap_or` fallback
- **PR body refinement** — the PR body remains the current format (`"Automated PR for task...\nCloses #..."`)
- **Changes to `ralph auto` prompt format** — only the body (refined task description) is passed to the child process, same as today