---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 2
project: issue-156
backend: claude
role: implementer
created_at: 2026-03-04T16:46:09Z
---

# Apply-Fixes: Stray Impl Artifact Cleanup

## Changes Made

### Fix 1 (Critical): Untracked file cleanup in `remove_stray_impl_artifacts` (`src/git/commit.rs:310-325`)

The root cause: `git rm --force --ignore-unmatch` exits 0 for untracked files without actually removing them, and the fallback `fs::remove_file` only ran on `Err`. Fixed by:

1. **Always checking if the file still exists after `git rm`** and deleting it via `fs::remove_file` if so — handles untracked files regardless of when cleanup is called relative to `git add -A`.
2. **Running `git rm --cached --ignore-unmatch`** afterward to ensure the file is also removed from the index if it was staged by a prior `git add -A`.

### Fix 2 (Low): Strengthened unit test (`src/git/commit.rs:862`)

The test `remove_stray_impl_artifacts_cleans_tracked_and_untracked` now:
- Commits stray files first with `git add -A && git commit`, making them truly tracked from HEAD
- Verifies they appear in `git ls-files` before cleanup
- After cleanup, verifies removal from both disk and index using `git ls-files` assertions instead of the weak `git status --porcelain` check

### Test Results

- All 11 `is_stray_impl_artifact` unit tests: **pass**
- `remove_stray_impl_artifacts_cleans_tracked_and_untracked` unit test: **pass**
- All 5 `stray_cleanup::` validate conformance tests: **pass**
  - `stray_cleanup::quick_dev_cleanup`
  - `stray_cleanup::user_files_preserved`
  - `stray_cleanup::multi_iteration_cleanup`
  - `stray_cleanup::regular_implementing_to_reviewing`
  - `stray_cleanup::regular_user_files_preserved`
