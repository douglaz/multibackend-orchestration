## Summary

Enhance `handle_pr_flow` in `src/daemon/runtime.rs` to generate descriptive PR titles and rich PR bodies instead of the current minimal placeholders. The PR title uses the refined title (or falls back to the issue title / task ID), and the PR body includes a `git diff --stat` summary of changed files, the original issue context, a project reference derived from the worktree branch name, and a task/issue footer. For existing PRs that are reused, the title and body are updated via `gh pr edit` on a best-effort basis, targeting the PR by its known URL. All formatting logic lives in pure helper functions in `src/daemon/runtime.rs`, keeping the PR flow testable and side-effect-free at the formatting layer. Shell-calling helpers (`diff_stat`, `edit_pr`) live in `src/daemon/github.rs`.

Note: The feature requirements mention `src/daemon/mod.rs`, but the PR creation logic lives entirely in `src/daemon/runtime.rs` (the `handle_pr_flow` function). The `mod.rs` file defines `DaemonTask` and `TaskStore` but contains no PR logic. All implementation changes target `runtime.rs` and `github.rs`; `mod.rs` is unchanged.

## Acceptance Criteria

1. **PR titles are descriptive and capped at 80 characters** — uses `refined_title` if present, falls back to the original issue title extracted from `raw_idea`, and only uses `ralph: {task_id}` as a last resort. Titles are sanitized (newlines replaced with spaces, leading/trailing whitespace stripped) before length enforcement. Truncated to 77 characters plus `...` suffix when over 80, guaranteeing the final string is always <= 80 Unicode scalar values.
2. **PR bodies include a change summary** — a `## Changes` section populated from `git diff --stat` output against the base branch, showing which files were modified. If `diff_stat` fails or returns empty output, a fallback message (`"No file-level diff summary available."`) is shown.
3. **PR bodies include rationale/context** — a `## Context` section containing the issue body (extracted from `raw_idea`). If `raw_idea` is absent or has no body portion, the section displays `"No additional context available."`.
4. **PR bodies reference the project** — a footer line with `Task: \`{task_id}\``, the project reference (extracted from the branch name when it matches `ralph/{project_id}`), and `Closes #{issue_number}` for traceability.
5. **PR body size is bounded** — the diff stat output is capped at 100 lines (approximately 10 KB), and the issue body portion is capped at 4,000 characters. The total assembled body is written to a temporary file and passed via `--body-file` to `gh pr create` and `gh pr edit`, avoiding OS argv size limits.
6. **Existing PR edit targets the known URL** — when `find_existing_pr` returns a PR URL, `edit_pr` uses that URL directly as the positional argument to `gh pr edit` (not a branch-based lookup), ensuring deterministic targeting. Edit is best-effort: log a warning and continue on failure, and do **not** fall through to `create_pr` after a failed edit.
7. **All context gathering failures degrade gracefully** — if `git diff --stat` fails, the Changes section shows the fallback message; if `raw_idea` is absent, the Context section shows its fallback. The PR is always created.
8. **No regression** — existing daemon PR creation, reuse, no-diff, and failure-terminal flows continue to work. All existing conformance tests pass.

## Technical Approach

### 1. Add `diff_stat` helper to `src/daemon/github.rs`

```rust
pub fn diff_stat(worktree_path: &Path) -> Result<String>
```

Runs `git diff --stat {base}...HEAD` in the worktree (reusing the existing `detect_base_branch` logic). Returns the raw stat output as a `String`. On failure, returns `Err` so the caller can degrade to the fallback message.

**Edge case — uncommitted vs committed changes:** `has_diff` detects both uncommitted working-tree changes and committed divergence from base. However, `git diff --stat {base}...HEAD` only reflects *committed* changes. This means it is possible for `has_diff` to return `true` (due to uncommitted changes) while `diff_stat` returns an empty string. This is acceptable: the daemon's child process (`ralph auto`) is expected to commit its changes before exiting, so in the normal flow `diff_stat` will reflect the work done. If uncommitted changes are the sole diff source (an abnormal exit), the Changes section will show `"No file-level diff summary available."`, and the PR will still be created from whatever is pushed.

### 2. Add `edit_pr` helper to `src/daemon/github.rs`

```rust
pub fn edit_pr(pr_url: &str, title: &str, body_file: &Path) -> Result<()>
```

Runs `gh pr edit {pr_url} --title {title} --body-file {body_file}`. The `pr_url` parameter is the URL returned by `find_existing_pr`, providing a deterministic target without branch-name ambiguity. Returns `Ok(())` on success, `Err` on failure.

### 3. Update `create_pr` to accept `--body-file` in `src/daemon/github.rs`

Add a new function (or overload):

```rust
pub fn create_pr_with_body_file(
    owner: &str, repo: &str, branch: &str, title: &str, body_file: &Path
) -> Result<String>
```

Identical to the existing `create_pr` but passes `--body-file {path}` instead of `--body {string}`. The original `create_pr` function is left unchanged to avoid breaking existing callers. `handle_pr_flow` switches to the new variant.

### 4. Add pure formatting helpers to `src/daemon/runtime.rs`

Three pure functions (no I/O, fully unit-testable):

**`build_pr_title(task: &DaemonTask) -> String`**

Returns the PR title with precedence: `refined_title` → `extract_original_title(raw_idea)` → `"ralph: {task_id}"`. Before truncation:
- Replace all `\n` and `\r` with a single space.
- Trim leading/trailing whitespace.
- If the result exceeds 80 Unicode scalar values (`.chars().count()`), truncate to 77 chars and append `...`. The final string is guaranteed <= 80 chars.

**`extract_issue_body(raw_idea: &str) -> Option<String>`**

Splits on `\n\n`, returns the remainder after the title line (the issue body portion), or `None` if empty after trimming. Used internally by `build_pr_body`.

**`build_pr_body(task: &DaemonTask, diff_stat: Option<&str>, project_ref: Option<&str>) -> String`**

Assembles the markdown body. Applies truncation/capping before assembly:
- `diff_stat`: if present, cap at 100 lines (take first 100 lines, append `\n... (truncated)` if exceeded).
- Issue body (from `extract_issue_body`): if present, cap at 4,000 characters (truncate at char boundary, append `... (truncated)` if exceeded).

Output format:
```
## Changes

```
{diff_stat or "No file-level diff summary available."}
```

## Context

{issue_body or "No additional context available."}

---
Task: `{task_id}`{" | Project: `{project_ref}`" if present} | Closes #{issue_number}
```

**`extract_project_ref(branch: &Option<String>) -> Option<String>`**

If the branch matches the pattern `ralph/{project_id}` (i.e., the orchestrator switched from the daemon's `ralph/daemon/{task_id}` branch), extracts and returns the `project_id` segment. Returns `None` for `ralph/daemon/*` branches or if no branch is set. This provides the project reference required by the feature constraints without needing to read `.ralph/projects/<id>/state.json`.

### 5. Update `handle_pr_flow` in `src/daemon/runtime.rs`

Modify the existing function:

1. After confirming `has_changes` and pushing, call `diff_stat` on the worktree (best-effort; pass `None` to `build_pr_body` on failure).
2. Extract `project_ref` from `task.branch` via `extract_project_ref`.
3. Compute title via `build_pr_title(task)`.
4. Compute body via `build_pr_body(task, diff_stat, project_ref)`.
5. Write body to a temp file (using `tempfile::NamedTempFile`).
6. In the existing-PR path (where `find_existing_pr` returns `Some(url)`): call `edit_pr(url, title, body_file)` best-effort. Log warning on failure. Persist `pr_url`. Return. Do **not** fall through to create.
7. In the new-PR path: call `create_pr_with_body_file(owner, repo, branch, title, body_file)` (replacing the current inline `create_pr` call).

### 6. Data flow

All inputs come from the existing `DaemonTask` struct fields (`refined_title`, `raw_idea`, `task_id`, `issue_number`, `branch`, `owner`, `repo`) and the worktree filesystem (for `diff_stat`). The project reference is derived from the branch name. No new struct fields, no state.json reads, no new dependencies beyond what is already available (`tempfile` is already a dependency).

## Files & Modules

| File | Changes |
|---|---|
| `src/daemon/github.rs` | Add `diff_stat()`, `edit_pr()`, and `create_pr_with_body_file()` functions |
| `src/daemon/runtime.rs` | Add `build_pr_title()`, `extract_issue_body()`, `build_pr_body()`, `extract_project_ref()` pure helpers; update `handle_pr_flow()` to use them; add unit tests |
| `src/validate/tests_daemon.rs` | Add conformance tests for new PR metadata flow; update existing PR-reuse test to verify `pr edit` path |
| `src/validate/mock_scripts.rs` | Update mock `gh` scripts to handle `pr edit` and capture `--title`/`--body-file` args |

Note: `src/daemon/mod.rs` is **not** changed. The `DaemonTask` struct and `TaskStore` are unchanged.

## Testing Strategy

### Unit tests (in `runtime.rs::tests`)

Pure function tests with no shell or network access:

- **`build_pr_title_uses_refined_title`** — verifies `refined_title` takes precedence over `raw_idea`.
- **`build_pr_title_uses_original_issue_title`** — verifies extraction from `raw_idea` when no `refined_title`.
- **`build_pr_title_fallback`** — verifies `ralph: {task_id}` when both `refined_title` and `raw_idea` are absent.
- **`build_pr_title_truncates_long_title`** — verifies 80-char truncation with `...` suffix; asserts `result.chars().count() <= 80`.
- **`build_pr_title_sanitizes_newlines`** — verifies newlines in `refined_title` or `raw_idea` are replaced with spaces and do not produce multiline titles.
- **`extract_issue_body_with_body`** — verifies body extraction from `raw_idea` with `\n\n` separator.
- **`extract_issue_body_without_body`** — verifies `None` when `raw_idea` has no body portion.
- **`build_pr_body_with_all_context`** — verifies full body with diff stat, issue context, and project ref.
- **`build_pr_body_missing_diff_stat`** — verifies `"No file-level diff summary available."` fallback.
- **`build_pr_body_missing_issue_body`** — verifies `"No additional context available."` when no issue body.
- **`build_pr_body_truncates_long_diff_stat`** — verifies diff stat capped at 100 lines with `... (truncated)` suffix.
- **`build_pr_body_truncates_long_issue_body`** — verifies issue body capped at 4,000 chars.
- **`build_pr_body_no_context_legacy_task`** — verifies correct formatting when both `raw_idea` and `refined_title` are `None` (legacy task shape).
- **`extract_project_ref_from_project_branch`** — verifies `ralph/my-project-123` yields `Some("my-project-123")`.
- **`extract_project_ref_from_daemon_branch`** — verifies `ralph/daemon/owner-repo-42` yields `None`.
- **`extract_project_ref_from_none`** — verifies `None` branch yields `None`.

### Conformance tests (in `tests_daemon.rs`)

End-to-end tests using `RalphHarness` + mock scripts:

- **`daemon::runtime_pr_metadata_title_and_body`** — seeds a task with `raw_idea` and `refined_title`. Mock `gh` captures `--title` and `--body-file` args passed to `pr create` (writes them to a log file; reads and logs body-file content). Asserts:
  - Captured title matches the refined title.
  - Captured body contains `## Changes`, `## Context`, the task ID reference, and `Closes #N`.
  - Title length <= 80 characters.

- **`daemon::runtime_pr_edit_existing`** — seeds a task; mock `gh pr list` returns an existing PR URL. Asserts:
  - `gh pr edit` was called (logged to file).
  - The positional argument to `pr edit` is the exact PR URL (not a branch reference).
  - Updated title and body-file contents are correct.
  - `gh pr create` was **not** called (no create log entry).

- **`daemon::runtime_pr_edit_failure_no_duplicate_create`** — seeds a task; mock `gh pr list` returns an existing PR URL; mock `gh pr edit` exits with error. Asserts:
  - Warning was logged for the edit failure.
  - `gh pr create` was **not** called (edit failure does not fall through to create).
  - `pr_url` is still persisted from the `find_existing_pr` result.

- **`daemon::runtime_pr_diff_stat_failure_fallback`** — seeds a task where `git diff --stat` will fail (e.g., detached HEAD or missing base). Asserts:
  - PR is still created successfully.
  - Body contains `"No file-level diff summary available."`.

### Compilation and regression

- `cargo check` after all changes.
- `cargo test --lib` to run all unit tests.
- All existing conformance tests (`runtime_pr_reuse_no_diff`, `runtime_pr_create_failure_terminal`, `runtime_no_diff_pr_path`, `runtime_push_before_pr_create`, `refinement_title_in_pr`, `refinement_disabled_pr_uses_original_title`, `refinement_failure_pr_uses_original_title`, `legacy_task_without_raw_idea_pr_uses_fallback`) continue to pass.

## Out of Scope

- **Reading `.ralph/projects/<id>/state.json`** — the daemon module has no existing access to the project directory from within `handle_pr_flow`. The project reference is instead derived from the branch name (when the orchestrator switches to `ralph/{project_id}`), and the issue context comes from `raw_idea` on `DaemonTask`. This satisfies the "reference to the generating project" requirement without introducing a new dependency between the daemon and project-state modules. Full project-state integration (e.g., including PRD content) can be added in a future iteration if deeper context is needed.
- **LLM-generated PR descriptions** — the diff stat + issue context approach is deterministic and free.
- **Changing the PR title format for the refinement system prompt** — that's in `refine.rs` and works independently.
- **Modifying the `DaemonTask` struct or persisted task schema** — no new fields needed.
- **Adding `--draft` PR support or PR label/reviewer/assignee assignment.**
- **Customizable PR templates** — the body format is hardcoded; template configurability is deferred.
- **Updating PR on re-push** — `edit_pr` is only called during the completion flow when an existing PR is found, not on subsequent pushes.
