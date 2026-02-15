The updated spec has been written. Here's a summary of how each review issue was addressed:

### Review Issue 1: Retry State Transition (claim-first ordering)

**Change 4** now uses **claim-first ordering**: the GitHub `claim_issue` call happens *before* the `Failed → Pending` state transition. If the claim fails, the task stays `Failed` (no stranded state). The CAS transition includes a double-check that the task is still `Failed` to handle concurrent state changes. This is explicitly called out in the "Why claim-first ordering" rationale.

### Review Issue 2: Legacy Failed Task Backfill

**Change 3** adds **dispatch-time backfill**: before spawning the child process in `dispatch_task()`, if `task.project_id` is `None`, the worktree is scanned for an existing `.ralph/projects/<id>/state.json`. If found, the `project_id` is persisted to the store and used for the spawn decision (`ralph run --project` instead of `ralph auto --idea`). This is lazy rather than startup-wide — it only runs for tasks being re-triggered — and is documented in a new acceptance criterion. The "Out of Scope" section explicitly notes that startup-wide backfill is deferred in favor of this approach.

### Review Issue 3: Project Discovery Robustness

**Change 1** now validates project directories by requiring the presence of `state.json` (not just checking that a directory exists). A new unit test (`discover_project_id_ignores_dirs_without_state_json`) verifies that stray directories without `state.json` are ignored. The multi-project fallback behavior is explicitly documented as safe (fresh `ralph auto` uses a different branch namespace than existing project branches).

### Review Issue 4: Inconsistent Worktree/Branch Handling

**Change 5** adds two hardening measures to `create_worktree`: (1) `git worktree prune` before `git worktree add` to clear stale metadata from previously-removed worktrees, and (2) `verify_worktree_branch()` which checks the checked-out branch when reusing an existing worktree and attempts a best-effort force-checkout to the expected daemon branch. A new integration test (`runtime_create_worktree_handles_stale_metadata`) validates the prune-then-add recovery path.

### Review Issue 5: Test Coverage Gaps

Two new tests directly cover the gaps identified:
- **Test 11** (`runtime_retrigger_claim_failure_preserves_failed_state`): Verifies that a failed claim during re-trigger leaves the task in `Failed` state, not `Pending`.
- **Test 13** (`runtime_retrigger_preserves_project_artifacts`): Verifies that `.ralph/projects/<id>/loops/` artifacts survive across retries (asserts file continuity after dispatch + `clean_worktree`).