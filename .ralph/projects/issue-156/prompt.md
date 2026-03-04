## Summary

Add a cleanup step between the implementation and review phases that detects and removes stray `*-impl-notes*.md` and `*-impl-response*.md` files from the worktree root. These files are duplicates left behind by implementer backends (the canonical copies are already saved to `.ralph/projects/<id>/loops/<NNN>/`). In issue #146, reviewers repeatedly flagged these stray files, causing a 7+ hour infinite loop.

## Acceptance Criteria

- [ ] After the implementation phase completes, any `*-impl-notes*.md` or `*-impl-response*.md` files at the worktree root are removed from both the working tree and the git index
- [ ] Removal occurs before the review phase (or QA phase) so reviewers never see the stray files
- [ ] Works for both the regular orchestrator (`orchestrator.rs`) and the quick-dev orchestrator (`quick_dev_orchestrator.rs`), covering all implementing→reviewing/QA transition points
- [ ] No false positives: only files matching exact canonical forms (`YYYYMMDDHHMMSS-impl-notes.md` and `YYYYMMDDHHMMSS-impl-response-NNN.md` where NNN is exactly 3 digits) are removed
- [ ] Each removed file is logged at `info` level
- [ ] Both tracked and untracked stray files are handled correctly

## Technical Approach

**1. Add `remove_stray_impl_artifacts(workdir: &Path) -> Result<()>` and `is_stray_impl_artifact(file_name: &str) -> bool` in `src/git/commit.rs`**

`is_stray_impl_artifact` validates filenames against exact canonical forms:

```rust
fn is_stray_impl_artifact(file_name: &str) -> bool {
    let Some(_ts) = parse_artifact_filename_timestamp(file_name) else {
        return false;
    };
    // Skip past "YYYYMMDDHHMMSS-"
    let suffix = &file_name[ARTIFACT_TIMESTAMP_LEN + 1..];
    if suffix == "impl-notes.md" {
        return true;
    }
    // Exact match: "impl-response-NNN.md" where NNN is exactly 3 ASCII digits
    if let Some(rest) = suffix.strip_prefix("impl-response-") {
        if let Some(seq) = rest.strip_suffix(".md") {
            return seq.len() == 3 && seq.chars().all(|c| c.is_ascii_digit());
        }
    }
    false
}
```

This reuses `parse_artifact_filename_timestamp` (`artifacts.rs:241-248`) which validates a 14-digit numeric prefix, and enforces the exact 3-digit sequence suffix for `impl-response` files — rejecting non-canonical names like `impl-response-draft.md`.

`remove_stray_impl_artifacts` handles both tracked and untracked files:

```rust
pub fn remove_stray_impl_artifacts(workdir: &Path) -> Result<()> {
    let entries = match fs::read_dir(workdir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else { continue };
        if !is_stray_impl_artifact(name) {
            continue;
        }
        info!("removing stray impl artifact: {name}");
        // Try git rm first (handles tracked files — removes from index + working tree).
        let git_rm_result = run_git(workdir, &["rm", "--force", "--ignore-unmatch", "--", name]);
        if git_rm_result.is_err() {
            // Fallback: delete from filesystem directly (untracked or index-only edge cases).
            let _ = fs::remove_file(entry.path());
        }
        // If the file was untracked and only staged (by a prior `git add -A`),
        // git rm --force removes it from both index and disk. If it was never
        // staged, git rm --ignore-unmatch is a no-op and the fs::remove_file
        // fallback handles it.  Either way the file is gone before commit.
    }
    Ok(())
}
```

Key design choices:
- **`git rm --force`** (not plain `git rm`): `--force` is required because after `git add -A` the file is staged but has no HEAD entry, so git considers it "to be added" — plain `git rm` refuses to remove such files. `--force` overrides this safety check.
- **`--ignore-unmatch`**: Prevents errors when a file is untracked and not in the index.
- **Filesystem fallback**: Covers the edge case where `git rm` returns an error (e.g., corrupt index) — the file is still removed from disk so it won't be re-staged.

**2. Place cleanup inside the two shared phase-transition helpers, after `git add -A`**

This is the key architectural decision. Instead of calling `remove_stray_impl_artifacts` from each orchestrator call site (which risks missing paths), place it inside the two shared helpers that ALL transition paths funnel through:

**a) `stage_implementation_changes` (`commit.rs:256`) — used by the regular orchestrator**

All three regular orchestrator transition points (`orchestrator.rs` lines ~888, ~1057, ~1212) call `stage_changes_for_review` → `stage_implementation_changes`. Insert the cleanup after `git add -A` and before `unstage_non_commit_artifacts`:

```rust
pub fn stage_implementation_changes(workdir: &Path) -> Result<()> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["add", "-A"])?;
    remove_stray_impl_artifacts(workdir)?;          // NEW
    unstage_non_commit_artifacts(workdir);
    Ok(())
}
```

Placing cleanup after `git add -A` ensures that previously-untracked stray files are now in the index, so `git rm --force` can remove them from both the index and the working tree.

**b) `commit_and_push_phase_transition` (`commit.rs:193`) — used by the quick-dev orchestrator**

Both quick-dev transition points (`quick_dev_orchestrator.rs` lines ~347, ~603) call `persist_destination_and_checkpoint` → `checkpoint_if_enabled` → `commit_and_push_phase_transition`. Insert the cleanup after `git add -A` and before the commit:

```rust
pub fn commit_and_push_phase_transition(/* ... */) -> Result<()> {
    ensure_git_repo(repo_root)?;
    if has_conflicts(repo_root)? { /* ... */ }
    run_git(repo_root, &["add", "-A"])?;
    remove_stray_impl_artifacts(repo_root)?;        // NEW
    let message = build_ralph_commit_message(/* ... */);
    // ... commit and push
}
```

**Why this placement covers all paths:**

| Orchestrator | Transition | Call chain | Cleanup location |
|---|---|---|---|
| Regular | Implementing → QA/Reviewing (~888) | `stage_changes_for_review` → `stage_implementation_changes` | Inside `stage_implementation_changes` |
| Regular | QA iteration (~1057) | `stage_changes_for_review` → `stage_implementation_changes` | Inside `stage_implementation_changes` |
| Regular | Reviewing iteration (~1212) | `stage_changes_for_review` → `stage_implementation_changes` | Inside `stage_implementation_changes` |
| Quick-dev | PlanAndImplement → CodexReview (~347) | `persist_destination_and_checkpoint` → `checkpoint_if_enabled` → `commit_and_push_phase_transition` | Inside `commit_and_push_phase_transition` |
| Quick-dev | ApplyFixes → CodexReview (~603) | `persist_destination_and_checkpoint` → `checkpoint_if_enabled` → `commit_and_push_phase_transition` | Inside `commit_and_push_phase_transition` |

Any future transition path that uses either helper automatically inherits the cleanup.

**3. The `git rm --force` after `git add -A` removes files from both index and working tree**, so the deletions are included in the subsequent commit. No separate commit or amend is needed.

## Files & Modules

| File | Change |
|---|---|
| `src/git/commit.rs` | Add `pub fn remove_stray_impl_artifacts(workdir: &Path) -> Result<()>` and `fn is_stray_impl_artifact(file_name: &str) -> bool`; insert `remove_stray_impl_artifacts` call inside `stage_implementation_changes` (after `git add -A`, before `unstage_non_commit_artifacts`) and inside `commit_and_push_phase_transition` (after `git add -A`, before commit) |
| `src/project/artifacts.rs` | No change needed (`parse_artifact_filename_timestamp` and `ARTIFACT_TIMESTAMP_LEN` are already `pub`) |
| `src/workflow/orchestrator.rs` | No change needed (cleanup is inherited via `stage_implementation_changes`) |
| `src/workflow/quick_dev_orchestrator.rs` | No change needed (cleanup is inherited via `commit_and_push_phase_transition`) |
| `src/validate/tests_stray_cleanup.rs` | New: validate conformance tests for stray artifact cleanup |
| `src/validate/mod.rs` | Register `tests_stray_cleanup` module |

## Testing Strategy

**Unit tests for `is_stray_impl_artifact` (in `src/git/commit.rs` `#[cfg(test)]` module):**

| Case | Input | Expected |
|---|---|---|
| Canonical impl-notes | `20260304123456-impl-notes.md` | `true` |
| Canonical impl-response | `20260304123456-impl-response-001.md` | `true` |
| impl-response sequence 999 | `20260304123456-impl-response-999.md` | `true` |
| No timestamp prefix | `impl-notes.md` | `false` |
| Non-canonical suffix (draft) | `20260304123456-impl-response-draft.md` | `false` |
| Non-canonical suffix (4 digits) | `20260304123456-impl-response-0001.md` | `false` |
| Non-canonical suffix (extra text) | `20260304123456-impl-notes-custom.txt` | `false` |
| Review artifact | `20260304123456-review-001-feedback.md` | `false` |
| Unrelated file | `README.md` | `false` |
| Spec file | `SPEC.md` | `false` |
| Short timestamp (13 digits) | `2026030412345-impl-notes.md` | `false` |

**Integration test for `remove_stray_impl_artifacts` (in `src/git/commit.rs` `#[cfg(test)]` module):**

- Create a temp git repo with an initial commit
- Add a mix of: (a) tracked canonical stray files, (b) untracked canonical stray files, (c) decoy files (`README.md`, `impl-notes.md` without timestamp, `20260304123456-review-001-feedback.md`)
- Run `git add -A` then `remove_stray_impl_artifacts`
- Verify: all canonical stray files are removed from both working tree and index; all decoy files remain in both working tree and index

**Validate conformance tests (new module `src/validate/tests_stray_cleanup.rs`):**

1. **`stray_cleanup::tracked_impl_artifacts_removed`** — Full orchestration loop (mock implementer writes stray impl-notes and impl-response files to worktree root). Verify after the implementing→reviewing transition: stray files are absent from the committed tree, and the transition commit does not contain them.

2. **`stray_cleanup::untracked_impl_artifacts_removed`** — Same as above but the mock implementer creates the stray files without staging them (simulating the common case where backends write files without `git add`). Verify cleanup still works.

3. **`stray_cleanup::user_files_preserved`** — Mock implementer creates both stray artifacts and a non-matching `impl-notes.md` (no timestamp) and `my-notes.md` in the worktree root. Verify only the timestamped artifacts are removed; user files survive.

4. **`stray_cleanup::multi_iteration_cleanup`** — Run two implementing→reviewing iterations (reviewer requests changes, implementer re-implements). Both iterations produce stray files. Verify cleanup occurs at each transition, not just the first.

5. **`stray_cleanup::quick_dev_cleanup`** — Run a quick-dev orchestration loop. Verify stray files are cleaned up at the PlanAndImplement→CodexReview transition.

All conformance tests use the `RalphHarness` framework with mock backends, following the pattern in existing modules like `tests_pr_lifecycle.rs`.

**Manual validation:** Run a full orchestration loop with a real backend and verify no stray `*-impl-notes*.md` or `*-impl-response*.md` files remain in the worktree root after the implementation phase.

## Out of Scope

- Cleaning up other artifact types (e.g., `review-feedback`, `qa-pass`) — only `impl-notes` and `impl-response` are known to cause the infinite loop issue
- Retroactive cleanup of stray files from past runs
- Changes to how backends generate these files in the first place (that would be a backend-level fix, orthogonal to this orchestrator-level safeguard)
- Cleanup of files in subdirectories (only worktree root is affected)
- Guard-railing non-implementing transitions (e.g., reviewing→completing) — stray impl artifacts are only produced during implementing phases