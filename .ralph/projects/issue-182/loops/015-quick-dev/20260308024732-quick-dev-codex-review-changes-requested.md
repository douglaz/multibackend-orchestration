---
artifact: quick-dev-codex-review
loop: 15
project: issue-182
backend: codex
role: reviewer
created_at: 2026-03-08T02:47:32Z
---

# Review: CHANGES REQUESTED

1. [HIGH] PID/PGID metadata is not cleared on all child-removal paths.  
   Affected paths:
   - [`kill_aborted_children` child branch](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs#L2074)
   - [`drain_all_children` forced-kill branch](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs#L2126)  
   With PID/PGID now persisted, these paths leave stale process metadata behind. That can later produce incorrect reconciliation/adoption decisions if labels are moved back to `ralph:in-progress`.
   Actionable fix: add the same load-modify-save clear used in `collect_children` (`pid=None`, `pgid=None`) in both branches immediately after removing/killing the child.

2. [MEDIUM] Orphan liveness polling does not enforce the session-leader invariant (`pid == pgid`).  
   In [`poll_adopted_orphans`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs#L1069), dead detection only checks `pid_exists`/`pgid_exists`.  
   The spec requires liveness checks to include `pid == pgid` to mitigate PID reuse.
   Actionable fix: treat an orphan as dead when `info.pid != info.pgid` OR either liveness probe fails.

3. [MEDIUM] Required test coverage from the spec is missing.  
   The patch adds behavior but does not add corresponding unit/conformance tests (only a struct-literal field update in [`tests_pr_lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_pr_lifecycle.rs#L206)).  
   Actionable fix:
   - Add `pgid_exists` unit tests in [`src/daemon/process.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/process.rs#L521).
   - Add TaskMetadata PID/PGID roundtrip/backward-compat tests in [`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs#L704).
   - Add daemon validate tests for orphan reconciliation/adoption/slot accounting/no-duplicate-dispatch/abort behavior, and register them in [`src/validate/mod.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/mod.rs#L110).

Assumption: I reviewed the branch diff against `origin/master` and also ran `nix develop -c cargo check` (passes).
