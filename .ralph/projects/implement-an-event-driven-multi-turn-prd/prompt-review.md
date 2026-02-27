---
artifact: prompt-review
project: implement-an-event-driven-multi-turn-prd
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-22T04:08:17Z
---

# Prompt Review

## Issues Found
- The prompt conflicts on execution granularity: it says one step per poll tick, but also says one backend call per issue per cycle while question generation needs at least three backend calls (A, B, synthesis). This blocks deterministic implementation.
- Idempotency rules are incomplete: markers are defined, but no exact rules for when to create a new comment vs skip vs replace. This risks duplicate spam or missed updates.
- Append-only comments conflict with “final spec posted/updated.” Editing is out of scope, so “updated” is ambiguous and can cause inconsistent behavior.
- User-comment detection relies on marker absence, which can misclassify other bots as users. Author-based filtering is required for correctness.
- Approval detection is underspecified for negations and code blocks (for example, “not approved” or quoted text), which can trigger false positives.
- Multi-comment handling is unclear: no rule for whether to use first, latest, or aggregated feedback, and no cursor for “already processed” comments.
- Failure policy is ambiguous: “unrecoverable error” is not defined, and no retry threshold/backoff behavior is specified.
- Label precedence is unclear when an issue has both `ralph:ready` and `ralph:prd`, creating potential workflow collisions.
- Config validation is missing for `daemon_prd_question_backends` cardinality and backend spec parsing, which can fail at runtime.
- Testing requirements do not include validate conformance coverage for this new feature, which is required by project conventions.

## Refined Prompt
Implement a daemon-only interactive PRD workflow triggered by GitHub issues labeled `ralph:prd`. The workflow must be state-machine-driven, restart-safe via persisted JSON state, and non-blocking within the existing 60-second poll architecture.

### Goal
Add an event-driven, multi-turn PRD conversation flow where Ralph:
1. Picks up `ralph:prd` issues.
2. Asks clarifying questions using two backends plus synthesis.
3. Waits for user answers in issue comments.
4. Produces a 6-section engineering spec using the existing quick-prd writer/reviewer pattern.
5. Iterates on user feedback until approval.
6. Marks completion with lifecycle labels and persisted terminal state.

### Hard Constraints
- No long-running child processes.
- The daemon loop remains polling-based.
- At most one state transition per issue per poll tick.
- A transition may perform multiple backend calls, but must complete within `daemon_prd_backend_timeout_secs` (default `120`).
- All state transitions are persisted atomically to disk (write temp + rename).
- Existing `ralph:ready` workflow behavior must not regress.

### Lifecycle Labels
Ensure these labels exist at daemon startup (idempotent create):
- `ralph:prd`
- `ralph:prd-active`
- `ralph:prd-approved`
- `ralph:prd-done`
- `ralph:prd-failed`

### State Machine
Use `PrdWorkflowState` persisted in `InteractivePrdState`:

- `Pending`
- `AwaitingAnswers`
- `AwaitingFeedback`
- `Done`
- `Failed`

Transition rules:
1. `Pending` -> `AwaitingAnswers`
- Trigger: issue has `ralph:prd`.
- Actions:
- Swap labels: remove `ralph:prd`, add `ralph:prd-active`.
- If `ralph:ready` exists, remove it to avoid dual workflows.
- Generate questions:
- Backend A: 3-5 clarifying questions.
- Backend B: 3-5 clarifying questions.
- Synthesis backend: merge/dedupe/prioritize into final numbered list.
- Post questions comment with marker `<!-- ralph:prd:{issue_number}:questions-v{n} -->`.
- Save `questions_comment_id`, `questions_posted_at`, `question_revision`.
- Idempotency: if same marker already exists, do not post duplicate.

2. `AwaitingAnswers` -> `AwaitingFeedback`
- Trigger: first unprocessed non-bot comment after `questions_posted_at`.
- Actions:
- Extract answer text from that comment.
- Generate draft using existing quick-prd pipeline (writer + reviewer + section validation with `check_spec_sections()`).
- Respect `daemon_prd_max_revisions` for internal writer/reviewer retries (default `3`).
- Post draft comment with marker `<!-- ralph:prd:{issue_number}:draft-v{n} -->`.
- Save `draft_revision`, `latest_draft_comment_id`, `latest_draft_body`, `last_processed_comment_id`.
- Idempotency: one draft comment per `draft-v{n}` marker.

3. `AwaitingFeedback` -> `Done`
- Trigger (either):
- Any new unprocessed non-bot comment after latest draft that passes approval detection.
- Issue has label `ralph:prd-approved`.
- Actions:
- Post final status comment with marker `<!-- ralph:prd:{issue_number}:status-approved-v{n} -->` referencing the latest draft (append-only; do not edit old comments).
- Label update: remove `ralph:prd-active`, add `ralph:prd-done` (leave `ralph:prd-approved` if user applied it).
- Persist terminal `Done`.

4. `AwaitingFeedback` -> `AwaitingFeedback` (revision loop)
- Trigger: new unprocessed non-bot feedback comments with no approval.
- Actions:
- Aggregate all new feedback comments since `last_processed_comment_id` in chronological order.
- Send revision prompt to writer backend with current draft + aggregated feedback.
- Run reviewer/section validation as in draft generation.
- Increment `draft_revision`, post new draft marker `draft-v{n}`, update cursor fields.

5. Any state -> `Failed`
- Trigger: unrecoverable error or retry exhaustion.
- Recoverable errors increment `error_count`; retry next tick.
- Unrecoverable condition: `error_count >= 3` for the same stage.
- Actions:
- Post error comment marker `<!-- ralph:prd:{issue_number}:status-failed -->`.
- Label update: remove `ralph:prd-active`, add `ralph:prd-failed`.
- Persist terminal `Failed` with `last_error`.

### Comment and Approval Rules
- Fetch comments as structured data: `id`, `author_login`, `body`, `created_at`.
- “Bot comment” is determined by `author_login == daemon bot login`, not marker absence.
- Ignore bot comments for user-input detection.
- Approval detection helper:
- Strip fenced code blocks and inline code before matching.
- Negative phrases checked first: `not approved`, `don't approve`, `do not approve`, `not lgtm`.
- Positive phrases with word boundaries (case-insensitive): `approved`, `lgtm`, `ship it`, `looks good`.
- If both positive and negative exist, treat as non-approval and continue revision flow.

### Persistence
Persist per issue at:
`{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json`

Minimum persisted fields:
- `issue_number`, `owner`, `repo`
- `state`
- `question_revision`, `draft_revision`
- `questions_comment_id`, `questions_posted_at`
- `latest_draft_comment_id`, `latest_draft_body`
- `user_answers`
- `last_processed_comment_id`
- `error_count`, `last_error`
- `last_advanced_at`

### Configuration
Add to `WorkspaceConfig` with defaults:
- `daemon_prd_enabled: bool = true`
- `daemon_prd_question_backends: Vec<String> = ["claude", "codex"]`
- `daemon_prd_writer_backend: String = "claude"`
- `daemon_prd_reviewer_backend: String = "codex"`
- `daemon_prd_max_revisions: u32 = 3`
- `daemon_prd_backend_timeout_secs: u64 = 120`

Validation:
- `daemon_prd_question_backends` must contain exactly 2 backend specs.
- All backend specs must parse with existing backend spec parser.
- Invalid config fails fast at startup with clear error.

### Required Code Changes
- New: `src/daemon/interactive_prd.rs`
- Optional split: `src/daemon/interactive_prd/state.rs`
- Modify: `src/daemon/mod.rs` to export module
- Modify: `src/daemon/runtime.rs` to call PRD poll/advance alongside existing loops
- Modify: `src/daemon/github.rs` to add `fetch_issue_comments()` and label helpers
- Modify: `src/config/global.rs` for config fields/defaults
- Modify: `src/error.rs` for explicit interactive PRD error variant(s)

### Acceptance Criteria
- Daemon polls and advances `ralph:prd` issues without breaking `ralph:ready` flow.
- PRD lifecycle labels are ensured at startup.
- First PRD pickup posts one idempotent questions comment and moves to active.
- User answers produce one idempotent draft comment containing required sections.
- Feedback produces incremented draft revisions.
- Approval via comment or `ralph:prd-approved` moves issue to `ralph:prd-done`.
- Failures after retry exhaustion move issue to `ralph:prd-failed` with error comment.
- State survives daemon restart and resumes from last persisted state.
- No duplicate comments are posted when reprocessing same tick inputs.
- Existing tests pass.

### Testing Requirements
Add tests in three layers:

1. Unit tests (`src/daemon/interactive_prd.rs`):
- State serialization round-trip for all states.
- Approval detection positive/negative/edge cases.
- Comment extraction and bot filtering.
- Idempotent marker detection.
- State transitions for each path, including retries and failure.

2. Integration tests (`tests/` if existing daemon integration pattern is present):
- Persistence across simulated restart.
- Label conflict behavior (`ralph:prd` vs `ralph:ready`).
- End-to-end mock flow Pending -> Done.

3. Validate conformance tests (required):
- Add `src/validate/tests_interactive_prd.rs`.
- Export `pub fn tests() -> Vec<ConformanceTest>`.
- Register in `src/validate/mod.rs`.
- Cover: pickup/question posting, answer-to-draft, feedback revision, approval by comment, approval by label, restart continuity, failure labeling.

### Out of Scope
- Auto-starting implementation after PRD completion.
- GitHub webhooks (polling only).
- Multi-issue dependency orchestration.
- Rich formatting/reactions/editing existing comments.
- Role-based approval authorization.
- New CLI command for interactive PRD.
