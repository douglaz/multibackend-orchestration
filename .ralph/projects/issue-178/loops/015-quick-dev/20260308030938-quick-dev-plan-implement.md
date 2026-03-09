---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T03:09:38Z
---

The implementation is already fully complete. All changes from the specification have been committed to the branch. Let me do a final systematic verification against acceptance criteria:

# Implementation Review — Soft Rollback (`ralph rollback`)

All acceptance criteria are satisfied by the existing committed code. Here's the verification:

## `src/cli/rollback.rs`

1. **`hard_ref` gated on `args.hard`** — Line 54: `let hard_ref = if args.hard { ... } else { None };` — soft rollback skips all git operations.

2. **Soft rollback writes `.rollback-ceiling`** — Line 230: `fs::write(&ceiling_path, args.loop_number.to_string())?;` in the `else` (no `hard_ref`) branch.

3. **Hard rollback deletes marker only on successful push** — Lines 204-226: `PushOutcome::Succeeded` deletes the file; `Failed`/`Skipped` retain it with a warning message.

4. **Push failure as warning, not early return** — Lines 134-142: `if let Err(e) = run_git(...)` sets `push_outcome = PushOutcome::Failed` and `eprintln!`s a warning. Artifact cleanup (lines 151-163) and session invalidation (lines 170-186) always run afterward.

5. **Dry-run distinguishes soft vs hard** — Lines 73-86: "dry-run (hard rollback)" with git ref vs "dry-run (soft rollback)" with no git reset.

6. **`rollback 0`** — Lines 188-200 set `current_loop = args.loop_number` (0→stored, but reconstruct defaults to 1/planning), `current_phase = Phase::Planning`, and `status = Pending` when loops/attempts are empty.

## `src/project/lifecycle.rs`

7. **`read_rollback_ceiling` helper** — Lines 402-407: reads and parses the marker file.

8. **Ceiling enforcement in `reconstruct_project_state_internal`** — Lines 285-311: reads ceiling, checks staleness (`checkpoint_loop > ceiling && max_artifact_loop <= ceiling`), caps checkpoint-derived position, filters `commit_by_loop` and `loop_dirs`.

9. **Staleness check** — Line 292: marker is inert when artifacts have advanced past the ceiling, preventing stale markers from capping forward progress.

## `src/validate/tests_commands.rs`

10. **`rollback_with_completion_attempts`** — Uses `rollback --hard 1` and `rollback --hard --dry-run 1` (lines 506, 519).

11. **`rollback_force_push`** — Uses `rollback --hard 1` and `rollback --hard --dry-run 1` (lines 573, 591).

12. **`rollback_removes_loops`** — Uses `rollback 1` without `--hard`, tests soft rollback artifact removal (line 286).

13. **`rollback_resets_phase`** — Uses `rollback 0` without `--hard`, asserts `current_loop == 1`, `current_phase == planning` (lines 331, 346-347).

14. **`rollback_dry_run`** — Tests both soft (no "git reset --hard" in output, mentions "soft rollback") and hard (`--hard --dry-run` contains "git reset --hard" and "hard rollback") (lines 394-467).

15. **`rollback_push_failure_continues`** (new) — Removes origin remote, runs `rollback --hard 1`, asserts exit 0, stderr contains "force-push failed", loop-2 artifacts removed, `.rollback-ceiling` retained, sessions invalidated (lines 1192-1263).

## `src/validate/tests_sessions.rs`

16. **`session_invalidation_on_rollback`** — No changes needed, already tests soft rollback (calls `rollback 0` without `--hard`, line 266).

No further code changes are required — the implementation is complete and matches the specification.
