---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T08:17:41Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Hard Rollback Can Resurrect a Remote-Deleted Branch from Stale Tracking Refs

### Problem
When local project branch is missing, hard rollback trusts local `origin/<branch>` existence first and recreates from it ([rollback.rs:128](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L128), [rollback.rs:131](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L131)).  
If that tracking ref is stale and the branch was deleted upstream, `rollback --hard` can still proceed and then `push --force`, recreating the deleted remote branch. Dry-run has the same false-positive path ([rollback.rs:79](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L79)).

### Proposed Change
Use authoritative remote existence when local branch is missing:
1. Check remote branch existence on origin first (or fetch+verify), not stale local tracking refs.
2. If upstream branch does not exist, fail hard rollback/dry-run.
3. Only create local branch after fetching the confirmed remote branch.
4. Add validate coverage for: upstream deleted branch + stale local tracking ref -> hard rollback fails.

### Affected Files
- [rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - branch recovery and dry-run branch existence checks.
- [tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - add regression test for stale tracking ref behavior.

## Amendment: [P3] `rollback_push_failure_continues` Session Assertion Is Vacuous

### Problem
`rollback_push_failure_continues` claims to verify session invalidation, but it never enables session reuse or proves any pre-rollback session records exist ([tests_commands.rs:1275](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs#L1275), [tests_commands.rs:1352](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs#L1352)).  
Default config has `session_reuse_enabled = false` ([global.rs:660](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/config/global.rs#L660)), so this check can pass even if invalidation is broken.

### Proposed Change
Make the test assert real invalidation behavior:
1. Enable session reuse and relevant roles.
2. Use a mock response path that emits `session_id`.
3. Assert pre-rollback records include loop `2`.
4. Assert post-rollback records for loops `> 1` are removed.

### Affected Files
- [tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - strengthen assertions.
- [mock_scripts.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/mock_scripts.rs) - add/reuse session-id emitting mock behavior.

---
