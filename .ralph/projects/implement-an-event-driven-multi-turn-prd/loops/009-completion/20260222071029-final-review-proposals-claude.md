---
artifact: final-review-proposals
loop: 9
project: implement-an-event-driven-multi-turn-prd
backend: claude
role: final_reviewer
created_at: 2026-02-22T07:10:29Z
---

I'll conduct a thorough cross-file audit of the interactive PRD implementation. Let me start by reading the key source files.
Let me read the runtime.rs in parts and the other key files in parallel.
Now let me read the runtime.rs sections that contain PRD integration, and the test files.
Now let me read the rest of the conformance tests and check for the `ensure_prd_labels` call in the runtime startup, and verify the validate module registration.
Let me now verify a few remaining cross-file dependencies: the backend module imports, the prd::quick module, and the cli daemon startup.
Good - `run_review_with_retry` is `async`. Now let me verify one more important cross-file concern: the `resolve_daemon_config` function to confirm PRD config is plumbed through.
Now I have a comprehensive understanding of the entire implementation. Let me finalize my audit.
# Final Review: NO AMENDMENTS

## Summary

The interactive PRD workflow implementation is complete, correct, and well-tested across all layers. I verified the following by reading source files and tracing cross-file dependencies:

**State Machine & Persistence** (`src/daemon/interactive_prd.rs`):
- All 5 states (`Pending`, `AwaitingAnswers`, `AwaitingFeedback`, `Done`, `Failed`) implemented with correct serialization via `serde`.
- `InteractivePrdState` contains all minimum persisted fields per spec (lines 39-55).
- Atomic writes via `tempfile::NamedTempFile` + `persist()` (lines 83-89).
- State path follows spec: `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json` (line 182-189).

**Transition Logic**:
- `Pending -> AwaitingAnswers`: Label swap (add active before removing prd), `ralph:ready` removal, two-backend question generation with synthesis, idempotent marker posting with timestamp hydration from GitHub `created_at` (lines 399-516).
- `AwaitingAnswers -> AwaitingFeedback`: Bot login resolution, first non-bot answer detection after `questions_posted_at`, draft generation via writer/reviewer pipeline with section validation (lines 535-630).
- `AwaitingFeedback -> Done`: Both label-based (`ralph:prd-approved`) and comment-based approval detection. Boundary-safe label swap (add `ralph:prd-done` first, remove `ralph:prd-active` second) (lines 653-825).
- `AwaitingFeedback -> AwaitingFeedback` (revision): Feedback aggregation across all new post-draft comments, revision generation with section validation (lines 730-776).
- `Any -> Failed`: Error count accumulation with threshold at 3, error comment posting, label cleanup (lines 927-1244).

**Approval Detection** (`detect_approval`, lines 119-146):
- Code stripping (fenced + inline) before matching.
- Negative phrases checked first, positive second, mixed returns false.
- Word-boundary matching for all positive patterns.

**Comment Boundary Filtering** (`find_new_feedback_comments`, lines 833-860):
- Correctly uses `max(last_processed_comment_id, latest_draft_comment_id)` as boundary, ensuring pre-draft comments are excluded.

**Configuration** (`src/config/global.rs`, lines 61-72):
- All 6 PRD config fields present with correct defaults matching spec (`daemon_prd_enabled=true`, `daemon_prd_question_backends=["claude","codex"]`, `daemon_prd_writer_backend="claude"`, `daemon_prd_reviewer_backend="codex"`, `daemon_prd_max_revisions=3`, `daemon_prd_backend_timeout_secs=120`).
- Validation at startup (`validate_interactive_prd_workspace_config`) enforces exactly 2 question backends and valid backend specs.

**Runtime Integration** (`src/daemon/runtime.rs`):
- PRD phase runs before claim/dispatch (lines 544-550), preventing dual workflow ownership.
- `has_prd_label()` filters PRD issues from normal claim path (line 728).
- `DaemonRuntimeConfig` has all 7 PRD fields (lines 61-72).

**GitHub Helpers** (`src/daemon/github.rs`):
- `fetch_issue_comments()` returns structured `IssueComment` with `id`, `author_login`, `body`, `created_at` (lines 1172-1237).
- `post_comment_with_marker_metadata()` supports idempotent posting with metadata return (lines 1291-1330).
- `ensure_prd_labels_best_effort()` creates all 5 PRD labels at startup (lines 1333-1381).
- `fetch_authenticated_login()` resolves bot identity (lines 1242-1260).

**Error Handling** (`src/error.rs`):
- `InteractivePrdFailed` variant with exit code 14 (lines 127-128, 150).

**Module Registration** (`src/daemon/mod.rs`):
- `interactive_prd` module properly exported (line 3).

**Testing** - Three layers verified:
- **Unit tests** (31 tests in `interactive_prd.rs`): Serialization roundtrip, approval detection (positive/negative/mixed/code-block/word-boundary), marker generation, save/load roundtrip, comment filtering, transition path coverage, error accumulation.
- **Integration tests** (12 tests in `tests/daemon_interactive_prd.rs`): State persistence across restart, label conflict detection, AwaitingAnswers->AwaitingFeedback transition with mock backends, revision loop, approval by comment, approval by label, mixed comments, multi-tick end-to-end (Pending->Done in 3 ticks), pre-draft comment exclusion, bot-login exhaustion in both AA and AF states, partial label failure recovery.
- **Conformance tests** (26 tests in `src/validate/tests_interactive_prd.rs`): Registered in `src/validate/mod.rs` (line 105). Covers pickup/question posting, answer-to-draft, feedback revision, approval by comment, approval by label, restart continuity, failure labeling, draft boundary filtering, bot-login exhaustion, and partial failure recovery.

All acceptance criteria from the master prompt are met. The implementation is restart-safe, non-blocking, idempotent, and correctly prevents dual workflow ownership with the existing `ralph:ready` flow.
