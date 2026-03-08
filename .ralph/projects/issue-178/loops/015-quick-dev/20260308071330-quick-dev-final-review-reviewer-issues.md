---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T07:13:30Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Refresh Remote Before Recovery Hard Rollback

### Problem
In the hard-rollback recovery path, if the local project branch is missing, code branches directly from local `origin/<branch>` without fetching first ([src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L128), [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L133)).  
If that tracking ref is stale, rollback reference resolution is computed from stale history, and the subsequent force-push can overwrite newer remote branch state.

### Proposed Change
Always refresh `origin/<branch>` (`git fetch origin <branch>`) before creating the local recovery branch when local branch is absent. Then resolve/reset from the refreshed branch state.  
Add a validate regression that simulates stale `origin/<branch>` and verifies rollback uses fresh remote history.

### Affected Files
- `src/cli/rollback.rs` - fetch before branch recovery from remote-tracking ref.
- `src/validate/tests_commands.rs` - add stale remote-tracking recovery test.

## Amendment: [P2] New Push-Failure Test Does Not Prove Session Invalidation

### Problem
`rollback_push_failure_continues` asserts session invalidation but never enables session reuse or creates session records before rollback ([src/validate/tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs#L1277), [src/validate/tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs#L1361)).  
Default config has `session_reuse_enabled = false` ([src/config/global.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/config/global.rs#L660)), so the checked record list can be empty regardless of rollback behavior.

### Proposed Change
Seed real session data before rollback (enable session reuse + create sessions), then assert records for loops above rollback target are removed after `rollback --hard 1` with push failure.  
Use an assertion that cannot pass vacuously with an always-empty store.

### Affected Files
- `src/validate/tests_commands.rs` - strengthen `rollback_push_failure_continues` session assertions.

---

## Context Provided
Reviewed `git diff f0ce67e...HEAD -- . ':(exclude).ralph'` and audited changed files:
- `src/cli/rollback.rs`
- `src/git/branch.rs`
- `src/project/lifecycle.rs`
- `src/validate/tests_commands.rs`

## Master Prompt
Audit prioritized correctness/safety/robustness, including destructive rollback paths, checkpoint reconstruction behavior, failure handling, and whether tests validate what they claim.

## Summary
Most rollback behavior is implemented cleanly and targeted rollback validations pass, but there is one destructive recovery edge case and one false-confidence test that should be amended.
