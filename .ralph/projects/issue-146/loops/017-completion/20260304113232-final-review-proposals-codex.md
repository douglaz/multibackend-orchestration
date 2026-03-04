---
artifact: final-review-proposals
loop: 17
project: issue-146
backend: codex
role: final_reviewer
created_at: 2026-03-04T11:32:32Z
---

Reviewing the completed quick-dev implementation by diffing the branch against its base and then tracing the changed code paths for correctness, safety, resume behavior, and test validity. I’ll start by identifying the base branch and collecting the full changed-file diff set.
I have the branch context and started diff inspection; next I’m narrowing to a clean changed-file list and then reviewing each critical module (orchestrator, parser, CLI, daemon dispatch, state/config, and tests) for real failure modes.
I’ve captured the full patch and file list. I’m now auditing the orchestrator and parser/state paths first since that’s where crash safety, resume behavior, and backend isolation risks usually hide.
I’ve walked the core quick-dev state machine and parsing entry points; next I’m checking helper functions and tests for edge-case correctness (especially resume, counters, and parser strictness).
I found one likely daemon-label interaction risk and I’m now tracing runtime/process command dispatch end-to-end to confirm whether `ralph:quick` issues are actually claimable and routed correctly.
I’ve validated the daemon dispatch path and command builders; next I’m checking test harness scripts and conformance assertions for cases that could pass despite real regressions (false-positive tests).
I’ve finished static review of most changed modules and I’m running targeted test subsets next to validate the suspect edges (resume state handling, quick-label dispatch path, and quick-dev guards) against actual execution.
# Final Review: AMENDMENTS

## Amendment: AMEND-QD-CRASH-GUARD-001

### Problem
`quick-dev` guard enforcement is not crash-durable in two counter-persist windows.

In [src/workflow/quick_dev_orchestrator.rs:443](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:443), `review_iteration` is persisted before the max-review guard check at [line 447](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:447).  
In [src/workflow/quick_dev_orchestrator.rs:715](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:715), `final_review_attempts` is persisted before the max-final-review-retries guard check at [line 719](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:719).

If the process crashes between persistence and guard evaluation, resume re-enters `CodexReview` / `FinalReview` and executes backend calls again (see [FinalReview entry at line 582](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:582)) instead of immediately honoring already-reached limits. That can bypass intended guard outcomes after restart.

### Proposed Change
Add guard checks at phase entry, before any backend invocation:

1. In `CodexReview`: if `review_iteration >= max_review_iterations`, perform the warning/transition-to-`FinalReview` path immediately.
2. In `FinalReview`: if `final_review_attempts >= max_final_review_retries`, perform force-complete immediately (artifact + completed state + checkpoint), without running final-review backends.

Add regression tests that seed persisted maxed counters and assert resume enforces guard behavior without extra review calls.

### Affected Files
- [src/workflow/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs) - enforce guard-at-entry logic for crash-durable resume.
- [tests/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs) - add resume tests for pre-guard crash windows.

## Amendment: AMEND-REPO-STRAY-FILE-002

### Problem
A generated implementation artifact file was committed at repo root: [20260304T103437-impl-notes.md](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T103437-impl-notes.md). This is outside the runtime/source tree and appears to be stray output, not product code.

### Proposed Change
Remove the stray root artifact from version control.

### Affected Files
- [20260304T103437-impl-notes.md](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T103437-impl-notes.md) - delete file.

---
