Implement support for a cleaned GitHub issue body during daemon refinement, while preserving all existing refinement and dispatch behavior.

### Objective
Extend refinement so it can return three artifacts:
- `title` (optional, existing behavior)
- `body` (structured task description, existing behavior, strict validation)
- `cleaned_body` (new, optional, best-effort validation)

If `cleaned_body` is available, update the GitHub issue body via `gh issue edit --body`.

### Required Refinement Output Format
Update the refinement system prompt in `src/daemon/refine.rs` so the model outputs exactly this shape:

```text
TITLE: <concise title, max 80 chars>
---
<structured task description for coding agent>
=== CLEANED BODY ===
<cleaned issue body only>
```

Requirements for prompt instructions:
- The cleaned-body section is a light editorial pass of the original issue body only.
- Preserve intent, scope, and structure (headers, bullets, code blocks).
- Fix typos/grammar/readability only; do not add new scope or merge with task description.
- The cleaned-body section must contain only issue body text, never the title.
- Delimiters must be exact and in the order shown above.

### Data Model Changes
In `src/daemon/refine.rs`, extend:

```rust
pub struct RefinedPrompt {
    pub title: Option<String>,
    pub body: String,
    pub cleaned_body: Option<String>,
}
```

### Parser Behavior (`parse_refined_output`)
Implement line-based parsing with strict delimiter matching:
- Use standalone line matching: `line.trim() == "=== CLEANED BODY ==="`.
- Do not split on substring/mid-line occurrences.
- Parsing flow:
1. Parse optional `TITLE:` line as today.
2. Find `---` separator as today.
3. In the remainder, find first standalone cleaned-body delimiter line.
4. If found:
- Lines before delimiter => structured `body`
- Lines after delimiter => `cleaned_body` candidate
5. If not found:
- Entire remainder => `body`
- `cleaned_body = None`

If multiple standalone cleaned-body delimiter lines exist, split only on the first; treat the rest as content.

### Validation Rules
- `body`: keep existing strict validation (`validate_output` behavior unchanged). If invalid, refinement fails as today.
- `cleaned_body`: best-effort validation only.
- If missing, empty, whitespace-only, or too short (`MIN_OUTPUT_LENGTH`), set `cleaned_body = None`.
- Never fail refinement because of cleaned-body issues.
- Optional debug logging is allowed for dropped cleaned-body content.

### GitHub Integration
In `src/daemon/github.rs`, add:

```rust
pub fn update_issue_body(owner: &str, repo: &str, issue_number: u32, body: &str) -> Result<()>
```

Implementation pattern must mirror `update_issue_title`, but call:

```bash
gh issue edit <number> --repo <owner/repo> --body <body>
```

### Runtime Integration
In `src/daemon/runtime.rs`:
- Destructure `cleaned_body` from refinement result.
- After existing best-effort title update, perform best-effort body update when `cleaned_body.is_some()`.
- Body update failure must log warning and continue (no dispatch failure).
- Existing flows must remain unchanged:
- `ralph auto --idea` receives structured `body` only
- `refined-prompt` comment uses title + structured `body` only
- `cleaned_body` is not persisted to task state

### Backward Compatibility
Two-section refinement outputs (without cleaned-body delimiter) must continue to work unchanged:
- `cleaned_body = None`
- No `gh issue edit --body` call
- Normal dispatch continues

### Acceptance Criteria
- [ ] Refinement prompt defines three-section output and explicitly states cleaned body must exclude title.
- [ ] `RefinedPrompt` includes `cleaned_body: Option<String>`.
- [ ] Parser uses line-level delimiter matching for `=== CLEANED BODY ===`.
- [ ] `body` remains strict-validated; invalid still fails refinement.
- [ ] `cleaned_body` validation is best-effort; invalid/missing yields `None` without failing refinement.
- [ ] `github.rs` exposes `update_issue_body(...)` using `gh issue edit --body`.
- [ ] `runtime.rs` performs best-effort body update after refinement.
- [ ] Existing comment and `--idea` paths continue using structured `body` only.
- [ ] Missing cleaned-body section skips issue-body update.
- [ ] Unit and conformance coverage added for new behavior and fallback paths.

### Required Tests

Unit tests in `src/daemon/refine.rs`:
- `parse_refined_output_three_section_success`
- `parse_refined_output_no_cleaned_body_fallback`
- `parse_refined_output_empty_cleaned_body_degraded`
- `parse_refined_output_cleaned_body_preserves_structure`
- `parse_refined_output_delimiter_in_content_not_split`
- Add one explicit multi-delimiter test: first standalone delimiter is the split point.

Conformance tests in `src/validate/tests_daemon.rs`:
- `daemon::refinement_cleaned_body_dispatched`
- `daemon::refinement_no_cleaned_body_skips_edit`
- `daemon::refinement_cleaned_body_failure_non_blocking`

Conformance assertions must verify:
- `--body` is called only when valid cleaned body exists.
- `--idea` always uses structured body.
- `refined-prompt` comment excludes cleaned body.
- Failure of `--body` edit is non-blocking (use stable substring match for warning, not exact full-line match).

### Files To Modify
- `src/daemon/refine.rs`
- `src/daemon/github.rs`
- `src/daemon/runtime.rs`
- `src/validate/tests_daemon.rs`

No new files required.

### Out of Scope
- Persisting `cleaned_body` in daemon task storage
- PR body updates
- New config toggles for cleaned-body behavior
- Retry logic for `gh issue edit --body`
- Human approval/diff preview before writing issue body