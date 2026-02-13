# TODO

## Open

### Acceptance QA: short-circuit on first failure
The acceptance QA loop runs both backends sequentially even when the first one fails. Since all must pass, a failure from the first backend means the verdict is already "continue" — the second QA call wastes money. Change the `for` loop at `src/workflow/orchestrator.rs:~1363` to break after the first FAIL, record only that failure, and proceed to the "continue" verdict immediately.

## Resolved

### Bug: QA phase fails with "No such file or directory" — artifact name mismatch
**Fixed in:** `master` branch
**Fix:** Two changes:
1. `render_template_with_fallback()` falls back to embedded default templates when workspace template files are missing (e.g. `templates/qa.md` absent in older workspaces).
2. `response_rel_path()` searches for both `impl-response-NNN.md` and `impl-qa-response-NNN.md` suffixes.

### Bug: Rollback leaves untracked files, blocking `--until-complete`
**Fixed in:** `ralph/untracked-fix` branch (merged)
**Fix:** `reset_and_clean_working_tree()` in `src/git/commit.rs` runs `git clean -fd --exclude .ralph` during rollback.

### Bug: Reviewer sees incomplete diff — untracked files invisible during review
**Fixed in:** `ralph/untracked-fix` branch (merged)
**Fix:** `stage_implementation_changes()` runs `git add -A` after implement phase, before transitioning to review.
