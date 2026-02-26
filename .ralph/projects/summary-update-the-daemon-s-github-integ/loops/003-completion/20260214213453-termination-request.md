---
artifact: termination-request
loop: 3
project: summary-update-the-daemon-s-github-integ
backend: claude(opus)
role: planner
created_at: 2026-02-14T21:34:53Z
---

# Project Completion Request

## Rationale
All eight acceptance criteria from the master prompt have been fully implemented and verified across two completed loops:

1. **Title sanitization/truncation** — `build_pr_title` replaces `\n`/`\r` with spaces, trims, and truncates to 80 chars (77 + `...`). Covered by `build_pr_title_sanitizes_newlines` and `build_pr_title_truncates_long_title` unit tests.
2. **PR body with project-reference footer** — `build_pr_body` calls `extract_project_ref(branch)` and appends a footer with the project ID, or explicit fallback text when extraction fails.
3. **`edit_pr` uses PR URL positional target** — `edit_pr` in `github.rs` takes `pr_url` as a positional arg to `gh pr edit`, not branch. Conformance test `runtime_pr_edit_existing_uses_body_file` locks this.
4. **Edit failure does not trigger duplicate create** — When `find_existing_pr` returns a URL, only edit is attempted; failure logs a warning and does not fall through to create. Conformance test `runtime_pr_edit_failure_no_duplicate_create` verifies no `pr create` call occurs.
5. **Both create and edit use `--body-file` via `NamedTempFile`** — `create_pr_with_body_file` and `edit_pr` both accept a `body_file` path and pass `--body-file <path>` to `gh`. Conformance tests `runtime_pr_create_uses_body_file` and `runtime_pr_edit_existing_uses_body_file` lock this.
6. **Diff stat failure produces fallback body** — When `diff_stat()` fails or returns `None`, `build_pr_body` emits "Diff stat unavailable." and the PR flow continues. Conformance test `runtime_pr_diff_stat_fallback` verifies PR creation succeeds with fallback content.
7. **Legacy task with all `None` fields** — `build_pr_body` gracefully handles all-`None` optional fields. Unit test `build_pr_body_no_context_legacy_task` covers this.
8. **`src/daemon/mod.rs` unchanged** — Confirmed via git history; no modifications to `mod.rs`.

All required unit tests pass: `build_pr_title_sanitizes_newlines`, `build_pr_title_truncates_long_title`, `build_pr_body_no_context_legacy_task`, `extract_project_ref` success/failure cases, `runtime_pr_diff_stat_failure_fallback`, plus additional coverage for diff-stat cap, context cap, and body-file writing.

All required conformance tests pass: `runtime_pr_edit_existing_uses_body_file`, `runtime_pr_edit_failure_no_duplicate_create`, `runtime_pr_create_uses_body_file`, `runtime_pr_diff_stat_fallback`.

## Summary of Work

**Loop 1 — Pure PR metadata helpers and unit tests**
- Added `extract_project_ref`, `build_pr_title`, `build_pr_body`, `extract_issue_body` as pure functions in `src/daemon/runtime.rs`
- 11 unit tests covering sanitization, truncation, caps, legacy tasks, and project ref extraction

**Loop 2 — Deterministic PR edit/create flow with body-file safety**
- Rewired `handle_pr_flow` to use the pure helpers for title/body construction
- Added `create_pr_with_body_file` and `edit_pr` in `src/daemon/github.rs` using `--body-file`
- Enforced edit-only path when existing PR found (no create fallthrough on failure)
- Added fallback diff content on `diff_stat` failure
- Extended `src/validate/tests_daemon.rs` with 4 conformance tests
- Extended `src/validate/mock_scripts.rs` with mock `gh` support for `pr edit`, `--body-file`, and failure scenarios

## Remaining Items
- None — all requirements from the master prompt are satisfied.

---
