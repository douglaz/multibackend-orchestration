---
artifact: quick-dev-plan-implement
loop: 15
project: issue-156
backend: claude
role: implementer
created_at: 2026-03-04T16:31:18Z
---

# Stray Impl Artifact Cleanup Implementation

## Changes Made

### `src/git/commit.rs` — Core cleanup logic + integration points

**New functions:**
- `is_stray_impl_artifact(file_name: &str) -> bool` — Validates filenames against exact canonical forms (`YYYYMMDDHHMMSS-impl-notes.md` and `YYYYMMDDHHMMSS-impl-response-NNN.md`). Reuses `parse_artifact_filename_timestamp` from `artifacts.rs` and enforces exact 3-digit sequence suffix.
- `remove_stray_impl_artifacts(workdir: &Path) -> Result<()>` — Scans worktree root, removes matching files via `git rm --force --ignore-unmatch` (handles staged files after `git add -A`) with filesystem fallback for edge cases.

**Integration points (after `git add -A`, before commit/unstage):**
- `stage_implementation_changes()` (line ~301) — serves all regular orchestrator transitions
- `commit_and_push_phase_transition()` (line ~218) — serves all quick-dev orchestrator transitions

### `src/validate/mock_scripts.rs` — New mock script

- `quick_dev_implementer_with_stray_files_script()` — Variant of the standard quick-dev implementer that creates stray `20260304120000-impl-notes.md` and `20260304120000-impl-response-001.md` files at the worktree root during PlanAndImplement, and different stray files during ApplyFixes.

### `src/validate/tests_stray_cleanup.rs` — New conformance test module

Three conformance tests:
1. **`stray_cleanup::quick_dev_cleanup`** — Verifies stray files are removed after PlanAndImplement→CodexReview transition
2. **`stray_cleanup::user_files_preserved`** — Verifies non-matching files (no timestamp prefix, different names) survive cleanup
3. **`stray_cleanup::multi_iteration_cleanup`** — Two iterations with reviewer rejection; verifies cleanup at each transition and that stray files never appear in any commit

### `src/validate/mod.rs` — Module registration

Registered `tests_stray_cleanup` module and added to `register_tests()`.

## Test Results

- **12 new unit tests** for `is_stray_impl_artifact` (all canonical/non-canonical patterns) — all pass
- **1 integration test** for `remove_stray_impl_artifacts` (tracked + untracked files, decoys) — passes
- **All 959 existing tests** continue to pass
- **Clean build** with no warnings
