---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T03:32:33Z
---

# Final Review: AMENDMENTS

## Amendment: A1 [P1] Rollback Ceiling Is Disabled Too Early

### Problem
In `reconstruct_project_state_internal`, the ceiling is only enforced when `checkpoint_loop > ceiling && max_artifact_loop <= ceiling` (`src/project/lifecycle.rs:285-310`).  
That means **any** artifact directory above the ceiling disables protection, even if no new checkpoint was written after rollback. In interrupted forward-progress runs, stale pre-rollback checkpoint commits can be resurrected and advance `current_loop/current_phase` incorrectly.

### Proposed Change
Make ceiling inertness depend on evidence of new checkpoint progress, not just artifact presence.  
A robust approach: persist marker metadata (ceiling + checkpoint head hash at rollback time), then keep enforcing the ceiling while the newest checkpoint hash is unchanged. Ignore the marker only after a newer checkpoint exists.

### Affected Files
- [`src/cli/rollback.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:202) - write structured rollback marker metadata.
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:285) - enforce marker until a newer checkpoint is observed.
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:1866) - add unit test for “artifact advanced, checkpoint not advanced” interrupted-run case.

## Amendment: A2 [P2] Hard Rollback Branch Recovery Uses Only Local Tracking Ref

### Problem
Hard rollback recovery checks `origin/<branch>` via local ref existence (`src/cli/rollback.rs:104-113` using `remote_ref_exists`), but does not query/fetch the remote. If local tracking refs were pruned/deleted while the remote branch still exists, rollback fails with “branch does not exist” and exits early.

### Proposed Change
Before failing, verify remote branch existence from origin directly (`git ls-remote` or targeted `git fetch origin <branch>`). Recreate local branch from confirmed remote branch when available.

### Affected Files
- [`src/cli/rollback.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:103) - improve branch recovery logic.
- [`src/git/branch.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/branch.rs:69) - add a true remote-branch existence helper.
- [`src/validate/tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:612) - adjust missing-branch test to remove the actual remote branch when asserting failure.

## Amendment: A3 [P3] Push-Failure Test Does Not Prove Session Invalidation

### Problem
`rollback_push_failure_continues` checks session invalidation (`src/validate/tests_commands.rs:1240-1254`), but this test never enables session reuse or asserts pre-existing loop>target session records. The assertion can pass vacuously with an empty session store.

### Proposed Change
Set session-reuse config and create at least one reusable session record for loop 2 before rollback. Assert loop 2 session records exist pre-rollback and are removed post-rollback.

### Affected Files
- [`src/validate/tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1192) - make session invalidation assertion non-vacuous.
