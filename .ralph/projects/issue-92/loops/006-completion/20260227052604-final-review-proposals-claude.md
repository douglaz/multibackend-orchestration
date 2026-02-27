---
artifact: final-review-proposals
loop: 6
project: issue-92
backend: claude
role: final_reviewer
created_at: 2026-02-27T05:26:04Z
---

Now I have a complete picture. Let me compile my findings.

**Summary of verification:**

1. **Constants**: `PRD_LABELS` (6 entries, includes `ralph:waiting-feedback` with correct color/description), `PRD_LABEL_NAMES` (6 entries, includes it), `IN_PROGRESS_PRD_LABEL_NAMES` (4 entries, correctly excludes it). All correct.

2. **`ensure_waiting_feedback_label` helper** (line 616): Correctly checks if label is already present (no-op), then adds best-effort with `let _ =` (ignoring errors). Correct.

3. **Call sites**: Called at the top of all 3 transition wrapper functions (lines 994, 1163, 1305), before any branch-specific logic. This satisfies the spec requirement for reconciliation running before branch-specific logic.

4. **Terminal removal in Done path** (`do_approval_transition`): State saved at line 1494. If save fails, returns error at line 1498-1500 — label removal at line 1518-1524 is unreachable. Correct: removal only after durable save.

5. **Terminal removal in Failed path** (`transition_to_failed`): State saved at line 2119. If save fails, returns error at line 2130 — label removal at line 2150-2156 is unreachable. Correct: removal only after durable save.

6. **Tests**: All 3 layers verified — unit tests, integration tests, and conformance tests. All 286 nix build tests pass. All 15 relevant conformance tests pass.

7. **No stray files**: Only 3 files changed, all expected.

8. **Dedup test fix**: The `concurrent_dedup_invariant` test was correctly updated to count only `--remove-label ralph:prd` operations (not all `issue edit` calls), since the new waiting-feedback label adds legitimate extra `issue edit` calls that shouldn't be confused with duplicate processing.

# Final Review: NO AMENDMENTS

## Summary

The implementation is correct, safe, and complete across all three modified files:

- **`src/daemon/interactive_prd.rs`**: The `ralph:waiting-feedback` label is properly added to `PRD_LABELS` (line 184, with color `#e4e669` and correct description), `PRD_LABEL_NAMES` (line 583), and correctly excluded from `IN_PROGRESS_PRD_LABEL_NAMES` (lines 591-596). The `ensure_waiting_feedback_label` helper (line 616) is idempotent and best-effort. It is called at the top of all three waiting-state transition wrappers (lines 994, 1163, 1305) — before any branch-specific logic, satisfying the reconciliation ordering requirement. Terminal removal occurs in `do_approval_transition` (line 1518) and `transition_to_failed` (line 2150), both strictly after the durable state save succeeds (lines 1494 and 2119 respectively), and both unreachable if the save fails due to early error returns.

- **`src/validate/tests_interactive_prd.rs`**: Two new conformance tests added (`pickup_with_existing_active_adds_waiting_label`, `awaiting_answers_noop_waiting_label_reconciliation`). Existing tests updated to assert waiting-label behavior: startup ensure (line 597), pickup (line 921), answer processing (line 1573), feedback revision (line 1685), approval Done (line 1785), Failed terminal (line 1987), bot-login failure paths (lines 2723, 2841), terminal save failure no-removal (lines 3599, 3816). The `concurrent_dedup_invariant` test was correctly narrowed to count `--remove-label ralph:prd` instead of all `issue edit` calls.

- **`tests/daemon_interactive_prd.rs`**: Integration tests updated: new `waiting_feedback_label_classification_matches_exports` test (line 66), `PRD_LABELS.len()` assertions updated to 6 (line 135), `PRD_LABEL_NAMES` membership verified (line 84), and dedup integration test counter logic narrowed (line 2700).

All 286 `nix build` tests pass. All 15 conformance tests relevant to the `ralph:waiting-feedback` lifecycle pass. No stray files. No changes outside the PRD workflow scope.
