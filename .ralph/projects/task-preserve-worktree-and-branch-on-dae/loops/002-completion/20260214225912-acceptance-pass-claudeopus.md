---
artifact: acceptance-pass
loop: 2
project: task-preserve-worktree-and-branch-on-dae
backend: claude(opus)
role: qa
created_at: 2026-02-14T22:59:12Z
---

# QA: PASS

## Manual Testing

All code paths were manually traced through the diff and source files:

1. **Worktree cleanup policy** (`should_cleanup_worktree`): Verified at `src/daemon/runtime.rs:59-61`. Returns `true` only for `Completed | Aborted`, meaning `Failed` always preserves the worktree.

2. **All 4 worktree removal call sites** are gated through `cleanup_worktree_for_terminal_state` or `should_cleanup_worktree`:
   - `reconcile_worktrees` (startup) — filters active IDs via `!should_cleanup_worktree(&t.state)`
   - `dispatch_task` CAS-failure path — re-reads persisted state, calls `cleanup_worktree_for_terminal_state`
   - `complete_task` already-terminal path — calls `cleanup_worktree_for_terminal_state` with existing state
   - `complete_task` normal terminal path — calls `cleanup_worktree_for_terminal_state` with new terminal state

3. **Dispatch routing** in `dispatch_task` at lines 420-442: `match task.project_id.as_deref()` drives the decision. `Some(project_id)` → `spawn_ralph_run`, `None` → `spawn_ralph_auto`. No `effective_project_id` variable exists in any `.rs` source file.

4. **`spawn_ralph_run`** in `process.rs` correctly builds `["run", "--project", project_id]` with unit test coverage.

## Automated Tests

**`nix develop -c cargo test`**: All unit tests pass (15 daemon tests + all other crate tests, 0 failures).

**`./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon`**: All **33 conformance tests pass**, 0 failures, 0 skipped.

New required tests verified passing:
- `daemon::runtime_task_fails_worktree_preserved` — seeds pending task with `project_id`, mocks non-zero exit, asserts `failed` state, worktree preserved, `project_id` retained
- `daemon::runtime_activation_failed_task_preserved` — pre-creates failed-task worktree with marker file, runs daemon, asserts worktree and marker survive startup reconciliation
- `daemon::runtime_fresh_dispatch_ignores_discovered_project` — creates discovered project context, seeds task with `project_id=None`, asserts `ralph auto --idea` dispatched (not `ralph run`)

Existing tests verified passing:
- `daemon::runtime_abort_during_dispatch_preserves_terminal` (aborted cleanup)
- `daemon::runtime_reconciliation_on_startup` (worktree reconciliation)
- `daemon::runtime_branch_switch_updates_task_and_pr` (branch resolution)
- All 30 other daemon conformance tests

## Acceptance Criteria Verification

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | No code path removes worktree when terminal state is `Failed` | **PASS** | `should_cleanup_worktree` returns `false` for `Failed`; all 4 removal call sites gated through this function |
| 2 | Aborted-task cleanup behavior remains unchanged | **PASS** | `should_cleanup_worktree` returns `true` for `Aborted`; `runtime_abort_during_dispatch_preserves_terminal` passes |
| 3 | Fresh tasks (`project_id == None`) always use fresh dispatch | **PASS** | `task.project_id.as_deref()` match: `None` → `spawn_ralph_auto`; no `effective_project_id` in source |
| 4 | Resume dispatch only when `task.project_id` is present | **PASS** | `Some(project_id)` → `spawn_ralph_run(&ralph_bin, &wt, project_id, &log_path)` |
| 5 | All 3 required new conformance tests pass | **PASS** | All 3 registered in `tests()` vector and pass in validate run |
| 6 | Existing related conformance tests pass | **PASS** | All 33 daemon conformance tests pass |
| 7 | `nix develop -c cargo test` passes | **PASS** | 0 failures across all test targets |
| 8 | Validate tests pass via built binary | **PASS** | `./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon` → 33 passed, 0 failed |
