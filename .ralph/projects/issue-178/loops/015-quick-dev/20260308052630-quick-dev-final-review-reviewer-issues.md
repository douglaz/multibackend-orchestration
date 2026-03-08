---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T05:26:30Z
---

# Final Review: AMENDMENTS

## Amendment: RB-001 [P2]

### Problem
`--hard --dry-run` computes the reset ref in a different path than real hard rollback.  
Dry-run resolves immediately ([`src/cli/rollback.rs:53`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:53)), while real hard rollback recovers/creates the project branch first and only then resolves the ref ([`src/cli/rollback.rs:96`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:96), [`src/cli/rollback.rs:129`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:129)).  
Because checkpoint discovery only consults local refs/tracking refs ([`src/git/ralph_commit.rs:76`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/ralph_commit.rs:76)), dry-run can print an incorrect reset target when local branch/tracking refs are missing but remote still has the branch.

### Proposed Change
Unify hard-ref resolution between dry-run and real hard rollback:
- Extract shared “hard rollback prep + ref resolution” logic.
- For dry-run, run remote-branch discovery/fetch needed for correct ref resolution, but skip checkout/reset/push.
- Add a validate case asserting `rollback --hard --dry-run <loop>` emits the same reset target used by actual `rollback --hard <loop>` in the recover-from-remote branch-missing scenario.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs` - share hard-ref resolution path across dry-run and non-dry-run.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs` - add conformance coverage for dry-run/actual ref parity in branch-recovery path.

## Amendment: RB-002 [P1]

### Problem
Hard rollback branch recovery trusts stale local tracking refs (`origin/<branch>`) as proof the remote branch exists ([`src/cli/rollback.rs:109`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:109), [`src/cli/rollback.rs:111`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:111)).  
If the remote branch was deleted but `refs/remotes/origin/<branch>` is stale locally, rollback can recreate local branch from stale data and then `push --force` re-create the deleted remote branch. This is destructive and can resurrect history unexpectedly. Current missing-branch coverage removes the tracking ref first, so this path is not tested ([`src/validate/tests_commands.rs:694`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:694)).

### Proposed Change
When local project branch is missing:
- Verify remote existence against the real remote (`ls-remote`) before accepting any local `origin/<branch>` tracking ref.
- If remote branch is absent, fail hard rollback and do not create/push the branch.
- Add a conformance test where remote branch is deleted but stale local tracking ref remains; assert rollback fails and remote branch is not recreated.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs` - tighten branch recovery to avoid stale-tracking resurrection.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs` - add stale-tracking regression test.
