---
artifact: quick-dev-codex-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T12:26:14Z
---

# Review: CHANGES REQUESTED
1. [P1] `quick-dev-auto` no longer fails fast on invalid/missing reviewer config, and now executes quick-PRD/backend work before returning an error.
[run_quick_dev_auto_task](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:302) removed the old preflight validation that existed in the CLI path, so it starts quick-PRD immediately ([tasks.rs:310](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:310)) and only discovers quick-dev backend invalidity later when orchestration runs ([tasks.rs:386](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:386)).  
This affects both callers: CLI [quick_dev_auto.rs:90](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/quick_dev_auto.rs:90) and daemon quick dispatch [runtime.rs:1582](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/runtime.rs:1582).  
Existing conformance expectations explicitly require fail-fast-before-side-effects ([tests_quick_dev.rs:926](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_quick_dev.rs:926)); they currently fail (`quick_dev::auto_*_fails_fast` all fail with exit code `1` instead of `2`).

Concrete fix:
- In [run_quick_dev_auto_task](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/daemon/tasks.rs:302), restore preflight backend validation **before** quick-PRD and project creation:
  - Resolve effective implementer/reviewer using the same chain as before (`CLI override -> workflow config -> default_backend` for implementer; reviewer required).
  - Enforce reviewer presence and distinctness via `quick_dev_orchestrator::validate_distinct_backends`.
  - Validate specs with `config::validate_required_backend_spec`.
- Re-run conformance: `target/debug/ralph validate --bin target/debug/ralph --filter quick_dev::auto_` and ensure all five fail-fast tests pass.
