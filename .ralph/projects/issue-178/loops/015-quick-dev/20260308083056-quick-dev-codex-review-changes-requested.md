---
artifact: quick-dev-codex-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T08:30:56Z
---

# Review: CHANGES REQUESTED

1. **High: rollback cleanup can miss artifact directories (and violate `rollback 0` expectations) when ceiling-capped reconstruction hides loops.**  
   - `rollback` derives `to_remove` only from reconstructed state (`state.loops` + `state.completion_attempts`) in [src/cli/rollback.rs:45](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L45).  
   - Reconstruction can hide loop directories above the ceiling via `loop_dirs.retain(|(n,_,_)| *n <= ceiling)` in [src/project/lifecycle.rs:309](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs#L309).  
   - Then deletion in [src/cli/rollback.rs:191](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs#L191) only targets those hidden-from-state loop numbers, so real on-disk `loops/NNN-*` dirs can survive rollback.  
   - This is especially problematic for `rollback 0`: it can leave loop artifacts behind even though acceptance requires all loop artifacts removed.

   **Actionable fix:**  
   - In [src/cli/rollback.rs](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs), compute removal targets from disk (`{project_dir}/loops/*`) and union with state-derived loop numbers before filtering `> args.loop_number`.  
   - Use that union for both artifact deletion and `session_store.remove_for_loop(...)`.  
   - Add a conformance test in [src/validate/tests_commands.rs](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) that leaves a loop dir above ceiling while capped state excludes it, then asserts `rollback 0` removes all `loops/` dirs.
