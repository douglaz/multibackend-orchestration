# TODO

### Bug: QA phase fails with "No such file or directory" — artifact name mismatch

**Status:** Open
**Observed in:** `ralph run --project fix-stage-skip --until-complete` (loop 1, QA iteration 1)
**Error:** `io error: No such file or directory (os error 2)`

**Root cause:** Artifact naming mismatch between what QA writes and what the review/QA phase reads back.

- When the implementer responds to QA failure, it writes `ArtifactKind::ImplQaResponse { iteration }` → file named `impl-qa-response-NNN.md` (`src/workflow/orchestrator.rs:469-572`, `src/project/artifacts.rs` `ArtifactKind::file_name()`)
- When the QA/review phase looks for the previous implementer response, `response_rel_path()` (`src/workflow/orchestrator.rs:2092-2106`) searches for suffix `impl-response-NNN.md`
- `resolve_artifact_path_by_suffix()` (`src/project/artifacts.rs:150-202`) scans the loop directory but the suffix doesn't match → returns `Ok(None)` → propagates as file-not-found error

**Separate issue (same run):** On the initial implementation pass, Codex produced an `impl-notes` artifact but no `impl-response` artifact. The QA phase expected to read the impl-response for the iteration and failed. This suggests the QA phase should handle the case where the first implementation pass only produces `impl-notes` (which is the normal initial artifact) and no `impl-response` (which is only produced in response to QA feedback).

**Possible fixes:**
1. `response_rel_path()` should also search for `impl-qa-response-NNN.md` suffix as a fallback
2. Or: the QA phase entry should not require an `impl-response` artifact on the first iteration — it should use `impl-notes` instead
3. Or: unify `ImplResponse` and `ImplQaResponse` into a single artifact kind

## Resolved

### Bug: Rollback leaves untracked files, blocking `--until-complete`
**Fixed in:** `ralph/untracked-fix` branch (merged)
**Fix:** `reset_and_clean_working_tree()` in `src/git/commit.rs` runs `git clean -fd --exclude .ralph` during rollback.

### Bug: Reviewer sees incomplete diff — untracked files invisible during review
**Fixed in:** `ralph/untracked-fix` branch (merged)
**Fix:** `stage_implementation_changes()` runs `git add -A` after implement phase, before transitioning to review.
