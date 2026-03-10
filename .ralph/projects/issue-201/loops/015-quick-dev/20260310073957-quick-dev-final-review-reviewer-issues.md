---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T07:39:57Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] PRR-FAILURE-ISOLATION

### Problem
A single staging/persistence failure can abort the entire PR-review phase and block unrelated tasks.

- In [src/daemon/pr_review.rs#L569](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L569) and [src/daemon/pr_review.rs#L576](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L576), `stage_amendment(...)` / `state.save(...)` use `?`.
- In [src/daemon/runtime.rs#L2510](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L2510), `poll_pr_reviews(...)` is also propagated with `?`.

If one task has a persistent I/O/path problem, `pr_review_phase` exits early each cycle and pre-staged amendments for other tasks are not dispatched.

### Proposed Change
Handle polling/staging failures per task/comment (log and continue), and make `pr_review_phase` resilient to polling errors so it still processes already-staged amendments discovered from task metadata.

### Affected Files
- [src/daemon/pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs) - isolate stage/save failures instead of aborting the function.
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - avoid phase-wide abort on polling failure.

## Amendment: [P1] PRR-LABEL-ROLLBACK-FAILURE-STATE

### Problem
Dispatch-failure rollback can silently fail and leave issues stuck in `ralph:in-progress` with no child process.

- In [src/daemon/runtime.rs#L2689](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L2689) to [#L2697](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L2697), rollback uses `let _ = ...` and ignores errors.
- Reconciliation of `in-progress -> ready` runs only at startup ([src/daemon/runtime.rs#L792](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L792)), so same-session recovery may never happen.

### Proposed Change
Do not ignore rollback errors. On rollback failure, log explicitly with issue/task identity and force a terminal label transition (for example `in-progress -> failed`) or enqueue a deterministic retry path so the task cannot remain silently stuck.

### Affected Files
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - make rollback failure handling durable and observable.

## Amendment: [P3] PRR-TEST-ASSERTION-STRENGTH

### Problem
Several new conformance tests can pass without proving their stated behavior.

- [src/validate/tests_pr_review.rs#L552](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs#L552) and [#L797](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs#L797) fall back to “dispatch attempted” checks if state files are not found, which can mask missing state-reset behavior.
- [src/validate/tests_pr_review.rs#L661](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs#L661) only checks rollback log content if the log file exists, allowing a pass even when rollback evidence is absent.

### Proposed Change
Require the expected artifacts (worktree `state.json`, label log) to exist and assert the exact fields/label transitions; remove permissive fallback branches that allow false positives.

### Affected Files
- [src/validate/tests_pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - tighten assertions to match test names and intended guarantees.

---

## Context Provided
- Diff reviewed with `git diff master...HEAD -- . ':(exclude).ralph'`.
- Key files audited: daemon runtime/github/pr_review integration, config wiring, amendments model, and new validate tests.
- Validation run: `nix develop -c cargo test daemon::github`, `nix develop -c cargo test daemon::pr_review`, `nix build -L`, and `./result/bin/ralph validate --bin ./result/bin/ralph --filter pr_review` (all passing).
