---
artifact: acceptance-pass
loop: 4
project: task-preserve-worktree-and-branch-on-dae
backend: claude(opus)
role: qa
created_at: 2026-02-14T23:47:00Z
---

# QA: PASS

## Manual Testing

All code paths in the daemon runtime were manually traced through the diff and source files:

1. **Worktree cleanup centralization**: All cleanup call sites (`complete_task` normal path, `complete_task` already-terminal path, `dispatch_task` CAS-failure path, `reconcile_worktrees`) funnel through `cleanup_worktree_for_terminal_state()` which gates on `should_cleanup_worktree()`. The single policy function `should_cleanup_worktree()` returns `true` only for `Completed | Aborted`, never for `Failed`.

2. **Dispatch routing**: `dispatch_task()` at line 426 uses `task.project_id.as_deref()` exclusively. No reference to `effective_project_id` exists anywhere in the codebase (confirmed via grep). `Some(project_id)` routes to `spawn_ralph_run()` (`ralph run --project`); `None` routes to `spawn_ralph_auto()` (`ralph auto --idea`).

3. **CAS-failure hardening**: The dispatch CAS-failure path (lines 498-534) re-reads persisted state before deciding cleanup. If persisted state is `Failed`, worktree is preserved. If state cannot be read or task is missing, worktree is also preserved (fail-safe).

4. **DaemonTask schema**: `project_id: Option<String>` added with `#[serde(default)]` for backward compatibility. Serialization round-trip tests confirm correct behavior. New tasks from `poll_and_claim` set `project_id: None`.

5. **Logging**: All decision points include log messages documenting the policy decision (e.g., `"dispatch-terminal-race: preserving worktree for {task_id} (state=failed)"`).

## Automated Tests

- **`nix develop -c cargo test`**: All unit tests pass (15 daemon unit tests + all other crates).
- **`./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon`**: All 36 conformance tests pass, including:
  - 3 required new tests: `runtime_task_fails_worktree_preserved`, `runtime_activation_failed_task_preserved`, `runtime_fresh_dispatch_ignores_discovered_project`
  - 3 additional coverage tests: `runtime_failed_worktree_preserved_and_reused_on_retry`, `runtime_aborted_task_worktree_cleaned`, `runtime_succeeded_task_worktree_cleaned`
  - All pre-existing tests: `runtime_abort_during_dispatch_preserves_terminal`, `runtime_worktree_isolation`, and 27 others

## Acceptance Criteria Verification

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | No code path removes worktree when terminal state is `Failed` | PASS | `should_cleanup_worktree()` returns `false` for `Failed`; all 6 cleanup call sites go through this gate; `runtime_task_fails_worktree_preserved` and `runtime_activation_failed_task_preserved` tests confirm |
| 2 | Aborted-task cleanup behavior remains unchanged and enforced | PASS | `should_cleanup_worktree()` returns `true` for `Aborted`; `runtime_aborted_task_worktree_cleaned` test confirms worktree removal; `runtime_abort_during_dispatch_preserves_terminal` confirms abort state preservation |
| 3 | Fresh tasks (`task.project_id == None`) always use fresh dispatch path | PASS | `dispatch_task()` line 426: `match task.project_id.as_deref()` — `None` arm always calls `spawn_ralph_auto`; no `effective_project_id` reference exists; `runtime_fresh_dispatch_ignores_discovered_project` test confirms with active discovered project |
| 4 | Resume dispatch occurs only when `task.project_id` is present | PASS | Same match expression: `Some(project_id)` arm calls `spawn_ralph_run`; `runtime_task_fails_worktree_preserved` confirms `ralph run --project` used when `project_id` is set |
| 5 | All 3 required new conformance tests pass | PASS | `runtime_task_fails_worktree_preserved`, `runtime_activation_failed_task_preserved`, `runtime_fresh_dispatch_ignores_discovered_project` — all pass in validate output |
| 6 | Existing related conformance tests pass | PASS | `runtime_failed_worktree_preserved_and_reused_on_retry`, `runtime_aborted_task_worktree_cleaned`, `runtime_succeeded_task_worktree_cleaned`, `runtime_abort_during_dispatch_preserves_terminal` — all pass |
| 7 | `nix develop -c cargo test` passes | PASS | Full test suite passes with 0 failures |
| 8 | Validate tests pass via built binary | PASS | All 36 daemon conformance tests pass: `test result: ok. 36 passed; 0 failed; 0 skipped` |
