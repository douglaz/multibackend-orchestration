### Title
Harden retry dispatch for failed tasks with safe state transitions, legacy project backfill, and robust worktree reuse.

### Objective
Implement deterministic retry behavior for failed runtime tasks so retries are concurrency-safe, preserve project context, and do not lose project artifacts.

### In Scope
1. Retry state transition ordering and conflict handling.
2. Dispatch-time project ID backfill for legacy failed tasks.
3. Project discovery robustness (`state.json`-based validation).
4. Worktree creation/reuse hardening.
5. Tests covering all above behaviors.

### Out of Scope
1. Startup-wide migration/backfill of all historical tasks.
2. Changing branch namespace strategy.
3. Changing artifact file format or loop naming conventions.

### Functional Requirements
1. Retry transition must be claim-first.  
On re-trigger of a `Failed` task, call `claim_issue` before any state transition.  
If `claim_issue` fails, return failure and keep task state `Failed`.  
After successful claim, perform CAS transition `Failed -> Pending` with a double-check that current state is still `Failed`.  
If CAS fails, do not set `Pending`; perform best-effort claim release if available; return conflict/error.

2. Dispatch-time backfill for legacy failed tasks.  
Inside `dispatch_task()`, before child process spawn, if `task.project_id` is `None`, scan the task worktree for `.ralph/projects/<id>/state.json`.  
Treat a project as valid only when `state.json` exists.  
If exactly one valid project is found, persist `project_id` to the task store before spawn.  
If zero valid projects are found, continue with `project_id = None`.  
If multiple valid projects are found, treat as ambiguous and continue with `project_id = None` (log warning).  
If persistence of discovered `project_id` fails, abort dispatch and keep task non-pending/non-running.

3. Spawn decision must use persisted project context.  
If `project_id` is present, spawn with `ralph run --project <id>`.  
If `project_id` is absent, spawn with `ralph auto --idea ...`.

4. Project discovery robustness.  
Ignore stray directories under `.ralph/projects/` that do not contain `state.json`.  
Do not infer project validity from directory presence alone.

5. Worktree hardening.  
In `create_worktree`, run `git worktree prune` before `git worktree add`.  
When reusing an existing worktree, run `verify_worktree_branch()` to confirm expected daemon branch is checked out.  
If branch mismatches, attempt best-effort force checkout to expected daemon branch.  
If force checkout fails, fail dispatch with explicit error.

6. Artifact preservation across retries.  
Retry dispatch and `clean_worktree` must not delete `.ralph/projects/<id>/loops/` artifacts for the retried project.  
Artifacts created in prior failed attempts must remain readable after retry flow completes.

### Non-Functional Requirements
1. All state transitions must be atomic/CAS-based where applicable.
2. Add structured logs for claim failure, CAS conflict, ambiguous project discovery, and branch correction failure.
3. No destructive git operations outside target worktree/branch context.

### Required Tests
1. `discover_project_id_ignores_dirs_without_state_json`.  
Asserts stray project directories without `state.json` are ignored.

2. `runtime_retrigger_claim_failure_preserves_failed_state`.  
Asserts failed claim leaves task in `Failed`, never `Pending`.

3. `runtime_retrigger_claim_then_cas_conflict_does_not_set_pending`.  
Asserts concurrent state change after claim prevents `Pending` transition.

4. `runtime_dispatch_backfills_legacy_failed_task_project_id`.  
Asserts dispatch-time scan discovers single valid project, persists `project_id`, and uses `ralph run --project`.

5. `runtime_create_worktree_handles_stale_metadata`.  
Asserts prune-then-add recovers from stale `git worktree` metadata.

6. `runtime_reuse_worktree_corrects_branch_mismatch`.  
Asserts branch mismatch is detected and force-checkout to expected branch is attempted.

7. `runtime_retrigger_preserves_project_artifacts`.  
Asserts files under `.ralph/projects/<id>/loops/` remain after retry dispatch and cleanup.

### Acceptance Criteria
1. Re-trigger never transitions `Failed -> Pending` if claim fails.
2. Re-trigger never leaves task stranded in `Pending` due to claim/CAS ordering races.
3. Legacy failed tasks with discoverable project state are retried via project mode (`ralph run --project`).
4. Stray project directories without `state.json` do not affect discovery.
5. Stale worktree metadata does not block worktree creation.
6. Reused worktrees are on expected daemon branch or fail loudly.
7. Project loop artifacts persist across retries.
8. All listed tests pass.