---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T09:22:10Z
---

# Final Review: AMENDMENTS

## Amendment: [P0] Daemon Conformance Migration Is Incomplete (and Some Migrated Tests No Longer Prove Their Claims)

### Problem
The conformance suite still contains subprocess-era assertions based on `RALPH_DAEMON_BIN`, which no longer applies after in-process dispatch, and several “migrated” tests were weakened to generic log checks.

Concrete failures I reproduced:
- `interactive_prd::prd_done_dispatch_uses_approved_spec` fails because captured child args are now always empty.
- `interactive_prd::prd_done_mixed_labels_not_blocked` fails for the same reason.
- `pr_runtime::pr_url_plumbed_through_child_args` fails because no child process is spawned to capture `--pr-url`.

Root cause locations:
- `RALPH_DAEMON_BIN`-based capture helper in [tests_interactive_prd.rs:5525](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs:5525) and env injection in [tests_interactive_prd.rs:5556](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs:5556)
- Subprocess arg assertion in [tests_interactive_prd.rs:5598](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs:5598)
- PR-url child-arg capture in [tests_pr_runtime.rs:274](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_pr_runtime.rs:274) and assertion in [tests_pr_runtime.rs:314](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_pr_runtime.rs:314)

Coverage quality regressions:
- “PR metadata verification” no longer verifies PR metadata, only dispatch/terminal logs in [tests_e2e_conformance.rs:393](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:393)
- Concurrency test dropped exit-code assertion and now allows very broad pass conditions in [tests_daemon_concurrency.rs:147](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs:147)

### Proposed Change
Migrate remaining daemon/interactive PRD/PR-runtime conformance tests to in-process observability (state files, labels, task logs, artifacts) instead of child-arg capture. Restore behavior-specific assertions (not generic stderr substring checks) so test names match what they prove.

### Affected Files
- [src/validate/tests_interactive_prd.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs) - remove `RALPH_DAEMON_BIN` assumptions; assert payload via in-process side effects.
- [src/validate/tests_pr_runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_pr_runtime.rs) - replace child-arg checks with in-process propagation checks.
- [src/validate/tests_e2e_conformance.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs) - restore metadata assertions (`gh pr create` title/head/body expectations).
- [src/validate/tests_daemon_concurrency.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon_concurrency.rs) - tighten assertions and restore exit-code checks.

## Amendment: [P1] Cancellation Is Not Honored During Retry Backoff

### Problem
Both orchestration retry loops sleep unconditionally after timeout retries, so cancellation can be delayed for long backoff intervals instead of being prompt.

- Blocking backoff sleep in [orchestrator.rs:6132](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:6132)
- Blocking backoff sleeps in [quick_dev_orchestrator.rs:1471](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs:1471) and [quick_dev_orchestrator.rs:1489](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs:1489)

### Proposed Change
Wrap each backoff sleep in `tokio::select!` against `cancel.cancelled()` and return `RalphError::Cancelled` immediately when cancellation arrives. Add tests that cancel during backoff and assert fast cancellation.

### Affected Files
- [src/workflow/orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs) - cancellation-aware backoff in retry loop.
- [src/workflow/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs) - same fix for quick-dev path.
- [src/workflow/orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs) - add/extend unit tests for cancel-during-backoff.
- [src/workflow/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs) - add/extend unit tests for cancel-during-backoff.

## Amendment: [P1] Tmux Backend Bypasses Env Sanitization and Cancellation Cleanup

### Problem
`SANITIZED_ENV_VARS` is only applied in `CliBackend::execute_streaming` ([backend/mod.rs:535](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:535)).  
`TmuxBackend` builds shell commands directly and does not unset sanitized vars ([tmux_backend.rs:123](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/tmux_backend.rs:123)), so daemon env vars like `CLAUDECODE` can leak in tmux mode.

Also, tmux window cleanup runs only after awaited completion ([tmux_backend.rs:223](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/tmux_backend.rs:223) to [tmux_backend.rs:302](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/tmux_backend.rs:302)). If the backend future is dropped on cancellation, this cleanup path is skipped.

### Proposed Change
Apply sanitization consistently in tmux path by prepending `unset`/`env -u` for all sanitized vars in tmux shell command generation. Add a drop guard for tmux window lifecycle so cancellation-triggered future drops still kill the window/process.

### Affected Files
- [src/backend/tmux_backend.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/tmux_backend.rs) - sanitize inherited env and add cancellation drop cleanup.
- [src/backend/mod.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs) - keep shared sanitization list as single source of truth.
- [src/backend/tmux_backend.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/tmux_backend.rs) - add tests for sanitization + cancel-drop cleanup.

## Amendment: [P2] `KillOnDrop` Reaping Is Best-Effort and Can Leave Zombies

### Problem
`KillOnDrop` sends `SIGKILL` and performs only a single non-blocking `waitpid(..., WNOHANG)` ([backend/mod.rs:64](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:64) to [backend/mod.rs:66](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:66)). If the child has not exited yet at that moment, no further reap is guaranteed.

### Proposed Change
Guarantee post-cancel reaping by either:
1. moving cancellation handling into an explicit async cleanup path that calls `kill_and_reap_child`, or
2. spawning a dedicated reaper task/thread that blocks on `waitpid(pid, 0)` after `SIGKILL`.

Add a cancellation-path test equivalent to the timeout reaping test (`kill(pid, 0)` should report `ESRCH` after cancellation).

### Affected Files
- [src/backend/mod.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs) - make cancellation-path reaping deterministic.
- [src/backend/mod.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs) - add cancellation reaping regression test.
