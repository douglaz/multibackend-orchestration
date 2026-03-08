---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T03:07:53Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Rollback Ceiling Can Disable Too Early And Resurrect Stale Checkpoints

### Problem
Ceiling enforcement currently turns off whenever any artifact loop is above the ceiling:
- Condition: [`src/project/lifecycle.rs:285-293`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:285)
- It only caps when `checkpoint_loop > ceiling && max_artifact_loop <= ceiling`.

This is unsafe in the crash window where artifacts are written before checkpoint commit:
- Artifact write in planning: [`src/workflow/orchestrator.rs:635`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/workflow/orchestrator.rs:635)
- Checkpoint commit happens later: [`src/workflow/orchestrator.rs:2662`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/workflow/orchestrator.rs:2662)

If a run crashes after creating loop `N+1` artifacts but before checkpointing, the marker becomes inert and stale pre-rollback checkpoint state can come back.

### Proposed Change
Refine ceiling inertness so artifact presence alone is not enough. Keep capping when checkpoint is ahead of artifact frontier, e.g. enforce when:
- `checkpoint_loop > ceiling` and
- (`max_artifact_loop <= ceiling` or `checkpoint_loop > max_artifact_loop`)

Add a regression test for: stale checkpoint above ceiling + artifacts slightly above ceiling (partial forward progress, no new checkpoint) to ensure stale checkpoint is still blocked.

### Affected Files
- `src/project/lifecycle.rs` - tighten ceiling capping condition and add regression unit test(s).

## Amendment: [P2] Hard Rollback Branch Recovery Uses Stale Local Tracking Refs Instead Of Remote Truth

### Problem
When local project branch is missing, hard rollback checks only local `origin/<branch>` tracking ref and fails if that ref is absent:
- Logic: [`src/cli/rollback.rs:104-113`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:104)

This can false-fail even if the branch exists on remote but local tracking refs are pruned/stale.

The added conformance test also codifies this false-failure path by deleting only the tracking ref:
- [`src/validate/tests_commands.rs:638-643`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:638)

### Proposed Change
Before failing for missing remote branch:
- query remote heads (`git ls-remote --heads origin <branch>`) or fetch the branch/refspec first, then retry create.
- only return “does not exist locally or on origin” after actual remote check fails.

Update tests:
- `rollback_hard_missing_branch` should delete the real remote branch (not only local tracking ref) for negative case.
- add positive case: local branch missing + tracking ref missing + remote branch exists => rollback succeeds by recreating branch.

### Affected Files
- `src/cli/rollback.rs` - remote branch existence check should hit origin, not just local tracking refs.
- `src/validate/tests_commands.rs` - fix missing-branch test semantics and add recovery-positive case.

## Amendment: [P3] Push-Failure Conformance Test Does Not Actually Prove Session Invalidation

### Problem
`rollback_push_failure_continues` claims to verify session invalidation, but it never creates session records first:
- Test body: [`src/validate/tests_commands.rs:1192-1255`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1192)

Current assertion iterates over whatever is present and passes vacuously when records are empty.

### Proposed Change
Make the test non-vacuous:
- enable session reuse config,
- assert loop>target session records exist before rollback,
- then assert records for loops above target are removed after rollback.

### Affected Files
- `src/validate/tests_commands.rs` - strengthen `rollback_push_failure_continues` preconditions and assertions.

---
