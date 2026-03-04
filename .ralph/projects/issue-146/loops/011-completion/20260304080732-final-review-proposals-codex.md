---
artifact: final-review-proposals
loop: 11
project: issue-146
backend: codex
role: final_reviewer
created_at: 2026-03-04T08:07:32Z
---

Reviewing the branch as a final auditor now. I’ll start by identifying the base branch, run the required `git diff <base>...HEAD -- . ':(exclude).ralph'`, then read the changed files and tests for correctness/safety issues.
I identified `master` as the base branch and will now inspect the exact delta with the required diff command, then walk through the highest-risk files and tests.
I have the full change list and I’m moving through the high-risk files first: `quick_dev_orchestrator`, parser/state changes, and daemon dispatch/build-command paths, then I’ll validate whether tests actually assert the intended behavior.
I’ve reviewed the orchestrator core and I’m now checking parser/state/config/daemon changes for edge-case bugs and mismatches that tests might miss.
I’m now auditing daemon routing and process command construction, since mis-threaded labels/args there can silently pick the wrong orchestration flow.
I’m in the conformance suite now (`tests_quick_dev` + mocks) to verify the tests genuinely prove the behaviors they claim, not just pass through superficial assertions.
I’ve identified a few risk points and now I’m running targeted tests to confirm whether those paths are actually safe in practice, especially around quick-dev resume/counter persistence and daemon dispatch.
Targeted tests passed. I’m now validating one remaining behavioral gap I found in the quick-dev state accounting and then I’ll return the amendment list with exact file/line references.
# Final Review: AMENDMENTS

## Amendment: QD-CRASH-COUNTERS-001

### Problem
`quick-dev` counter state is not durably updated at the moment counters change.

- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs):427 increments `review_iteration` only in a local variable.
- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs):695 increments `final_review_attempts` only in a local variable.
- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs):721-724 force-completes and saves state without copying the incremented `final_review_attempts` into `state.quick_dev_final_review_attempts`.

If a crash/error occurs after increment but before the next loop-head persistence, guard accounting can be stale on resume. In force-complete, persisted attempt count is wrong.

### Proposed Change
Persist counters immediately when they change, not only at phase-loop entry.

- After `review_iteration += 1`, assign `state.quick_dev_review_iteration = review_iteration` and save state before transition/checkpoint work.
- After `final_review_attempts += 1`, assign `state.quick_dev_final_review_attempts = final_review_attempts` and save state before transition/checkpoint work.
- In force-complete path, ensure incremented attempt count is persisted in `state.json` before return.
- Add regression tests asserting persisted counter values in force-complete and transition-error paths.

### Affected Files
- [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs) - persist counter mutations at mutation points.
- [`tests/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs) - add assertions/tests for persisted counter accuracy.

## Amendment: QD-BACKEND-EQUALITY-002

### Problem
Distinct-backend validation is a raw string equality check, which is bypassable by formatting differences.

- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs):811-815 compares `implementer == reviewer` directly.

Semantically identical specs like `"claude"` vs `" claude "` can pass this check and still resolve to the same backend, violating the quick-dev “distinct backend specs” requirement.

### Proposed Change
Canonicalize both specs before comparison.

- Parse with `parse_backend_spec`, compare normalized `name` + `model` (+ optional flag if desired), and reject if semantically equal.
- Keep the existing clear error message.
- Add tests for whitespace-normalized equality rejection.

### Affected Files
- [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs) - normalize backend specs in `validate_distinct_backends`.
- [`src/validate/tests_quick_dev.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs) - add conformance coverage for normalization edge cases.

## Amendment: QD-STRAY-FILE-003

### Problem
A non-source, loop-specific notes artifact was committed in repo root:

- [`20260304T070323-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T070323-impl-notes.md)

This is unintended scope creep and repository noise outside `.ralph` runtime state.

### Proposed Change
Remove this file from the tracked source tree (or relocate to `.ralph` artifacts if it must be kept as runtime output).

### Affected Files
- [`20260304T070323-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T070323-impl-notes.md) - delete from version control.

---
