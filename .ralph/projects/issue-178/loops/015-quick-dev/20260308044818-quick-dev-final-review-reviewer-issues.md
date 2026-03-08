---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T04:48:18Z
---

# Final Review: AMENDMENTS

## Amendment: RBK-REMOTE-BRANCH-CHECK [P2]

### Problem
Hard rollback can falsely fail when the local project branch is missing but the branch still exists on `origin`.

At [`src/cli/rollback.rs#L104`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L104), the code checks `remote_ref_exists(repo_root, "origin/<branch>")`, which only verifies local remote-tracking refs. If `refs/remotes/origin/<branch>` is absent locally (stale cache or manually pruned), rollback errors at [`src/cli/rollback.rs#L110`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L110) even if the branch is present on the remote.

The new validate case currently codifies this false-negative behavior by deleting only the local tracking ref and expecting failure at [`src/validate/tests_commands.rs#L640`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs#L640) and [`src/validate/tests_commands.rs#L651`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs#L651).

### Proposed Change
Before declaring the branch missing, perform a real remote lookup (`git ls-remote --exit-code origin refs/heads/<branch>`) or fetch the branch (`git fetch origin <branch>`) and then re-check.  
Only fail when both local branch and true remote branch are absent.

Update the validate case so that:
1. Deleting only local branch + tracking ref still allows `rollback --hard` to recover from remote.
2. A separate test covers true missing-local-and-remote failure.

### Affected Files
- [`src/cli/rollback.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - make remote existence check authoritative (not cache-only).
- [`src/validate/tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - adjust/replace `rollback_hard_missing_branch` expectations.

## Amendment: LIFECYCLE-ABSENT-CEILING-TEST [P3]

### Problem
`reconstruct_ignores_absent_ceiling` does not verify what its name/comment claims.  
At [`src/project/lifecycle.rs#L1921`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs#L1921), the comment says checkpoint-derived position is unaffected without a ceiling, but the test runs with no git context at [`src/project/lifecycle.rs#L1949`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs#L1949) and only asserts loop count at [`src/project/lifecycle.rs#L1954`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs#L1954).  
This can pass even if no-ceiling checkpoint behavior regresses.

### Proposed Change
Convert this test to include git checkpoints (like the other ceiling tests) and assert `current_loop/current_phase` come from checkpoint data when `.rollback-ceiling` is absent.

### Affected Files
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs) - strengthen the absent-ceiling test to validate checkpoint behavior directly.
