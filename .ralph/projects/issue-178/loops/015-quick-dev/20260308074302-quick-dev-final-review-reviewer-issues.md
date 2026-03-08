---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T07:43:02Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Do Not Recover Hard Rollback From Stale `origin/<branch>` Cache Alone

### Problem
Hard rollback branch recovery currently trusts local remote-tracking refs without verifying the branch still exists on the actual remote server.  
In [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L128), `remote_ref_exists("origin/<branch>")` is treated as sufficient for recovery and `create_branch(...)` proceeds. The same assumption is used in hard dry-run at [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L79).  
If `refs/remotes/origin/<branch>` is stale but the branch was deleted on origin, `rollback --hard` can recreate/force-push a branch that should be considered missing.

### Proposed Change
Require authoritative remote existence (`ls-remote`) before recovery when local branch is absent:
1. In execute path, when local branch is missing, check `remote_branch_exists_on_remote` first.
2. Only recover/create the local branch after a successful fetch from origin.
3. If remote branch is missing, fail even if stale `origin/<branch>` exists locally.
4. Mirror this rule in hard dry-run so dry-run and execute have identical safety behavior.
5. Add a conformance case where remote branch is deleted but stale remote-tracking ref remains; both hard dry-run and hard rollback must fail.

### Affected Files
- [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - tighten branch recovery checks.
- [src/validate/tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - add stale-remote-tracking regression coverage.

## Amendment: [P2] Rollback Ceiling Becomes Inert Too Early (Artifacts-Only Heuristic)

### Problem
Ceiling enforcement is disabled when `checkpoint_loop <= max_artifact_loop` due the condition at [src/project/lifecycle.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs#L292).  
This allows `.rollback-ceiling` to become inert as soon as a loop directory reappears, even if no new checkpoint commit has been created yet. In a crash window between artifact creation and checkpoint commit, stale pre-rollback checkpoint phase can be reused.

### Proposed Change
Make ceiling inactivation checkpoint-driven, not artifact-driven:
1. Persist extra marker metadata on soft rollback (e.g., latest checkpoint hash at rollback time) in [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L277).
2. In reconstruction, keep enforcing ceiling until a newer/different checkpoint is observed; do not disable enforcement based only on artifact loop directories.
3. Extend tests to assert phase correctness (not just loop count) for forward-progress/inert cases.

### Affected Files
- [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - write richer rollback marker metadata.
- [src/project/lifecycle.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs) - replace artifact-only inert check with checkpoint-based validation.
- [src/validate/tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) and [src/project/lifecycle.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs) - add tests proving phase-safe behavior across crash/interruption windows.
