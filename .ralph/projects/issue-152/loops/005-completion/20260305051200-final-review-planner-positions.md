---
artifact: final-review-planner-positions
loop: 5
project: issue-152
backend: claude(opus)
role: planner
created_at: 2026-03-05T05:12:00Z
---

Now I have all the information needed to evaluate each amendment.

# Planner Positions

## Amendment: DOCCOMMENT-MISPLACEMENT

### Position
ACCEPT

### Rationale
Verified at `src/validate/assertions.rs:327-341`. The doc comment at lines 327-329 (`/// Normalize a backend string...`) was clearly written for the `normalize_backend` function at line 341, but `strip_ansi` (lines 330-339) was inserted between them. As a result:
1. `strip_ansi` inherits a misleading doc comment block — lines 327-329 describe normalization, not ANSI stripping. Rustdoc will render all consecutive `///` lines 327-335 as `strip_ansi`'s documentation.
2. `normalize_backend` at line 341 has no doc comment.

This is a genuine documentation correctness bug. The fix is trivial — reorder the two functions or move the doc comment.

## Amendment: STRAY-IMPL-NOTES

### Position
ACCEPT

### Rationale
Confirmed `20260305-impl-notes.md` exists at the repository root. It contains development-process notes ("Decisions Made", "Spec Deviations", "Testing" with nix build results) that are specific to this branch's development cycle. This is not project documentation — it's a transient artifact that should not be committed to the source tree.

## Amendment: FR-152-001

### Position
ACCEPT

### Rationale
Verified at `src/validate/tests_resume_backend_resolution.rs:1178-1183`. The test `same_run_completion_no_panel_reresolution` uses the predicate `!stderr.contains("role=\"completer\"") || !stderr.contains("backend drift detected...")`. This is `!A || !B`, which is equivalent to `!(A && B)`. It fails only if **both** substrings appear together. If `resolve_completion_panel` were incorrectly called but no drift occurred (same config), no drift warning would be emitted, and the test would pass despite the contract violation (no re-resolution should happen at all on same-run). The amendment correctly identifies that the test doesn't actually prove the absence of re-resolution — it only proves the absence of a drift *warning*, which is a weaker guarantee. Adding an observable signal for panel resolution (e.g., a skip warning from an unavailable optional backend) would genuinely strengthen this test.

## Amendment: FR-152-002

### Position
ACCEPT

### Rationale
Verified at three locations:

- **Lines 969-978** (completion planner drift): Asserts only that stderr contains `"backend drift detected..."` and `role="planner"`. Does not verify the `original=` or `resolved=` values, so the test passes even if the warning is emitted but execution proceeds with the stale backend.
- **Lines 1043-1064** (completer panel drift): This test actually *does* check for `original=` and `resolved=` field presence (lines 1057-1063), which partially contradicts the amendment's claim. However, it only checks that these fields *exist* — not that they contain the expected values (`claude` → `codex`). So the test can still pass with incorrect resolution.
- **Lines 1122-1131** (final-review planner drift): Same pattern as the completion planner test — checks only message substring and role field, not the actual backend values.

The core observation is valid: these tests verify warning *emission* but not that execution actually *uses* the re-resolved backend. Asserting specific `original=` and `resolved=` values would be a meaningful improvement. The panel drift test is partially stronger than claimed (it checks field presence) but still doesn't validate field values.

## Amendment: FR-152-003

### Position
ACCEPT

### Rationale
This is a duplicate of STRAY-IMPL-NOTES. The file `20260305-impl-notes.md` exists and should be removed. Both amendments identify the same real issue.
