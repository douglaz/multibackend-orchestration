# TODO

No open items.

## Resolved

### Bug: Rollback leaves untracked files, blocking `--until-complete`
**Fixed in:** `ralph/untracked-fix` branch (merged)
**Fix:** `reset_and_clean_working_tree()` in `src/git/commit.rs` runs `git clean -fd --exclude .ralph` during rollback.

### Bug: Reviewer sees incomplete diff — untracked files invisible during review
**Fixed in:** `ralph/untracked-fix` branch (merged)
**Fix:** `stage_implementation_changes()` runs `git add -A` after implement phase, before transitioning to review.
