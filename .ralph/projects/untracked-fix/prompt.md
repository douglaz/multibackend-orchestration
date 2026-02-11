# Fix: Untracked Files Break Review Diff and Rollback Cleanup

## Objective

Fix two related issues in the implement-review loop:

1. New files created by implementer are missing from reviewer diff context.
2. Rollback leaves implementer-created files behind, which blocks the next loop with dirty-tree validation.

Both issues come from the same root cause: we only run `git add -A` at commit time (`commit_feature_loop()`), so untracked files are invisible to `git diff HEAD` during review and survive rollback unless explicitly cleaned.

## Current Failure Modes

### Failure A: Reviewer misses newly created files

- Reviewer prompt diff is built from `current_git_diff()` -> `working_tree_diff_excluding_orchestration_state()` -> `git diff HEAD`.
- `git diff HEAD` does not include untracked files.
- Result: reviewer can see edits like `mod foo;` but not the new `foo.rs`, causing false negatives and repeated review cycles.

### Failure B: Rollback does not restore clean tree

- `rollback_current_loop()` currently updates state and removes loop artifacts under `.ralph/projects/.../loops/...`.
- It does not reset git working tree or clean untracked files outside `.ralph/`.
- Result: implementer-created files remain and `ensure_clean_start_for_new_loop()` correctly blocks the next loop.

## Implementation Plan

### 1) Stage implementer changes before each review phase

Add a helper in `src/git/commit.rs`:

```rust
/// Stage all non-orchestration changes so reviewer diff (`git diff HEAD`)
/// includes newly created files.
pub fn stage_implementation_changes(workdir: &Path) -> Result<()> {
    ensure_git_repo(workdir)?;
    run_git(
        workdir,
        &["add", "-A", "--", ".", ":(exclude).ralph/**"],
    )?;
    Ok(())
}
```

Why this form:

- Avoids staging `.ralph/**` in the first place.
- Avoids `reset HEAD -- .ralph` edge cases.
- Makes behavior deterministic for reviewer diff generation.

Call this helper in `src/workflow/orchestrator.rs` immediately before each transition to `Phase::Reviewing`:

- after initial implementer notes are written
- after each implementer response to review feedback

Guard it the same way existing git operations are guarded:

- resolve repo root from `self.workspace.root.parent()`
- run only when `is_git_repo(repo_root)` is true

### 2) Clean git tree during rollback, while preserving `.ralph/**`

Add helper in `src/git/commit.rs`:

```rust
/// Undo non-orchestration working-tree/index changes and remove non-orchestration
/// untracked files. Preserve `.ralph/**`.
pub fn reset_and_clean_working_tree(workdir: &Path) -> Result<()> {
    ensure_git_repo(workdir)?;

    if ref_exists(workdir, "HEAD")? {
        // Restore tracked files outside `.ralph/**`.
        run_git(
            workdir,
            &["checkout", "HEAD", "--", ".", ":(exclude).ralph/**"],
        )?;
        // Unstage non-orchestration entries.
        let _ = run_git(
            workdir,
            &["reset", "HEAD", "--", ".", ":(exclude).ralph/**"],
        );
    } else {
        // Unborn branch: clear index entries if any.
        let _ = run_git(workdir, &["reset"]);
    }

    // Remove untracked files/dirs outside orchestration state.
    run_git(workdir, &["clean", "-fd", "--exclude", ".ralph"])?;
    Ok(())
}
```

Important: do not use `reset --hard` here, because that can clobber `.ralph/**` tracked state.

Update `rollback_current_loop()` in `src/workflow/orchestrator.rs`:

- signature: `fn rollback_current_loop(state: &mut ProjectState, project_dir: &Path, workspace_root: &Path) -> Result<()>`
- near the top (after `!state.has_in_progress_loop()` guard), call `reset_and_clean_working_tree(repo_root)?` when inside a git repo
- keep existing artifact-dir deletion and state rewinding behavior

Update both call sites to pass `workspace_root`:

- review iteration limit handling in `run()`
- `PromptChangeAction::RestartLoop` path in `handle_prompt_change()`

Add imports for `stage_implementation_changes` and `reset_and_clean_working_tree`.

## Files to Modify

| File | Required changes |
|---|---|
| `src/git/commit.rs` | Add `stage_implementation_changes()` and `reset_and_clean_working_tree()` |
| `src/workflow/orchestrator.rs` | Invoke staging before both `Phase::Reviewing` transitions; update rollback signature/callers; run git cleanup during rollback |
| `tests/git.rs` | Add focused tests for new git helpers |
| `tests/orchestrator.rs` | Extend rollback integration coverage |

## Tests

### A) `tests/git.rs`: staging includes new files in reviewer diff input

Add a test that:

1. Creates a repo with initial commit
2. Writes a new non-`.ralph` file and a `.ralph` file
3. Calls `stage_implementation_changes()`
4. Asserts `working_tree_diff_excluding_orchestration_state()` contains the new non-`.ralph` file
5. Asserts staged diff does not include `.ralph` paths

### B) `tests/git.rs`: rollback helper removes non-`.ralph` changes only

Add a test that:

1. Modifies tracked file(s), creates untracked file(s), and creates `.ralph` runtime file(s)
2. Calls `reset_and_clean_working_tree()`
3. Verifies non-`.ralph` tracked/untracked changes are gone
4. Verifies `.ralph` runtime file remains
5. Verifies `changed_paths_excluding_prefixes(..., [".ralph/"])` is empty

### C) `tests/orchestrator.rs`: review-limit rollback removes implementer-created file

Extend `review_iteration_limit_rollback` scenario so implementer writes a new file (for example `new_module.rs`) before returning implementation notes/response. After rollback:

1. `new_module.rs` no longer exists
2. Project state still rewinds to planning as today
3. A subsequent run is not blocked by dirty-tree validation outside `.ralph/`

## Non-Goals

- Do not change `commit_feature_loop()`
- Do not change `ensure_clean_start_for_new_loop()`
- Do not add config fields, CLI flags, or schema changes
- Do not change prompt template content
- Do not add dependencies

## Acceptance Criteria

1. Newly created implementation files are visible in reviewer diff context.
2. Rollback removes non-`.ralph` implementation changes (including untracked files).
3. `.ralph/**` orchestration state is preserved by rollback cleanup.
4. `--until-complete` can continue past review-limit rollbacks without manual git cleanup.
5. `nix build` and relevant tests pass.
