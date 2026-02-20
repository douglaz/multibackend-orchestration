## Summary

The `ralph rollback <loop>` command unconditionally performs `git reset --hard` + `git push --force` regardless of the `--hard` flag, and can leave the repository in a partial state if the force-push fails after the local reset has already happened. This spec adds a functional `--hard` flag with soft rollback as the default, introduces a rollback marker file to prevent `reconstruct_project_state` from undoing non-destructive rollbacks, and reorders hard-rollback operations to prevent partial failure states.

## Acceptance Criteria

1. `ralph rollback <loop>` (without `--hard`) performs a soft rollback: removes loop artifact directories, clears sessions, and writes a rollback marker — no `git reset` or `git push --force`.
2. `ralph rollback <loop> --hard` performs a hard rollback: `git reset --hard` to the checkpoint ref, `git push --force` to sync the remote, removes artifacts, clears sessions.
3. `reconstruct_project_state` checks for the rollback marker and caps its derived position/loop-set at the marker's target loop, preventing checkpoint commits on the unchanged branch from undoing a soft rollback.
4. Hard rollback does not leave the repo in a partial state: artifact cleanup and session invalidation complete even if `git push --force` fails; the force-push is attempted last and its failure is reported but does not roll back local cleanup.
5. Dry-run output distinguishes between soft and hard rollback.
6. All existing rollback tests pass; new tests cover both flag states and the marker mechanism.

## Technical Approach

### 1. Soft rollback via marker file

**Problem:** State is not persisted — `reconstruct_project_state` re-derives position from git checkpoint commits and loop artifact directories every invocation. Without `git reset --hard`, checkpoint commits remain on the branch and the next reconstruction undoes the rollback.

**Solution:** Write a rollback marker file at `<project_dir>/.rollback-marker` containing the target loop number. `reconstruct_project_state_internal` reads this file early, and after collecting loop artifacts and checkpoint commits, discards any loops/checkpoints above the marker's target. The marker is consumed (deleted) at the start of the next successful orchestrator run, or overwritten by a subsequent rollback.

Marker format (plain text, single line):
```
<target_loop_number>
```

**In `reconstruct_project_state_internal` (lifecycle.rs:216–326):**
- After line 266 (loop artifact collection), filter `loop_dirs` to exclude entries where `loop_number > marker_loop`.
- After line 247 (checkpoint position derivation), cap `checkpoint_loop` to `min(checkpoint_loop, marker_loop)` and reset `checkpoint_phase` to `Planning` if capped.
- This ensures both artifact-based and checkpoint-based state agree with the rollback intent.

**In `rollback.rs:execute()`:**
- When `!args.hard`: skip the entire git-operations block (lines 86–119), write the marker file, then proceed to artifact cleanup and state mutation as today.
- When `args.hard`: do NOT write a marker (the git reset is the persistence mechanism); proceed with git operations as today but with reordered cleanup.

### 2. Gate git operations behind `args.hard`

**In `rollback.rs:execute()` (lines 54–119):**
- Move the `hard_ref` computation inside an `if args.hard { ... }` block so `resolve_hard_reset_ref` is only called when needed.
- The entire git-operations block (checkout, reset, restore, force-push) executes only when `args.hard` is true.
- When `!args.hard`, set `hard_ref = None` without computing it.

### 3. Reorder hard-rollback to prevent partial states

Current order creates a vulnerability: `reset --hard` (destructive, local) → restore workspace → `push --force` (can fail) → artifact cleanup → session invalidation.

If force-push fails, the local branch is already reset but the remote retains stale commits. On next `resolve_checkpoint_ref`, the local-preferred-when-not-behind logic (ralph_commit.rs:109–112) means the local reset IS respected — so this is not as catastrophic as originally described. However, artifacts may not be cleaned up if the push error propagates via `?`.

**New order for `args.hard`:**
1. Checkout project branch
2. `git reset --hard <ref>`
3. Restore workspace files (prompt.md, config.toml)
4. Artifact directory cleanup (lines 121–133) — moved before force-push
5. Session invalidation and state mutation (lines 135–170) — moved before force-push
6. `git push --force` — attempted last, failure logged as warning but does not prevent the rest of rollback from completing

This ensures artifact cleanup and session invalidation always complete. A failed force-push leaves the remote stale, but `resolve_checkpoint_ref` already prefers local when ahead/diverged, so reconstruction still sees the correct position. The warning message tells the user to manually push.

### 4. Dry-run output differentiation

Update dry-run block (lines 71–84) to show:
- Without `--hard`: "dry-run: would soft-rollback to loop N (remove loops [...], write rollback marker)"
- With `--hard`: "dry-run: would hard-rollback to loop N (remove loops [...], git reset --hard <ref>, force-push)"

### 5. Marker lifecycle

- **Written by:** `rollback.rs:execute()` when `!args.hard`
- **Read by:** `reconstruct_project_state_internal()` at reconstruction time
- **Deleted by:** The orchestrator's main loop entry (`run_loop` or equivalent) after successfully reconstructing state, to prevent the marker from interfering with normal operation. Alternatively, the marker is self-clearing: once artifact directories for loops > target are deleted and no new checkpoint commits exist above target, reconstruction naturally agrees with the marker, making it a no-op. However, explicit deletion is cleaner.
- **Overwritten by:** A subsequent `ralph rollback` to a different loop number.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/rollback.rs:16–184` | Gate `hard_ref` computation and git ops behind `args.hard`; write `.rollback-marker` for soft rollback; reorder hard-rollback so force-push is last and non-fatal; update dry-run output |
| `src/project/lifecycle.rs:216–326` | In `reconstruct_project_state_internal`, read `.rollback-marker`, cap checkpoint position and filter loop artifacts to respect marker |
| `src/project/lifecycle.rs:136–145` | No change to `reconstruct_project_state` signature — marker is internal to `_internal` |
| `src/git/ralph_commit.rs` | No changes needed — `resolve_checkpoint_ref` already prefers local when ahead/diverged, which handles the hard-rollback-without-push case correctly |
| `src/cli/mod.rs:196–205` | No changes — `RollbackArgs.hard` field already exists |
| Orchestrator entry (likely `src/orchestrator/mod.rs` or `src/cli/run.rs`) | Delete `.rollback-marker` after successful state reconstruction at start of orchestrator run |

## Testing Strategy

### Unit tests (in `src/cli/rollback.rs` or a new `src/cli/rollback_tests.rs`)

1. **Soft rollback writes marker:** Execute rollback without `--hard` on a temp workspace with loop artifacts. Assert `.rollback-marker` exists with correct content, no git reset called, artifact dirs for loops > target are deleted.
2. **Hard rollback does not write marker:** Execute rollback with `--hard`. Assert `.rollback-marker` does not exist, git reset was called.
3. **Dry-run distinguishes modes:** Assert dry-run output differs between `--hard` and non-`--hard`.

### Unit tests (in `src/project/lifecycle.rs`)

4. **Marker caps reconstruction:** Create a workspace with loop artifacts for loops 1–3 and a `.rollback-marker` containing `1`. Call `reconstruct_project_state`. Assert `state.loops` contains only loop 1, `current_loop == 1`, `current_phase == Planning`.
5. **No marker means no capping:** Same setup without marker. Assert all loops present.
6. **Marker with checkpoint commits:** Requires a git repo fixture. Create checkpoint commits for loops 1–3, write marker `2`. Assert `derive_position` returns loop 3 but reconstruction caps to loop 2.

### Integration tests (in `src/git/ralph_commit.rs` existing test module)

7. **Hard rollback + failed push still has correct local state:** Use existing git test fixtures. Perform hard rollback, simulate push failure (e.g., no remote configured). Assert local branch is reset, artifacts cleaned, sessions invalidated, error message mentions push failure.
8. **Existing `resolve_checkpoint_ref` tests still pass** (lines 577–708) — no changes to that module, but verify no regressions.

### Existing test verification

All 5 existing `resolve_checkpoint_ref` branch-preference tests (lines 578–688) must continue to pass unchanged, since `ralph_commit.rs` is not modified.

## Out of Scope

- **Rollback undo/redo:** No mechanism to reverse a soft rollback beyond running `ralph rollback` to a higher loop number.
- **Remote-only rollback:** The soft rollback does not touch git at all; there is no mode that resets only the remote.
- **Automatic force-push retry:** If the force-push fails during hard rollback, the user is warned but no retry logic is added.
- **Marker-based reconstruction for non-rollback callers:** The marker is only written by rollback; other code paths that call `reconstruct_project_state` will see the marker if present but this is by design.
- **State serialization/persistence beyond markers:** The project state remains ephemeral (reconstructed each time); no database or state file is introduced.
- **Changes to `resolve_checkpoint_ref` branch preference logic:** The existing local-preferred-when-not-behind behavior already handles the hard-rollback partial-push case correctly.
