---
artifact: quick-dev-codex-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T11:20:30Z
---

# Review: CHANGES REQUESTED

1. High: `quick-dev-auto` lost fail-fast validation and now executes side effects before backend validation.
- Affected path: [`src/cli/quick_dev_auto.rs:75`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/quick_dev_auto.rs:75) now delegates directly to [`run_quick_dev_auto_task`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:302), and daemon dispatch also calls that same function via [`src/daemon/runtime.rs:1569`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:1569).
- In [`src/daemon/tasks.rs:310`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:310) the quick-PRD phase runs immediately, then project creation happens at [`src/daemon/tasks.rs:373`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:373), and only afterward does `QuickDevOrchestrator::run()` validate reviewer/distinct backend requirements.
- This regresses behavior expected by conformance tests (must fail with validation error before quick-PRD/project creation). I reproduced failures with:
  - `quick_dev::auto_missing_reviewer_fails_fast`
  - `quick_dev::auto_equal_backends_fails_fast`
  - `quick_dev::auto_optional_reviewer_fails_fast`
  - `quick_dev::auto_unknown_reviewer_fails_fast`
  - `quick_dev::auto_whitespace_equal_backends_fails_fast`
- Concrete fix:
  1. Reintroduce quick-dev preflight validation at the start of `run_quick_dev_auto_task` (before quick-PRD and before `create_project`).
  2. Resolve implementer/reviewer with the same precedence as before (CLI override -> workflow config -> default backend for implementer; reviewer required).
  3. Validate via `validate_required_backend_spec(...)` and `quick_dev_orchestrator::validate_distinct_backends(...)`, returning `RalphError::Validation` on failure.
  4. Keep `QuickDevOrchestrator::run()` validation as a second-line guard, but do not allow side effects before preflight passes.
