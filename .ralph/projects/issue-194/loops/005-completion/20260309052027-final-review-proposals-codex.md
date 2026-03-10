---
artifact: final-review-proposals
loop: 5
project: issue-194
backend: codex
role: final_reviewer
created_at: 2026-03-09T05:20:27Z
---

Review underway. I’ll start by identifying the correct base commit, run the requested `git diff <base>...HEAD` (excluding `.ralph`), then trace the changed files and tests for correctness, safety, and robustness.
I found the branch state and there are only untracked `.ralph` artifacts locally. Next I’ll diff against the merge-base with `origin/master` to isolate the full implementation delta you want reviewed.
I have the change list. I’m now tracing each modified module in detail, starting with the queue implementation and orchestration call sites, then I’ll validate test quality and look for hidden regressions or race/error-path gaps.
I’ve finished the queue module pass and will now trace config/CLI wiring and how these options flow into runtime behavior, including merge precedence and command validation paths.
Next I’m reviewing CLI and config command behavior to ensure runtime flags, defaults, and validation failures are wired correctly and that tests prove the right behavior rather than just argument parsing.
I’ve finished source tracing and identified a few potential risk areas. I’m now running targeted tests (unit + integration for amendments paths) to confirm whether the suspected issues are real failures or test-coverage gaps.
Unit/integration tests are green so far. I’m running the new conformance suite segment (`validate --filter amendments::`) to verify the end-to-end CLI/orchestrator behavior, not just isolated modules.
I’m waiting on the full `nix build -L` completion; it’s currently executing the full release test/check phases and has progressed through the new amendment-related tests successfully.
# Final Review: AMENDMENTS

## Amendment: FR-194-001 [P2]

### Problem
Completion is only guarded against pending queue items at planner decision time ([`src/workflow/orchestrator.rs:739`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:739)).  
The run can still return completed without a second queue check ([`src/workflow/orchestrator.rs:2797`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:2797)).  
If an amendment arrives during completing/final-review windows, this run may still report success as completed while leaving pending amendments unprocessed.

### Proposed Change
Add a final pending-queue check immediately before the completed return path. If `pending_amendment_count > 0`, do not finalize completion in that run (either error out with count or transition back to planning). Add a conformance test that enqueues during late phases and verifies completion is blocked.

### Affected Files
- [`src/workflow/orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs) - add late-stage queue guard before final completed return.
- [`src/validate/tests_amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs) - add coverage for amendment arrival after planner completion request.

## Amendment: FR-194-002 [P3]

### Problem
`amend_cli_multiple_amendments_drain_in_order` claims order verification but only checks membership with `contains` ([`tests/amend_cli.rs:191`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs:191)-[`tests/amend_cli.rs:221`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs:221)).  
This test passes even if drain ordering regresses.

### Proposed Change
Assert the exact drained ID sequence (or rename the test to remove the order claim). Prefer exact sequence to preserve intended contract.

### Affected Files
- [`tests/amend_cli.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs) - make assertion match test intent.

---
