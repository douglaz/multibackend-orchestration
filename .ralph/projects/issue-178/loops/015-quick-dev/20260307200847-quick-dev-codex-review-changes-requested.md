---
artifact: quick-dev-codex-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-07T20:08:47Z
---

# Review: CHANGES REQUESTED

1. **High: `.rollback-ceiling` staleness logic is effectively dead, so soft rollback can permanently cap future progress.**  
[Lifecycle ceiling logic](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:285), [stale condition](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:294), [enforcement branch](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:296), [loop filtering](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:313)  
`stale` is defined as `checkpoint_loop <= ceiling && ...`, but enforcement requires `checkpoint_loop > ceiling`, so `!stale` is always true in that branch. Result: once checkpoint moves above ceiling, reconstruction keeps capping forever and hides newer loop dirs. This affects all reconstruct callers, including orchestrator resume and user-facing state (`[orchestrator](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/workflow/orchestrator.rs:235)`, `[project show](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/project.rs:99)`, `[status](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/status.rs:29)`).  
**Fix:** gate enforcement on an actual “needs cap” condition, e.g. `checkpoint_loop > ceiling && max_artifact_loop <= ceiling`, and skip capping/filtering when marker is stale/inert.

2. **Medium: validate coverage is missing for key new rollback semantics.**  
[Test registry](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:14), [rollback dry-run test](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:382)  
The diff updates existing rollback tests, but there is still no conformance test for:
- soft rollback marker becoming inert after successful forward progress, and
- hard rollback push-failure path (warning surfaced, cleanup still performed, marker retained).  
**Fix:** add explicit tests in `src/validate/tests_commands.rs` for both paths and register them in `tests()`.
