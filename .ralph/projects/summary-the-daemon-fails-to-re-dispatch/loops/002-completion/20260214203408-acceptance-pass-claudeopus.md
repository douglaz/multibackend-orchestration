---
artifact: acceptance-pass
loop: 2
project: summary-the-daemon-fails-to-re-dispatch
backend: claude(opus)
role: qa
created_at: 2026-02-14T20:34:08Z
---

I now have all the evidence needed to make my assessment. Let me systematically verify each acceptance criterion:

**AC1: Cleanup order is exactly: remove-if-exists -> prune-always -> delete-branch-after-prune.**
- `worktree.rs:80-99`: `wt_path.exists()` guard around `git worktree remove --force` (step 1, conditional)
- `worktree.rs:101-115`: `git worktree prune` runs unconditionally (step 2, always)
- `worktree.rs:117-131`: `git show-ref --verify` runs unconditionally (step 3, always)
- `worktree.rs:133-147`: `git branch -D` runs only if show-ref succeeds (step 4, conditional on existence)
- PASS

**AC2: Missing worktree path does not skip prune or branch cleanup.**
- When `wt_path.exists()` is false, only step 1 is skipped. Steps 2-4 still execute unconditionally.
- PASS

**AC3: Branch existence checks and test assertions use `git show-ref --verify`.**
- `worktree.rs:117-131`: Uses `git show-ref --verify refs/heads/<branch>` for branch detection
- No `git branch --list` found in source code
- Test helper `git_show_ref_verify_status()` at line 3328 uses `show-ref --verify`
- All test assertions use `git_show_ref_verify_status()` (lines 631, 635, 702, 730, 768)
- PASS

**AC4: Restarting with stale `in_progress` + stale branch successfully reaches redispatch and terminal task state.**
- `runtime_restart_in_progress_with_stale_branch` test (line 709): seeds `in_progress` task with fake PID, creates pre-existing branch, starts daemon, asserts dispatch signal, asserts terminal state, asserts branch cleanup
- `runtime_reconciliation_on_startup` test also updated to verify the same flow for two tasks
- PASS

**AC5: No optional wording remains for branch cleanup; behavior is required.**
- `worktree.rs:65`: doc says "Remove a task worktree and its local branch." (mandatory)
- No "optional", "recommended", "should", "may" wording for branch cleanup found
- `remove_worktree` returns `Result<()>` (errors propagate), not best-effort void
- PASS

**AC6: No test depends on worktree directory existence.**
- The new/modified tests (`runtime_reconciliation_on_startup`, `runtime_restart_in_progress_with_stale_branch`) do not assert on worktree directory existence
- They assert on: dispatch signals in stderr, terminal state in tasks.json, branch ref cleanup via `show-ref --verify`
- The pre-existing `wt_base.exists()` at line 923 is in `runtime_worktree_isolation` (unmodified, out of scope)
- PASS

**Additional verification:**
- `reconcile_worktrees` delegates to `remove_worktree` for all cleanup (no separate orphan-branch sweep) - satisfies out-of-scope constraint
- Error handling: all command failures return contextual `RalphError::Orchestration` errors with command + target info
- `show-ref --verify` non-zero exit is non-fatal (returns `Ok(())` at line 130)
- Project compiles successfully

# QA: PASS

## Manual Testing
- Verified `remove_worktree()` cleanup order by reading `src/daemon/worktree.rs:72-149`: remove-if-exists (line 80-98), prune-always (line 101-115), show-ref-verify (line 117-131), branch-D-if-exists (line 133-147). Exact order matches spec.
- Verified `reconcile_worktrees()` at `src/daemon/worktree.rs:162-213` handles both on-disk orphans and active tasks with missing worktree directories, delegating all cleanup to `remove_worktree()`. No separate orphan-branch sweep exists.
- Verified `runtime.rs:155-183` builds `task_branches` map and passes it through to `worktree::reconcile_worktrees()`, propagating errors with `?`.
- Confirmed no usage of `git branch --list` anywhere in source code (only in project documentation/spec files).
- Confirmed no optional/recommended wording for branch cleanup in `worktree.rs`.

## Automated Tests
- `cargo check` passes — project compiles cleanly with all changes.
- Test `runtime_reconciliation_on_startup` (line 578): Seeds 2 `in_progress` tasks with fake PIDs and pre-existing branches, runs daemon with `--single-iteration`, asserts reconciliation message, dispatch signals for both tasks, terminal states, and branch cleanup via `show-ref --verify`.
- Test `runtime_restart_in_progress_with_stale_branch` (line 709): Seeds 1 `in_progress` task with stale PID/PGID and a pre-existing local branch, starts daemon, asserts dispatch success signal in stderr, terminal task state in `tasks.json`, and branch removal via `git show-ref --verify` non-zero exit code.
- Neither new test asserts on worktree directory existence.

## Acceptance Criteria Verification
| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | Cleanup order: remove-if-exists -> prune-always -> delete-branch-after-prune | PASS | `worktree.rs:80-147` — sequential steps with conditional guards only on remove and branch-D |
| 2 | Missing worktree path does not skip prune or branch cleanup | PASS | `worktree.rs:80` — only `git worktree remove` is inside the `if wt_path.exists()` block; prune and show-ref/branch-D are outside |
| 3 | Branch checks use `git show-ref --verify` | PASS | `worktree.rs:118-119` uses `show-ref --verify`; tests use `git_show_ref_verify_status()` helper; no `git branch --list` in src |
| 4 | Restart with stale in_progress + stale branch reaches redispatch and terminal state | PASS | `runtime_restart_in_progress_with_stale_branch` test (line 709-770) asserts all three: dispatch signal, terminal state, branch cleanup |
| 5 | No optional wording for branch cleanup | PASS | `remove_worktree` doc says "Remove a task worktree and its local branch" — no optional/recommended language; returns `Result<()>` |
| 6 | No test depends on worktree directory existence | PASS | New/modified tests use `git_show_ref_verify_status()`, stderr signals, and `tasks.json` state — no `.exists()` on worktree paths |
