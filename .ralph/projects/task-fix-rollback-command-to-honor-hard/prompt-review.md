---
artifact: prompt-review
project: task-fix-rollback-command-to-honor-hard
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-20T19:05:01Z
---

# Prompt Review

## Issues Found
- The prompt does not define rollback loop boundaries explicitly (whether to remove loops `> target` or `>= target`), which can cause destructive off-by-one behavior.
- Push-failure behavior is underspecified for command result semantics (success vs error), making automation and retries ambiguous.
- Marker-file behavior is incomplete for malformed/out-of-range content, which risks nondeterministic `reconstruct_project_state` behavior.
- Operation ordering is discussed but not defined as a strict execution contract, so different implementations could still leave partial state.
- Dry-run expectations do not explicitly require zero side effects (no git, no filesystem writes), which can cause accidental mutations.
- Testing requirements miss mandatory conformance coverage in `src/validate/` per project guidelines.
- Marker lifecycle is only partially defined; it should be explicit when it is created, overwritten, deleted, and when it must persist after degraded hard rollback.
- Error-path guarantees for artifact/session cleanup are not formalized, so cleanup could be skipped on git failures.

## Refined Prompt
Implement a rollback correctness fix for `ralph rollback <loop>` so `--hard` is truly optional and state reconstruction remains consistent after soft rollback.

**Objective**
- Make non-`--hard` rollback a soft rollback with no git-history mutation.
- Preserve current destructive git behavior only when `--hard` is set.
- Prevent hard rollback push failures from leaving local/remote/artifact state inconsistent.
- Ensure reconstructed project state respects a rollback boundary marker.

**Definitions**
- `target_loop`: the `<loop>` CLI argument.
- `project_dir`: `.ralph/projects/<id>`.
- `marker_path`: `<project_dir>/.rollback-target`.
- Rollback artifact cleanup removes loop artifacts for loops strictly greater than `target_loop` (keep `target_loop` and below).

**Required Behavior**

1. **Soft rollback (`ralph rollback <loop>` without `--hard`)**
- Must not run `git reset --hard`, `git push --force`, or any equivalent history-rewriting git command.
- Must perform existing non-git rollback work: artifact cleanup, session invalidation, and state updates.
- Must write `marker_path` with `target_loop` as plain integer text (newline optional).

2. **Hard rollback (`ralph rollback <loop> --hard`)**
- Must preserve destructive behavior: reset local branch to target reference and force-push remote.
- Must delete `marker_path` after a fully successful hard rollback (if file exists).

3. **Hard rollback push-failure handling**
- Required order:
1. Resolve hard target reference (read-only).
2. Capture `original_head`.
3. Perform local hard reset to target reference.
4. Attempt force push.
5. If push fails: attempt local reset back to `original_head`.
6. Run artifact cleanup/session invalidation/state update regardless of push outcome.
- If push fails and revert to `original_head` succeeds:
1. Write `marker_path` with `target_loop` (degraded to soft rollback semantics).
2. Return a non-zero error indicating hard rollback failed and soft fallback was applied.
- If push fails and revert also fails:
1. Still run cleanup/state/session steps best-effort.
2. Return a non-zero error indicating repository may be inconsistent.

4. **State reconstruction marker support**
- In `reconstruct_project_state_internal` (`src/project/lifecycle.rs`), read `marker_path`.
- If marker is present and valid integer `m` and `m < checkpoint_loop`:
1. Clamp reconstructed loop to `m`.
2. Set reconstructed phase to `Phase::Planning`.
3. Ignore checkpoint commits with loop number `> m`.
- If marker absent: keep existing behavior unchanged.
- If marker is malformed: ignore marker and log/trace a warning (do not crash reconstruction).

5. **Marker lifecycle after new work**
- In orchestrator checkpoint creation (`checkpoint_phase_transition`), remove `marker_path` only after a successful new checkpoint commit.
- Do not remove marker on failed checkpoint attempts.

6. **Dry-run behavior**
- Dry-run must perform zero mutations (no git operations, no marker writes/deletes, no artifact/session/state writes).
- Dry-run output must clearly differ by mode:
- Soft: indicates soft rollback + marker write intent, explicitly “no git reset/push”.
- Hard: indicates hard reset + force push intent.

7. **Non-regression**
- `rollback_current_loop` behavior remains unchanged (artifact/in-memory only; no git reset/push changes).

**Implementation Targets**
- `src/cli/rollback.rs`
- `src/project/lifecycle.rs`
- `src/workflow/orchestrator.rs`

**Acceptance Criteria**
1. Without `--hard`, HEAD and remote history are unchanged; marker is written; artifacts/sessions/state are rolled back.
2. With `--hard` and successful push, history is reset/pushed and marker is removed.
3. With `--hard` and failed push, local reset is reverted to `original_head` when possible, cleanup still runs, marker is written, and command exits non-zero.
4. `reconstruct_project_state` honors marker boundary and does not resurrect loops beyond marker.
5. New successful checkpoint commit removes stale marker.
6. Dry-run in both modes has zero side effects and mode-specific messaging.
7. Existing rollback-related tests continue to pass.

**Testing Requirements**
- Add/adjust unit/integration tests for:
1. Soft rollback writes marker and does not mutate git history.
2. Hard rollback deletes marker on success.
3. Hard rollback push failure reverts local head and writes marker fallback.
4. Reconstruction clamping with marker; unchanged behavior without marker.
5. Dry-run mode differences and no side effects.
- Add conformance tests in `src/validate/` (required by project policy), covering at minimum:
1. Soft rollback path (`--hard` absent).
2. Hard rollback path (`--hard` present).
3. Reconstruction behavior with marker boundary.

**Out of Scope**
- Changing CLI arguments or adding new flags.
- Changing orchestrator-internal `rollback_current_loop` semantics.
- Marker schema versioning beyond plain integer content.
