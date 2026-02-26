---
artifact: prompt-review
project: add-a-ralph-waiting-feedback-label-to-th
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-26T16:44:38Z
---

# Prompt Review

## Issues Found
- The prompt is overly tied to approximate line numbers and exact insertion points, which makes implementation brittle as files change.
- Acceptance criteria repeat the same behavior in multiple bullets, increasing the chance of partial or inconsistent interpretation.
- One proposed test expects proving operation ordering by file-write/call sequence, which is hard to assert reliably in conformance harnesses; behavior-based assertions are safer.
- Test file locations are inconsistent (`tests/...` vs `src/validate/...`), which can cause implementation in the wrong suite.
- Reconciliation timing is described in multiple places but not as a single required invariant, making “after every tick” easy to miss.
- Manual verification is listed like a hard acceptance gate; without stating it is non-blocking, CI completion criteria are ambiguous.
- The prompt does not explicitly state expected behavior for pre-existing waiting issues missing the label at daemon start; this should be covered by first-tick reconciliation.
- The prompt implies best-effort semantics but does not require non-blocking behavior verification in tests for both add and remove failures.

## Refined Prompt
## Summary
Implement a new informational GitHub label, `ralph:waiting-feedback`, for the interactive PRD workflow so waiting issues are easily filterable across repositories.

## Objective
Ensure `ralph:waiting-feedback` accurately reflects workflow waiting states with persistence-safe ordering:
- Add only after durable state save succeeds when entering or remaining in waiting states.
- Remove only after durable state save succeeds when entering terminal states.
- Reconcile drift after every per-issue tick using persisted post-tick state.

## State Definitions
- Waiting states: `AwaitingAnswers`, `AwaitingFeedback`
- Terminal states: `Done`, `Failed`
- Non-waiting non-terminal state relevant here: `Pending`

## Functional Requirements
- Register `ralph:waiting-feedback` as the 6th PRD label in both `PRD_LABELS` and `PRD_LABEL_NAMES`.
- Do not include this label in `IN_PROGRESS_PRD_LABEL_NAMES`.
- `has_prd_label` must recognize `ralph:waiting-feedback`.
- On transitions whose persisted end-state is waiting (`Pending -> AwaitingAnswers`, `AwaitingAnswers -> AwaitingFeedback`, `AwaitingFeedback -> AwaitingFeedback` revision), best-effort add `ralph:waiting-feedback` after successful save.
- On terminal transitions (`Done`, `Failed`), best-effort remove `ralph:waiting-feedback` after successful save.
- Add end-of-tick reconciliation after every per-issue dispatch path (success, error, panic recovery):
- If persisted post-tick state is waiting and label is missing, best-effort add it.
- If persisted post-tick state is non-waiting and non-terminal (`Pending`) and label is present, best-effort remove stale label.
- If persisted post-tick state is terminal, do nothing in reconciliation.
- No-op waiting tick with label already present must not call add/remove for this label.
- All label operations are best-effort and must never block workflow progress.

## Implementation Constraints
- Primary file: `src/daemon/interactive_prd.rs`.
- Update existing PRD label assertions in unit/integration tests to expect 6 labels.
- Update conformance tests in `src/validate/` interactive PRD suite.
- No functional changes required in label bootstrap helpers if they already iterate `PRD_LABELS`.
- Avoid line-number-driven edits; use function names and behavior as anchors.

## Test Requirements
Add or update automated tests to cover:
- PRD label set now contains 6 entries including `ralph:waiting-feedback`.
- Label is added after successful transition save into `AwaitingAnswers`.
- Label is added after successful transition save into `AwaitingFeedback`.
- Label remains present through non-approval revision loops.
- Label is removed after successful transition save to `Done`.
- Label is removed after successful transition save to `Failed`.
- No-op waiting tick with missing label triggers reconciliation add.
- No-op waiting tick with label already present triggers no add/remove call.
- Failed dispatch where state remains waiting still runs reconciliation and re-adds if missing.
- Pending state with stale waiting label triggers reconciliation removal.
- Terminal removal failure is non-blocking and does not prevent terminal state persistence.
- Persistence-safe ordering is validated behaviorally (for example: if save fails, label must not be newly added), not by brittle call timestamp checks.

## Acceptance Criteria
- `ralph:waiting-feedback` exists in PRD label constants as the 6th entry and is created at startup through existing PRD label bootstrap flow.
- Waiting states always converge to label-present, including external label deletion and transient GitHub failures.
- Terminal transitions attempt label removal post-save without blocking completion.
- Pending state converges to label-absent if stale label is present.
- Existing tests updated from 5 to 6 labels where applicable.
- New/updated conformance tests in `src/validate/` cover the lifecycle and reconciliation scenarios above.
- Manual daemon verification is recommended for tracking, but automated tests are the merge gate.

## Out of Scope
- Using `ralph:waiting-feedback` for poll routing.
- Terminal-state reconciliation retries after workflow completion.
- Any non-interactive PRD/standard daemon workflow label changes.
- Webhook/event-driven behavior outside current polling model.
