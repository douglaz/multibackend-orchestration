---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T20:45:46Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Keep Backend Kill Guard Armed Until Non-zero Exit Cleanup
### Problem
In [`src/backend/mod.rs:782`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:782), `kill_guard` is disarmed at line 806 before checking `status.success()` (line 808).  
If a backend exits non-zero after spawning a detached descendant that closes stdio quickly, the function returns `BackendCommandFailed` without `kill_and_reap_child`, and the disarmed guard cannot clean up the descendant process group.

### Proposed Change
Move `kill_guard.disarm()` to the true success path only (`status.success()`), and explicitly kill/reap on non-zero status before returning error.  
Add a regression test where a backend exits 1 but leaves a detached child alive with redirected stdio, and assert the child is terminated.

### Affected Files
- [`src/backend/mod.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs) - fix disarm/cleanup ordering and add regression coverage.

## Amendment: [P2] Restore End-to-End Assertions in `pr_metadata_verification`
### Problem
`pr_metadata_verification` no longer validates daemon-driven `gh pr create` behavior. In [`src/validate/tests_e2e_conformance.rs:367`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:367), it sets up GH logging, but assertions at [`:406`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:406) only validate helper output (`build_pr_title`/`build_pr_body`) rather than observed daemon PR creation.  
This test can pass even if PR creation integration regresses.

### Proposed Change
Reintroduce checks that `gh_log_path` exists and contains expected `gh pr create` args (`--title`, `--head`, `--body-file`) and expected body content sections.  
Keep helper-only validation in the separate `e2e_pr_create_body_file_verification` test.

### Affected Files
- [`src/validate/tests_e2e_conformance.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs) - strengthen E2E assertions to match test intent.

## Amendment: [P2] Remove Global Env Race in `e2e_pr_create_body_file_verification`
### Problem
In [`src/validate/tests_e2e_conformance.rs:503`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:503), the test mutates process-global `PATH` and `RALPH_E2E_GH_LOG` via `set_var/remove_var`.  
The validate runner defaults to parallel jobs ([`src/validate/mod.rs:91`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/mod.rs:91)), so this introduces cross-test interference and flaky outcomes.

### Proposed Change
Protect env mutations with a global mutex + RAII restore guard, or avoid global mutation entirely by exercising the code path in a subprocess with explicit `.env(...)`.  
Ensure restoration happens even on panic.

### Affected Files
- [`src/validate/tests_e2e_conformance.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs) - make env handling deterministic under parallel conformance execution.
