---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T05:51:42Z
---

# Final Review: AMENDMENTS

## Amendment: RVW-001 [P2]

### Problem
Hard dry-run resolves the reset ref *before* branch recovery, while real hard rollback resolves it *after* recovery.

- Dry-run path resolves early at [src/cli/rollback.rs:53](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:53)-[67](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:67).
- Real hard path recovers/fetches branch first at [src/cli/rollback.rs:107](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:107)-[117](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:117), then resolves at [src/cli/rollback.rs:129](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:129).

This can produce an incorrect dry-run `git reset --hard <ref>` preview when refs become visible only after recovery.

### Proposed Change
Unify hard dry-run and hard execution resolution semantics:
- In dry-run hard mode, run the same branch-visibility checks first.
- If ref cannot be determined without recovery side effects, print an explicit unresolved placeholder/message instead of a potentially wrong ref.
- Do not emit a concrete reset target unless it is derived from the same visible refs execution would use.

### Affected Files
- [src/cli/rollback.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - align dry-run ref derivation with hard rollback behavior.

## Amendment: RVW-002 [P3]

### Problem
`rollback_push_failure_continues` does not prove that hard-reset still happened on the push-failure path.

At [src/validate/tests_commands.rs:1260](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1260)-[1323](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1323), the test checks warnings/artifacts/sessions/marker but never asserts local HEAD moved to rollback target. A regression skipping `reset_hard` could still pass.

### Proposed Change
Strengthen the test by asserting local git state:
1. Capture rollback target from `rollback --hard --dry-run 1`.
2. Capture `head_before`.
3. After `rollback --hard 1` with push failure, assert `head_after != head_before` and `head_after == target_commit`.
4. Assert `session_store` shape strictly (object expected), not conditional pass-through.

### Affected Files
- [src/validate/tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - tighten conformance coverage for hard rollback push-failure semantics.
