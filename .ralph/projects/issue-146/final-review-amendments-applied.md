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


## Round 2

### Amendment: QD-BACKEND-EQUALITY-002

### Problem
Distinct-backend validation is a raw string equality check, which is bypassable by formatting differences.

- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs):811-815 compares `implementer == reviewer` directly.

Semantically identical specs like `"claude"` vs `" claude "` can pass this check and still resolve to the same backend, violating the quick-dev “distinct backend specs” requirement.

### Proposed Change
Canonicalize both specs before comparison.

- Parse with `parse_backend_spec`, compare normalized `name` + `model` (+ optional flag if desired), and reject if semantically equal.
- Keep the existing clear error message.
- Add tests for whitespace-normalized equality rejection.

### Affected Files
- [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs) - normalize backend specs in `validate_distinct_backends`.
- [`src/validate/tests_quick_dev.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs) - add conformance coverage for normalization edge cases.

### Reviewer
codex

### Amendment: QD-CRASH-COUNTERS-001

### Problem
`quick-dev` counter state is not durably updated at the moment counters change.

- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs):427 increments `review_iteration` only in a local variable.
- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs):695 increments `final_review_attempts` only in a local variable.
- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs):721-724 force-completes and saves state without copying the incremented `final_review_attempts` into `state.quick_dev_final_review_attempts`.

If a crash/error occurs after increment but before the next loop-head persistence, guard accounting can be stale on resume. In force-complete, persisted attempt count is wrong.

### Proposed Change
Persist counters immediately when they change, not only at phase-loop entry.

- After `review_iteration += 1`, assign `state.quick_dev_review_iteration = review_iteration` and save state before transition/checkpoint work.
- After `final_review_attempts += 1`, assign `state.quick_dev_final_review_attempts = final_review_attempts` and save state before transition/checkpoint work.
- In force-complete path, ensure incremented attempt count is persisted in `state.json` before return.
- Add regression tests asserting persisted counter values in force-complete and transition-error paths.

### Affected Files
- [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs) - persist counter mutations at mutation points.
- [`tests/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs) - add assertions/tests for persisted counter accuracy.

### Reviewer
codex

### Amendment: QD-STRAY-FILE-003

### Problem
A non-source, loop-specific notes artifact was committed in repo root:

- [`20260304T070323-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T070323-impl-notes.md)

This is unintended scope creep and repository noise outside `.ralph` runtime state.

### Proposed Change
Remove this file from the tracked source tree (or relocate to `.ralph` artifacts if it must be kept as runtime output).

### Affected Files
- [`20260304T070323-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T070323-impl-notes.md) - delete from version control.

---

### Reviewer
codex


## Round 3

### Amendment: QD-FR-001

### Problem
A stray non-source artifact was committed at repo root: `20260304T082736-impl-response-001.md` (starts at line 1).  
This is implementation-process output, not runtime/source/test code, and it is outside the project’s intended deliverables.

### Proposed Change
Remove the file from the repository history for this branch/PR.

### Affected Files
- `20260304T082736-impl-response-001.md` - delete stray artifact file.

### Reviewer
codex

### Amendment: QD-FR-002

### Problem
`QuickDevOrchestrator` hard-caps phase transitions at 100 (`src/workflow/quick_dev_orchestrator.rs:281`, `:781-783`).  
This can cause false failures (`"quick-dev: exceeded maximum phase transitions (100)"`) before user-configured guards (`--max-review-iterations`, `--max-final-review-retries`) are reached, so configured limits are not reliably honored for larger values.

### Proposed Change
Replace the fixed `0..100` cap with a bound derived from configured limits (or remove the fixed cap and rely on guard-based termination). Add a regression test with elevated limits to prove no premature cap-triggered failure.

### Affected Files
- `src/workflow/quick_dev_orchestrator.rs` - remove/replace fixed 100-step bound with config-aware termination logic.
- `tests/quick_dev_orchestrator.rs` - add regression coverage for high iteration/retry settings.

### Reviewer
codex

### Amendment: QD-FR-003

### Problem
Quick-dev state persistence is non-atomic: `save_state_to_disk` writes directly via `fs::write` (`src/workflow/quick_dev_orchestrator.rs:892-896`).  
During crash/power-loss windows, `state.json` can be partially written/corrupted, undermining the “crash-safe resumable” guarantee. Recovery currently silently ignores parse failure (`src/project/lifecycle.rs:458-503`), which can drop persisted phase/counter state.

### Proposed Change
Write `state.json` atomically (temp file in same dir, flush/fsync, then rename; optionally fsync parent dir). Also emit a warning/error log when `state.json` parsing fails during reconstruction so state-loss is observable.

### Affected Files
- `src/workflow/quick_dev_orchestrator.rs` - implement atomic state write path.
- `src/project/lifecycle.rs` - log parse failures for `state.json` quick-dev metadata loading.

---

### Reviewer
codex

### Amendment: STRAY-001

### Problem
`20260304T082736-impl-response-001.md` is a committed build-artifact/implementation-response file in the repository root (added in commit `f1a8dde`). It is not source code and should not be shipped. Prior stray files (`20260304T082736-impl-notes.md`) were already cleaned up per the file's own contents, but this response file was committed in the same loop.

### Proposed Change
Delete `20260304T082736-impl-response-001.md` from the repository root.

### Affected Files
- `20260304T082736-impl-response-001.md` — delete

---

### Reviewer
claude


## Round 4

### Amendment: FR-QD-002

### Problem
Several tests named as resume-phase validations do not actually prove phase-correct resume behavior; they only assert eventual completion.  
Examples:
- [src/validate/tests_quick_dev.rs:430](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs:430), [src/validate/tests_quick_dev.rs:489](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs:489), [src/validate/tests_quick_dev.rs:548](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs:548)
- [tests/quick_dev_orchestrator.rs:688](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs:688), [tests/quick_dev_orchestrator.rs:744](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs:744), [tests/quick_dev_orchestrator.rs:799](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs:799)

A regression where resume always restarts from `PlanAndImplement` could still pass these tests.

### Proposed Change
Strengthen these tests with phase-sensitive assertions, e.g.:
- `resume_from_codex_review`: assert no new plan-implement artifact is created on resume, and a codex-review artifact is produced first.
- `resume_from_final_review`: assert no new plan/apply-fixes artifacts are created on resume.
- `resume_from_none`: assert plan-implement artifact creation (or first prompt marker) to prove start phase is `PlanAndImplement`.

### Affected Files
- [src/validate/tests_quick_dev.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs) - strengthen conformance assertions for resume semantics.
- [tests/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs) - strengthen integration assertions for phase-accurate resume behavior.

---

### Reviewer
codex

### Amendment: STRAY-001

### Problem
`20260304T094223-impl-notes.md` is committed to the repo root. This is an implementation scratchpad from loop 14 that does not belong in the source tree — it's not referenced by any code, test, or documentation, and pollutes the project root.

### Proposed Change
Delete `20260304T094223-impl-notes.md` from the repository.

### Affected Files
- `20260304T094223-impl-notes.md` - delete (stray implementation notes file)

### Reviewer
claude


## Round 5

### Amendment: AMEND-QD-CRASH-GUARD-001

### Problem
`quick-dev` guard enforcement is not crash-durable in two counter-persist windows.

In [src/workflow/quick_dev_orchestrator.rs:443](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:443), `review_iteration` is persisted before the max-review guard check at [line 447](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:447).  
In [src/workflow/quick_dev_orchestrator.rs:715](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:715), `final_review_attempts` is persisted before the max-final-review-retries guard check at [line 719](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:719).

If the process crashes between persistence and guard evaluation, resume re-enters `CodexReview` / `FinalReview` and executes backend calls again (see [FinalReview entry at line 582](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:582)) instead of immediately honoring already-reached limits. That can bypass intended guard outcomes after restart.

### Proposed Change
Add guard checks at phase entry, before any backend invocation:

1. In `CodexReview`: if `review_iteration >= max_review_iterations`, perform the warning/transition-to-`FinalReview` path immediately.
2. In `FinalReview`: if `final_review_attempts >= max_final_review_retries`, perform force-complete immediately (artifact + completed state + checkpoint), without running final-review backends.

Add regression tests that seed persisted maxed counters and assert resume enforces guard behavior without extra review calls.

### Affected Files
- [src/workflow/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs) - enforce guard-at-entry logic for crash-durable resume.
- [tests/quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/tests/quick_dev_orchestrator.rs) - add resume tests for pre-guard crash windows.

### Reviewer
codex

### Amendment: STRAY-001

### Problem
The file `20260304T103437-impl-notes.md` exists in the repository root. This is a development artifact from loop 16 that was committed to the branch but should not be part of the final deliverable. It is tracked by git (appears in `git diff master...HEAD`).

### Proposed Change
Delete `20260304T103437-impl-notes.md` from the repository root and commit the removal.

### Affected Files
- `20260304T103437-impl-notes.md` - delete this stray implementation-notes artifact from the repo root

---

### Reviewer
claude

