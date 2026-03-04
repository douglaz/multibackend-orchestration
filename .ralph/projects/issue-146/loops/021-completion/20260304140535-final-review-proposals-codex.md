---
artifact: final-review-proposals
loop: 21
project: issue-146
backend: codex
role: final_reviewer
created_at: 2026-03-04T14:05:35Z
---

Reviewing the implementation directly from source now. I’ll identify the correct base branch, run the requested `git diff <base>...HEAD -- . ':(exclude).ralph'`, then audit changed code paths and tests for correctness/safety issues.
I’ve confirmed `master` as the base and started from the full diff. Next I’m enumerating changed files and auditing the core runtime paths first (`orchestrator`, `daemon`, `parser`, `state`), then test quality and stray artifacts.
I’m drilling into `quick_dev_orchestrator.rs` in full (not just diff headers) to verify phase transitions, persistence timing, and guard behavior under crash/resume paths.
I’ve verified the main phase loop and now I’m checking the helper logic and tests in the rest of the orchestrator file for hidden regressions (especially bounds, parser handling, and backend normalization).
I’m moving through the CLI, config merge, and daemon/process command-builder changes next, since those are where dispatch/runtime bugs typically hide.
I’ve confirmed CLI and process wiring; now I’m validating template/config resolution and parser/prompt contracts to ensure runtime and tests align.
Next I’m checking the non-quick files that were touched (`reconstruct`, artifacts, standard orchestrator, mock scripts) to catch unintended regressions outside quick-dev scope.
I’m validating reconstruction logic next, because state recovery changes can accidentally alter non-quick behavior if completion/status inference order is wrong.
I’ve completed static review of the core logic and I’m now running targeted test suites (`quick_dev_orchestrator` and quick-dev validate paths) to verify behavior matches what the code appears to do.
I validated the integration test crate; I’m now spot-checking conformance registration/execution for the new quick-dev tests and daemon quick-label dispatch coverage.
# Final Review: AMENDMENTS

## Amendment: QD-AMEND-001

### Problem
Quick-dev transition state is only persisted at loop entry, not at transition points. The phase is saved at the top of each iteration ([quick_dev_orchestrator.rs:297](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:297)), but several transitions only call `checkpoint_if_enabled(...)` and then mutate `current_qd_phase` in memory ([quick_dev_orchestrator.rs:409](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:409), [quick_dev_orchestrator.rs:499](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:499), [quick_dev_orchestrator.rs:565](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:565), [quick_dev_orchestrator.rs:770](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:770)).  
If the process crashes between checkpoint and next loop-entry persistence, resume re-enters the prior phase and can re-run non-idempotent LLM decisions (for example, re-running `CodexReview` instead of entering `ApplyFixes`), which can change outcomes and skip intended fix application.

### Proposed Change
Persist destination phase/counters immediately before each transition checkpoint (or immediately after decision, before return/continue), not only at next loop entry.  
Implement a shared helper for transition persistence to avoid missing branches, and update crash-resume tests to assert resume continues from persisted destination phase after an injected mid-transition failure.

### Affected Files
- [src/workflow/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs) - persist destination phase on all transitions
- [tests/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs) - tighten crash-transition resume assertions to verify destination-phase durability

## Amendment: QD-AMEND-002

### Problem
A root-level implementation-notes artifact was committed into source changes ([20260304T103437-impl-notes.md](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T103437-impl-notes.md)). This is a stray non-product file and outside intended runtime/test source scope.

### Proposed Change
Delete the file from the repository. If this note must be retained, move it to ephemeral orchestration output under `.ralph/` (which is excluded from source diffs).

### Affected Files
- [20260304T103437-impl-notes.md](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T103437-impl-notes.md) - remove stray artifact

---
