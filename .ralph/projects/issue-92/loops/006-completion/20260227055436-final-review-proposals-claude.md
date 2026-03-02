---
artifact: final-review-proposals
loop: 6
project: issue-92
backend: claude
role: final_reviewer
created_at: 2026-02-27T05:54:36Z
---

The `ensure_waiting_feedback_label` call at line 994 is unconditional — it's called at the top of `transition_pending_to_awaiting_answers` regardless of any `has_active` check. This satisfies the requirement.

Now I have a complete picture. Let me summarize my findings.

# Final Review: NO AMENDMENTS

## Summary

The implementation of the `ralph:waiting-feedback` label lifecycle is correct, complete, and well-tested. I verified the following across all 3 changed files:

**Constants (`src/daemon/interactive_prd.rs`)**
- `PRD_LABELS` at line 184 includes `("ralph:waiting-feedback", "#e4e669", "PRD workflow is waiting for user input")` — correct color and description per spec.
- `PRD_LABEL_NAMES` at line 583 includes `"ralph:waiting-feedback"` — enables `has_prd_label` detection.
- `IN_PROGRESS_PRD_LABEL_NAMES` at line 591 correctly **omits** the new label, ensuring `has_in_progress_prd_label` returns false for it.
- `WAITING_FEEDBACK_LABEL` const at line 589 avoids string duplication.

**Ensure helper (`src/daemon/interactive_prd.rs:616-628`)**
- `ensure_waiting_feedback_label` checks label presence first (no-op/idempotent), then adds best-effort via `let _` (non-fatal).

**Apply/reconcile call sites:**
- `transition_pending_to_awaiting_answers` line 994 — unconditional, before `get_or_fetch_bot_login`. Satisfies spec requirement for non-`has_active`-gated call.
- `transition_awaiting_answers_to_awaiting_feedback` line 1163 — before branch logic, covers no-op, processing, and error paths.
- `transition_awaiting_feedback` line 1305 — before branch logic, covers no-op, revision, and approval paths.

**Terminal removal:**
- Done path (`do_approval_transition` lines 1514-1521): removes `ralph:waiting-feedback` **after** state save succeeds (line 1494). If save fails, state reverts to AwaitingFeedback and labels are preserved (line 1497).
- Failed path (`transition_to_failed` lines 2150-2156): removes `ralph:waiting-feedback` **after** state save succeeds (line 2119). If save fails, state reverts and labels are preserved (line 2129).

**Startup ensure:**
- `github::ensure_prd_labels_best_effort_with_gh_bin` iterates `PRD_LABELS` which automatically includes the new label. No code changes needed in `github.rs`.

**Test coverage verified:**
- Unit tests: `prd_label_constants_include_waiting_feedback`, `has_prd_label_matches_waiting_feedback_label`, `has_in_progress_prd_label_rejects_done_empty_and_unrelated_labels` — all assert correct classification.
- Integration test: `waiting_feedback_label_classification_matches_exports` — verifies exported API classification.
- Conformance tests: `pickup_with_existing_active_adds_waiting_label`, `awaiting_answers_noop_waiting_label_reconciliation` (both cases: missing and present), plus assertions added to `pickup_and_question_posting`, `answer_to_draft`, `feedback_revision`, `approval_by_comment`, `feedback_stage_failure_labeling`, `bot_login_failure_exhaustion_*`, `terminal_save_failure_keeps_retry_visibility`, and `terminal_save_failure_failed_path_keeps_retry_visibility`.
- Dedup tests updated: counter now tracks `remove-label ralph:prd` specifically rather than all `edit` calls, correctly accounting for the new waiting-label add operations.

**Build & test results:**
- `cargo check` — passes.
- `cargo test` — all tests pass.
- `nix build` — succeeds.
- Conformance validation: 271 passed, 15 failed. All 15 failures are pre-existing (none in the diff's 3 files; confirmed by verifying zero overlap between failing test names and the diff).

**No stray files:** `git status` shows no untracked files outside `.ralph/`.
