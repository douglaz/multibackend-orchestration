---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T06:18:37Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Rollback ceiling is disabled too early and can resurrect stale checkpoints

### Problem
The rollback ceiling enforcement in [`src/project/lifecycle.rs:292`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:292) only runs when `max_artifact_loop <= ceiling`.  
That makes the marker inert as soon as *any* artifact appears above the ceiling, even if checkpoint history is still stale (for example, stale checkpoint loop `3`, ceiling `1`, artifacts now at loop `2` after a failed run before a new checkpoint commit). In that state, reconstruction can jump back to the stale checkpoint loop instead of respecting rollback protection.

### Proposed Change
Change the capping condition so stale checkpoint protection remains active until checkpoint progress has clearly caught up to post-rollback artifacts (for example, cap when `checkpoint_loop > ceiling` and `checkpoint_loop > max_artifact_loop`).  
Add a regression unit test for: ceiling `1`, stale checkpoint at loop `3`, artifacts up to loop `2` -> reconstruction must not restore loop `3`.

### Affected Files
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs) - tighten ceiling enforcement logic and add regression test(s).

## Amendment: [P2] `--hard --dry-run` can report success for a rollback that would fail

### Problem
In hard dry-run mode, when the local project branch is missing, code prints a placeholder and exits success at [`src/cli/rollback.rs:79`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:79).  
But the real hard rollback path fails for the same “missing locally and on origin” case at [`src/cli/rollback.rs:131`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:131). This makes dry-run misleading for automation/safety checks.

### Proposed Change
Make hard dry-run return the same validation failure when branch recovery is impossible (missing both local and remote).  
Keep the placeholder behavior only for cases where recovery is possible but exact ref cannot be resolved without mutation.

### Affected Files
- [`src/cli/rollback.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - align hard dry-run error behavior with real execution.
- [`src/validate/tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - add/extend validate coverage for failing hard dry-run on truly missing branch.

---
