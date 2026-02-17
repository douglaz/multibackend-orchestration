I now have a thorough understanding of the entire codebase. Here is the engineering specification:

---

## Summary

Extend the daemon's refinement step to produce a **cleaned-up issue body** in addition to the existing structured task description and title. After refinement, write the cleaned body back to GitHub via `gh issue edit --body`, so the issue itself reads well — independent of the structured task description posted as a comment.

## Acceptance Criteria

- [ ] Refinement prompt instructs the model to output a cleaned issue body (light editorial pass) as a distinct section alongside the existing title + structured task description
- [ ] `RefinedPrompt` struct gains a `cleaned_body: Option<String>` field
- [ ] `parse_refined_output` extracts the cleaned body from the new output section
- [ ] `github.rs` exposes `update_issue_body(owner, repo, issue_number, body)` using `gh issue edit --body`
- [ ] `runtime.rs` calls `update_issue_body` after refinement (best-effort, same pattern as title update)
- [ ] Existing behavior preserved: structured task description still posted as `refined-prompt` comment and passed to `ralph auto --idea`
- [ ] Fallback: if the model omits the cleaned-body section, `cleaned_body` is `None` and the issue body is left untouched
- [ ] Unit tests cover parsing of the new three-section format and graceful fallback

## Technical Approach

### 1. New output format (prompt change in `refine.rs`)

Replace the current two-section format with a three-section format. Use a distinct, unambiguous delimiter (`=== CLEANED BODY ===`) to separate the cleaned body from the structured task description, avoiding collision with the existing `---` delimiter:

```
TITLE: <concise title, max 80 chars>
---
<refined task description for the coding agent>
=== CLEANED BODY ===
<cleaned-up issue body>
```

Update `REFINEMENT_SYSTEM_PROMPT` to:
- Describe the new section and its purpose
- Instruct the model to perform a **light editorial pass only** on the original issue body: fix typos, grammar, and readability
- Explicitly state: preserve original intent, scope, structure (bullets, headers, code blocks); do NOT expand, add details, or merge with the task description

### 2. Extend `RefinedPrompt` struct (`refine.rs:28-32`)

Add one field:

```rust
pub struct RefinedPrompt {
    pub title: Option<String>,
    pub body: String,
    pub cleaned_body: Option<String>,  // NEW
}
```

`cleaned_body` is `Option` so the fallback path (no `=== CLEANED BODY ===` section found) returns `None` gracefully, leaving the issue body untouched.

### 3. Update parser (`refine.rs:71-109`)

In the structured branch (after extracting title and finding `---`):
1. Join all lines after the `---` delimiter.
2. Split on `=== CLEANED BODY ===`.
3. If the delimiter is found: the first half is the structured task description (`body`), the second half is `cleaned_body`.
4. If the delimiter is absent: current behavior — entire remainder is `body`, `cleaned_body` is `None`.
5. Validate both sections individually with `validate_output`.

In the fallback branch (no `TITLE:` line), `cleaned_body` remains `None`.

### 4. New GitHub helper (`github.rs`)

Add `update_issue_body` directly below the existing `update_issue_title` (after line 308). It follows the identical pattern — `Command::new("gh")` with `--body` instead of `--title`:

```rust
pub fn update_issue_body(owner: &str, repo: &str, issue_number: u32, body: &str) -> Result<()> {
    // gh issue edit <number> --repo <owner/repo> --body <body>
}
```

### 5. Call site in `runtime.rs`

After the existing title-update block (lines 700-716), add an analogous block for body update:

```rust
// Update GitHub issue body with cleaned body (best-effort).
if let Some(ref cleaned) = cleaned_body {
    let owner = task.owner.clone();
    let repo = task.repo.clone();
    let issue_number = task.issue_number;
    let cleaned = cleaned.clone();
    if let Err(err) = spawn_blocking_op(move || {
        github::update_issue_body(&owner, &repo, issue_number, &cleaned)
    }).await {
        eprintln!("warning: failed to update issue body for {}: {err}", task.task_id);
    }
}
```

The destructure at line 666 changes from `(refined.body, refined.title)` to also capture `refined.cleaned_body`.

### 6. Data flow (unchanged for existing paths)

```
refine_prompt()
  → RefinedPrompt { title, body, cleaned_body }

runtime.rs dispatch_task():
  idea         = refined.body            → passed to ralph auto --idea (unchanged)
  refined_title = refined.title          → persisted + gh issue edit --title (unchanged)
  cleaned_body  = refined.cleaned_body   → gh issue edit --body (NEW)
  comment_body  = title + idea           → posted as refined-prompt comment (unchanged)
```

## Files & Modules

| File | Change |
|---|---|
| `src/daemon/refine.rs` | Update `REFINEMENT_SYSTEM_PROMPT` with three-section format and editorial instructions. Add `cleaned_body: Option<String>` to `RefinedPrompt`. Update `parse_refined_output` to split on `=== CLEANED BODY ===`. Add unit tests for new parsing. |
| `src/daemon/github.rs` | Add `pub fn update_issue_body(owner, repo, issue_number, body) -> Result<()>` modeled on `update_issue_title`. |
| `src/daemon/runtime.rs` | Destructure `cleaned_body` from refinement result at ~line 666. Add best-effort `update_issue_body` call after the title-update block (~line 716). |

No new files. No changes to `DaemonTask` persistent storage (the cleaned body is ephemeral — only used at dispatch time to update GitHub, not stored or reused later).

## Testing Strategy

### Unit tests (`refine.rs`)

1. **`parse_refined_output_three_section_success`** — Full three-section output → extracts title, body, and cleaned_body correctly.
2. **`parse_refined_output_no_cleaned_body_fallback`** — Two-section output (current format) → `cleaned_body` is `None`, title and body extracted normally. Ensures backward compatibility with backends that don't produce the new section.
3. **`parse_refined_output_empty_cleaned_body_rejected`** — `=== CLEANED BODY ===` present but section is empty/too short → `cleaned_body` is `None` (graceful degradation, not an error).
4. **`parse_refined_output_cleaned_body_preserves_structure`** — Cleaned body containing markdown (headers, bullets, code blocks) is preserved verbatim.

### Integration / Manual test

- Create a GitHub issue with intentional typos and unclear phrasing.
- Run the daemon with refinement enabled.
- Verify: (1) issue title updated, (2) issue body updated with cleaned version, (3) `refined-prompt` comment posted with the structured task description, (4) child process receives the structured task description via `--idea`.

## Out of Scope

- **Persisting `cleaned_body` to `DaemonTask` store** — not needed; the cleaned body is only used at dispatch time to push to GitHub.
- **Updating PR body with cleaned text** — PR body generation is a separate flow with its own logic.
- **Configurable toggle for body cleanup** — follows the existing `refinement_enabled` flag; no separate toggle needed at this stage.
- **Retry logic for `gh issue edit --body` failures** — follows existing best-effort pattern (log warning and continue).
- **Diff or preview of body changes before writing** — the editorial pass is intentionally light; no approval step is needed.
- **Structured task description merged into the issue body** — explicitly excluded per requirements; the two artifacts remain separate.