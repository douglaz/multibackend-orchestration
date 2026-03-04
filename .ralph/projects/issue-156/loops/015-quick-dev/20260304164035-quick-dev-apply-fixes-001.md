---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-156
backend: claude
role: implementer
created_at: 2026-03-04T16:40:35Z
---

# Apply-Fixes: Reviewer-Requested Changes

## Changes Made

### Fix #1 (High): Stray cleanup now runs even when commits are disabled

**File:** `src/workflow/quick_dev_orchestrator.rs`

The `checkpoint_if_enabled` function previously returned early at line 1132 when `!auto_commit || skip_commit`, skipping `commit_and_push_phase_transition` (and its `remove_stray_impl_artifacts` call). 

**Fix:** Restructured `checkpoint_if_enabled` to:
1. Resolve `repo_root` and validate it's a git repo **before** the early return
2. Call `remove_stray_impl_artifacts(repo_root)` on any `from_phase == Phase::Implementing` transition **before** the commit-skip early return
3. Added `remove_stray_impl_artifacts` to the import from `crate::git::commit`

This ensures stray files are cleaned up on implementing→reviewing transitions regardless of `--skip-commit` / `auto_commit=false`.

### Fix #2 (Medium): Added regular orchestrator conformance tests

**File:** `src/validate/tests_stray_cleanup.rs`

Added two new conformance tests:

- **`stray_cleanup::regular_implementing_to_reviewing`** — Runs a full regular orchestrator loop (`ralph run --loops 1`) with a mock backend that creates stray impl artifacts. Verifies stray files are removed from both the worktree and all commits.

- **`stray_cleanup::regular_user_files_preserved`** — Runs a regular orchestrator loop with a mock that creates both stray artifacts and non-matching user files (`impl-notes.md` without timestamp, `my-notes.md`). Verifies only timestamped artifacts are removed; user files survive.

**File:** `src/validate/mock_scripts.rs`

Added `standard_mock_with_stray_files_script()` — a variant of `standard_mock_script()` that creates stray `20260304120000-impl-notes.md` and `20260304120000-impl-response-001.md` files during implementation, used by the new regular orchestrator conformance tests.

### Verification

- `cargo check` passes cleanly
- All 12 existing unit tests (`is_stray_impl_artifact_*` + `remove_stray_impl_artifacts_cleans_tracked_and_untracked`) pass
