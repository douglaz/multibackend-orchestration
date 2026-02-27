---
artifact: prompt-review
project: issue-92
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-27T03:37:16Z
---

# Prompt Review

## Issues Found
- The prompt mixes behavior requirements with brittle implementation details (exact line numbers, helper name, and call-site placement). This makes valid implementations fail review even if behavior is correct.
- It does not clearly define what should happen if `Pending -> AwaitingAnswers` work fails before the state transition is durably saved. That can leave `ralph:waiting-feedback` on non-waiting states unless explicitly accepted or prevented.
- “Reconcile on every waiting-state tick, including error ticks” is underspecified when label/comment fetch fails early. A precondition for reconciliation (labels available) is missing.
- The no-op requirement is split across multiple sections and can be read as conflicting (“always reconcile” vs “no add/remove on no-op ticks”). The exact condition should be “reconcile only when absent.”
- The test plan is overly coupled to specific test names and ordering, which is fragile and high-effort; it should require behavior coverage, not exact test function identities.
- “No code change needed” statements are assumptions, not requirements. The prompt should define observable outcomes (label is created at startup) and let implementation choose how.

## Refined Prompt
### Title
Add `ralph:waiting-feedback` label management to interactive PRD waiting states

### Goal
Introduce an informational PRD label, `ralph:waiting-feedback`, that indicates the issue is blocked on user input.  
This label must be managed only by the interactive PRD daemon workflow.

### Definitions
- Waiting states: `AwaitingAnswers`, `AwaitingFeedback`
- Terminal states: `Done`, `Failed`
- Tick: one daemon poll cycle for a workflow issue
- Reconciliation: best-effort re-add of `ralph:waiting-feedback` when missing

### Scope
- In scope: interactive PRD workflow label lifecycle and tests
- Out of scope:
  - Non-PRD daemon workflows
  - Poll-gating behavior changes
  - Webhooks/notifications/UI changes
  - Historical backfill beyond normal tick-based reconciliation

### Functional Requirements
1. Label catalog
- Add `ralph:waiting-feedback` to `PRD_LABELS` with:
  - color: `#e4e669`
  - description: `PRD workflow is waiting for user input`
- Add the name to `PRD_LABEL_NAMES`.
- Do not add it to `IN_PROGRESS_PRD_LABEL_NAMES`.

2. Startup label ensure
- Daemon startup must create/ensure `ralph:waiting-feedback` together with other PRD labels via existing PRD label ensure flow.
- Outcome required: startup ensure includes this label (implementation mechanism may vary).

3. Label detection helpers
- `has_prd_label(...)` must return `true` when `ralph:waiting-feedback` is present.
- `has_in_progress_prd_label(...)` must return `false` when `ralph:waiting-feedback` is the only PRD label.

4. Apply/reconcile in waiting flows
- Add a private helper (name not mandated) that:
  - receives current labels for the issue
  - calls GitHub add-label best-effort only when `ralph:waiting-feedback` is absent
  - never fails the transition/tick if add fails
- Call this helper in:
  - `Pending -> AwaitingAnswers` handling, unconditionally (not gated by `!has_active`)
  - each `AwaitingAnswers` tick
  - each `AwaitingFeedback` tick
- Reconciliation must run before branch-specific logic in waiting handlers so it applies to no-op, processing, and retry/error paths when labels are available.

5. No-op behavior
- If `ralph:waiting-feedback` is already present on a waiting-state tick, do not call add/remove for that label.
- No toggling on no-op ticks.

6. Terminal removal behavior
- On successful transition to `Done`:
  - remove `ralph:waiting-feedback` best-effort
  - perform removal only after durable state save succeeds
  - preserve existing ordering guarantees used for other terminal label removals
- On successful transition to `Failed`:
  - same rule: remove only after durable state save succeeds
- If terminal state save fails, do not remove `ralph:waiting-feedback`.

### Implementation Targets
- Primary file: `src/daemon/interactive_prd.rs`
- Label ensure behavior validated through daemon startup path (`src/daemon/github.rs` integration point)
- No changes required outside PRD workflow unless needed for compilation/tests

### Acceptance Criteria
- `PRD_LABELS` and `PRD_LABEL_NAMES` include `ralph:waiting-feedback`; `IN_PROGRESS_PRD_LABEL_NAMES` does not.
- Startup label ensure attempts creation of `ralph:waiting-feedback`.
- Waiting label is enforced on:
  - successful `Pending -> AwaitingAnswers` path
  - each `AwaitingAnswers` tick
  - each `AwaitingFeedback` tick
- Missing waiting label in waiting states is re-added best-effort on the next eligible tick.
- Label is removed on successful `Done` and `Failed` terminal commits, and not removed when terminal save fails.
- No management of this label in non-PRD workflows.

### Test Requirements
Cover behavior at three levels:

1. Unit tests (`src/daemon/interactive_prd.rs`)
- Label constants include/exclude the new label in correct sets.
- `has_prd_label` positive case for `ralph:waiting-feedback`.
- `has_in_progress_prd_label` negative case for `ralph:waiting-feedback`.

2. Integration tests (`tests/daemon_interactive_prd.rs`)
- PRD label set contains the new label and expected count.
- Helper behavior in exported APIs reflects new classification.

3. Conformance tests (`src/validate/tests_interactive_prd.rs`)
- Startup ensure includes the new label.
- Add on pickup from `Pending`, including retry scenario where `ralph:prd-active` already exists.
- Reconciliation when waiting label is missing during:
  - `AwaitingAnswers` no-op tick
  - `AwaitingAnswers` answer-processing tick
  - `AwaitingFeedback` revision tick
- No redundant add/remove when label already present on waiting no-op tick.
- Removal on successful `Done` and `Failed`.
- No removal on terminal save failure.

### Verification Commands
- `nix develop -c cargo check`
- `nix develop -c cargo test`
- Build + conformance:
  - `nix build -L`
  - `./result/bin/ralph validate --bin ./result/bin/ralph`
