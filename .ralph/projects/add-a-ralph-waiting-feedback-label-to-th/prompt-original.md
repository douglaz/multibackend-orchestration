## Summary

Add a `ralph:waiting-feedback` label to the interactive PRD pipeline that serves as a cross-issue filterable signal indicating the workflow is waiting for user input. The label is applied when the workflow enters `AwaitingAnswers` or `AwaitingFeedback` states and removed on terminal transitions (`Done`, `Failed`). All label additions are placed **after durable state save succeeds** to maintain persistence-safe ordering — the label is never present unless the underlying state actually reflects a waiting state. A self-healing reconciliation step runs at the end of every per-issue tick (after both successful and failed dispatches) to re-add the label if it was externally removed while the issue remains in a waiting state, and to remove stale labels if the issue is in a non-waiting, non-terminal state (e.g., `Pending` after a failed transition revert). All label operations use best-effort semantics to avoid blocking workflow progress.

## Acceptance Criteria

- `ralph:waiting-feedback` is defined in `PRD_LABELS` (6th entry), `PRD_LABEL_NAMES` (6th entry), and created by `ensure_prd_labels_best_effort` at daemon startup
- After `Pending → AwaitingAnswers` completes (questions posted) **and durable state save succeeds**, `ralph:waiting-feedback` is present on the issue
- After `AwaitingAnswers → AwaitingFeedback` completes (answers received, draft posted) **and durable state save succeeds**, `ralph:waiting-feedback` is present on the issue
- When `AwaitingFeedback` processes non-approval feedback and posts a revised draft, `ralph:waiting-feedback` remains present
- On terminal transitions (`Done`, `Failed`), `ralph:waiting-feedback` is removed only after durable state save succeeds
- When no user input is detected in `AwaitingAnswers` or `AwaitingFeedback` (no-op tick) and `ralph:waiting-feedback` is already present, no `add_label` or `remove_label` calls are made for `ralph:waiting-feedback`
- When no user input is detected in `AwaitingAnswers` or `AwaitingFeedback` (no-op tick) and `ralph:waiting-feedback` is missing, reconciliation re-adds the label as a best-effort operation
- `has_prd_label` detects `ralph:waiting-feedback` as a PRD label
- On every `AwaitingAnswers` and `AwaitingFeedback` tick where the end-of-tick state is still a waiting state, if `ralph:waiting-feedback` is not present, it is reconciled (re-added) as a best-effort operation
- On every tick where the end-of-tick state is `Pending` (e.g., after a failed transition revert) and `ralph:waiting-feedback` is present, reconciliation removes the stale label as a best-effort operation
- Unit tests updated to expect 6 PRD labels instead of 5 across all three assertion sites
- Conformance tests added covering label lifecycle across all transition types, including failed dispatch reconciliation and terminal removal failure resilience
- Manual verification confirms label behavior in running daemon (listed as final acceptance criterion for tracking)

## Technical Approach

### 1. Label constant registration (`interactive_prd.rs`)

Add `ralph:waiting-feedback` as the 6th entry in `PRD_LABELS` (line ~173) with color `#d876e3` (purple, signaling "user action required") and description `"Interactive PRD workflow is waiting for user input"`. Add `"ralph:waiting-feedback"` to `PRD_LABEL_NAMES` (line ~575) as the 6th entry. Do **not** add it to `IN_PROGRESS_PRD_LABEL_NAMES` — the waiting-feedback label is an informational overlay, not a poll-routing label.

No changes needed to `ensure_prd_labels_best_effort` or `ensure_prd_labels_best_effort_with_gh_bin` in `github.rs` — those functions already iterate over `PRD_LABELS` dynamically, so adding the entry is sufficient.

### 2. Apply label after durable state save on forward transitions (`interactive_prd.rs`)

The label must only be added **after** the transition's durable state save succeeds. This prevents the label from appearing on issues whose in-memory state was reverted (e.g., if question generation or save fails, state remains `Pending` and the label must not be present). The primary add sites are in `finish_transition`, not in the individual `do_*` functions.

**`finish_transition` (line ~1617):** After the successful state save (line ~1642–1671) and before returning `Ok(result)`, add a best-effort label application for transitions that land in a waiting state:

```rust
// After successful save, best-effort add ralph:waiting-feedback for waiting states.
if matches!(state.state, PrdWorkflowState::AwaitingAnswers | PrdWorkflowState::AwaitingFeedback) {
    let _ = github::add_label_with_retry_with_gh_bin(
        &config.gh_bin, &config.owner, &config.repo,
        state.issue_number, "ralph:waiting-feedback",
    );
}
```

This placement is safe because:
- It runs after `state.save()` succeeds, maintaining the persistence-safe ordering invariant.
- It covers all forward transitions into waiting states (`Pending → AwaitingAnswers`, `AwaitingAnswers → AwaitingFeedback`, `AwaitingFeedback → AwaitingFeedback` revision loop) via a single call site.
- It is idempotent — GitHub's `add_label` is a no-op when the label is already present.
- It also covers the special case of the `Done` early-return path (line ~1635–1637): since `Done` is not a waiting state, the condition does not match, so no label is added on that path (which is correct — `do_approval_transition` handles its own save and label removal).

**`do_pending_to_awaiting`, `do_awaiting_answers_to_awaiting_feedback`, `do_awaiting_feedback` revision path:** No `add_label` calls for `ralph:waiting-feedback` in these functions. Label addition is centralized in `finish_transition` post-save. This eliminates the ordering violation where a label could be added before save.

### 3. Remove label on terminal transitions (`interactive_prd.rs`)

**`do_approval_transition` (line ~1408):** After the durable state save succeeds (line ~1466) and before/alongside the existing `ralph:prd-active` removal (line ~1477), add a best-effort removal:

```rust
// Best-effort: remove ralph:waiting-feedback on terminal Done
let _ = github::remove_label_with_retry_with_gh_bin(
    &config.gh_bin, owner, repo, issue_number,
    "ralph:waiting-feedback",
);
```

Placement: between the save success check (line ~1473) and the `ralph:prd-active` removal (line ~1477). This follows the same "save before remove" pattern. The `let _ =` semantics mean failure is logged but does not block the transition.

**`transition_to_failed` (line ~2022):** After the durable state save succeeds (line ~2097) and alongside the existing `ralph:prd-active` / `ralph:prd` removals (lines 2100–2113), add the same best-effort removal:

```rust
// Best-effort: remove ralph:waiting-feedback on terminal Failed
let _ = github::remove_label_with_retry_with_gh_bin(
    &config.gh_bin, owner, repo, issue_number,
    "ralph:waiting-feedback",
);
```

### 4. Self-healing reconciliation (`interactive_prd.rs`)

Add a helper function that handles both re-adding missing labels in waiting states **and** removing stale labels in non-waiting, non-terminal states:

```rust
/// Best-effort reconciliation: ensure `ralph:waiting-feedback` consistency
/// after each per-issue tick.
///
/// - **Waiting states** (`AwaitingAnswers`, `AwaitingFeedback`): add the
///   label if missing (covers external removal and failed-add recovery).
/// - **Non-waiting, non-terminal states** (`Pending`): remove the label if
///   present (covers revert-on-save-failure where the transition rolled back
///   to Pending but the label was added by a partially-failed prior attempt
///   or external drift).
/// - **Terminal states** (`Done`, `Failed`): no action — terminal removal
///   is handled in `do_approval_transition`/`transition_to_failed` and is
///   not retried on subsequent ticks per spec.
fn reconcile_waiting_feedback_label(
    config: &PrdPollConfig,
    state: &InteractivePrdState,
    issue_labels: &[String],
) {
    let is_waiting = matches!(
        state.state,
        PrdWorkflowState::AwaitingAnswers | PrdWorkflowState::AwaitingFeedback
    );
    let is_terminal = matches!(
        state.state,
        PrdWorkflowState::Done | PrdWorkflowState::Failed
    );
    let has_label = issue_labels.iter().any(|l| l == "ralph:waiting-feedback");

    if is_waiting && !has_label {
        let _ = github::add_label_with_retry_with_gh_bin(
            &config.gh_bin, &config.owner, &config.repo,
            state.issue_number, "ralph:waiting-feedback",
        );
    } else if !is_waiting && !is_terminal && has_label {
        // Stale label on non-waiting, non-terminal state (e.g., Pending after
        // revert). Remove best-effort.
        let _ = github::remove_label_with_retry_with_gh_bin(
            &config.gh_bin, &config.owner, &config.repo,
            state.issue_number, "ralph:waiting-feedback",
        );
    }
    // Terminal states: no action. Removal is handled in the terminal
    // transition functions and not retried here.
}
```

**Call site:** Inside the per-issue worker loop in `poll_and_advance_prd` (line ~824), after the `match result` block completes (after both `Ok` and `Err` paths, and after the panic `catch_unwind` recovery block at line ~879), invoke reconciliation unconditionally. This ensures reconciliation runs after every dispatch — on success, failure, and even after panic recovery.

```rust
// Post-tick reconciliation: ensure ralph:waiting-feedback consistency.
// Runs unconditionally after every dispatch (success, failure, panic).
if let Ok(Some(post_state)) = InteractivePrdState::load(
    &worker_config.data_dir, &worker_config.owner,
    &worker_config.repo, issue_number,
) {
    if !post_state.is_terminal() {
        if let Ok(labels) = github::fetch_issue_labels_with_gh_bin(
            &worker_config.gh_bin, &worker_config.owner,
            &worker_config.repo, issue_number,
        ) {
            reconcile_waiting_feedback_label(&worker_config, &post_state, &labels);
        }
    }
}
```

The `!post_state.is_terminal()` guard avoids an unnecessary label fetch for terminal states (where reconciliation takes no action). The label fetch uses `fetch_issue_labels_with_gh_bin` which is already used in `do_awaiting_feedback` and follows existing patterns.

### 5. Ordering guarantees

The `ralph:waiting-feedback` label follows a simpler ordering model than the existing lifecycle labels because it is an **informational overlay**, not a poll-routing label:

- **Add (forward transition):** Best-effort, after durable state save succeeds in `finish_transition`. This is the critical ordering difference from the original spec — the label is never added before state persistence. Reconciliation covers any missed adds.
- **Add (reconciliation):** Best-effort, at end-of-tick after re-reading persisted state and live labels. Catches external removal and any adds missed by transient GitHub API failures.
- **Remove (terminal):** Best-effort, after durable state save succeeds in `do_approval_transition` / `transition_to_failed`. Follows the same "save before remove" pattern as `ralph:prd-active`. No reconciliation retry on subsequent ticks — the label may linger if removal fails, but this is acceptable since the issue is in a terminal state and won't be polled again.
- **Remove (stale):** Best-effort, via reconciliation when post-tick state is non-waiting and non-terminal (e.g., `Pending`). Catches labels stranded by failed transitions that reverted to `Pending`.

## Files & Modules

| File | Changes |
|------|---------|
| `src/daemon/interactive_prd.rs` | Add `ralph:waiting-feedback` to `PRD_LABELS` and `PRD_LABEL_NAMES`. Add post-save best-effort `add_label` in `finish_transition` for waiting states. Add best-effort `remove_label` calls in `do_approval_transition` and `transition_to_failed`. Add `reconcile_waiting_feedback_label` helper (handles both re-add and stale removal). Add reconciliation call site in the worker loop of `poll_and_advance_prd` after the `catch_unwind` match block. Update inline unit test `prd_labels_alias_matches_lifecycle_labels` to expect 6 labels (line ~2407). |
| `tests/daemon_interactive_prd.rs` | Update `prd_labels_have_expected_entries` to expect 6 labels and add `"ralph:waiting-feedback"` to the `names.contains()` assertions (line ~131). |
| `src/validate/tests_interactive_prd.rs` | Update `prd_labels_are_complete` to expect 6 labels (line ~279). Update `prd_label_detection_filters_correctly` to include `ralph:waiting-feedback`. Add conformance tests for label lifecycle (see Testing Strategy). |

No changes to `src/daemon/github.rs` — `ensure_prd_labels_best_effort_with_gh_bin` already iterates `PRD_LABELS` dynamically. No changes to `src/daemon/runtime.rs` — PRD phase dispatch is unchanged.

## Testing Strategy

### Existing test updates

1. **`prd_labels_alias_matches_lifecycle_labels`** (inline unit test in `interactive_prd.rs`, line ~2407): Change `assert_eq!(PRD_LABELS.len(), 5)` to `assert_eq!(PRD_LABELS.len(), 6)`.

2. **`prd_labels_have_expected_entries`** (integration test in `tests/daemon_interactive_prd.rs`, line ~131): Change `assert_eq!(PRD_LABELS.len(), 5)` to `assert_eq!(PRD_LABELS.len(), 6)`. Add `assert!(names.contains(&"ralph:waiting-feedback"))`.

3. **`prd_labels_are_complete`** (conformance test in `tests_interactive_prd.rs`, line ~279): Change expected count from 5 to 6. Add `"ralph:waiting-feedback"` to the `expected` array.

4. **`prd_label_detection_filters_correctly`**: The existing loop over `PRD_LABEL_NAMES` already covers any new entry. No code change needed beyond the constant update, but verify the test still passes.

### New conformance tests

All new tests follow the existing `ConformanceTest` pattern with `RalphHarness` and mock `gh` scripts (same pattern as `pickup_and_question_posting`, `approval_by_comment`, etc.).

5. **`waiting_feedback_label_applied_on_pickup`**: Exercise `Pending → AwaitingAnswers` via mock backends. Assert `ralph:waiting-feedback` is present in the `add_label` calls recorded by the mock `gh` script **after** the state save (verify ordering by checking that the label-add call appears after the state file is written).

6. **`waiting_feedback_label_applied_on_draft_posting`**: Exercise `AwaitingAnswers → AwaitingFeedback` via mock backends. Assert `ralph:waiting-feedback` is in the recorded label-add calls after state save.

7. **`waiting_feedback_label_persists_on_revision`**: Exercise the feedback revision loop. Assert `ralph:waiting-feedback` is **not removed** during the revision (remains present via reconciliation or idempotent re-add).

8. **`waiting_feedback_label_removed_on_done`**: Exercise `AwaitingFeedback → Done` (approval by comment). Assert `ralph:waiting-feedback` appears in the recorded `remove_label` calls, and only after the durable state save.

9. **`waiting_feedback_label_removed_on_failed`**: Exercise error exhaustion → `Failed`. Assert `ralph:waiting-feedback` appears in the recorded `remove_label` calls after save.

10. **`waiting_feedback_reconciliation_re_adds_on_noop_tick`**: Set up an issue in `AwaitingAnswers` state with mock labels that exclude `ralph:waiting-feedback`. Run a no-op tick (no new answer). Assert reconciliation adds the label.

11. **`waiting_feedback_noop_tick_does_not_toggle`**: Set up an issue in `AwaitingAnswers` state with `ralph:waiting-feedback` already present. Run a no-op tick. Assert no `add_label` or `remove_label` calls for `ralph:waiting-feedback`.

12. **`waiting_feedback_reconciliation_after_failed_dispatch`**: Set up an issue in `AwaitingAnswers` state with `ralph:waiting-feedback` missing. Configure mock backends to fail the transition (e.g., question generation error). Run a tick that results in a dispatch error. Assert that reconciliation still adds `ralph:waiting-feedback` at end-of-tick because the post-tick state is still `AwaitingAnswers` (error count < 3).

13. **`waiting_feedback_terminal_removal_failure_does_not_block`**: Exercise `AwaitingFeedback → Done` with mock `gh` script configured to fail the `remove_label` call for `ralph:waiting-feedback`. Assert the terminal `Done` state was still persisted successfully and the transition completed. Verify no retry attempt for the label removal on a subsequent tick.

14. **`waiting_feedback_stale_label_removed_on_pending`**: Set up an issue in `Pending` state with `ralph:waiting-feedback` present (simulating a label stranded by a failed transition revert). Run a tick. Assert reconciliation removes the stale label.

### Manual verification

15. **Manual daemon test**: Run the daemon against a test repository, trigger a full PRD lifecycle (`Pending → AwaitingAnswers → AwaitingFeedback → Done`), and confirm via GitHub UI that `ralph:waiting-feedback` appears when entering waiting states and disappears on terminal transitions. Manually remove the label during `AwaitingFeedback` and confirm it is re-added on the next poll tick. This is tracked as an acceptance criterion.

## Out of Scope

- Adding `ralph:waiting-feedback` to `IN_PROGRESS_PRD_LABEL_NAMES` or using it for poll-routing decisions — it is informational only
- Reconciliation for terminal states — if `remove_label` fails on `Done`/`Failed`, the label may linger; this is acceptable
- Applying the label to the standard daemon workflow (`ralph:ready` → `ralph:in-progress` → `ralph:completed`)
- Webhook-based real-time label updates — the daemon operates on a polling model
- UI or notification integrations beyond the GitHub label itself