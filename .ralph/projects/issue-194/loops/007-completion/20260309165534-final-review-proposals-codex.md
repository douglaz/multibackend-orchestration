---
artifact: final-review-proposals
loop: 7
project: issue-194
backend: codex
role: final_reviewer
created_at: 2026-03-09T16:55:34Z
---

I’m auditing the completed changes directly from the repository state and will start by identifying the correct base commit, then running the requested `git diff ...` and tracing key code paths and tests for correctness/safety.
I identified the active branch (`ralph/issue-194`) and I’m now diffing it against its merge-base with `master` to inspect the full code delta (excluding `.ralph`) before tracing critical paths.
I’ve confirmed the exact changed file set; next I’m reading the new queue/model code and orchestrator integrations line-by-line to check crash-safety, concurrency handling, and error paths.
I’ve loaded the primary files and now I’m narrowing to the exact hunks in each changed file so I can validate the new logic against existing control flow and persistence behavior.
I’m moving through CLI/config and test additions next to verify input validation, default handling, and whether tests truly assert the intended behavior rather than incidental side effects.
I’m now auditing the added conformance and integration tests in detail, including mock backend scripts, to confirm they validate the real behaviors (drain/inject/guard/mirroring) and don’t pass for accidental reasons.
I’ve covered the tests and am now tracing the orchestrator’s changed execution paths (planning drain, completion guards, final-review mirroring, prompt assembly) with exact line-level checks.
I’ve finished static tracing of the changed paths and I’m now executing focused test runs (queue unit tests + CLI integration paths) to validate behavior under actual execution and catch any hidden regressions.
The environment lacks `cargo` directly, so I’m rerunning the same targeted tests through `nix develop -c` to match the project’s required build/test workflow.
Both test runs are in progress under Nix and currently waiting on shared build/cache locks; I’m polling them until completion and will use the results in the final audit findings.
Unit and integration tests for the new queue/CLI paths are passing. I’m running the new conformance test module next to verify the orchestration-boundary behaviors under the harness.
The previous command didn’t execute conformance cases (those aren’t Rust `#[test]`s), so I’m now running `ralph validate` with an amendments filter to execute the actual conformance suite entries.
# Final Review: AMENDMENTS

## Amendment: A-194-REVIEW-001 [P1]

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
