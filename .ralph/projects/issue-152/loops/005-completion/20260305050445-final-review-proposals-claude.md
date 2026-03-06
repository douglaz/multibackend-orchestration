---
artifact: final-review-proposals
loop: 5
project: issue-152
backend: claude
role: final_reviewer
created_at: 2026-03-05T05:04:45Z
---

# Final Review: AMENDMENTS

## Summary

The core orchestrator changes are correct and well-structured. All six key requirements are properly implemented:

- **Feature-loop phases** (Implementing, QA, Reviewing) always re-resolve backends via `assign_feature_backends()` and use reconstructed values only for drift logging.
- **Completion planner** always re-resolves via `assign_completion_backends()`.
- **Completer panel** is correctly gated by `is_resumed_state` — re-resolved on resume, used as-is on same-run.
- **FinalReview** resolves planner at the call site and passes it into `run_final_review_phase`; the function no longer reads `completion.backends.planner`.
- **`is_resumed_state`** lifecycle is correct: `true` after reconstruction, `false` after first outer-loop iteration.
- **Drift logging** correctly emits `warn!` with role/loop_number/original/resolved only on mismatch.
- **State schema**, `register_feature_loop`, `register_completion_attempt`, and quick-dev orchestrator are all unchanged.
- All 8 required tests are present and cover the specified conformance/regression cases.
- No changes outside the expected scope (orchestrator.rs + validate module).

Two minor issues found:

## Amendment: DOCCOMMENT-MISPLACEMENT

### Problem
In `src/validate/assertions.rs:327-339`, the `strip_ansi` function was inserted between `normalize_backend`'s doc comment and its function definition. This causes:
1. `strip_ansi` has an incorrect merged doc comment starting with "Normalize a backend string by stripping model suffixes" (lines 327-329 belong to `normalize_backend`).
2. `normalize_backend` at line 341 has lost its doc comment entirely.

### Proposed Change
Move the `strip_ansi` function (with its own doc comment, lines 330-339) to **after** `normalize_backend`, or place it **before** line 327 so that `normalize_backend`'s doc comment remains attached to `normalize_backend`.

### Affected Files
- `src/validate/assertions.rs` — reorder `strip_ansi` so it does not split `normalize_backend`'s doc comment from its function definition

## Amendment: STRAY-IMPL-NOTES

### Problem
`20260305-impl-notes.md` was added to the repository root and committed. This is a development-time notes file that does not belong in the project source tree — it documents implementation decisions specific to this branch's development process, not the project itself.

### Proposed Change
Delete `20260305-impl-notes.md` from the repository.

### Affected Files
- `20260305-impl-notes.md` — delete

---
