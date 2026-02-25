---
artifact: final-review-proposals
loop: 5
project: task-implement-parallel-prd-issue-proces
backend: codex
role: final_reviewer
created_at: 2026-02-24T23:45:09Z
---

I’m going to audit the implementation against the master prompt by first listing all source changes with `git diff <base>...HEAD`, then I’ll trace the modified code paths (daemon PRD flow, backend execution, runtime config wiring, and tests) for cross-file correctness and edge cases.
I’ve identified the changed source set and now I’m reading the full diff file-by-file, then I’ll trace call sites and tests to verify concurrency behavior, panic/error isolation, dedup, and refresh ordering across modules.
I’m now tracing `interactive_prd.rs` directly with line-level reads to validate the new worker pool, panic/error handling, repo refresh sequencing, and whether backend invocations still follow the required state-machine invariants.
Next I’m validating `CliBackend` cross-file behavior: where `cwd` is set, where it defaults to `None`, and whether command spawning honors it without affecting existing callers.
I’ve confirmed the core refactor points in source; next I’m running targeted tests (daemon interactive PRD integration + related backend/validate coverage) to check for behavioral regressions and verify the new concurrency paths actually execute as intended.
The integration test run is currently stuck in the new bounded-concurrency case (`bounded_concurrency_peak_never_exceeds_max`), which is a likely signal of a real issue; I’m going to inspect that test path and the mock script assumptions while keeping the process monitored.
The integration suite passes, but it took ~270s, so I’m checking validate/conformance coverage next to confirm the new PRD concurrency guarantees are actually wired into the conformance runner and still green.
# Final Review: AMENDMENTS

## Amendment: FR-001

### Problem
The new concurrency tests are not validating the required “issue advances in one tick” behavior end-to-end, and they currently depend on real backend execution/timeouts.

In integration tests, PATH is only mocked for `gh`/`git` (no `claude`/`codex` backend mocks), but the config still uses real backends with large timeouts (examples: [tests/daemon_interactive_prd.rs:2721](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:2721), [tests/daemon_interactive_prd.rs:2749](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:2749), [tests/daemon_interactive_prd.rs:3189](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3189), [tests/daemon_interactive_prd.rs:3357](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3357)). Assertions then check only label-edit side effects/flags, not successful state advancement (example: [tests/daemon_interactive_prd.rs:3202](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3202)).

The same issue exists in conformance tests (examples: [src/validate/tests_interactive_prd.rs:3478](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:3478), [src/validate/tests_interactive_prd.rs:3497](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:3497), [src/validate/tests_interactive_prd.rs:3514](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:3514), [src/validate/tests_interactive_prd.rs:3778](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:3778)).  
Observed behavior confirms this: tests pass while logging `PRD backend timeout exceeded`, so they are false-positive for advancement correctness and add major runtime cost.

### Proposed Change
Make these tests fully deterministic and advancement-accurate:
1. Add local mock backend scripts (`claude`, `codex`) in each test’s `scripts_dir` and prepend to `PATH`.
2. Return deterministic question/synthesis output so `Pending -> AwaitingAnswers` can succeed quickly without real CLI/network.
3. Replace “label edit reached” assertions with state assertions (`InteractivePrdState::load` and expected state/revision fields) for the issue(s) that should advance.
4. Reduce backend timeout values in these tests once mocked (e.g., 1-2s) to avoid long hangs if a mock regresses.

### Affected Files
- [tests/daemon_interactive_prd.rs](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs) - Mock backends + assert persisted state transitions, not just label-edit side effects.
- [src/validate/tests_interactive_prd.rs](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs) - Same deterministic backend mocking and stronger advancement assertions.

## Amendment: FR-002

### Problem
A loop artifact file was added at repository root and is unrelated to product code/tests: [1740440931-impl-notes.md:1](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/1740440931-impl-notes.md:1).

### Proposed Change
Remove this file from the tracked source tree (or relocate it under `.ralph` artifacts if it must be preserved).

### Affected Files
- [1740440931-impl-notes.md](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/1740440931-impl-notes.md) - Remove from repo root.

---
