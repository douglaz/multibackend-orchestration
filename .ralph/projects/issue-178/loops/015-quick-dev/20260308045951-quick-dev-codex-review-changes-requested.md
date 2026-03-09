---
artifact: quick-dev-codex-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T04:59:51Z
---

# Review: CHANGES REQUESTED
1. **High: `--hard` can reset to the wrong commit when the local project branch and local tracking ref are missing.**  
   In [`rollback.rs:55`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:55), `hard_ref` is resolved *before* branch recovery.  
   Branch recovery happens later in [`rollback.rs:105`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:105), but reset uses the precomputed ref at [`rollback.rs:127`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:127).  
   If refs are missing locally, `resolve_hard_reset_ref` falls back too far (often base), causing over-rollback (observed by failing conformance test at [`tests_commands.rs:657`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:657): got 0 loops instead of 1).

   **Fix:** resolve the hard reset ref only after branch recovery/checkout in the non-dry-run hard path.  
   - Keep `--dry-run` ref resolution separate (read-only).  
   - In execute hard path: recover/create branch -> checkout branch -> call `resolve_hard_reset_ref(...)` -> `reset_hard(...)`.  
   - Use that post-recovery ref for output message and marker handling.

   **Verification:** `nix develop -c cargo run -- validate --bin target/debug/ralph --filter commands::rollback_hard_missing_branch --jobs 1` currently fails; it should pass after the fix.
