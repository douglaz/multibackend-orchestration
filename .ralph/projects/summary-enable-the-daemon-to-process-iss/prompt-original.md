## Summary

Enable the daemon to dispatch task execution for issues that have completed the interactive PRD workflow. Currently, `poll_and_claim()` in `runtime.rs:735-744` unconditionally skips any issue carrying a PRD label via `interactive_prd::has_prd_label()`. This blocks issues with `ralph:prd-done` from ever being dispatched as tasks, even after they receive `ralph:ready`. This spec covers: (1) narrowing the PRD label guard to only skip in-progress PRD labels, with `ralph:prd-done` taking precedence over `ralph:prd-approved` in mixed-label scenarios, (2) extracting the approved draft spec from GitHub comments using the highest approved revision to avoid stale selections, (3) shared constants for draft heading and footer text to prevent format drift between generation and extraction, and (4) graceful fallback when the approved spec cannot be located.

## Acceptance Criteria

- Issues with `ralph:prd-done` + `ralph:ready` are claimed and dispatched by `poll_and_claim()`.
- Issues with in-progress PRD labels (`ralph:prd`, `ralph:prd-active`, `ralph:prd-approved`, `ralph:prd-failed`) continue to be skipped — unless `ralph:prd-done` is also present, in which case `ralph:prd-done` takes precedence and the issue is allowed through.
- Task input (`raw_idea`) for PRD-done issues is the body of the approved draft spec comment, with marker lines, heading, and footer text stripped.
- When multiple `status-approved-vN` markers exist, the highest revision number `N` is selected (not the first match).
- Only bot-authored comments are considered when scanning for markers; user-authored comments containing marker strings are ignored.
- If the approved spec cannot be found (missing `status-approved-vN` marker, missing `draft-vN` comment, or API failure), the daemon falls back to `issue.title + "\n\n" + issue.body` and emits a warning log.
- No reliance on local daemon state (`InteractivePrdState`) — everything is derived statelessly from GitHub issue comments.
- Existing PRD workflow behavior for in-progress issues is unchanged.
- Draft heading and footer strings are defined as shared constants, used by both generation (draft posting) and extraction (spec recovery), so that a wording change cannot silently break extraction.

## Technical Approach

### 1. Shared constants for draft comment format

**File:** `src/daemon/interactive_prd.rs`

Add public constants for the heading prefix, footer line, and a formatting helper:

```rust
/// Heading prefix for draft spec comments. Used by both generation and extraction.
pub const DRAFT_HEADING_PREFIX: &str = "## Draft Engineering Specification (Revision ";

/// Footer line appended to draft spec comments. Used by both generation and extraction.
pub const DRAFT_FOOTER: &str =
    "*Reply with feedback. Reply with \"approved\" or \"lgtm\" when this draft is ready.*";

/// Build a complete draft comment body from its parts.
pub fn format_draft_comment(revision: u32, spec_body: &str) -> String {
    format!(
        "{DRAFT_HEADING_PREFIX}{revision})\n\n{spec_body}\n\n{DRAFT_FOOTER}"
    )
}
```

Update the two existing draft posting sites (lines 1181-1183 and 1342-1344) to use `format_draft_comment(next_revision, &draft_spec)` and `format_draft_comment(next_revision, &revised_spec)` respectively, replacing the inline `format!` calls.

### 2. Narrow the PRD label guard in `poll_and_claim()`

**File:** `src/daemon/interactive_prd.rs`

Add a new public function `has_in_progress_prd_label()` that returns `true` only for PRD labels indicating an active/in-progress workflow, with explicit precedence logic for `ralph:prd-done`:

```rust
const PRD_IN_PROGRESS_LABEL_NAMES: &[&str] = &[
    "ralph:prd",
    "ralph:prd-active",
    "ralph:prd-approved",
    "ralph:prd-failed",
];

/// Returns `true` if the issue carries an in-progress PRD label AND does NOT
/// carry `ralph:prd-done`.  When `ralph:prd-done` is present it takes
/// precedence: the PRD workflow is complete and the issue is eligible for
/// task dispatch regardless of residual labels like `ralph:prd-approved`.
pub fn has_in_progress_prd_label(labels: &[String]) -> bool {
    let has_done = labels.iter().any(|l| l == "ralph:prd-done");
    if has_done {
        return false;
    }
    labels.iter().any(|l| PRD_IN_PROGRESS_LABEL_NAMES.contains(&l.as_str()))
}
```

This handles the mixed-label scenario (`ralph:prd-done` + `ralph:prd-approved`) correctly: `ralph:prd-approved` is never removed by the daemon during the approval transition (only `ralph:prd-active` is removed at line 1448), so label-based approvals will always leave `ralph:prd-approved` on the issue alongside `ralph:prd-done`. Without precedence logic, such issues would be permanently blocked.

**File:** `src/daemon/runtime.rs` (line 736)

Replace `interactive_prd::has_prd_label(&issue.labels)` with `interactive_prd::has_in_progress_prd_label(&issue.labels)`. Update the verbose log message:

```rust
if interactive_prd::has_in_progress_prd_label(&issue.labels) {
    if config.verbose {
        eprintln!(
            "verbose: skipping issue #{} — carries in-progress PRD label, \
             handled by interactive PRD workflow",
            issue.number
        );
    }
    continue;
}
```

### 3. Extract approved spec from GitHub comments

**File:** `src/daemon/interactive_prd.rs`

Add a new public function:

```rust
pub fn extract_approved_spec(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
) -> Option<String>
```

Algorithm:
1. Resolve the bot login via `github::fetch_authenticated_login_with_gh_bin(gh_bin)`. On failure, return `None`.
2. Fetch all issue comments via `github::fetch_issue_comments_with_gh_bin(gh_bin, owner, repo, issue_number)`. On failure, return `None`.
3. Scan bot-authored comments for `status-approved-vN` markers using the prefix `<!-- ralph:prd:{issue_number}:status-approved-v`. Extract the version number `N` from each match. **Select the highest `N`** (max), not the first match. This ensures that if multiple approval markers exist (e.g. due to re-approval after further revision), the latest approved revision is used.
4. Build the draft marker `prd_marker(issue_number, "draft", N)` and find the bot-authored comment containing it.
5. Strip marker lines using the existing `strip_prd_marker_lines()` (change visibility to `pub(crate)`).
6. Strip the heading: remove lines matching the `DRAFT_HEADING_PREFIX` constant (line starts with `## Draft Engineering Specification (Revision `).
7. Strip the footer: remove trailing lines matching the `DRAFT_FOOTER` constant.
8. Trim whitespace.
9. Return `Some(cleaned_body)` if non-empty, or `None` if the result is empty.

Steps 6-7 use the shared constants from section 1, ensuring that any future wording change to draft generation automatically propagates to extraction.

**Helper for marker version extraction:**

```rust
/// Extract all approved revision numbers from bot-authored comments.
/// Returns them in ascending order. Only considers bot-authored comments
/// to prevent user spoofing.
fn find_approved_versions(
    comments: &[github::IssueComment],
    bot_login: &str,
    issue_number: u32,
) -> Vec<u32> {
    let prefix = format!("<!-- ralph:prd:{issue_number}:status-approved-v");
    let mut versions: Vec<u32> = comments
        .iter()
        .filter(|c| c.author_login == bot_login)
        .filter_map(|c| {
            let idx = c.body.find(&prefix)?;
            let rest = &c.body[idx + prefix.len()..];
            let end = rest.find(" -->")?;
            rest[..end].parse::<u32>().ok()
        })
        .collect();
    versions.sort();
    versions.dedup();
    versions
}
```

### 4. Wire extraction into `poll_and_claim()` dispatch path

**File:** `src/daemon/runtime.rs` (lines 772-777)

After the existing claim logic succeeds and before `dispatch_task()` is called, check whether the issue carries `ralph:prd-done`:

```rust
let is_prd_done = issue.labels.iter().any(|l| l == "ralph:prd-done");

let raw_idea = if is_prd_done {
    let owner = config.owner.clone();
    let repo = config.repo.clone();
    let gh_bin = config.gh_bin.clone();
    let issue_number = issue.number;
    match spawn_blocking_op(move || {
        Ok(interactive_prd::extract_approved_spec(
            &gh_bin, &owner, &repo, issue_number,
        ))
    }).await {
        Ok(Some(spec)) => {
            eprintln!(
                "prd-done: using approved spec for issue #{} ({} chars)",
                issue.number, spec.len()
            );
            spec
        }
        _ => {
            eprintln!(
                "warning: issue #{} has ralph:prd-done but approved spec not found, \
                 falling back to issue body",
                issue.number
            );
            compose_raw_idea(&issue.title, issue.body.as_deref())
        }
    }
} else {
    compose_raw_idea(&issue.title, issue.body.as_deref())
};
```

This uses the existing `compose_raw_idea()` helper (line 1176) for the fallback path.

### 5. Footer and heading stripping detail

Draft comments have a known format (now enforced by shared constants):
```
<!-- ralph:prd:{issue}:draft-vN -->
## Draft Engineering Specification (Revision N)

{spec}

*Reply with feedback. Reply with "approved" or "lgtm" when this draft is ready.*
```

The extraction function strips in order:
1. The `<!-- ralph:prd:... -->` marker lines (via `strip_prd_marker_lines()`)
2. Lines starting with `DRAFT_HEADING_PREFIX` (`## Draft Engineering Specification (Revision `)
3. Lines matching `DRAFT_FOOTER` exactly (`*Reply with feedback...`)
4. Leading and trailing whitespace via `.trim()`

What remains is the clean spec body, which becomes the `raw_idea` input.

## Files & Modules

| File | Changes |
|---|---|
| `src/daemon/interactive_prd.rs` | Add `DRAFT_HEADING_PREFIX` and `DRAFT_FOOTER` constants, `format_draft_comment()` helper. Refactor two draft posting sites to use `format_draft_comment()`. Add `PRD_IN_PROGRESS_LABEL_NAMES` const, `has_in_progress_prd_label()` function, `find_approved_versions()` helper, `extract_approved_spec()` function. Change `strip_prd_marker_lines()` visibility from `fn` to `pub(crate) fn`. |
| `src/daemon/runtime.rs` | Replace `has_prd_label()` call with `has_in_progress_prd_label()` at line 736. Modify `raw_idea` construction (lines 772-777) to branch on `ralph:prd-done` and call `extract_approved_spec()`. |
| `src/validate/tests_interactive_prd.rs` | Add conformance tests for `has_in_progress_prd_label()` behavior, `extract_approved_spec()` parsing, and end-to-end daemon dispatch path (see Testing Strategy). |

## Testing Strategy

### Unit tests (`src/daemon/interactive_prd.rs`)

1. **`has_in_progress_prd_label` correctly filters**: Verify `ralph:prd`, `ralph:prd-active`, `ralph:prd-approved`, `ralph:prd-failed` all return `true`. Verify `ralph:prd-done` returns `false`. Verify empty labels and unrelated labels return `false`.

2. **`has_in_progress_prd_label` precedence with mixed labels**: Verify `["ralph:prd-approved", "ralph:prd-done"]` returns `false` (prd-done takes precedence). Verify `["ralph:prd-active", "ralph:prd-done"]` returns `false`. Verify `["ralph:prd-approved"]` alone returns `true`.

3. **`has_prd_label` unchanged**: Existing tests for `has_prd_label` continue to pass — `ralph:prd-done` still returns `true` (this function is used elsewhere and must not change).

4. **Approved version extraction — highest wins**: Test that given comments with both `status-approved-v1` and `status-approved-v3` markers, `find_approved_versions()` returns `[1, 3]` and the extraction selects version 3. Test single marker returns that version. Test missing markers returns empty.

5. **Approved version extraction — bot-scoped**: Test that a user-authored comment containing a `status-approved-v2` marker is ignored, while a bot-authored comment with `status-approved-v1` is selected. This validates that user-spoofed markers cannot influence revision selection.

6. **Draft body stripping uses shared constants**: Test that given a full draft comment body (marker + heading + spec + footer), the extraction returns only the clean spec. Construct the test input using `format_draft_comment()` to ensure the test fails if generation format diverges from extraction logic. Test edge cases: empty body after stripping, body with no footer, body with extra whitespace.

7. **`format_draft_comment` round-trip**: Test that `format_draft_comment(3, "body text")` produces the expected string, and that extracting from it recovers `"body text"`.

### Conformance tests (`src/validate/tests_interactive_prd.rs`)

These tests use `daemon start --single-iteration` with mocked `gh` and `RALPH_DAEMON_BIN` to exercise the full daemon dispatch path.

8. **`prd_done_issue_claimed_and_dispatched_with_approved_spec`**: Mock `gh` returns an issue with `["ralph:ready", "ralph:prd-done"]` labels. Mock `gh issue view --json comments` returns bot-authored comments containing a `draft-v2` comment (built with `format_draft_comment(2, spec_body)`) and a `status-approved-v2` marker comment. Mock `ralph` captures the `--idea` argument to a log file. Assert: (a) the issue is claimed (label log shows `ralph:ready` → `ralph:in-progress`), (b) the dispatched `--idea` argument equals the clean spec body (not the issue title+body), (c) stderr contains `"prd-done: using approved spec"`.

9. **`prd_done_issue_with_mixed_labels_not_blocked`**: Mock `gh` returns an issue with `["ralph:ready", "ralph:prd-done", "ralph:prd-approved"]` labels. Assert the issue is claimed and dispatched (not blocked by the `ralph:prd-approved` label).

10. **`prd_done_fallback_on_missing_markers`**: Mock `gh` returns a `ralph:prd-done` + `ralph:ready` issue but mock comments contain no `status-approved-vN` marker. Assert: (a) the task is still dispatched, (b) the `--idea` argument equals `title + "\n\n" + body` (the fallback), (c) stderr contains `"approved spec not found, falling back"`.

11. **`prd_done_fallback_on_api_failure`**: Mock `gh issue view --json comments` returns exit code 1 (API failure). Assert: (a) fallback to `title + "\n\n" + body`, (b) warning log emitted.

12. **`prd_done_ignores_user_spoofed_markers`**: Mock comments include a user-authored comment containing `<!-- ralph:prd:50:status-approved-v99 -->` and a bot-authored comment with `status-approved-v1` + matching `draft-v1`. Assert the dispatched `--idea` uses draft v1, not v99.

13. **`prd_done_selects_highest_approved_revision`**: Mock comments include bot-authored `status-approved-v1` and `status-approved-v3` markers, with corresponding `draft-v1` and `draft-v3` comments. Assert the dispatched `--idea` uses the draft-v3 spec body.

14. **`prd_active_labels_still_blocked`**: Existing tests `prd_ready_label_conflict_detection` and `prd_ready_conflict_in_claim_path` continue to pass, validating that in-progress PRD issues (without `ralph:prd-done`) are still skipped.

### Integration testing (manual / CI)

15. **End-to-end PRD-done dispatch**: Create an issue, run it through the PRD workflow to `ralph:prd-done`, add `ralph:ready`, verify the daemon picks it up and uses the approved spec as task input.

16. **Fallback path**: Create a `ralph:prd-done` + `ralph:ready` issue without any approval comments, verify fallback to issue body with warning log.

## Out of Scope

- Removing the `ralph:prd-done` label after task dispatch (label lifecycle after dispatch is unchanged).
- Removing the `ralph:prd-approved` label during approval transition — this label is never removed by the daemon today and that behavior is unchanged. The new `has_in_progress_prd_label()` function handles mixed labels via precedence instead.
- Modifying the PRD workflow state machine or adding new `PrdWorkflowState` variants.
- Adding new PRD label types.
- Changing how `dispatch_task()` or `refine_prompt()` processes the `raw_idea` downstream.
- Automatically adding `ralph:ready` to issues when PRD completes — this remains a manual/external step.
- Caching the bot login within `poll_and_claim()` across iterations (acceptable to call `fetch_authenticated_login` per PRD-done issue; volume is low).
- Changes to the `InteractivePrdState` struct or its persistence.