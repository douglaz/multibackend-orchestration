---
artifact: quick-dev-codex-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T06:37:02Z
---

# Review: CHANGES REQUESTED
1. **High: opposite-backend reformatter regression introduced by `set_cwd` cache clearing**
[orchestrator.rs:241](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:241) now calls `registry.set_cwd(...)`, which clears cached backends; then parse-retry reformatter lookup at [orchestrator.rs:5927](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:5927) uses `registry.get(...)` and falls back to the original backend at [orchestrator.rs:5932](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:5932). When reformatter is bare `codex` (no role model), it is no longer preloaded, so reformatter silently runs on the original backend and parse retries can exhaust.
Evidence: `nix develop -c cargo test parse_retry_reformat_without_role_model_uses_bare_opposite_backend -- --nocapture` fails with `ParseRetriesExhausted`.
**Fix:** materialize the reformatter backend instead of `get` fallback (preferred: make `execute_with_parse_retries` take `&mut BackendRegistry` and call `get_or_create_for_spec(&reformatter_spec)`), or preload bare defaults right after `set_cwd`.

2. **Medium: `QuickDevRunOptions.max_backend_retries` is only applied to outer walltime timeout, not backend timeout errors**
In [quick_dev_orchestrator.rs:1438](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs:1438), `execute_backend` retries only when `tokio::time::timeout` fires (`Err(_)`), but `Ok(Err(RalphError::BackendTimeout{...}))` returns immediately at [quick_dev_orchestrator.rs:1448](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs:1448). That bypasses configured retry count for common timeout paths coming from `CliBackend`.
**Fix:** split match arms to retry on both walltime timeout and `RalphError::BackendTimeout`, mirroring `execute_with_timeout_retries` behavior in the main orchestrator.

3. **Medium: conformance test migration is incomplete (acceptance criterion #12 not met)**
`src/validate/tests_daemon.rs` still contains many `RALPH_DAEMON_BIN`/mock-child assumptions, e.g. [tests_daemon.rs:1480](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:1480), [tests_daemon.rs:1513](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:1513), [tests_daemon.rs:2934](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:2934). Also retry conformance still expects env-var behavior at [tests_e2e_conformance.rs:113](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:113).
Evidence: `nix develop -c cargo run -- validate --bin ./target/debug/ralph --filter retry_override_set_to_one` fails (`left: 3`, `right: 1`).
**Fix:** remove remaining `RALPH_DAEMON_BIN` assertions, migrate to in-process task behavior checks, and add the missing cancellation/env-sanitization/log-isolation coverage required by the spec.
