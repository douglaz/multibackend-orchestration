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


## Round 2

### Amendment: A-194-REVIEW-001

### Problem
Queued amendments can be silently lost on phase failure.

In standard orchestration, amendments are drained and deleted before planner execution ([`src/workflow/orchestrator.rs#L603`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs#L603)), then the run can fail later during prompt build/backend execution ([`src/workflow/orchestrator.rs#L623`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs#L623), [`src/workflow/orchestrator.rs#L660`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs#L660)) with no requeue path.

Quick-dev has the same pattern: drain first ([`src/workflow/quick_dev_orchestrator.rs#L345`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs#L345)), then backend call can fail ([`src/workflow/quick_dev_orchestrator.rs#L363`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs#L363)) and drained items are gone.

This violates safety for external amendment intake under transient backend/template failures.

### Proposed Change
Make drain handling at-least-once for phase failures.

1. Keep drained amendments in memory for the active phase.
2. If the phase errors before a durable success transition, re-enqueue the drained amendments with original fields (`id`, `body`, `priority`, `source`, `source_detail`, `created_at`).
3. Add regression tests that intentionally fail immediately after drain and assert queue contents are preserved for retry.

### Affected Files
- [`src/workflow/orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs) - protect planning-drained amendments from loss on downstream errors.
- [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs) - same protection for quick-dev `PlanAndImplement`.
- [`src/validate/tests_amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs) - conformance coverage for drain+failure persistence.

---

### Reviewer
codex


## Round 3

### Amendment: AMEND-QUEUE-LOSS-001

### Problem
`drain_amendment_queue_with_hook` can delete already-processed queue items and still return `Err` on a later file operation, which creates a loss path for drained amendments.  
Key points:
- It processes files incrementally and deletes each parsed inflight file ([src/project/amendments.rs:239](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs:239)).
- Any later `?`-propagated IO error aborts the whole drain ([src/project/amendments.rs:168](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs:168)).
- Callers treat drain failure as fatal and cannot rollback because they never receive the partial drained vector ([src/workflow/orchestrator.rs:604](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:604), [src/workflow/quick_dev_orchestrator.rs:347](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs:347)).

### Proposed Change
Make drain failure non-lossy:
1. On fatal mid-drain error, best-effort re-enqueue already drained items before returning `Err`.
2. Add a unit test that injects a mid-drain failure and asserts no amendment disappears.

### Affected Files
- `src/project/amendments.rs` - add internal rollback-on-error behavior in drain path and test coverage.

### Reviewer
codex

### Amendment: AMEND-TEST-SEMANTICS-002

### Problem
The conformance test `quick_dev_checkpoint_failure_no_rollback_after_durable_success` does not actually assert that the checkpoint failure path occurred; it ignores command status (`let _output = ...`) and only checks queue emptiness ([src/validate/tests_amendments.rs:736](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs:736), [src/validate/tests_amendments.rs:775](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs:775)).  
That means the test can pass even when no checkpoint failure happened, so the name/claim is stronger than what it proves.

### Proposed Change
Make the test prove the intended path:
1. Assert non-zero run result and checkpoint/commit failure evidence in stderr, or
2. If deterministic failure cannot be guaranteed, rename the test to reflect current semantics and add a separate deterministic failure-path test.

### Affected Files
- `src/validate/tests_amendments.rs` - tighten assertions (or rename + split test).

---

### Reviewer
codex

### Amendment: AMQ-001

### Problem
Two implementation artifact files from Loop 8 were committed to the branch root:
- `20260309T165947Z-impl-notes.md`
- `20260309T165947Z-impl-response-001.md`

These are workflow-generated response files (committed in `b112058` and `6a606dc`) and do not belong in the source tree. They would be included in any merge to master.

### Proposed Change
`[P2]` Remove both files and commit the deletion.

### Affected Files
- `20260309T165947Z-impl-notes.md` — delete
- `20260309T165947Z-impl-response-001.md` — delete

---

### Reviewer
claude

