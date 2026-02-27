---
artifact: acceptance-pass
loop: 2
project: when-a-daemon-task-fails-e-g-backend-tim
backend: claude(opus)
role: qa
created_at: 2026-02-14T21:49:23Z
---

All verification steps are complete. Here is my QA report:

# QA: PASS

## Manual Testing

All four verification commands specified in the project master prompt were executed successfully:

1. **`nix develop -c cargo check`** — Passed. No errors, clean compilation.
2. **`nix develop -c cargo test`** — Passed. All unit tests pass across all test suites (0 failures).
3. **`nix build -L`** — Passed. Clean Nix build produced `./result/bin/ralph`.
4. **`./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon`** — Passed. **35/35 daemon conformance tests pass**, including all 5 newly required tests.

## Automated Tests

All 5 required conformance tests are registered in `src/validate/tests_daemon.rs` and pass:

| Test | Status |
|------|--------|
| `daemon::runtime_failed_worktree_preserved_and_reused_on_retry` | PASS |
| `daemon::task_store_project_id_serde_compat` | PASS |
| `daemon::runtime_resume_dispatch_uses_ralph_run_args` | PASS |
| `daemon::runtime_reconciliation_failed_to_pending` | PASS |
| `daemon::runtime_worktree_reconcile_preserves_retryable_tasks` | PASS |

Additionally, all 30 pre-existing daemon conformance tests continue to pass, confirming no regressions.

## Acceptance Criteria Verification

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | Fresh task dispatch behavior does not regress | PASS | The `else` branch in `dispatch_task()` preserves the full original path: raw idea fetch, refinement, GitHub comment posting, and `spawn_ralph_auto`. All 30 pre-existing conformance tests pass. |
| 2 | Resume path does not call `create_project()` and does not create duplicate projects | PASS | No call to `create_project` exists anywhere in `src/daemon/`. The resume path only calls `spawn_ralph_run()` which runs `ralph run --project <id> --until-complete`. |
| 3 | Failed tasks are automatically retried after daemon restart without manual state edits | PASS | `reconcile_tasks()` matches `TaskState::InProgress | TaskState::Failed`, transitions both to `Pending`, and clears PID/PGID. Confirmed by `runtime_reconciliation_failed_to_pending` conformance test. |
| 4 | Required daemon conformance tests pass and are registered correctly | PASS | All 5 tests are registered in the `tests()` function and pass via `ralph validate --filter daemon`. |
| 5 | `project_id: Option<String>` with `#[serde(default)]` on `DaemonTask` | PASS | Field present in `src/daemon/mod.rs:66-67`. Legacy JSON without `project_id` deserializes correctly (confirmed by `task_store_project_id_serde_compat` test). |
| 6 | `spawn_ralph_run()` and `build_ralph_run_command()` in `process.rs` with correct args | PASS | Both functions exist. Command args are `["run", "--project", project_id, "--until-complete"]`. Unit test `run_spawn_command_uses_expected_args` confirms. |
| 7 | Dispatch rule branches on `project_id` | PASS | `effective_project_id.as_deref()` branches to `spawn_ralph_run` when `Some`, else falls through to fresh-dispatch. |
| 8 | Project ID discovery: active-project first, single-project fallback | PASS | `discover_task_project_id()` calls `read_active_project()` first, validates against `.ralph/projects/`, then falls back to `discover_single_project_id()` (exactly-one check). Stale IDs are ignored with a warning. |
| 9 | `project_id` persisted after CAS activation and before `cleanup_worktree()` | PASS | Persisted in `dispatch_task()` during CAS store update (lines 536-538) and via `persist_project_id_before_cleanup()` called twice in `complete_task()` (lines 816, 907). |
| 10 | `reconcile_tasks()` runs before worktree reconciliation | PASS | In `run()`, `reconcile_tasks(&store)` precedes `reconcile_worktrees(&store, &config)`, ensuring retryable tasks are marked `Pending` before worktree cleanup decisions. Confirmed by `runtime_worktree_reconcile_preserves_retryable_tasks` test. |
| 11 | `create_worktree()` retries without `-b` when branch exists | PASS | `is_branch_exists_error()` detects the failure, then retries with `["worktree", "add", <path>, <branch>]` (no `-b`). Unit test `create_worktree_retries_without_b_when_branch_exists` confirms. |
| 12 | No new CLI commands or flags added | PASS | Diff is scoped to `src/daemon/{mod,process,runtime,worktree}.rs` and `src/validate/tests_daemon.rs`. No CLI surface changes. |
