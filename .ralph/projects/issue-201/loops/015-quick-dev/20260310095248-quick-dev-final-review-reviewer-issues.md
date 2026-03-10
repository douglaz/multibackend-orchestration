---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T09:52:48Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Block `PrReviewResume` from falling back to fresh dispatch

### Problem
`pr_review_phase` builds a synthetic placeholder idea and dispatches with `DispatchOrigin::PrReviewResume` ([`src/daemon/runtime.rs:2692`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2692)).  
`dispatch_task` assumes the project is resumable, but does not enforce it after `should_resume_issue_project` ([`src/daemon/runtime.rs:1450`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1450)). If resume detection fails, it proceeds through the fresh-dispatch path ([`src/daemon/runtime.rs:1513`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1513), [`src/daemon/runtime.rs:1640`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1640), [`src/daemon/runtime.rs:1694`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1694)) using that placeholder prompt.  
That can start an unintended new implementation cycle and leave PR-review intent unfulfilled.

### Proposed Change
If `origin == DispatchOrigin::PrReviewResume` and `resume_existing_project == false`, fail fast and return an error so `pr_review_phase` rolls labels back without spawning a fresh task. Add a conformance test for missing/corrupt resume project state to verify rollback and staged amendment preservation.

### Affected Files
- [`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - enforce resume-only invariant for `PrReviewResume`.
- [`src/validate/tests_pr_review.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - add regression coverage.

## Amendment: [P1] Recover correctly from crash after PR-review dispatch succeeds but before amendments are consumed

### Problem
Staged files are purged immediately after spawn success ([`src/daemon/runtime.rs:1774`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1774)) and the resume marker is cleared immediately on dispatch success ([`src/daemon/runtime.rs:2703`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2703)).  
Candidate discovery later only considers newly staged or still-staged tasks ([`src/daemon/runtime.rs:2520`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2520), [`src/daemon/runtime.rs:2556`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2556)).  
For quick-dev, a resumed completed state can still early-exit if not reset ([`src/workflow/quick_dev_orchestrator.rs:157`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs:157)).  
So a daemon crash in that window can strand amendment intent (no staged files, no marker, ready label after reconciliation).

### Proposed Change
Make crash recovery source-of-truth durable until amendment processing is guaranteed:
1. Keep resume-pending marker until terminal completion handling, not immediate dispatch success.
2. Extend PR-review candidate discovery to include resume-pending/queued-amendment recovery signals (not only staged files).
3. Add conformance coverage for “crash after dispatch success” to prove amendments are still resumed.

### Affected Files
- [`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - marker lifecycle and recovery candidate logic.
- [`src/daemon/pr_review.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs) - helper(s) for recovery signal detection.
- [`src/validate/tests_pr_review.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - crash-window regression test.

## Amendment: [P2] Fix false-positive rollback assertion in PR-review conformance test

### Problem
`dispatch_failure_preserves_staged_amendments` asserts rollback by checking for `"ralph:completed"` in the label log ([`src/validate/tests_pr_review.rs:679`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs:679)).  
That can pass even without rollback, because the forward transition command already contains `--remove-label ralph:completed`.

### Proposed Change
Assert both transitions explicitly and in order:
1. `completed -> in-progress`
2. `in-progress -> completed`
Parse log lines as operations (or log structured fields) and verify the rollback operation is present as a distinct second transition.

### Affected Files
- [`src/validate/tests_pr_review.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - strengthen rollback verification.

---

## Context Provided
- Reviewed diff from `67ed2ccb9bdc2e1e4a453d0e48d2a2bee8911b7b...HEAD` with `.ralph/**` excluded.
- Audited all changed source modules, especially `src/daemon/runtime.rs`, `src/daemon/pr_review.rs`, `src/daemon/github.rs`, and `src/validate/tests_pr_review.rs`.
- Ran focused tests: `nix develop -c cargo test pr_review -- --nocapture`.
