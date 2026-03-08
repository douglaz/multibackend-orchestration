---
artifact: quick-dev-final-review
loop: 15
project: issue-182
backend: codex
role: reviewer
created_at: 2026-03-08T04:16:21Z
---

# Final Review: AMENDMENTS

## Amendment: ORPHAN-01 [P1]

### Problem
The orphan terminalization fallback checks the wrong branch name when `pr_url` is missing.  
In [`runtime.rs#L1107`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs#L1107), it uses `ralph/issue-{issue_number}`, but daemon task branches are `ralph/daemon/{task_id}` (see [`runtime.rs#L1663`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs#L1663) and [`worktree.rs#L47`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/worktree.rs#L47)).  
Result: merged orphan work can be mislabeled `ralph:failed`.

### Proposed Change
Derive fallback branch from `task_id` using the daemon branch convention (`ralph/daemon/{task_id}`), ideally via a shared helper used by dispatch/worktree/orphan polling to prevent drift. Add a conformance case for “no `pr_url`, merged PR on daemon branch => `ralph:completed`”.

### Affected Files
- [`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs) - fix fallback branch resolution and add helper usage.
- [`src/daemon/worktree.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/worktree.rs) - optional: share branch-name helper source of truth.
- [`src/validate/tests_daemon_orphan.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs) - add merged-fallback coverage.

## Amendment: ORPHAN-02 [P0]

### Problem
`nix build -L` fails conformance due `daemon_orphan::orphan_terminalization_routes_through_complete_task` (395 passed, 1 failed).  
The test relies on `sh -c "setsid sleep 300 ... & echo $!"` and treats `$!` as the stable orphan PID ([`tests_daemon_orphan.rs#L923-L931`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs#L923)). In some environments this can capture a short-lived wrapper PID, so reconciliation sees it as dead and resets to ready.

### Proposed Change
Make orphan PID capture deterministic and hermetic:
1. Assert spawn command exit status before parsing PID.
2. Capture PID from inside the detached session process (for example via pidfile from inside `setsid sh -c 'echo $$ ...; exec sleep ...'`), not from parent-shell `$!`.
3. Optionally gate daemon start on confirmed `pid_exists(pid)` to remove startup race.

### Affected Files
- [`src/validate/tests_daemon_orphan.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs) - stabilize orphan process setup.
- [`src/validate/mock_scripts.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/mock_scripts.rs) - optional helper for deterministic detached-process spawning if reused.
