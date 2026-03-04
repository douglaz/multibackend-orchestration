---
artifact: final-review-proposals
loop: 18
project: issue-146
backend: codex
role: final_reviewer
created_at: 2026-03-04T12:16:13Z
---

Review started. I’ll identify the correct base branch, run the requested `git diff <base>...HEAD -- . ':(exclude).ralph'`, then inspect the changed files and tests for correctness/safety issues before issuing amendments if needed.
I found the branch context (`ralph/issue-146` against `master`) and will now run the exact three-dot diff excluding `.ralph`, then inspect each touched source and test file.
I have the changed file set and I’m moving through the high-risk code paths first: the new quick-dev orchestrator, parser contracts, state persistence, and daemon dispatch. I’ll then validate tests and look for stray/out-of-scope artifacts.
I’ve finished the first pass over the orchestrator core; next I’m checking CLI entrypoints and daemon command builders for dispatch errors, argument propagation bugs, and incorrect defaults.
I’m now validating parser strictness and state/serialization changes, then I’ll cross-check that tests actually assert the intended behavior rather than incidental artifacts.
I’m in the conformance suite now. I’ll check each quick-dev validate case to confirm it proves the claimed behavior and isn’t passing by side effect (especially resume and guard tests).
I’ll now inspect daemon/process test coverage and command construction details to catch argument-order bugs, missing flags, or regressions in spawn/process-group handling.
I found a high-risk area in project state reconstruction and I’m validating it now with targeted checks, because it may overwrite checkpoint-derived phase data for non-quick workflows.
I’m running a focused test subset to validate the suspected crash-resume edge cases and confirm whether the behavior matches the intended quick-dev reliability guarantees.
# Final Review: AMENDMENTS

## Amendment: QD-CRASH-TRANSITION-STATE-001

### Problem
Crash-resume can re-run the wrong phase after `ChangesRequested` / `IssuesFound`, which can consume guard counters without executing required fix phases.

- In `CodexReview -> ApplyFixes`, review iteration is incremented and persisted ([src/workflow/quick_dev_orchestrator.rs:441](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:441), [src/workflow/quick_dev_orchestrator.rs:445](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:445)), but the phase is not durably switched to `ApplyFixes` before the transition checkpoint ([src/workflow/quick_dev_orchestrator.rs:499](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:499)-[513](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:513)).
- In `FinalReview -> PlanAndImplement` (issues path), final-review attempts are incremented and persisted ([src/workflow/quick_dev_orchestrator.rs:713](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:713)-[717](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:717)), but phase is not durably switched to `PlanAndImplement` before checkpoint ([src/workflow/quick_dev_orchestrator.rs:770](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:770)-[790](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:790)).

If the process crashes between checkpoint and next loop persist, resume restarts in the previous phase, which can prematurely hit `max_review_iterations` / `max_final_review_retries` without actually running `ApplyFixes` / `PlanAndImplement`.

### Proposed Change
Persist the target quick-dev phase before transition checkpoints on non-terminal transitions (at minimum: `CodexReview -> ApplyFixes` and `FinalReview -> PlanAndImplement`; ideally all transitions for consistency). Add regression tests that simulate crash at these boundaries and assert resume executes the intended next phase, not the previous one.

### Affected Files
- [src/workflow/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs) - persist transition target phase before checkpoint for crash-durable resume semantics.
- [tests/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs) - add boundary-crash regression assertions for phase progression.

## Amendment: QD-STRAY-ROOT-ARTIFACT-002

### Problem
A non-source artifact file is committed at repo root: [20260304T103437-impl-notes.md](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T103437-impl-notes.md). This is implementation-loop metadata and appears out of scope for production code.

### Proposed Change
Remove the file from the commit history for this feature branch (or move it to an explicitly ignored artifact location if it must be retained locally).

### Affected Files
- [20260304T103437-impl-notes.md](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T103437-impl-notes.md) - delete from tracked source changes.

---
