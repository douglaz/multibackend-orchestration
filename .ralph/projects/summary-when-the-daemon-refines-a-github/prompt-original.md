I now have all the context needed. Let me produce the revised specification.

---

## Summary

Extend the daemon's refinement step to produce a **cleaned-up issue body** in addition to the existing structured task description and title. After refinement, write the cleaned body back to GitHub via `gh issue edit --body`, so the issue itself reads well — independent of the structured task description posted as a comment.

## Acceptance Criteria

- [ ] Refinement prompt instructs the model to output a cleaned issue body (light editorial pass) as a distinct section alongside the existing title + structured task description
- [ ] Prompt explicitly instructs the model that the cleaned body must contain **only the issue body** (not the title), to prevent overwriting the GitHub issue body with a title+body combination
- [ ] `RefinedPrompt` struct gains a `cleaned_body: Option<String>` field
- [ ] `parse_refined_output` extracts the cleaned body from the new output section using line-level delimiter matching (`line.trim() == "=== CLEANED BODY ==="`)
- [ ] Cleaned-body validation is **best-effort**: if the section is missing, empty, or too short, `cleaned_body` is `None` and refinement/dispatch proceeds normally — cleaned-body problems never fail refinement
- [ ] Structured `body` validation remains **strict**: empty or too-short body still fails refinement (existing behavior)
- [ ] `github.rs` exposes `update_issue_body(owner, repo, issue_number, body)` using `gh issue edit --body`
- [ ] `runtime.rs` calls `update_issue_body` after refinement (best-effort, same pattern as title update)
- [ ] Existing behavior preserved: structured task description still posted as `refined-prompt` comment and passed to `ralph auto --idea`
- [ ] Fallback: if the model omits the cleaned-body section, `cleaned_body` is `None` and the issue body is left untouched
- [ ] Unit tests cover parsing of the new three-section format, graceful fallback, and delimiter-in-content robustness
- [ ] Conformance tests in `tests_daemon.rs` verify end-to-end behavior: `gh issue edit --body` called with cleaned body, `--idea` and comment still use structured task description only, and missing section skips body edit

## Technical Approach

### 1. New output format (prompt change in `refine.rs`)

Replace the current two-section format with a three-section format. The `=== CLEANED BODY ===` delimiter is matched at the **line level** (an entire line whose trimmed content equals the delimiter string), not by substring search, to avoid false matches when the token appears inside content such as code blocks:

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
- **Explicitly state: the cleaned body section must contain only the issue body text, NOT the issue title.** The title is already handled separately via `TITLE:`. This prevents the model from returning `title\n\nbody` which would overwrite the GitHub issue body incorrectly. (The original issue is fed to the prompt as `title\n\nbody` via `raw_idea`, so without this instruction the model may echo both.)

### 2. Extend `RefinedPrompt` struct (`refine.rs:28-32`)

Add one field:

```rust
pub struct RefinedPrompt {
    pub title: Option<String>,
    pub body: String,
    pub cleaned_body: Option<String>,  // NEW
}
```

`cleaned_body` is `Option` so the fallback path (no `=== CLEANED BODY ===` section found, or section invalid) returns `None` gracefully, leaving the issue body untouched.

### 3. Update parser (`refine.rs:71-109`)

In the structured branch (after extracting title and finding `---`):

1. Collect all lines after the `---` delimiter.
2. Scan for the **first line** where `line.trim() == "=== CLEANED BODY ==="`. This is a **line-level match**, not a substring split — meaning the delimiter must occupy an entire line on its own. If the token `=== CLEANED BODY ===` appears mid-line (e.g., inside a code block or prose), it is **not** treated as the section break.
3. If the delimiter line is found: lines before it form the structured task description (`body`), lines after it form `cleaned_body`.
4. If the delimiter line is absent: current behavior — entire remainder is `body`, `cleaned_body` is `None`.

**Validation discipline (addresses review issue #1):**

- **Structured `body`**: validated with `validate_output` as today. Failure → refinement error → fallback to raw idea. This is unchanged.
- **`cleaned_body`**: validated with a separate **best-effort** path. If the cleaned-body text is empty, whitespace-only, or shorter than `MIN_OUTPUT_LENGTH`, set `cleaned_body = None` silently (log a debug message but do **not** return `Err`). Cleaned-body validation must never cause refinement or dispatch to fail. This is implemented as a `try` block or equivalent that catches validation errors for the cleaned-body section only:

```rust
// After splitting on the delimiter line:
let cleaned_body = match validate_output(&cleaned_raw) {
    Ok(text) => Some(text),
    Err(_) => None, // graceful degradation — skip body update
};
```

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
| `src/daemon/refine.rs` | Update `REFINEMENT_SYSTEM_PROMPT` with three-section format, editorial instructions, and explicit body-only instruction for the cleaned section. Add `cleaned_body: Option<String>` to `RefinedPrompt`. Update `parse_refined_output` to scan for delimiter at line level and split. Cleaned-body validation is best-effort (errors → `None`). Add unit tests for new parsing. |
| `src/daemon/github.rs` | Add `pub fn update_issue_body(owner, repo, issue_number, body) -> Result<()>` modeled on `update_issue_title`. |
| `src/daemon/runtime.rs` | Destructure `cleaned_body` from refinement result at ~line 666. Add best-effort `update_issue_body` call after the title-update block (~line 716). |
| `src/validate/tests_daemon.rs` | Add conformance tests for cleaned-body dispatch behavior (see Testing Strategy). |

No new files. No changes to `DaemonTask` persistent storage (the cleaned body is ephemeral — only used at dispatch time to update GitHub, not stored or reused later).

## Testing Strategy

### Unit tests (`refine.rs`)

1. **`parse_refined_output_three_section_success`** — Full three-section output → extracts title, body, and cleaned_body correctly. Verifies `cleaned_body` contains only the body text (not the title).
2. **`parse_refined_output_no_cleaned_body_fallback`** — Two-section output (current format) → `cleaned_body` is `None`, title and body extracted normally. Ensures backward compatibility with backends that don't produce the new section.
3. **`parse_refined_output_empty_cleaned_body_degraded`** — `=== CLEANED BODY ===` present but section is empty/too short → `cleaned_body` is `None` (graceful degradation). Crucially, refinement **succeeds** — `body` and `title` are still returned normally. This validates that cleaned-body problems never fail refinement.
4. **`parse_refined_output_cleaned_body_preserves_structure`** — Cleaned body containing markdown (headers, bullets, code blocks) is preserved verbatim.
5. **`parse_refined_output_delimiter_in_content_not_split`** — Output where `=== CLEANED BODY ===` appears mid-line inside a code block (e.g., `` `=== CLEANED BODY ===` `` or indented inside a fenced block). Verifies the parser does **not** split on it — the entire remainder is treated as `body` and `cleaned_body` is `None`. This confirms line-level delimiter matching.

### Conformance tests (`src/validate/tests_daemon.rs`)

These tests use the existing harness pattern (mock `gh` script + mock refinement backend + mock ralph) to verify end-to-end runtime behavior:

6. **`daemon::refinement_cleaned_body_dispatched`** — Mock refinement backend outputs three-section format. Mock `gh` script logs `--body` argument to a file when `issue edit` is called with `--body`. Assert: (a) the logged body matches the cleaned body from the refinement output, (b) the `--idea` argument to the spawned child is the structured task description (not the cleaned body), (c) the `refined-prompt` comment body does not contain the cleaned body.

7. **`daemon::refinement_no_cleaned_body_skips_edit`** — Mock refinement backend outputs two-section format (no `=== CLEANED BODY ===`). Mock `gh` script logs all `issue edit` invocations. Assert: no `--body` argument is logged (i.e., `update_issue_body` is never called), while `--title` is still called if title is present. Dispatch completes normally.

8. **`daemon::refinement_cleaned_body_failure_non_blocking`** — Mock refinement backend outputs three-section format. Mock `gh` script fails when `--body` is passed (exit 1). Assert: dispatch still completes (task transitions to active), `--idea` is passed correctly, and a warning about the body update failure appears in stderr. This verifies the best-effort pattern.

### Integration / Manual test

- Create a GitHub issue with intentional typos and unclear phrasing.
- Run the daemon with refinement enabled.
- Verify: (1) issue title updated, (2) issue body updated with cleaned version (typos fixed, structure preserved), (3) `refined-prompt` comment posted with the structured task description, (4) child process receives the structured task description via `--idea`, (5) the updated issue body does not contain the title as a prefix.

## Out of Scope

- **Persisting `cleaned_body` to `DaemonTask` store** — not needed; the cleaned body is only used at dispatch time to push to GitHub.
- **Updating PR body with cleaned text** — PR body generation is a separate flow with its own logic.
- **Configurable toggle for body cleanup** — follows the existing `refinement_enabled` flag; no separate toggle needed at this stage.
- **Retry logic for `gh issue edit --body` failures** — follows existing best-effort pattern (log warning and continue).
- **Diff or preview of body changes before writing** — the editorial pass is intentionally light; no approval step is needed.
- **Structured task description merged into the issue body** — explicitly excluded per requirements; the two artifacts remain separate.