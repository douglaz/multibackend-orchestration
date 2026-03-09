---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T05:02:05Z
---

All tests pass. Here's the summary of the change:

# Fix: resolve hard reset ref after branch recovery

**Problem:** In `rollback.rs`, `resolve_hard_reset_ref` was called at line 55 *before* branch recovery (line 105). When the local project branch and tracking ref were missing, `find_checkpoint_commit` couldn't find commits on the non-existent branch and fell back to the merge-base, causing over-rollback.

**Fix in `src/cli/rollback.rs`:** Restructured the execute function into two distinct paths:

1. **Dry-run path** (lines 53-86): Resolves `hard_ref` early for display purposes (read-only, no branch mutations). This is acceptable since dry-run doesn't need branch recovery.

2. **Non-dry-run hard path** (lines 96-175): Performs operations in the correct order:
   - Branch recovery/creation from remote
   - `checkout_branch`
   - `resolve_hard_reset_ref` (now sees checkpoint commits on the recovered branch)
   - `reset_hard`
   - `restore_workspace_files`
   - Force-push

The `hard_ref` is now an `Option<String>` produced by the entire hard-rollback block, with the resolved reference available for the marker management and output message sections that follow.
