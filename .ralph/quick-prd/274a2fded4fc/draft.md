Now I have a comprehensive understanding of the codebase. Let me write the engineering spec.

## Summary

Enhance the daemon's PR creation flow (`handle_pr_flow` in `src/daemon/runtime.rs`) to generate descriptive PR titles and rich PR bodies instead of the current minimal output. Currently, PR titles fall through to the issue title or a generic `ralph: {task_id}` string, and bodies contain only `Automated PR for task \`{task_id}\`.\n\nCloses #{issue_number}`. The feature adds pure helper functions to build a title from available task metadata (refined title > original issue title > fallback) and a body that includes a change summary from `git diff --stat`, rationale from the issue body, and a project/task reference — all while degrading gracefully when context is unavailable.

## Acceptance Criteria

1. **PR title is descriptive and capped at 80 characters** — uses `refined_title` if available, falls back to the original issue title extracted from `raw_idea`, and only uses `ralph: {task_id}` as a last resort. Truncated with `...` if over 80 chars.
2. **PR body includes a change summary** — a `## Changes` section populated from `git diff --stat` output against the base branch, showing which files were modified.
3. **PR body includes rationale/context** — a `## Context` section containing the issue body (extracted from `raw_idea`) so reviewers understand *why* the changes were made.
4. **PR body references the project/task** — a footer section with `Task: \`{task_id}\`` and `Closes #{issue_number}` for traceability.
5. **Existing PR edit is best-effort** — when an existing PR is found for the branch, attempt `gh pr edit` to update its title/body with the new metadata; log a warning and continue if this fails.
6. **All context gathering failures degrade gracefully** — if `git diff --stat` fails, the Changes section shows a fallback message; if `raw_idea` is absent, the Context section is omitted. The PR is always created.
7. **No regression** — existing daemon PR creation, reuse, and failure-terminal flows continue to work.

## Technical Approach

### 1. Add `diff_stat` helper to `github.rs`

Add a new function `pub fn diff_stat(worktree_path: &Path) -> Result<String>` that runs `git diff --stat {base}...HEAD` in the worktree (reusing the existing `detect_base_branch` logic) and returns the stat output as a string. This provides the file-level change summary without pulling in full diffs.

### 2. Add `edit_pr` helper to `github.rs`

Add `pub fn edit_pr(owner: &str, repo: &str, branch: &str, title: &str, body: &str) -> Result<()>` that runs `gh pr edit` with `--title` and `--body` flags on the PR matching the given head branch. This is used when an existing PR is found.

### 3. Add pure builder functions to `runtime.rs`

Three pure, testable helpers:

- **`build_pr_title(task: &DaemonTask) -> String`** — returns `refined_title` if present, else extracts the first line of `raw_idea` (the original issue title), else `ralph: {task_id}`. Truncates to 80 chars with `...` suffix.

- **`extract_issue_body(raw_idea: &str) -> Option<String>`** — splits on `\n\n`, returns the remainder after the title line (the issue body portion), or `None` if empty.

- **`build_pr_body(task: &DaemonTask, diff_stat: Option<&str>) -> String`** — assembles the markdown body:
  ```
  ## Changes
  ```
  {diff_stat or "No diff summary available."}
  ```

  ## Context
  {issue_body or omitted}

  ---
  Task: `{task_id}` | Closes #{issue_number}
  ```

### 4. Update `handle_pr_flow` in `runtime.rs`

Reorder and update the existing flow:

1. Check diff and push (unchanged).
2. **New:** Gather `diff_stat` from the worktree (best-effort, pass `None` on failure).
3. **New:** Call `build_pr_title` and `build_pr_body`.
4. Check for existing PR — if found, **new:** call `edit_pr` best-effort to update title/body, persist URL, return.
5. Create new PR using the built title and body (replacing the current inline construction).

### 5. Data flow

All inputs come from the existing `DaemonTask` struct fields (`refined_title`, `raw_idea`, `task_id`, `issue_number`) and the worktree filesystem (for `diff_stat`). No new fields or state changes are needed.

## Files & Modules

| File | Changes |
|---|---|
| `src/daemon/github.rs` | Add `diff_stat()` and `edit_pr()` functions |
| `src/daemon/runtime.rs` | Add `build_pr_title()`, `extract_issue_body()`, `build_pr_body()` pure helpers; update `handle_pr_flow()` to use them |
| `src/validate/tests_daemon.rs` | Add conformance tests for the new PR metadata flow |
| `src/validate/mock_scripts.rs` | Update mock gh scripts to handle `pr edit` and `diff --stat` if needed by tests |

## Testing Strategy

### Unit tests (in `runtime.rs::tests`)

- **`build_pr_title_uses_refined_title`** — verifies `refined_title` takes precedence.
- **`build_pr_title_uses_original_issue_title`** — verifies extraction from `raw_idea` when no `refined_title`.
- **`build_pr_title_fallback`** — verifies `ralph: {task_id}` when both are absent.
- **`build_pr_title_truncates_long_title`** — verifies 80-char truncation with `...`.
- **`extract_issue_body_with_body`** — verifies body extraction from `raw_idea`.
- **`extract_issue_body_without_body`** — verifies `None` when `raw_idea` has no body.
- **`build_pr_body_with_all_context`** — verifies full body with diff stat and issue context.
- **`build_pr_body_missing_diff_stat`** — verifies fallback when diff stat is `None`.
- **`build_pr_body_missing_issue_body`** — verifies Context section omitted when no issue body.

### Conformance tests (in `tests_daemon.rs`)

- **`daemon::runtime_pr_metadata_title_and_body`** — end-to-end test using mock `gh` that captures `--title` and `--body` args passed to `pr create`, verifying the title is descriptive and the body contains Changes/Context sections and closes reference.
- **`daemon::runtime_pr_edit_existing`** — test that when an existing PR is found, `gh pr edit` is called with updated title/body.

### Unit tests (in `github.rs::tests`)

- **`diff_stat` parse tests** are not needed since it delegates to `git diff --stat` directly; the conformance tests cover integration.

## Out of Scope

- **LLM-generated PR summaries** — no backend calls for PR description generation; all metadata is derived from existing task fields and git state.
- **Project-level state (`state.json` / `.ralph/projects/`)** — the daemon does not currently read project state; this feature uses only `DaemonTask` fields. Reading project state (e.g., PRD content) can be added in a future iteration.
- **PR labels, reviewers, or assignees** — only title and body are updated.
- **Updating PR on re-push** — `edit_pr` is only called during the completion flow when an existing PR is found, not on subsequent pushes.
- **Customizable PR templates** — the body format is hardcoded; template configurability is deferred.