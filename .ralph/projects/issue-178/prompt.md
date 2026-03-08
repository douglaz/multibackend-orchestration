## Summary

`ralph rollback <loop>` unconditionally executes `git reset --hard` and `git push --force` regardless of the `--hard` flag. The `args.hard` field in `RollbackArgs` is dead code. This change introduces a **soft rollback** (default) that removes artifacts and invalidates sessions without touching git, and gates the destructive git operations behind `--hard`. A rollback marker file ensures soft rollbacks survive `reconstruct_project_state`. Additionally, the hard rollback path is reordered so push failures do not skip artifact cleanup or session invalidation.

## Acceptance Criteria

- `ralph rollback <loop>` without `--hard` performs soft rollback: removes loop artifact directories, invalidates sessions, but does not `git reset --hard` or `git push --force`.
- `ralph rollback <loop> --hard` performs the full destructive rollback (git reset + force push + artifacts + sessions).
- `hard_ref` is only computed when `args.hard` is `true`.
- Soft rollback writes a `.rollback-ceiling` marker file to `{project_dir}/` that `reconstruct_project_state` respects, capping checkpoint-derived position to the rollback target.
- The `.rollback-ceiling` marker is ignored when the current checkpoint position is at or below the ceiling and all artifact loop numbers are at or below the ceiling — this prevents stale markers from capping forward progress after subsequent successful runs advance past the ceiling.
- Hard rollback deletes the `.rollback-ceiling` marker only when the force-push succeeds. If the force-push fails, the marker is retained to protect against checkpoint resurrection from the remote.
- `rollback 0` sets `current_loop = 1` and `current_phase = planning` (matching `derive_position` defaults and the no-checkpoint baseline), removes all loop artifacts, and clears all sessions.
- Push failures in hard rollback do not prevent artifact cleanup or session invalidation; the push error is surfaced as a warning.
- Existing `rollback_with_completion_attempts` and `rollback_force_push` validate tests pass — updated to use `--hard` flag where they assert git-destructive behavior.
- Existing `session_invalidation_on_rollback` validate test passes (adapted for soft-rollback behavior).
- Dry-run output distinguishes soft vs. hard rollback.

## Technical Approach

### 1. Gate `hard_ref` on `args.hard` (`src/cli/rollback.rs`)

Move the entire `hard_ref` computation block (lines 54–69) inside an `if args.hard { ... } else { None }` guard. This eliminates the unconditional call to `resolve_hard_reset_ref` and the downstream `reset_hard` / force-push path for soft rollbacks.

The git operation block (lines 86–118) already checks `if let Some(reference) = hard_ref.as_deref()`, so gating the computation is sufficient — all git operations become unreachable when `args.hard` is false.

### 2. Rollback marker for soft rollback (`src/cli/rollback.rs` + `src/project/lifecycle.rs`)

**Problem:** Without a git reset, checkpoint commits remain on the branch. `derive_position` (called by `reconstruct_project_state_internal` at line 262) reads the most recent checkpoint commit and would restore the pre-rollback position on the next reconstruction.

**Solution:** Write a sentinel file `{project_dir}/.rollback-ceiling` containing the target loop number (as a decimal string, e.g. `"1"`) when performing a soft rollback. This file lives inside `.ralph/projects/<id>/`, which is gitignored and unaffected by `git reset --hard`.

In `reconstruct_project_state_internal` (lifecycle.rs), after the checkpoint resolution block (lines 260–267):

1. Read `.rollback-ceiling` from `project_dir`. If absent, skip.
2. Parse the ceiling value as `u32`.
3. **Staleness check:** Determine whether the marker is still relevant. The marker is *stale* when both conditions hold: (a) `checkpoint_loop <= ceiling`, and (b) all loop artifact directories have `loop_number <= ceiling`. When stale, skip all ceiling enforcement — the project has advanced past the rollback point and the marker is inert. (It is not deleted here; reconstruction is read-only. Deletion is deferred to the orchestrator as a follow-up.)
4. If the marker is not stale and `checkpoint_loop > ceiling`:
   - Filter `checkpoint_commits` to retain only entries where `loop_number <= ceiling`.
   - Re-derive position from the filtered list: use the first (most recent) remaining commit's `(loop_number, phase)`, or default to `(1, Phase::Planning)` if the list is empty (e.g. ceiling == 0).
5. Filter the `commit_by_loop` map (lines 269–276) to `loop_number <= ceiling`.

This approach requires no changes to `derive_position` itself and confines the ceiling logic to a single post-processing step.

**Rollback target 0 behavior:** When the ceiling is `0`, step 4 filters out all checkpoint commits and defaults to `(1, Phase::Planning)`. All loop artifacts are removed by the rollback itself, so `loops` and `completion_attempts` will be empty. The resulting state matches the no-checkpoint baseline: `current_loop = 1`, `current_phase = planning`, `status = pending`.

**Marker lifecycle:**
- **Soft rollback** writes the marker (or overwrites an existing one).
- **Hard rollback** deletes the marker only when the force-push succeeds. If the force-push fails, the marker is retained — this prevents `resolve_checkpoint_ref` from reading stale remote checkpoint commits and resurrecting the pre-rollback position on the next reconstruction.
- **Staleness:** The marker is inert (ignored during reconstruction) once the project has naturally advanced past the ceiling value, as described in step 3 above. This prevents a stale marker from artificially capping `current_loop`/`current_phase` after later successful runs.

### 3. Push failure safety (`src/cli/rollback.rs`)

Reorder the hard rollback path (lines 86–118) so that artifact cleanup and session invalidation always execute. Replace the `?` on `run_git(["push", "--force", ...])` with a match that captures the error and prints a warning via `eprintln!`. Track the push result in a `push_failed: bool` variable used to:
1. Gate `.rollback-ceiling` deletion (only delete on successful push).
2. Include a push-failure note in the final output message.

Use the existing `run_git` function (returns `Result<String>`) rather than switching to `run_git_status`, since `run_git`'s error message already includes stderr.

Revised hard rollback order:
1. `checkout_branch` + `reset_hard` — destructive, already applied.
2. `restore_workspace_files` — re-create `.ralph/` files nuked by reset.
3. `run_git(["push", "--force", ...])` — capture error, print `eprintln!("warning: ...")`, set `push_failed = true`, do **not** early return.
4. (Fall through to shared cleanup below.)

Artifact removal (lines 121–133) and session invalidation (lines 135–156) remain outside the `if let Some(reference) = hard_ref` block — they already execute unconditionally, which is correct for both soft and hard rollback.

### 4. Dry-run message updates

Update the dry-run output (lines 71–84) to reflect soft vs. hard:
- When `args.hard` is true: print the git reset ref and mention force-push (current behavior, requires computing `hard_ref` in dry-run mode only when `args.hard` is true).
- When `args.hard` is false: print that a soft rollback will be performed (artifact removal + session invalidation only, no git reset ref).

### 5. Output message updates

Update the final println (lines 172–182) to distinguish soft vs. hard completion, and include a push-failure note when `push_failed` is true (e.g. `"warning: force-push failed; .rollback-ceiling marker retained"`).

## Files & Modules

| File | Change |
|---|---|
| `src/cli/rollback.rs` | Gate `hard_ref` on `args.hard`; write `.rollback-ceiling` marker on soft rollback; conditionally delete marker on hard rollback (only when push succeeds); handle push failure as warning with `push_failed` tracking; update dry-run and output messages. |
| `src/project/lifecycle.rs` | In `reconstruct_project_state_internal`, read `.rollback-ceiling` and — when not stale — cap checkpoint-derived position + filter commits when ceiling is present. Add a helper `fn read_rollback_ceiling(project_dir: &Path) -> Option<u32>`. Add staleness check based on checkpoint position and artifact loop numbers. |
| `src/validate/tests_commands.rs` | Update `rollback_with_completion_attempts`: change `rollback 1` (without `--hard`) to `rollback --hard 1` since it asserts git HEAD movement and reset-target matching. Update its `--dry-run` call to `rollback --hard --dry-run 1` so it continues to parse `git reset --hard` from dry-run output. Update `rollback_force_push`: change `rollback 1` to `rollback --hard 1` since it asserts force-push behavior. Update its `--dry-run` call similarly. |
| `src/validate/tests_sessions.rs` | `session_invalidation_on_rollback` — this test calls `rollback 0` without `--hard` and only checks artifact/session state (no git assertions), so it already tests soft rollback correctly. No changes needed. |

## Testing Strategy

**Unit tests (`src/project/lifecycle.rs`):**
- `reconstruct_respects_rollback_ceiling`: Create a project dir with loop artifacts for loops 1–3, write `.rollback-ceiling` with value `1`, call `reconstruct_project_state_internal` (without git context), verify `current_loop == 1` and loops 2/3 are excluded from state.
- `reconstruct_ignores_absent_ceiling`: No `.rollback-ceiling` file — behavior unchanged.
- `reconstruct_ceiling_below_checkpoint`: Set up git context with checkpoint at loop 3, write ceiling `1`, verify position is capped to `<= 1`. (Requires test git repo setup, use patterns from `ralph_commit.rs` tests.)
- `reconstruct_stale_ceiling_ignored`: Create a project dir with loop artifacts only for loop 1, write `.rollback-ceiling` with value `1`, set up git context with checkpoint at loop 1. Verify the marker has no effect (staleness check triggers). Then advance: add loop 2 artifacts with checkpoint at loop 2, verify marker still ignored since `checkpoint_loop > ceiling` is true but the marker *is* enforced in that case — confirming the staleness check requires *both* conditions.
- `reconstruct_ceiling_zero`: Write ceiling `0` with no loop artifacts, verify state defaults to `current_loop = 1`, `current_phase = planning`, `status = pending`.

**Validate tests (`src/validate/tests_commands.rs`):**
- `rollback_with_completion_attempts`: Updated to use `rollback --hard 1` (and `rollback --hard --dry-run 1`). Continues to assert git HEAD movement, reset-target matching, and artifact removal.
- `rollback_force_push`: Updated to use `rollback --hard 1` (and `rollback --hard --dry-run 1`). Continues to assert local/remote HEAD matching after force-push.
- `rollback_removes_loops`: No change needed — calls `rollback 1` without `--hard` and only asserts artifact/state changes, which soft rollback provides.
- `rollback_resets_phase`: No change needed — calls `rollback 0` without `--hard` and only asserts loop/phase state, which soft rollback provides. Verify `current_loop == 1`, `current_phase == planning` (matching the `rollback 0` acceptance criterion).
- `rollback_dry_run`: Update to verify soft dry-run output (should NOT contain `git reset --hard`). Add a separate assertion or test case for `rollback --hard --dry-run 1` that verifies the git reset ref is printed.
- **New: `rollback_push_failure_continues`**: Set up a scenario where `git push --force` fails (e.g., remove the `origin` remote or make it unreachable), execute `rollback --hard 1`, and assert: command exits successfully (exit code 0), stderr contains a push-failure warning, loop artifacts for loop 2 are removed, sessions are invalidated, and `.rollback-ceiling` marker is present (retained due to push failure).

**Validate tests (`src/validate/tests_sessions.rs`):**
- `session_invalidation_on_rollback`: No changes needed — already tests soft rollback (calls `rollback 0` without `--hard`, asserts sessions cleared and loops empty).

**Unit tests (`src/cli/rollback.rs` or inline):**
- Verify `.rollback-ceiling` file is written on soft rollback with correct content (the target loop number as decimal string).
- Verify `.rollback-ceiling` file is deleted on hard rollback when push succeeds.
- Verify `.rollback-ceiling` file is retained on hard rollback when push fails.

## Out of Scope

- Automatic cleanup of stale `.rollback-ceiling` by the orchestrator after a successful run advances past the ceiling. The staleness check in `reconstruct_project_state_internal` makes stale markers inert during reconstruction; physical file cleanup can be added in a follow-up.
- Changes to `derive_position` in `src/git/ralph_commit.rs`. The ceiling logic is applied post-hoc in `reconstruct_project_state_internal`.
- Persisting `SessionStore` to disk. Session invalidation operates on the in-memory state; persistence is a separate concern.
- Rollback behavior for quick-dev projects or `state.json` interactions.
- Updating `ralph rollback --help` text (handled automatically by `clap` from `RollbackArgs` doc attributes, if any).