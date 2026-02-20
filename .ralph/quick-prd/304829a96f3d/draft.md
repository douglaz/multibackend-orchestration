Now I have a comprehensive understanding of the entire system. Let me write the specification.

## Summary

Make the `ralph rollback <loop>` command's `--hard` flag functional. Currently, the command always performs destructive git operations (`git reset --hard` + `git push --force`) regardless of the flag. Without `--hard`, rollback should perform a "soft" rollback — cleaning artifacts, updating session state, and writing a rollback marker file that `reconstruct_project_state` respects — but leaving git history untouched. With `--hard`, the existing destructive behavior is preserved. Additionally, reorder operations in the `--hard` path so that push failures cannot leave the repository in a partial state where the local branch is reset but remote/artifacts are inconsistent.

## Acceptance Criteria

1. `ralph rollback <loop>` (no `--hard`) performs soft rollback: removes loop artifacts from `<project_dir>/loops/`, invalidates sessions, updates project state, and writes a rollback marker — but does **not** run `git reset --hard` or `git push --force`.
2. `ralph rollback <loop> --hard` performs the full destructive rollback: `git reset --hard` to the target ref, `git push --force` to sync remote, artifact cleanup, and session invalidation (current behavior).
3. Soft rollback writes a marker file (e.g., `<project_dir>/.rollback-target`) containing the target loop number. `reconstruct_project_state` reads this marker and caps its derived state at that loop, preventing checkpoint commits from undoing the rollback.
4. `--hard` rollback deletes the marker file (if present) since git history is authoritative after a hard reset.
5. Push failures during `--hard` rollback do not leave the repo in a partial state: artifact cleanup and session invalidation still complete, and the local reset is reverted if the push fails.
6. Existing orchestrator-internal rollback (`rollback_current_loop`) is unaffected — it operates on in-memory state and artifacts only, with no git reset.
7. Dry-run output reflects whether `--hard` is set.
8. All existing rollback tests pass; new tests cover both soft and hard flag paths.

## Technical Approach

### 1. Gate git operations behind `args.hard` in `src/cli/rollback.rs`

The block at lines 54–119 currently always computes `hard_ref` and always executes `reset_hard` + `push --force`. Wrap the git-mutating section (lines 86–119) in `if args.hard { ... }`:

```rust
if args.hard {
    if let Some(reference) = hard_ref.as_deref() {
        // checkout, reset, restore, push (existing logic)
    }
}
```

When `args.hard` is false, skip the entire `hard_ref` computation and git operations.

### 2. Introduce a rollback marker for soft rollback

When `args.hard` is false, write a marker file at `<project_dir>/.rollback-target` containing the target loop number (as a plain integer string). This persists the rollback intent for `reconstruct_project_state` to honor.

When `args.hard` is true, delete the marker file if it exists (hard reset makes git history authoritative, so the marker is stale).

### 3. Teach `reconstruct_project_state` to respect the rollback marker

In `reconstruct_project_state_internal` (`src/project/lifecycle.rs:216`), after deriving `checkpoint_loop` and `checkpoint_phase` from git history:

- Read `<project_dir>/.rollback-target`.
- If present and the marker loop number < `checkpoint_loop`, clamp `checkpoint_loop` to the marker value and set `checkpoint_phase` to `Phase::Planning`.
- Filter `checkpoint_commits` to only those with `loop_number <= marker_loop`.

This ensures that even though stale checkpoint commits remain in git history, reconstruction respects the rollback boundary.

### 4. Harden the `--hard` path against push failures

Reorder operations so that failure at any point is recoverable:

1. Compute `hard_ref` (read-only).
2. Record current HEAD as `original_head` for rollback.
3. Perform `reset_hard`.
4. Attempt `push --force`.
5. **If push fails**: revert local branch to `original_head` via `reset_hard(repo_root, &original_head)`, then fall back to writing a soft rollback marker and continue with artifact/session cleanup (degraded but consistent).
6. If push succeeds: proceed normally.

Artifact cleanup (lines 121–133) and session invalidation (lines 140–156) run unconditionally after the git block, ensuring they complete regardless of push outcome.

### 5. Update dry-run output

Adjust the dry-run messages at lines 71–84 to reflect `--hard` vs soft:

- With `--hard`: current message ("would remove loops ... and git reset --hard ...")
- Without `--hard`: "would remove loops ... and write rollback marker (soft rollback, no git reset)"

### 6. Clear rollback marker on normal orchestration

In the orchestrator's `checkpoint_phase_transition`, delete `.rollback-target` if present when a new checkpoint commit is created. This ensures the marker doesn't persist once new work supersedes the rollback point.

## Files & Modules

| File | Change |
|------|--------|
| `src/cli/rollback.rs` | Gate git operations behind `args.hard`; write/delete `.rollback-target` marker; add push-failure recovery with `original_head` revert; update dry-run messages |
| `src/project/lifecycle.rs` | In `reconstruct_project_state_internal`, read `.rollback-target` and clamp checkpoint-derived position when marker is present |
| `src/workflow/orchestrator.rs` | In `checkpoint_phase_transition`, delete `.rollback-target` if present (marker becomes stale once new checkpoint is committed) |
| `src/cli/mod.rs` | No changes needed (`RollbackArgs.hard` already defined) |
| `src/git/ralph_commit.rs` | No changes needed (read-only functions) |

## Testing Strategy

1. **Unit test: soft rollback writes marker** — Call `execute()` with `hard: false` on a project with loops > target. Assert `.rollback-target` exists with correct content, and that no `git reset` or `push --force` was executed (verify HEAD unchanged).

2. **Unit test: hard rollback deletes marker** — Pre-create `.rollback-target`, then call `execute()` with `hard: true`. Assert marker is deleted and git reset occurred.

3. **Unit test: `reconstruct_project_state` respects marker** — Set up a project dir with checkpoint commits for loops 1–3, write `.rollback-target` containing `1`. Assert reconstructed state has `current_loop == 1` and no loops > 1.

4. **Unit test: `reconstruct_project_state` ignores absent marker** — Same setup without marker file. Assert state reflects all 3 loops (existing behavior).

5. **Unit test: push failure recovery** — Mock/simulate a push failure during `--hard` rollback. Assert local branch is reverted to `original_head` and a soft rollback marker is written as fallback.

6. **Unit test: dry-run output** — Verify dry-run messages differ for `--hard` vs soft.

7. **Integration test: orchestrator clears stale marker** — Run orchestrator after soft rollback. Assert `.rollback-target` is deleted after successful checkpoint commit.

8. **Existing tests**: `review_iteration_limit_rollback` and QA limit rollback in `tests/orchestrator.rs` must continue passing (they use `rollback_current_loop` which is unaffected).

## Out of Scope

- **Remote reachability pre-check**: Validating push viability before reset was a suggested approach but adds complexity (network probes are unreliable predictors of push success). The revert-on-failure approach is more robust.
- **Orchestrator-internal rollback changes**: The `rollback_current_loop` function in `orchestrator.rs` operates on in-memory state and artifacts only — it never does git reset or push. No changes needed.
- **Marker file format migration**: The marker is a simple integer file. No schema versioning needed for this initial implementation.
- **Multi-project marker conflicts**: Each project has its own project directory; markers are scoped by project inherently.
- **CLI UX for rollback status**: Exposing whether a soft rollback is active (e.g., in `ralph status`) is a follow-up concern.