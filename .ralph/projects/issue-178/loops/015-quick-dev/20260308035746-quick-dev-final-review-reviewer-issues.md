---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T03:57:46Z
---

# Final Review: AMENDMENTS

## Amendment: RB-01 [P1]

### Problem
The rollback ceiling can be bypassed in a crash/failure window, which can resurrect stale checkpoint state after a soft rollback.

In [`reconstruct_project_state_internal`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:285), capping is only applied when `checkpoint_loop > ceiling && max_artifact_loop <= ceiling` ([`lifecycle.rs:292`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs:292)).  
But loop artifacts are written before checkpoint commit in planning ([`orchestrator.rs:635`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/workflow/orchestrator.rs:635)), and the checkpoint is only created later at phase-transition commit ([`orchestrator.rs:2662`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/workflow/orchestrator.rs:2662)).  
If a run fails after creating `loops/002-*` but before checkpointing, `max_artifact_loop` becomes `2` while stale checkpoint may still be `3`; current logic disables capping and can jump back to stale loop `3`.

### Proposed Change
Use a stronger enforcement condition that still allows real forward progress but blocks stale resurrection from checkpoint/artifact mismatch. Example: enforce capping when `checkpoint_loop > ceiling` **and** checkpoint is ahead of artifact progress (e.g. `checkpoint_loop > max_artifact_loop`), not just when `max_artifact_loop <= ceiling`.

Add a regression test for the crash window:
- ceiling = 1
- stale checkpoint loop = 3
- artifacts exist for loops 1 and 2 only
- reconstruction must not restore loop 3.

### Affected Files
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs) - tighten rollback-ceiling enforcement logic.
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/project/lifecycle.rs) - add unit test covering partial post-rollback artifact creation before checkpoint.

## Amendment: RB-02 [P2]

### Problem
`rollback_push_failure_continues` does not actually prove session invalidation behavior; it can pass vacuously with an empty session store.

In [`tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1192), the test checks remaining records are `<= 1` ([`tests_commands.rs:1247`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs:1247)) but never asserts there were loop `>1` records before rollback. With default config, this assertion can pass without exercising invalidation.

### Proposed Change
Establish and assert a precondition with session records for loop `2` before rollback, then assert those records are removed after `rollback --hard 1` with push failure.

### Affected Files
- [`src/validate/tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - make session invalidation assertion non-vacuous.

---

## Context Provided
- Diff reviewed from merge-base `f0ce67e63ca66d1aec8bec01b33af6fa612e1d51` with:  
  `git diff <base>...HEAD -- . ':(exclude).ralph'`
- Changed files reviewed in detail:
  - `src/cli/rollback.rs`
  - `src/project/lifecycle.rs`
  - `src/validate/tests_commands.rs`
- Verification commands run:
  - `nix develop -c cargo test reconstruct_ -- --nocapture`
  - `nix build -L`
  - `./result/bin/ralph validate --bin ./result/bin/ralph --filter rollback --jobs 1`

## Master Prompt
Audit focused on correctness/safety first, including rollback behavior, checkpoint reconstruction safety, and whether validate assertions prove claimed behavior under failure paths.

## Summary
Most of the implementation is solid and rollback-focused conformance tests pass. The primary remaining risk is a real checkpoint-resurrection edge case in rollback-ceiling reconstruction logic, plus one validate test that currently passes without proving the session invalidation claim.
