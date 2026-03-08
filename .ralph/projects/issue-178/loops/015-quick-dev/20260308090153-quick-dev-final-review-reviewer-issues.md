---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T09:01:53Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Rollback Ceiling Can Uncap Stale Checkpoints Too Early

### Problem
The ceiling guard is only applied when `checkpoint_loop > ceiling && checkpoint_loop > max_artifact_loop` in [src/project/lifecycle.rs:285](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:285) through [src/project/lifecycle.rs:292](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:292).  
If a stale pre-rollback checkpoint has the same loop number as newly recreated artifacts (before any new checkpoint commit is written), the marker becomes inert and stale phase/position can be resurrected.

### Proposed Change
Persist rollback marker provenance (at least latest checkpoint hash at rollback time) and only treat the marker as inert after observing a newer checkpoint lineage.  
At minimum:
1. Write `{ceiling, checkpoint_hash_at_rollback}` in rollback.
2. During reconstruction, keep capping while latest checkpoint hash equals the stored rollback hash.
3. Only stop capping once a post-rollback checkpoint is observed.

### Affected Files
- [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - write richer marker payload.
- [src/project/lifecycle.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs) - enforce marker using checkpoint provenance, not loop-number heuristic alone.
- [src/project/lifecycle.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs) - add regression test for “stale checkpoint loop equals max artifact loop”.

## Amendment: [P2] Remote Branch Probe Misclassifies Connectivity/Auth Failures as “Branch Missing”

### Problem
`remote_branch_exists_on_remote` returns `false` on any non-zero `ls-remote` status in [src/git/branch.rs:79](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/branch.rs:79)-[src/git/branch.rs:90](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/branch.rs:90).  
Callers in rollback then emit a validation error (“branch does not exist”) at [src/cli/rollback.rs:112](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:112)-[src/cli/rollback.rs:115](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:115) and [src/cli/rollback.rs:158](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:158)-[src/cli/rollback.rs:161](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:161), even when the real issue is remote outage/auth failure.

### Proposed Change
Return a tri-state/typed result for remote branch lookup:
- `Exists`
- `Missing`
- `ProbeFailed(stderr)`
Then:
- use `Missing` for validation error,
- use `ProbeFailed` for orchestration error with actionable stderr.

### Affected Files
- [src/git/branch.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/branch.rs) - improve remote probe API.
- [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - handle probe failure separately in dry-run and execution paths.

## Amendment: [P3] Push-Failure Conformance Test Does Not Actually Validate Session Invalidation

### Problem
`rollback_push_failure_continues` explicitly notes no session IDs are produced by the mock backend in [src/validate/tests_commands.rs:1288](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1288)-[src/validate/tests_commands.rs:1293](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1293).  
The “session invalidation” assertions over `records` at [src/validate/tests_commands.rs:1384](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1384)-[src/validate/tests_commands.rs:1392](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1392) can pass vacuously with an empty store.

### Proposed Change
Use a mock script that emits deterministic `session_id` values for reusable roles, assert records exist before rollback, then assert records above rollback target are removed after rollback with push failure.

### Affected Files
- [src/validate/tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - strengthen rollback push-failure assertions.
- `src/validate/mock_scripts.rs` - add/adjust a mock that emits session IDs reliably.
