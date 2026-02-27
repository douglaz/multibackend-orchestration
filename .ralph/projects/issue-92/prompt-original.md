## Summary

Add a `ralph:waiting-feedback` label to the interactive PRD pipeline that signals when an issue is blocked waiting for user input. The label is applied when the workflow enters the `AwaitingAnswers` or `AwaitingFeedback` states and removed on terminal transitions (`Done`, `Failed`). Every waiting-state tick where the label is found absent triggers best-effort reconciliation (re-add) — regardless of whether the tick is a no-op, processes an answer/feedback, or hits an error — providing self-healing for failed adds, manual removals, and pre-deployment issues. The label is defined alongside existing PRD lifecycle labels and created at daemon startup via `ensure_prd_labels_best_effort`. It does not apply to non-PRD daemon workflows.

## Acceptance Criteria

- `ralph:waiting-feedback` is present in `PRD_LABELS` (with color and description), `PRD_LABEL_NAMES`, and created by `ensure_prd_labels_best_effort_with_gh_bin` at daemon startup.
- `has_prd_label` returns `true` when `ralph:waiting-feedback` is among the issue labels.
- `has_in_progress_prd_label` returns `false` when `ralph:waiting-feedback` is the only PRD label present (it is not a poll-gate label).
- After `Pending → AwaitingAnswers` completes successfully, `ralph:waiting-feedback` is present on the issue. The label add runs unconditionally (not gated by `!has_active`), so retry/restart scenarios where `ralph:prd-active` is already present still enforce label presence.
- After `AwaitingAnswers → AwaitingFeedback` completes (draft posted), `ralph:waiting-feedback` remains present on the issue.
- When `AwaitingFeedback` processes non-approval feedback and posts a revised draft, `ralph:waiting-feedback` remains present.
- On terminal transitions (`Done`, `Failed`), `ralph:waiting-feedback` is removed **after** durable state save succeeds, following the same persistence-safe ordering as existing label removals.
- When the terminal state save fails, `ralph:waiting-feedback` is **not** removed — the label stays intact alongside poll-visible labels for retry.
- On no-op ticks in `AwaitingAnswers` (no answer found) and `AwaitingFeedback` (no new feedback, no approval) where the label is already present, the label is **not** toggled — no add or remove call occurs.
- On every waiting-state tick where `ralph:waiting-feedback` is absent (detected via fetched labels or issue data), it is reconciled (re-added) as best-effort, enabling self-healing. This includes no-op ticks, answer-found ticks, feedback-revision ticks, and error-retry ticks.
- The label does not apply to non-PRD daemon workflows — it is only managed within the `interactive_prd` module.
- All unit tests (`src/daemon/interactive_prd.rs` in-module tests), integration tests (`tests/daemon_interactive_prd.rs`), and conformance tests (`src/validate/tests_interactive_prd.rs`) are updated and passing.

## Technical Approach

### 1. Label definition (`src/daemon/interactive_prd.rs`)

Add `ralph:waiting-feedback` to `PRD_LABELS` with a distinct color (e.g., `#e4e669` — muted yellow) and description `"PRD workflow is waiting for user input"`. Add `"ralph:waiting-feedback"` to `PRD_LABEL_NAMES`. Do **not** add it to `IN_PROGRESS_PRD_LABEL_NAMES` — it is an informational overlay, not a poll-visibility label.

### 2. Helper: `ensure_waiting_feedback_label`

Introduce a small private helper function to centralize the reconciliation pattern:

```rust
fn ensure_waiting_feedback_label(
    config: &PrdPollConfig,
    issue_number: u32,
    labels: &[String],
) {
    if !labels.iter().any(|l| l == "ralph:waiting-feedback") {
        let _ = github::add_label_with_retry_with_gh_bin(
            &config.gh_bin,
            &config.owner,
            &config.repo,
            issue_number,
            "ralph:waiting-feedback",
        );
    }
}
```

This checks label presence and performs a best-effort add only when absent. Wrapping in `let _ =` ensures failures are non-fatal. All call sites use this helper for consistent behavior.

### 3. Apply on `Pending → AwaitingAnswers` (`do_pending_to_awaiting`)

**After** the label swap block (after line 1027, where `ralph:ready` removal completes) and **before** question generation, call `ensure_waiting_feedback_label(config, issue_number, &issue.labels)`. This runs **unconditionally** — outside the `!has_active` branch — so that retry/restart scenarios where `ralph:prd-active` is already present (and the label swap is skipped) still enforce the waiting-feedback label. On a fresh `Pending → AwaitingAnswers` transition, the label will always be absent from `issue.labels` (fetched before the add), so the helper always adds it. On a retry where the label was previously added, the GitHub API `add_label` is idempotent (no error). The add is best-effort — failure is non-fatal because reconciliation on subsequent ticks will recover.

### 4. Reconciliation at waiting-state entry (`do_awaiting_answers_to_awaiting_feedback`)

At function entry (after line 1167, once comments are fetched), before the `find_first_answer_comment` branching point, call `ensure_waiting_feedback_label(config, issue_number, &issue.labels)`. This runs on **every tick** of the `AwaitingAnswers` state — both no-op ticks (no answer found) and answer-found ticks — using the issue's label list passed in from the poll. Placement before branching ensures reconciliation covers all code paths including early returns.

### 5. Reconciliation at waiting-state entry (`do_awaiting_feedback`)

At function entry (after line 1300, once labels are fetched), before the approval check and feedback branching, call `ensure_waiting_feedback_label(config, issue_number, &labels)`. This runs on **every tick** of the `AwaitingFeedback` state — no-op, revision, and approval ticks alike — using the freshly fetched `labels` vec. Placement before branching ensures reconciliation covers all code paths. On the approval path, the label will be added and then promptly removed by the terminal transition; this is correct and harmless.

### 6. No-op tick optimization

The `ensure_waiting_feedback_label` helper already skips the add when the label is present (`!labels.iter().any(...)`). On no-op ticks where the label is already present, no GitHub API call is made. This satisfies the "not toggled" requirement.

### 7. Remove on `Done` transition (`do_approval_transition`)

After the durable state save succeeds and after removing `ralph:prd-active` (line ~1488), add a `remove_label_with_retry_with_gh_bin` call for `ralph:waiting-feedback`. Wrapped in `let _ =` matching the pattern used for `ralph:prd-active` removal failure (the label is non-critical for terminal issues). If the save fails (line 1466–1473), the function returns early **before** reaching the removal block, so the label stays intact — the issue remains poll-visible with `ralph:waiting-feedback` present for retry.

### 8. Remove on `Failed` transition (`transition_to_failed`)

After the durable state save succeeds and after removing `ralph:prd-active` and `ralph:prd` (lines ~2100–2113), add a `remove_label_with_retry_with_gh_bin` call for `ralph:waiting-feedback`. Wrapped in `let _ =` matching the existing best-effort cleanup pattern. If the save fails (line 2083–2096), the function returns early **before** reaching the removal block, so the label stays intact for retry.

### 9. Startup creation (`src/daemon/github.rs`)

No code change needed — `ensure_prd_labels_best_effort_with_gh_bin` iterates over `PRD_LABELS`, so adding the entry to the constant is sufficient.

### 10. Preserve through `AwaitingAnswers → AwaitingFeedback` state change

No explicit label action needed for the transition itself. The label remains from the previous state. The reconciliation at `do_awaiting_feedback` entry (section 5) ensures it is re-added if missing when the new state is first ticked.

### 11. Error-retry reconciliation

When a transition error occurs and `finish_transition` routes through `apply_transition_result` (incrementing `error_count`), the issue remains in its current waiting state. On the next daemon tick, the wrapper functions (`transition_awaiting_answers_to_awaiting_feedback` or `transition_awaiting_feedback`) re-enter the `do_*` functions, which run reconciliation at entry before any branching. This covers the error-retry case without additional code.

## Files & Modules

| File | Change |
|---|---|
| `src/daemon/interactive_prd.rs` | Add `ralph:waiting-feedback` to `PRD_LABELS` and `PRD_LABEL_NAMES`. Add `ensure_waiting_feedback_label` private helper. Call helper unconditionally in `do_pending_to_awaiting` (after label swap, before question generation). Call helper at entry of `do_awaiting_answers_to_awaiting_feedback` (before branching). Call helper at entry of `do_awaiting_feedback` (before branching). Add label removal in `do_approval_transition` (after save, after `ralph:prd-active` removal). Add label removal in `transition_to_failed` (after save, after existing label removals). Update in-module unit tests: `prd_labels_alias_matches_lifecycle_labels` (5 → 6), add `has_in_progress_prd_label` negative assertion for `ralph:waiting-feedback`. |
| `tests/daemon_interactive_prd.rs` | Update `prd_labels_have_expected_entries` to expect 6 labels and assert `ralph:waiting-feedback` presence. Add explicit `has_prd_label_detects_waiting_feedback` test. |
| `src/validate/tests_interactive_prd.rs` | Update `prd_labels_are_complete` to expect 6 entries and include `ralph:waiting-feedback` in expected list. Add conformance tests (see Testing Strategy). |
| `src/validate/tests_daemon.rs` | No code change needed — existing `label_ensure_*` tests compute `total_labels` dynamically from `REQUIRED_LABELS.len() + PRD_LABELS.len()`, so adding to `PRD_LABELS` automatically updates the expected count. Verify all three label-ensure tests pass. |

## Testing Strategy

### In-module unit tests (`src/daemon/interactive_prd.rs`)

- **`prd_labels_alias_matches_lifecycle_labels`**: Update assertion from `PRD_LABELS.len() == 5` to `PRD_LABELS.len() == 6`.
- **New: `has_in_progress_prd_label_rejects_waiting_feedback`**: Assert `has_in_progress_prd_label(&["ralph:waiting-feedback".to_owned()])` returns `false`. This locks the constraint that the label is not a poll-gate label.
- **`has_in_progress_prd_label_matches_each_in_progress_label`**: No change needed — the test iterates a hardcoded list of 4 in-progress labels which does not include `ralph:waiting-feedback`.
- **`has_in_progress_prd_label_rejects_done_empty_and_unrelated_labels`**: No change needed — the test uses explicit label values.

### Integration tests (`tests/daemon_interactive_prd.rs`)

- **`prd_labels_have_expected_entries`**: Update assertion from 5 to 6 entries; assert `ralph:waiting-feedback` is in the list.
- **`has_prd_label_detects_all_prd_labels`**: Already iterates `PRD_LABEL_NAMES`, so adding the entry to the constant automatically covers it. Verify this passes.
- **New: `has_prd_label_detects_waiting_feedback`**: Explicit test that `has_prd_label(&["ralph:waiting-feedback".to_owned()])` returns `true`.

### Conformance tests (`src/validate/tests_interactive_prd.rs`)

- **`prd_labels_are_complete`**: Update expected count from 5 to 6; add `"ralph:waiting-feedback"` to the expected label list.
- **New: `waiting_feedback_label_applied_on_pickup`**: Mock `gh` to log label operations. Set up a `Pending` issue (with `ralph:prd` label). Run one daemon tick. Assert `--add-label ralph:waiting-feedback` appears in the log after `--add-label ralph:prd-active`.
- **New: `waiting_feedback_label_applied_on_pickup_when_active_already_present`**: Mock `gh` to log label operations. Set up a `Pending` issue with `ralph:prd-active` already in its label list (simulating retry/restart where active was already added). Run one daemon tick. Assert `--add-label ralph:waiting-feedback` still appears in the log (unconditional, not gated by `!has_active`).
- **New: `waiting_feedback_label_removed_on_done`**: Set up an `AwaitingFeedback` issue with approval. Run one daemon tick. Assert `--remove-label ralph:waiting-feedback` appears in the log after `ralph:prd-done` add and state save.
- **New: `waiting_feedback_label_removed_on_failed`**: Set up an issue at error exhaustion. Run one daemon tick. Assert `--remove-label ralph:waiting-feedback` appears in the log after state save.
- **New: `waiting_feedback_label_not_removed_on_done_save_failure`**: Set up an `AwaitingFeedback` issue with approval. Inject `RALPH_TEST_INJECT_SAVE_FAILURE=1`. Run one daemon tick. Assert `--remove-label ralph:waiting-feedback` does **not** appear in the label operation log. Verify state remains `AwaitingFeedback`.
- **New: `waiting_feedback_label_not_removed_on_failed_save_failure`**: Set up an `AwaitingFeedback` issue with error_count=2 (at exhaustion threshold). Inject `RALPH_TEST_INJECT_SAVE_FAILURE=1`. Run one daemon tick. Assert `--remove-label ralph:waiting-feedback` does **not** appear in the log. Verify state remains `AwaitingFeedback`.
- **New: `waiting_feedback_reconciliation_on_noop_tick`**: Set up an `AwaitingAnswers` issue with no answer comments. Mock `gh issue view` / label list to report `ralph:waiting-feedback` as absent. Run one daemon tick. Assert `--add-label ralph:waiting-feedback` appears in the log.
- **New: `waiting_feedback_reconciliation_on_answer_found_tick`**: Set up an `AwaitingAnswers` issue with a valid answer comment. Mock label list to report `ralph:waiting-feedback` as absent. Run one daemon tick. Assert `--add-label ralph:waiting-feedback` appears in the log (reconciliation runs before answer processing).
- **New: `waiting_feedback_reconciliation_on_feedback_revision_tick`**: Set up an `AwaitingFeedback` issue with non-approval feedback. Mock label list to report `ralph:waiting-feedback` as absent. Run one daemon tick. Assert `--add-label ralph:waiting-feedback` appears in the log (reconciliation runs before revision processing).
- **New: `waiting_feedback_noop_when_label_already_present`**: Set up an `AwaitingAnswers` issue with no answer comments. Mock label list to include `ralph:waiting-feedback` as already present. Run one daemon tick. Assert `--add-label ralph:waiting-feedback` does **not** appear in the log (no redundant API call). Assert `--remove-label ralph:waiting-feedback` also does not appear.

### Existing test validation

- All existing conformance tests (approval ordering, save failure recovery, concurrent processing, bot spoof resistance, etc.) must continue to pass. The label is added best-effort with `let _ =`, so existing tests that don't mock `ralph:waiting-feedback` removal in their gh scripts will see it as a no-op add that doesn't affect their assertions.
- The three `label_ensure_*` tests in `src/validate/tests_daemon.rs` compute expected counts dynamically from `PRD_LABELS.len()`, so they automatically expect 10 total (4 standard + 6 PRD) without code changes. Verify all three pass.

## Out of Scope

- Adding `ralph:waiting-feedback` to `IN_PROGRESS_PRD_LABEL_NAMES` or using it for poll visibility — it is informational only, not a poll-gate label.
- Applying the label to non-PRD daemon workflows (e.g., the standard `ralph:ready` → task pipeline).
- GitHub notification integration or webhook triggers based on the label.
- Backfilling the label onto issues that entered waiting states before deployment — reconciliation handles this on the next tick.
- UI or dashboard for filtering issues by this label — this is a GitHub-native label filter concern.
- Changing the label during the `AwaitingAnswers → AwaitingFeedback` transition — the label remains continuously applied across both waiting states.