---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T17:58:31Z
---

# Final Review: AMENDMENTS

## Amendment: DAEMON-CONCURRENCY-FAILURE-PATH [P1]

### Problem
`completion_failure_terminalization` claims to verify execution failure terminalization, but currently passes on cancellation semantics instead of a true task failure.

- The test description and strategy explicitly reference child/command failure (`MOCK_RALPH_EXIT_CODE=1`) at [src/validate/tests_daemon_concurrency.rs:712](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:712) and [src/validate/tests_daemon_concurrency.rs:717](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:717).
- The actual daemon invocation does not inject a failing backend/subprocess env in this test setup ([src/validate/tests_daemon_concurrency.rs:736](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:736)-[src/validate/tests_daemon_concurrency.rs:752](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:752)).
- In single-iteration mode, runtime always calls `drain_all_children` ([src/daemon/runtime.rs:875](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:875)-[src/daemon/runtime.rs:878](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:878)), which cancels all active tasks up front ([src/daemon/runtime.rs:2019](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:2019)-[src/daemon/runtime.rs:2026](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:2026)).
- Cancellation still yields `ralph:failed`, so current assertions ([src/validate/tests_daemon_concurrency.rs:764](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:764)-[src/validate/tests_daemon_concurrency.rs:795](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:795)) can pass without proving the failure path they claim.

### Proposed Change
Make the test intention and mechanics align:

1. Keep the existing single-iteration test but rename/reword it to cancellation terminalization semantics.
2. Add a separate failure-path conformance test that deterministically forces backend execution failure (non-zero exit), avoids drain-induced cancellation as the primary signal, and asserts failure came from task execution (e.g., `collect: ... failed:` path plus label transition).

### Affected Files
- [`src/validate/tests_daemon_concurrency.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs) - fix/rename current test and add a true execution-failure terminalization test.

## Amendment: VALIDATE-DEAD-MOCKS [P3]

### Problem
Subprocess-era daemon mock helpers remain in `mock_scripts.rs` but are unreferenced in the codebase, which increases maintenance noise and can mislead future test additions.

- Example unreferenced helper: [src/validate/mock_scripts.rs:3958](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/mock_scripts.rs:3958).
- Other `daemon_mock_ralph_*` helpers in the same file are similarly definition-only (no call sites).

### Proposed Change
Remove unused `daemon_mock_ralph_*` helpers that are no longer used after in-process dispatch migration. If any are intentionally retained for near-term work, mark explicitly with a comment and `#[allow(dead_code)]` to avoid ambiguity.

### Affected Files
- [`src/validate/mock_scripts.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/mock_scripts.rs) - delete or explicitly annotate unused legacy daemon mock helpers.

---
