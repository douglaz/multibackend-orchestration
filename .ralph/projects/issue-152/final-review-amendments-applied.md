# Final Review Amendments Applied

## Round 1

### Amendment: DOCCOMMENT-MISPLACEMENT

### Problem
In `src/validate/assertions.rs:327-339`, the `strip_ansi` function was inserted between `normalize_backend`'s doc comment and its function definition. This causes:
1. `strip_ansi` has an incorrect merged doc comment starting with "Normalize a backend string by stripping model suffixes" (lines 327-329 belong to `normalize_backend`).
2. `normalize_backend` at line 341 has lost its doc comment entirely.

### Proposed Change
Move the `strip_ansi` function (with its own doc comment, lines 330-339) to **after** `normalize_backend`, or place it **before** line 327 so that `normalize_backend`'s doc comment remains attached to `normalize_backend`.

### Affected Files
- `src/validate/assertions.rs` — reorder `strip_ansi` so it does not split `normalize_backend`'s doc comment from its function definition

### Reviewer
claude

### Amendment: FR-152-001

### Problem
The same-run gating regression test does not actually prove “no panel re-resolution/no re-health-check.”  
In [`tests_resume_backend_resolution.rs:1159`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:1159), the test only asserts absence of a drift-warning substring pair, and the predicate at [`:1178`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:1178) (`!A || !B`) can pass even if `resolve_completion_panel(...)` is incorrectly called on same-run entry (when no drift occurs).

### Proposed Change
Make same-run panel re-resolution observable and assert on that signal:
1. Configure `completion_backends` to include an unavailable optional backend (e.g. `?gemini`) so `resolve_completion_panel(...)` emits a deterministic skip warning when called.
2. In `same_run_completion_no_panel_reresolution`, assert that warning appears exactly once (planning-time resolution only), not twice.
3. Keep the no-drift warning assertion as a secondary check.

### Affected Files
- [`src/validate/tests_resume_backend_resolution.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs) - strengthen same-run gating test to prove no second panel resolution call.

### Reviewer
codex

### Amendment: FR-152-002

### Problem
Several new resume tests assert only generic warning text and do not prove execution used re-resolved backends:
- completion planner drift test checks only message/role at [`:969`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:969)
- completer-panel drift test checks only warning fields at [`:1043`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:1043)
- final-review planner drift test checks only message/role at [`:1122`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs:1122)

These can pass even if warning is emitted but stale backend is still used for execution.

### Proposed Change
Add execution-proof assertions:
1. Assert expected `original=` and `resolved=` values for completion/final-review planner drift warnings.
2. For panel drift, assert produced completer verdict artifact backend matches the re-resolved panel backend (not reconstructed one).
3. For final-review planner drift, drive a path that requires planner execution (amendments path) and assert planner-generated artifact frontmatter backend equals resolved planner.

### Affected Files
- [`src/validate/tests_resume_backend_resolution.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/src/validate/tests_resume_backend_resolution.rs) - strengthen drift tests so they verify execution backend selection, not only warning presence.

### Reviewer
codex

### Amendment: FR-152-003

### Problem
A top-level implementation diary file was added and is not part of runtime/test behavior for this feature:
- [`20260305-impl-notes.md:1`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/20260305-impl-notes.md:1)

This appears to be scope-external noise in source changes.

### Proposed Change
Remove the file from the change set (or move it to a non-versioned/local workflow location).

### Affected Files
- [`20260305-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-152/20260305-impl-notes.md) - delete as stray artifact.

---

## Context Provided
- Master prompt focused on resume backend re-resolution behavior in `orchestrator.rs`, drift logging semantics, and new validate conformance coverage.
- Project state summary indicated this round is final review after completion attempts.

### Reviewer
codex

### Amendment: STRAY-IMPL-NOTES

### Problem
`20260305-impl-notes.md` was added to the repository root and committed. This is a development-time notes file that does not belong in the project source tree — it documents implementation decisions specific to this branch's development process, not the project itself.

### Proposed Change
Delete `20260305-impl-notes.md` from the repository.

### Affected Files
- `20260305-impl-notes.md` — delete

---

### Reviewer
claude

