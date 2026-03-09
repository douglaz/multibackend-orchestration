# Final Review Amendments Applied

## Round 1

### Amendment: FR-194-001

### Problem
Completion is only guarded against pending queue items at planner decision time ([`src/workflow/orchestrator.rs:739`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:739)).  
The run can still return completed without a second queue check ([`src/workflow/orchestrator.rs:2797`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:2797)).  
If an amendment arrives during completing/final-review windows, this run may still report success as completed while leaving pending amendments unprocessed.

### Proposed Change
Add a final pending-queue check immediately before the completed return path. If `pending_amendment_count > 0`, do not finalize completion in that run (either error out with count or transition back to planning). Add a conformance test that enqueues during late phases and verifies completion is blocked.

### Affected Files
- [`src/workflow/orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs) - add late-stage queue guard before final completed return.
- [`src/validate/tests_amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs) - add coverage for amendment arrival after planner completion request.

### Reviewer
codex

### Amendment: FR-194-002

### Problem
`amend_cli_multiple_amendments_drain_in_order` claims order verification but only checks membership with `contains` ([`tests/amend_cli.rs:191`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs:191)-[`tests/amend_cli.rs:221`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs:221)).  
This test passes even if drain ordering regresses.

### Proposed Change
Assert the exact drained ID sequence (or rename the test to remove the order claim). Prefer exact sequence to preserve intended contract.

### Affected Files
- [`tests/amend_cli.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs) - make assertion match test intent.

---

### Reviewer
codex

