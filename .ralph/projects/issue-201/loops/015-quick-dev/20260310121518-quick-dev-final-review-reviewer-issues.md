---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T12:15:18Z
---

# Final Review: AMENDMENTS

## Amendment: PRR-001 [P1]

### Problem
PR-review resume assumes lifecycle swap is atomic, but it is implemented as separate remove/add operations in [src/daemon/github.rs:1434](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:1434).  
If `remove` succeeds and `add` fails, the issue can lose its lifecycle label. In that state, PR-review recovery may clear or mis-handle resume markers in failure branches ([src/daemon/runtime.rs:2701](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2701), [src/daemon/runtime.rs:2755](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2755)), which can leave staged PR-review work stranded until manual intervention.

### Proposed Change
Make label swap failure-safe:
1. In `swap_lifecycle_label`, if add fails after remove, best-effort re-add `from_label` before returning error.
2. Return enough error context to indicate whether rollback to `from_label` succeeded.
3. In PR-review swap-failure handling, clear resume markers only when label restoration is confirmed.
4. Add a conformance test that simulates remove-success/add-fail and asserts label + marker recovery behavior.

### Affected Files
- [src/daemon/github.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs) - harden lifecycle swap semantics.
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - make marker handling conditional on confirmed rollback state.
- [src/validate/tests_pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - add partial-swap failure regression test.

## Amendment: PRR-002 [P3]

### Problem
`pr_review::completed_project_resumes_with_state_reset` does not verify the “state reset” it claims to cover ([src/validate/tests_pr_review.rs:164](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs:164)).  
Current assertions validate label/log/staging behavior only, so a regression in project-state reset could still pass.

### Proposed Change
Extend the test to inspect resumed project state in the worktree and assert reset fields (at minimum `status == "in_progress"` for regular projects).

### Affected Files
- [src/validate/tests_pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - strengthen assertions to match test intent.

---

## Context Provided
- Reviewed diff with `git diff 67ed2ccb9bdc2e1e4a453d0e48d2a2bee8911b7b...HEAD -- . ':(exclude).ralph'`.
- Verified changed modules and tests (`runtime`, `pr_review`, `github`, config wiring, validate registration).
- Executed:
  - `nix develop -c cargo test pr_review -- --nocapture` (pass)
  - `nix build -L` (pass, including full checkPhase tests)
  - `./result/bin/ralph validate --bin ./result/bin/ralph --filter pr_review` (11/11 pass)
