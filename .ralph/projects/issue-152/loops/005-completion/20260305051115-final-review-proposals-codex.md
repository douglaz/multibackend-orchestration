---
artifact: final-review-proposals
loop: 5
project: issue-152
backend: codex
role: final_reviewer
created_at: 2026-03-05T05:11:15Z
---

Reviewing the completed changes now. I’ll start by identifying the base branch and running the required `git diff <base>...HEAD -- . ':(exclude).ralph'`, then I’ll trace the modified orchestrator and validate tests for correctness and safety.
I’ve run the required diff and identified the main touched areas: `src/workflow/orchestrator.rs`, new resume-resolution conformance tests, harness/assertion helpers, and an added top-level notes file. Next I’m reading the exact code paths and tests to check for functional and safety issues.
I’m now tracing the `Completing` and `FinalReview` paths end-to-end, including resume gating and warning behavior, since that’s where subtle regressions are most likely.
I’ve verified the main orchestrator changes and now I’m checking behavior via targeted test runs plus deeper test-quality review, especially around whether assertions actually prove backend selection instead of just log text.
I’ve finished the requirement-by-requirement trace and I’m now checking a subtle edge case: whether `is_resumed_state` could incorrectly trigger resume-only panel re-resolution in first-iteration paths that originate from planning.
I’ve completed the audit: runtime orchestrator changes look consistent with the intended resume behavior, but I found high-signal gaps in the new conformance tests (they can pass without proving the claimed backend-selection guarantees) plus one likely stray file. I’m preparing the amendment list now.
# Final Review: AMENDMENTS

## Amendment: FR-152-001

### Problem
The same-run gating regression test does not actually prove “no panel re-resolution/no re-health-check.”  
In [`tests_resume_backend_resolution.rs:1159`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:1159), the test only asserts absence of a drift-warning substring pair, and the predicate at [`:1178`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:1178) (`!A || !B`) can pass even if `resolve_completion_panel(...)` is incorrectly called on same-run entry (when no drift occurs).

### Proposed Change
Make same-run panel re-resolution observable and assert on that signal:
1. Configure `completion_backends` to include an unavailable optional backend (e.g. `?gemini`) so `resolve_completion_panel(...)` emits a deterministic skip warning when called.
2. In `same_run_completion_no_panel_reresolution`, assert that warning appears exactly once (planning-time resolution only), not twice.
3. Keep the no-drift warning assertion as a secondary check.

### Affected Files
- [`src/validate/tests_resume_backend_resolution.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs) - strengthen same-run gating test to prove no second panel resolution call.

## Amendment: FR-152-002

### Problem
Several new resume tests assert only generic warning text and do not prove execution used re-resolved backends:
- completion planner drift test checks only message/role at [`:969`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:969)
- completer-panel drift test checks only warning fields at [`:1043`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:1043)
- final-review planner drift test checks only message/role at [`:1122`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:1122)

These can pass even if warning is emitted but stale backend is still used for execution.

### Proposed Change
Add execution-proof assertions:
1. Assert expected `original=` and `resolved=` values for completion/final-review planner drift warnings.
2. For panel drift, assert produced completer verdict artifact backend matches the re-resolved panel backend (not reconstructed one).
3. For final-review planner drift, drive a path that requires planner execution (amendments path) and assert planner-generated artifact frontmatter backend equals resolved planner.

### Affected Files
- [`src/validate/tests_resume_backend_resolution.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs) - strengthen drift tests so they verify execution backend selection, not only warning presence.

## Amendment: FR-152-003

### Problem
A top-level implementation diary file was added and is not part of runtime/test behavior for this feature:
- [`20260305-impl-notes.md:1`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/20260305-impl-notes.md:1)

This appears to be scope-external noise in source changes.

### Proposed Change
Remove the file from the change set (or move it to a non-versioned/local workflow location).

### Affected Files
- [`20260305-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/20260305-impl-notes.md) - delete as stray artifact.

---

## Context Provided
- Master prompt focused on resume backend re-resolution behavior in `orchestrator.rs`, drift logging semantics, and new validate conformance coverage.
- Project state summary indicated this round is final review after completion attempts.
