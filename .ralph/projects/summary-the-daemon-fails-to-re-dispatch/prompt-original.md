The spec has been revised. Here's how each review issue was addressed:

**Issue 1 — Branch cleanup ordering**: The `remove_worktree()` rewrite now explicitly sequences the operations as: (1) `git worktree remove --force`, (2) `git worktree prune`, (3) `git branch -D`. The branch delete happens *after* prune, so git no longer considers the branch checked out. This is called out in both the prose and the code sample with explicit comments.

**Issue 2 — Missing-path cleanup edge case**: The early return at `!wt_path.exists()` is removed. The new structure wraps only the worktree removal in an `if wt_path.exists()` block, while prune and branch deletion always execute unconditionally. Acceptance criterion 5 explicitly requires this.

**Issue 3 — Branch detection/listing reliability**: `git branch --list` replaced with `git show-ref --verify refs/heads/<branch>`. The spec explains why: machine-parseable exit code (0/non-zero), no human-formatted markers or whitespace. The test assertions also use `git show-ref --verify` for consistency.

**Issue 4 — Scope/criteria consistency**: Branch cleanup is now unambiguously mandatory. Acceptance criteria 4-5 require it. The "recommended/optional" framing from the original PRD is gone. The Out of Scope section explicitly explains why a *separate* orphaned branch sweep in `reconcile_worktrees()` is unnecessary (since `remove_worktree()` now handles it).

**Issue 5 — Retry test completeness**: Added a second test (`runtime_restart_in_progress_with_stale_branch`) that seeds an `in_progress` task with fake PID/PGID plus a pre-existing branch, runs the daemon, and asserts the full reconcile → redispatch → terminal state flow. This validates the real restart path end-to-end.

**Issue 6 — Test assertion stability**: Removed all assertions on worktree directory existence. Tests now assert dispatch success (stderr message), state transitions (terminal state in `tasks.json`), and branch cleanup (via `git show-ref --verify` exit code). The spec notes explicitly why worktree directory assertions are unstable.