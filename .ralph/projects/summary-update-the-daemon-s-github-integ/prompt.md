# Task: Harden GitHub PR create/edit behavior in daemon runtime

Implement robust PR update/create logic in the daemon runtime so PRs are deterministic, non-duplicative, and safe for large bodies.

**Goal**
Ensure the runtime can reliably create or update a GitHub PR using `gh`, with clear title/body sanitization, body-size controls, and project reference metadata.

**Scope**
- Update PR logic in `src/daemon/runtime.rs`.
- Do not modify `src/daemon/mod.rs`.
- Keep current orchestration flow intact; only refine PR-related behavior.

**Required behavior**
1. Add `extract_project_ref(branch: &str) -> Option<String>`.
- Parse branch names of form `ralph/{project_id}`.
- Return `Some(project_id)` when matched, else `None`.
- Do not read `.ralph/projects/<id>/state.json`.

2. PR title sanitization (`build_pr_title`).
- Replace `\n` and `\r` with spaces.
- Trim surrounding whitespace.
- If length by Unicode scalar count (`.chars().count()`) is `> 80`, truncate to first 77 chars and append `...`.
- Final title must be `<= 80` chars by `.chars().count()`.

3. PR body construction (`build_pr_body`).
- Include a project reference footer derived from `extract_project_ref(branch)`.
- If extraction fails, include explicit fallback text indicating unavailable project ref plus branch value.
- Cap diff stat section to 100 lines.
- Cap issue/body context section to 4,000 characters.
- Support legacy tasks where optional context fields are all `None`.

4. Use `--body-file` for both create and edit.
- Write body to `tempfile::NamedTempFile`.
- `create_pr_with_body_file(...)` and `edit_pr(...)` must pass `--body-file <path>` to `gh`.

5. Change edit target to PR URL.
- `edit_pr` signature must be `(pr_url, title, body_file)` (plus existing context args as needed).
- Use PR URL returned by `find_existing_pr` as positional arg to `gh pr edit`.
- Do not target edit by branch.

6. Existing-PR flow and failure handling.
- If `find_existing_pr` returns a PR URL: attempt edit only.
- If edit fails, return error; do not fall through to create.
- If no existing PR: create new PR.
- If diff stat generation fails: use fallback message in body and continue.

7. Diff semantics clarity.
- Preserve documented behavior: `diff_stat` may only reflect committed changes while `has_diff` may detect uncommitted changes.
- In this mismatch case, show fallback diff summary text and still allow PR create/update.

**Acceptance criteria**
1. Title sanitization/truncation follows the exact algorithm above.
2. PR body contains a project-reference footer from branch-derived project ID.
3. `edit_pr` uses PR URL positional target (not branch).
4. Edit failure does not trigger duplicate PR creation.
5. Both create and edit use `--body-file` via `NamedTempFile`.
6. Diff stat failure produces fallback body content and does not abort PR operation.
7. Legacy task with all optional fields `None` produces valid PR body.
8. `src/daemon/mod.rs` remains unchanged.

**Tests**
Add/extend unit tests to cover:
- `runtime_pr_edit_failure_no_duplicate_create`
- `runtime_pr_diff_stat_failure_fallback`
- `build_pr_title_sanitizes_newlines`
- `build_pr_title_truncates_long_title` (assert `.chars().count() <= 80`)
- `build_pr_body_no_context_legacy_task`
- New tests for `extract_project_ref` success and non-matching branch fallback behavior

If this feature changes observable CLI/runtime behavior, add or extend conformance coverage under `src/validate/` (likely `tests_run`) to lock behavior.

**Out of scope**
- Reading project metadata from `state.json`.
- Refactoring unrelated daemon/orchestrator modules.