---
artifact: prompt-review
project: summary-the-daemon-fails-to-re-dispatch
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-14T20:14:19Z
---

# Prompt Review

## Issues Found
- The provided prompt is a change-summary, not an implementation prompt; it lacks a clear objective, scope, and deliverables for an execution loop.
- Error-handling behavior is under-specified (what is fatal vs non-fatal for `worktree remove`, `prune`, and `branch -D` failures), which can lead to inconsistent implementations.
- The restart-flow outcome says “terminal state” but does not define whether any terminal state is acceptable or a specific one is required.
- Test instructions are directionally clear but not fully operationalized (setup details, assertions, and required negative/edge cases are not all explicit).
- The prompt does not explicitly require where code/tests should be updated, which increases ambiguity for downstream implementers.
- Observability expectations (which stderr signal/message confirms redispatch) are implied but not clearly specified as a contract.

## Refined Prompt
### Title
Make worktree/branch cleanup deterministic during daemon restart reconciliation

### Objective
Fix restart reconciliation so stale local branches are always cleaned up when removing task worktrees, including when the worktree path is already missing. Ensure redispatch on restart is reliable and covered by stable tests.

### In Scope
- Update `remove_worktree()` cleanup behavior and ordering.
- Use machine-parseable branch existence checks.
- Ensure restart reconcile flow handles stale `in_progress` tasks with stale branches.
- Update/add tests for retry flow and branch cleanup assertions.

### Out of Scope
- Do not add a separate orphan-branch sweep in `reconcile_worktrees()`.
- Do not add assertions based on physical worktree directory existence in tests.

### Required Behavior
1. `remove_worktree()` must execute cleanup in this order:
1. If worktree path exists: `git worktree remove --force <path>`
2. Always: `git worktree prune`
3. Always check branch existence with `git show-ref --verify refs/heads/<branch>`
4. If branch exists: `git branch -D <branch>` (must happen after prune)
2. If worktree path does not exist, skip only step 1. Steps 2 and 3/4 must still run.
3. Branch detection must not use `git branch --list`.
4. Branch cleanup is mandatory behavior, not optional/recommended.
5. `reconcile_worktrees()` should rely on `remove_worktree()` for branch cleanup and must not implement a separate orphan-branch cleanup pass.

### Error Handling
- “Branch not found” from `show-ref --verify` is non-fatal and means skip delete.
- Failures in `worktree remove`, `worktree prune`, or `branch -D` (other than missing branch) must return contextual errors (include command + target path/branch).
- Do not silently ignore command failures.

### Testing Requirements
1. Keep/update existing restart retry test to assert branch cleanup via `git show-ref --verify` exit code.
2. Add `runtime_restart_in_progress_with_stale_branch`:
1. Seed an `in_progress` task with fake PID/PGID.
2. Create a pre-existing local branch for that task.
3. Start daemon and run restart reconciliation.
4. Assert reconcile -> redispatch -> terminal state flow completes.
5. Assert redispatch success using stderr signal/message already used by current tests.
6. Assert branch is gone using `git show-ref --verify refs/heads/<branch>` non-zero exit code.
3. Tests must not assert on worktree directory existence.
4. Tests must assert:
1. dispatch success signal
2. task state transition to a terminal state in `tasks.json`
3. branch ref cleanup result via `show-ref --verify`

### Acceptance Criteria
1. Cleanup order is exactly: remove-if-exists -> prune-always -> delete-branch-after-prune.
2. Missing worktree path does not skip prune or branch cleanup.
3. Branch existence checks and test assertions use `git show-ref --verify`.
4. Restarting with stale `in_progress` + stale branch successfully reaches redispatch and terminal task state.
5. No optional wording remains for branch cleanup; behavior is required.
6. No test depends on worktree directory existence.
