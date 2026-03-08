---
artifact: quick-dev-final-review
loop: 15
project: issue-182
backend: codex
role: reviewer
created_at: 2026-03-08T03:50:34Z
---

# Final Review: AMENDMENTS

## Amendment: [P2] ORPHAN-CRASH-WINDOW-TEST-DOES-NOT-EXERCISE-DISPATCH

### Problem
[`crash_after_spawn_before_stage3`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs#L1022 ) claims to validate the crash window “after spawn, before Stage 3,” but it manually spawns `sleep` and manually writes metadata ([`L1036-L1052`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs#L1036 )).  
That means it does **not** verify that [`dispatch_task`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs#L1509 ) actually persists PID/PGID in the real dispatch path. A regression in dispatch persistence would still pass this test.

### Proposed Change
Rework this case to go through real daemon dispatch (or a runtime unit test that calls `dispatch_task` directly), then assert PID/PGID persistence before child insertion/collection.  
If needed, add a test-only hook around dispatch result handling to pause between spawn and Stage 3 insertion so the crash window is genuinely exercised.

### Affected Files
- [`src/validate/tests_daemon_orphan.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs) - replace manual metadata simulation with a real dispatch-path assertion.
- [`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/daemon/runtime.rs) - optional test hook/support to expose the pre-insert crash point.

## Amendment: [P3] PID-LIFECYCLE-TEST-MISSES-THE-“SET-AFTER-DISPATCH” ASSERTION

### Problem
[`pid_lifecycle_dispatch_to_collect`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs#L642 ) says it verifies “set after dispatch, cleared after collect,” but it only checks the terminal cleared state ([`L735-L742`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs#L735 )).  
So a bug where PID/PGID is never written at dispatch time would still pass.

### Proposed Change
Split this into two observable phases:
1. Assert metadata has `pid/pgid = Some(...)` while the child is still running.
2. Then assert both fields are cleared after completion/collection.

### Affected Files
- [`src/validate/tests_daemon_orphan.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-182/src/validate/tests_daemon_orphan.rs) - add an intermediate in-progress metadata assertion before terminal cleanup checks.
