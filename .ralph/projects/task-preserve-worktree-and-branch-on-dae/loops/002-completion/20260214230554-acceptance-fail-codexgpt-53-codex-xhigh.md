---
artifact: acceptance-fail
loop: 2
project: task-preserve-worktree-and-branch-on-dae
backend: codex(gpt-5.3-codex-xhigh)
role: qa
created_at: 2026-02-14T23:05:54Z
---

# QA: FAIL
## Failures
1. Required CAS-failure validation for terminal `Failed` is not actually covered by the new test. `runtime_activation_failed_task_preserved` is a startup-reconciliation test (`src/validate/tests_daemon.rs:1847`), seeds the task already as `"failed"` (`src/validate/tests_daemon.rs:1861`), and therefore never drives dispatch activation race logic.
2. Because only `pending` tasks are dispatched (`src/daemon/runtime.rs:192`), the above test cannot execute the CAS-failure cleanup branch in `dispatch_task` (`src/daemon/runtime.rs:481`), so the acceptance requirement “force activation CAS failure where persisted task is already terminal Failed” is unproven.
3. Required regression coverage set is incomplete by acceptance spec: `daemon::runtime_failed_worktree_preserved_and_reused_on_retry` is not present in current validate inventory, and current nearby tests do not explicitly assert aborted/succeeded cleanup semantics (`src/validate/tests_daemon.rs:1675`, `src/validate/tests_daemon.rs:3325`).

## Suggested Fixes
1. Replace or rewrite `runtime_activation_failed_task_preserved` to force a real activation CAS race: start from `pending`, make persisted state transition to `failed` between spawn and CAS update, then assert worktree is preserved and cleanup is skipped for `Failed`.
2. Add explicit conformance tests for aborted cleanup and succeeded cleanup worktree removal, plus retry-reuse coverage for failed-task preserved worktrees.
3. Re-run and record: `nix develop -c cargo test` and `./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon` after adding the missing coverage.
