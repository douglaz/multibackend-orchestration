### Objective
Implement and validate daemon runtime fixes so failed tasks preserve worktrees for retry, aborted tasks continue to clean up, and fresh tasks are never misrouted to resume due to discovered workspace state.

### Problem Statement
Two runtime regressions must be fixed:
1. In `dispatch_task()` CAS-failure handling (currently around lines 546-576), worktrees can be removed even when persisted state is already terminal `Failed`.
2. Dispatch routing can incorrectly treat fresh tasks as resume tasks when discovery yields an `effective_project_id`.

### Required Product Policy
- `Failed`: preserve worktree and retain/persist `project_id` for retry.
- `Aborted`: always remove worktree (deliberate user cancellation).
- `Succeeded`: remove worktree.
- Resume behavior internal to `ralph run --project` is out of scope for this daemon task.

### Required Code Changes
1. Update `complete_task()` so `Failed` transition never removes the worktree and keeps `project_id` persisted.
2. Update `dispatch_task()` CAS-failure path to re-read persisted task state before cleanup; skip cleanup when state is `Failed`.
3. Ensure all cleanup call sites use one explicit terminal-state policy check (shared helper allowed) to prevent divergence.
4. Fix dispatch gating to use `task.project_id.as_deref()` for resume decision.
5. Do not use `effective_project_id.as_deref()` to decide resume vs fresh dispatch.
6. Enforce dispatch command routing:
- If `task.project_id` is `Some`, use `ralph run --project <id>`.
- If `task.project_id` is `None`, use `ralph auto --idea ...` even if discovery found a project.
7. Add/update logs/comments at decision points to document failed-preserve vs aborted-cleanup policy.

### Files and Modules to Touch
- Runtime module containing `dispatch_task()`.
- Runtime module containing `complete_task()`.
- Command construction/spawn path containing `spawn_ralph_run`.
- Validate conformance tests under `src/validate/` (extend existing runtime-related module or add dedicated one).

### Testing Strategy
#### Required New Conformance Tests (must be added and pass)
1. `runtime_task_fails_worktree_preserved`
- Seed pending task.
- Mock `ralph` exits non-zero.
- Assert final state `Failed`.
- Assert worktree still exists.
- Assert `project_id` retained for retry.
2. `runtime_activation_failed_task_preserved`
- Force activation CAS failure where persisted task is already terminal `Failed`.
- Assert CAS-failure handling does not remove worktree.
3. `runtime_fresh_dispatch_ignores_discovered_project`
- Discovery yields workspace project.
- Task has `project_id = None`.
- Assert dispatched command is `ralph auto --idea ...`, not `ralph run --project ...`.

#### Existing Tests That Must Continue to Pass
1. `runtime_failed_worktree_preserved_and_reused_on_retry`.
2. Existing aborted cleanup conformance test (use current repo test name).
3. Existing success cleanup conformance test (use current repo test name).
4. Existing explicit-`project_id` resume dispatch test (use current repo test name).

### Out of Scope
- Loop-level resume correctness inside `ralph run --project`.
- Any policy change for aborted-task cleanup.

### Acceptance Criteria
1. No code path removes worktree when terminal state is `Failed`.
2. Aborted-task cleanup behavior remains unchanged and enforced.
3. Fresh tasks (`task.project_id == None`) always use fresh dispatch path regardless of discovered project context.
4. Resume dispatch occurs only when `task.project_id` is present.
5. All 3 required new conformance tests pass.
6. Existing related conformance tests pass.
7. `nix develop -c cargo test` passes.
8. Validate tests for touched area pass via built binary (example: `./result/bin/ralph validate --bin ./result/bin/ralph --filter run`).