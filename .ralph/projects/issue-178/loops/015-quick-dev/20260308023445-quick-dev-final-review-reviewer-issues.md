---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T02:34:45Z
---

# Final Review: AMENDMENTS

## Amendment: RBK-001 [P2]

### Problem
`rollback --hard` can fail even when the project branch exists on the remote.  
In [`rollback.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:104), branch recovery depends on `remote_ref_exists(repo_root, "origin/<branch>")` before recreating the local branch. But `remote_ref_exists` in [`branch.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/branch.rs:70) uses `git rev-parse` on the local tracking ref only, so a pruned/missing tracking ref is treated as “remote missing” even if `origin` still has the branch.

### Proposed Change
Use an authoritative remote check when local branch is missing:
1. `git fetch origin <branch>` (or `fetch origin`) and retry local tracking lookup, or
2. check `git ls-remote --exit-code origin refs/heads/<branch>` directly.

If remote branch exists, recreate local branch and continue hard rollback. Only fail when both local and remote branch are truly absent.

### Affected Files
- [`src/cli/rollback.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:104) - replace tracking-ref-only recovery gate.
- [`src/git/branch.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/branch.rs:70) - add helper for authoritative remote-branch existence (if reused).

## Amendment: RBK-002 [P3]

### Problem
`commands::rollback_hard_missing_branch` does not validate what its name claims.  
In [`tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:616), the test deletes only the local branch and local tracking ref, then expects failure. It never deletes the actual remote branch, so the test can pass for the wrong reason (missing tracking ref, not missing remote branch).

### Proposed Change
Split into explicit cases:
1. “local branch missing, remote exists” -> assert hard rollback recovers and succeeds.
2. “local + remote branch missing” -> delete remote branch (`git push origin --delete <branch>`) and assert failure.

### Affected Files
- [`src/validate/tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:590) - adjust test setup/assertions to reflect true remote state.

---
