---
artifact: final-review-proposals
loop: 9
project: issue-146
backend: codex
role: final_reviewer
created_at: 2026-03-04T06:44:41Z
---

I’ll audit this as a full code review against the actual branch changes, starting by diffing against the base branch and then tracing key code paths (orchestrator, daemon dispatch, parser, CLI, and tests) for correctness and safety. I’ll report concrete amendments only if I can tie them to specific source lines and failure modes.
I confirmed this branch is `ralph/issue-146` based off `origin/master`; next I’m reviewing the exact patch with `git diff origin/master...HEAD -- . ':(exclude).ralph'` and then drilling into risk-heavy files.
I have the full changed-file list now. I’m inspecting the high-risk source paths first (`quick_dev_orchestrator`, parser, daemon dispatch, lifecycle/state), then I’ll validate whether tests truly assert the intended behavior and flag any out-of-scope/stray files.
I’m now validating the CLI and daemon wiring for quick-dev, including backend resolution, resume behavior, and command spawning paths to catch dispatch or argument bugs.
I’ve moved into test-quality review now. I’m checking whether quick-dev and daemon tests would fail on real regressions (not just pass due to weak assertions), and whether any failure paths are untested or asserted on the wrong signal.
I found unrelated-but-modified files (`main.rs`, daemon concurrency validate test) and am checking those too for unintended regressions since they’re in this branch diff.
I’ve identified a few potential edge cases and am now validating them by running focused tests and reading adjacent logic (`lifecycle` reconstruction and review-iteration guards) to determine whether they are real defects or just implementation choices.
# Final Review: AMENDMENTS

## Amendment: QD-REVIEW-001

### Problem
`quick-dev-auto` performs expensive side effects before validating quick-dev backend requirements. It runs quick-PRD and creates the project first, then only fails when `QuickDevOrchestrator::run()` validates reviewer presence/distinctness.

Evidence:
- Quick-PRD + project creation happen before orchestrator call in [quick_dev_auto.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/cli/quick_dev_auto.rs:133) and [quick_dev_auto.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/cli/quick_dev_auto.rs:193).
- Backend requirement errors are thrown inside orchestrator in [quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:796) and [quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:811).

This violates fail-fast behavior and can leave partially-created projects for invalid quick-dev backend configuration.

### Proposed Change
Add a preflight quick-dev backend resolution/validation step at the start of `quick-dev-auto` (before quick-PRD and before `create_project`), using the same precedence/error semantics as `quick-dev-run`:
- reviewer required (`"quick-dev requires a second backend for review"`)
- implementer/reviewer must be distinct specs

Add conformance coverage that `quick-dev-auto` with missing/equal reviewer backend fails with exit code 2 and does not create `.ralph/projects/<id>`.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/cli/quick_dev_auto.rs` - add preflight validation before side effects.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs` - add failure-without-project-creation tests.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs` - optionally expose/shared helper for consistent resolution logic.

## Amendment: QD-REVIEW-002

### Problem
Quick-dev reconstruction from `state.json` is incomplete and not safely scoped:

1. It restores `quick_dev_phase` and counters, but not `current_phase`/`phase_iteration`, so reconstructed state can show stale phase data.
- See loader in [lifecycle.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:438).
- Tests explicitly work around this by reading raw `state.json` because `reconstruct_project_state` does not propagate quick-dev phase fields for status display: [tests_quick_dev.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs:139).

2. Completed-status override is broad (`quick_dev_phase.is_none()`), which can also match non-quick projects:
- [lifecycle.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:461).

This can produce incorrect reconstructed state/reporting and risks non-quick behavior contamination.

### Proposed Change
Tighten and complete quick-dev state hydration in `load_quick_dev_phase_from_state_json`:
- Restore `current_phase` and `phase_iteration` from persisted quick-dev state.
- Scope completed-status override to explicit quick-dev state markers (not any `status=completed` + `quick_dev_phase=null` case).
- Add reconstruction tests using `reconstruct_project_state`/`h.load_state()` to verify quick-dev phase display is correct and non-quick projects are unaffected.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs` - complete/scoped quick-dev hydration.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs` - replace workaround assertions with reconstructed-state assertions.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs` (tests module, if present) - add unit tests for quick-dev/non-quick reconstruction boundaries.
