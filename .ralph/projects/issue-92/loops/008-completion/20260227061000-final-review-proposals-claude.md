---
artifact: final-review-proposals
loop: 8
project: issue-92
backend: claude
role: final_reviewer
created_at: 2026-02-27T06:10:00Z
---

Now let me compile my findings and deliver the review.

## Summary of Review Findings

**Source changes reviewed (3 files):**
1. `src/daemon/interactive_prd.rs` — +86/-10 lines
2. `src/validate/tests_interactive_prd.rs` — +665/-45 lines  
3. `tests/daemon_interactive_prd.rs` — +205/-13 lines

**Correctness verification:**

1. **Label catalog** — `ralph:waiting-feedback` correctly added to `PRD_LABELS` (color `#e4e669`, description correct), `PRD_LABEL_NAMES`, and excluded from `IN_PROGRESS_PRD_LABEL_NAMES`.

2. **Startup ensure** — `ensure_prd_labels_best_effort_with_gh_bin` iterates `PRD_LABELS`, so the new label is created on startup.

3. **Detection helpers** — `has_prd_label` uses `PRD_LABEL_NAMES` (includes waiting-feedback → true). `has_in_progress_prd_label` uses `IN_PROGRESS_PRD_LABEL_NAMES` (excludes waiting-feedback → false when alone).

4. **Apply/reconcile in waiting flows** — `ensure_waiting_feedback_label` correctly placed before branch-specific logic in: `transition_pending_to_awaiting_answers` (line 994, unconditional), `transition_awaiting_answers_to_awaiting_feedback` (line 1163), `transition_awaiting_feedback` (line 1305). Short-circuits when already present (line 617).

5. **Terminal Done removal** — Both `ralph:prd-active` and `ralph:waiting-feedback` removals are attempted independently after successful save. If either fails, state is reverted to `AwaitingFeedback` for retryability. This is a sound design.

6. **Terminal Failed removal** — `ralph:waiting-feedback` removed best-effort (with `let _ =`) after successful save, consistent with existing `ralph:prd-active` and `ralph:prd` removal pattern.

7. **No removal when save fails** — Both Done and Failed paths return early without label removal when save fails.

8. **Dedup test fix** — The dedup invariant test now counts only `--remove-label ralph:prd` operations (a stable side-effect of processing) instead of all label edits, correctly accommodating the new waiting-label reconciliation calls.

9. **Tests** — Comprehensive coverage: unit tests for constants and detection helpers, integration tests for classification exports and post-save cleanup retry, conformance tests for startup ensure, pickup, reconciliation, no-op idempotency, answer-processing, revision, Done removal, Failed removal, and save-failure no-removal.

10. **Nix build flakiness** — The nix static build has pre-existing intermittent test failures (different tests each run, also fails at base commit). Not introduced by this PR.

# Final Review: NO AMENDMENTS

## Summary

The implementation is correct, complete, and well-tested. All functional requirements from the master prompt are satisfied:

- **Label catalog**: `ralph:waiting-feedback` added to `PRD_LABELS` (verified at `src/daemon/interactive_prd.rs:184-188`) and `PRD_LABEL_NAMES` (line 583), excluded from `IN_PROGRESS_PRD_LABEL_NAMES` (lines 591-596).
- **Startup ensure**: `src/daemon/github.rs:1719` iterates `PRD_LABELS`, automatically including the new label.
- **Detection helpers**: `has_prd_label` (line 599-601) returns true; `has_in_progress_prd_label` (line 607-614) returns false for `ralph:waiting-feedback` alone.
- **Reconciliation**: `ensure_waiting_feedback_label` (lines 616-628) is called unconditionally before branch-specific logic in all three waiting-state handlers (lines 994, 1163, 1305), with a short-circuit when the label is already present.
- **Terminal removal**: Done path (lines 1503-1539) attempts both removals independently after durable save, reverting to AwaitingFeedback on failure for retryability. Failed path (lines 2165-2171) removes best-effort after save. Neither path removes when save fails.
- **No changes outside PRD workflow**: Only `src/daemon/interactive_prd.rs`, `src/validate/tests_interactive_prd.rs`, and `tests/daemon_interactive_prd.rs` were modified.
- **Test coverage**: Unit tests (5 new assertions), integration tests (3 new tests + 4 augmented assertions), and conformance tests (3 new test functions + 8 augmented assertions) cover all acceptance criteria. `cargo check` and `cargo test` pass cleanly.
