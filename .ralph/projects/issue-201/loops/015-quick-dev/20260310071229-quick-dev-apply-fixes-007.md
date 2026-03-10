---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 7
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T07:12:29Z
---

Build succeeds (exit 0). All three reviewer-requested changes are already correctly applied in the working tree:

# Applied Reviewer Fixes

## 1. High: Fixed broken conformance tests
- **`setup_project_branch`** and **`setup_project_branch_with_stale_counters`** now commit only `.ralph/projects/{project_id}/` instead of `.ralph/` wholesale, preventing workspace config files (e.g. `ralph.toml`) from being removed when switching back to master
- Added `git push origin` to push the branch so `sync_project_branch` doesn't force-reset it
- All git `Command` calls now check `status.success()` with descriptive assert messages
- Removed duplicate `write_dispatch_failure_mock_gh`; dispatch failure test now uses a worktree blocker file

## 2. Medium: Atomic dedup state persistence
- `PrReviewState::load()` now returns `Result<Self>` — parse failures return `Err` instead of silently defaulting to empty state (which would cause duplicate re-enqueue)
- `PrReviewState::save()` now writes to a `.json.tmp` temp file then atomically renames, preventing crash-induced truncation/corruption
- Callers updated to handle the `Result` (`poll_pr_reviews` skips the task on error; tests use `.expect()`)

## 3. Low: Case-insensitive login filtering
- Self-comment filtering uses `comment.author.eq_ignore_ascii_case(&self_login)` instead of `==`
- Whitelist filtering uses `w.eq_ignore_ascii_case(&comment.author)` instead of `w == &comment.author`
- Unit tests updated with mixed-case authors (`"Alice"`, `"BOB"`, `"Ralph-Bot"`) to verify case-insensitive matching
