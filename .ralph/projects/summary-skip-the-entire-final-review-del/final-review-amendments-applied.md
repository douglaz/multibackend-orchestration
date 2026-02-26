# Final Review Amendments Applied

## Round 1

### Amendment: FR-CAP-SKIP-ORCH-TESTS-001

### Problem
The implementation correctly added the early cap guard at [`src/workflow/orchestrator.rs:3331`](/tmp/ralph-daemon/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-80/src/workflow/orchestrator.rs:3331), but two existing integration tests still assert the old behavior with `max_final_review_restarts = 0` and expect a first-run failure after proposal generation: [`tests/orchestrator.rs:3321`](/tmp/ralph-daemon/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-80/tests/orchestrator.rs:3321), [`tests/orchestrator.rs:3328`](/tmp/ralph-daemon/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-80/tests/orchestrator.rs:3328), [`tests/orchestrator.rs:3353`](/tmp/ralph-daemon/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-80/tests/orchestrator.rs:3353), [`tests/orchestrator.rs:3361`](/tmp/ralph-daemon/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-80/tests/orchestrator.rs:3361).  
With the new boundary semantics, cap `0` force-completes immediately and those assertions are no longer valid, causing `cargo test` to fail.

### Proposed Change
Update the stale integration tests to align with the new semantics:
1. For resume/config-mismatch tests whose purpose is proposal-resume/invalidation, use a positive restart cap (for example `1`) so deliberation can run before cap exhaustion.
2. Keep/adjust assertions to verify their original intent (resume skips proposal re-run; config mismatch invalidates cached proposal artifacts).
3. Optionally add an explicit integration assertion for the `max_final_review_restarts = 0` immediate-skip case (force-complete + no deliberation artifacts), so the new behavior is covered at this layer too.

### Affected Files
- `tests/orchestrator.rs` - update restart-cap setup and first-run expectations in the two failing final-review tests to match intentional cap-boundary behavior.

---

### Reviewer
codex

