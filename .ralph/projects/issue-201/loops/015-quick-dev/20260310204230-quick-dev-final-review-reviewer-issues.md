---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T20:42:30Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] QUICK-DEV-FINAL-REVIEW-HANDOFF-DROPPED

### Problem
Final-review findings are no longer carried into the next quick-dev implementer round after a `FinalReview -> PlanAndImplement` reloop.

Evidence:
- The reloop path only increments counters and transitions phase, without persisting/injecting findings ([quick_dev_orchestrator.rs:1022](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs:1022), [quick_dev_orchestrator.rs:1087](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs:1087)).
- `build_plan_implement_prompt` no longer accepts/inserts a final-review handoff payload ([quick_dev_orchestrator.rs:1390](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs:1390)).
- The default template no longer contains a final-review handoff section ([quick_dev.rs:50](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/prompts/quick_dev.rs:50)).

Impact: quick-dev can re-enter implementation without the specific blocking issues that triggered reloop, increasing risk of repeated no-op cycles and force-completion with unresolved defects.

### Proposed Change
Restore a durable final-review handoff for quick-dev reloops:
1. Reconstruct latest final-review issue findings (or enqueue them as amendments) when final review returns amendments.
2. Inject that handoff into the next PlanAndImplement prompt.
3. Reinstate a regression test that captures resumed implementer prompt content and asserts those findings are present.

### Affected Files
- `src/workflow/quick_dev_orchestrator.rs` - restore handoff capture/injection on reloop.
- `src/prompts/quick_dev.rs` - restore template slot for final-review handoff.
- `tests/quick_dev_orchestrator.rs` - restore integration assertion for resumed prompt handoff.
- `src/validate/tests_quick_dev.rs` - restore conformance coverage for this behavior.

## Amendment: [P2] VALIDATE-ENV-LOCK-SPLIT-CAUSES-CROSS-MODULE-RACES

### Problem
Validate tests run in parallel threads ([runner.rs:117](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/runner.rs:117)), but env-var mutation locking is now module-local (for example: [tests_pr_runtime.rs:515](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_runtime.rs:515), [tests_pr_lifecycle.rs:432](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_lifecycle.rs:432), [tests_e2e_conformance.rs:15](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_e2e_conformance.rs:15), [tests_daemon_rebase.rs:14](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_daemon_rebase.rs:14), [tests_interactive_prd.rs:22](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_interactive_prd.rs:22)).

Because env mutation is process-global, separate mutexes do not prevent cross-module races/flakiness during parallel validate runs.

### Proposed Change
Reintroduce one shared process-wide env lock in validate core and use it from every env-mutating test module (instead of per-module locks).

### Affected Files
- `src/validate/mod.rs` - add shared env mutex helper back.
- `src/validate/tests_pr_runtime.rs` - switch to shared lock.
- `src/validate/tests_pr_lifecycle.rs` - switch to shared lock.
- `src/validate/tests_e2e_conformance.rs` - switch to shared lock.
- `src/validate/tests_daemon_rebase.rs` - switch to shared lock.
- `src/validate/tests_interactive_prd.rs` - switch to shared lock.

---
