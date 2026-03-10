---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T21:22:43Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Preserve Staged PR-Review Amendments When Task Metadata Is Corrupt

### Problem
`poll_and_claim` can silently delete staged PR-review amendments when task metadata JSON is corrupted.

- [`load_task_metadata` swallows parse failures and returns default metadata](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:720) (`unwrap_or_default` at line 723).
- In the PR-review ownership guard, that default is treated as “metadata missing / PR not open”, and staged artifacts are cleared:
  - [`pr_check_result` derivation](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1155)
  - [`clear_resume_pending_marker` + `clear_staged_amendments`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1210)

This is data-lossy for reviewer feedback and can happen with pre-existing corrupted metadata files.

### Proposed Change
Introduce a strict metadata read path for this guard:
1. Distinguish `NotFound` from parse/read errors.
2. Only clear staged PR-review artifacts on definitive stale cases (`NotFound` or PR definitively closed).
3. Treat parse/read errors as transient/unknown: log warning and skip claim for the cycle without clearing staged data.
4. Add a validate test for “corrupt metadata + staged amendments” asserting no staged-file deletion and no claim dispatch.

### Affected Files
- [`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - make metadata handling non-lossy in PR-review claim guard.
- [`src/validate/tests_pr_review.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - add regression coverage for corrupt metadata behavior.

## Amendment: [P2] Prevent Stale Final-Review Findings From Reappearing After Restart

### Problem
Quick-dev resume handoff reconstruction now ignores `*-complete.md` artifacts and can resurrect stale old findings.

- Resume path loads handoff whenever `PlanAndImplement && quick_dev_final_review_attempts > 0`:
  - [`resume handoff load`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs:304)
- Loader reads only `complete: false` artifacts (`*-issues.md`) for both roles:
  - [`load_final_review_findings` role suffixes](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs:1307)

If a prior issues round was later closed by newer `*-complete.md` and the daemon restarts in a later PlanAndImplement cycle (e.g. pre-commit failure path), old issues can be re-injected incorrectly.

### Proposed Change
Reconstruct handoff per role using both artifact types and recency:
1. For each role, resolve latest of `*-issues.md` vs `*-complete.md`.
2. Include role findings only when the latest artifact is `*-issues.md`.
3. Keep empty/no-handoff behavior when latest artifacts are complete or absent.
4. Restore regression tests for “newer complete suppresses older issues” (including same-timestamp tie behavior).

### Affected Files
- [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/workflow/quick_dev_orchestrator.rs) - fix resume handoff artifact selection logic.
- [`src/validate/tests_quick_dev.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_quick_dev.rs) - add restart/regression coverage for stale-hand-off suppression.

---
