# Final Review Amendments Applied

## Round 1

### Amendment: QD-REVIEW-001

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

### Reviewer
codex

### Amendment: QD-REVIEW-002

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

### Reviewer
codex

### Amendment: QD-STRAY-002

### Problem
Four implementation-note markdown files were committed to the repository root during the development process. These are not source code, tests, or documentation — they are development artifacts that should not be shipped:

- `1741059547-impl-response-001.md`
- `1741065332-impl-notes.md`
- `20260304023236-impl-notes.md`
- `20260304T040000-impl-notes.md`

### Proposed Change
Delete all four files.

### Affected Files
- `1741059547-impl-response-001.md` - Delete
- `1741065332-impl-notes.md` - Delete
- `20260304023236-impl-notes.md` - Delete
- `20260304T040000-impl-notes.md` - Delete

---

### Reviewer
claude

### Amendment: QD-TRIM-001

### Problem
In `src/workflow/parser.rs`, both `parse_codex_review_output` (line 194) and `parse_quick_final_review_output` (line 211) use `first_h1.trim_end()` for the match expression. Every other parser in the file (15 instances across `parse_planner_output`, `parse_implementer_output`, `parse_reviewer_output`, `parse_completer_output`, `parse_qa_output`, `parse_prompt_reviewer_output`, `parse_final_reviewer_output`, `parse_planner_positions`, `parse_vote_results`, `parse_arbiter_ruling`) uses `first_h1.trim()`.

The `first_h1_line()` helper finds lines where `line.trim_start().starts_with("# ")` but returns the original line including any leading whitespace. If a backend produces `"  # Review: SATISFIED"`, `trim_end()` yields `"  # Review: SATISFIED"` which fails the match, while `trim()` would correctly yield `"# Review: SATISFIED"`.

This is an inconsistency that makes the two quick-dev parsers less robust than every other parser in the same file.

### Proposed Change
Change `first_h1.trim_end()` to `first_h1.trim()` on lines 194 and 211, matching the convention used by all other parsers.

### Affected Files
- `src/workflow/parser.rs` - Change `trim_end()` to `trim()` on lines 194 and 211

---

### Reviewer
claude

