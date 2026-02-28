The spec has been updated. Here's how each review issue was addressed:

**Issue 1 (Requirements Alignment — project metadata):** Added `extract_project_ref(branch)` helper that derives the project ID from the branch name when the orchestrator switches to `ralph/{project_id}`. This is included in the PR body footer. The Out of Scope section now explicitly justifies why `state.json` isn't read (no existing daemon→project dependency) while explaining how the project reference requirement is still met.

**Issue 2 (Technical Feasibility — `edit_pr` targeting):** Changed `edit_pr` signature from `(owner, repo, branch, ...)` to `(pr_url, title, body_file)`. The PR URL returned by `find_existing_pr` is used as the positional argument to `gh pr edit`, eliminating branch-based ambiguity. Acceptance Criterion 6 makes this explicit.

**Issue 3 (Edge Cases — title truncation):** Added precise truncation algorithm: newlines/carriage-returns replaced with spaces, whitespace trimmed, then truncated to 77 chars + `...` if over 80 Unicode scalar values (`.chars().count()`). Added `build_pr_title_sanitizes_newlines` unit test. Acceptance Criterion 1 specifies this precisely.

**Issue 4 (Reliability — body size limits):** Added `--body-file` approach via new `create_pr_with_body_file` and updated `edit_pr` to use `--body-file`. Diff stat capped at 100 lines, issue body capped at 4,000 chars. Acceptance Criterion 5 codifies these limits. Body is written to a `tempfile::NamedTempFile` before passing to `gh`.

**Issue 5 (Edge Cases — uncommitted vs committed diff mismatch):** Added explicit documentation in Technical Approach §1 explaining that `diff_stat` only reflects committed changes while `has_diff` catches uncommitted ones. Defines the behavior: fallback message is shown, PR still created. This is the expected outcome since `ralph auto` should commit before exiting.

**Issue 6 (Testing Coverage — non-happy paths):** Added four new test cases:
- `runtime_pr_edit_failure_no_duplicate_create` — edit fails, no fallthrough to create
- `runtime_pr_diff_stat_failure_fallback` — diff_stat fails, body uses fallback
- `build_pr_title_truncates_long_title` — asserts `chars().count() <= 80`
- `build_pr_body_no_context_legacy_task` — legacy task with all `None` fields

**Issue 7 (Clarity — mod.rs vs runtime.rs):** Added an explicit note in the Summary and a dedicated row in Files & Modules stating `mod.rs` is **not** changed, with explanation that PR logic lives in `runtime.rs`.