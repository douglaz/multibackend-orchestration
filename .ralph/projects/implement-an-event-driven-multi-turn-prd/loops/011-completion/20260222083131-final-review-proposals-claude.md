---
artifact: final-review-proposals
loop: 11
project: implement-an-event-driven-multi-turn-prd
backend: claude
role: final_reviewer
created_at: 2026-02-22T08:31:31Z
---

# Final Review: NO AMENDMENTS

## Summary

The interactive PRD workflow implementation is complete, correct, and well-tested. I conducted an exhaustive cross-file audit by reading all source files, tracing dependencies across modules, and verifying the full config plumbing chain.

**State Machine (`src/daemon/interactive_prd.rs`):** All 5 states (`Pending`, `AwaitingAnswers`, `AwaitingFeedback`, `Done`, `Failed`) are implemented with correct transition rules. The `PrdWorkflowState` enum and `InteractivePrdState` struct are serializable, and atomic persistence via `tempfile::NamedTempFile` + `persist()` ensures restart-safety (lines 78-92). Terminal state detection (`is_terminal()`) correctly gates at lines 111-116.

**Transition Logic:**
- **Pending -> AwaitingAnswers** (lines 399-516): Label swap is boundary-safe (add `ralph:prd-active` before removing `ralph:prd`), `ralph:ready` conflict is handled, question generation uses two backends + synthesis, idempotent marker check prevents duplicate comments, and `questions_posted_at` is hydrated from GitHub's real `created_at` timestamp.
- **AwaitingAnswers -> AwaitingFeedback** (lines 535-630): Bot login caching, first-unprocessed-answer detection with time-and-id gating, draft generation with writer/reviewer/section validation loop, and idempotent draft marker posting.
- **AwaitingFeedback -> Done/Revision** (lines 653-825): Label-based approval checked first, then comment-based approval via `detect_approval()`. Draft boundary filtering (`find_new_feedback_comments()` at lines 833-860) correctly uses `max(last_processed_comment_id, latest_draft_comment_id)` to exclude pre-draft comments. Approval label swap is boundary-safe (add `ralph:prd-done` before removing `ralph:prd-active`).
- **Any -> Failed** (lines 1252-1281): `finish_transition()` at lines 942-956 handles error accumulation and triggers failure at `error_count >= 3`.

**Approval Detection (`detect_approval()`, lines 119-146):** Correctly strips fenced and inline code, checks negative patterns first, applies word-boundary matching for positive patterns, and returns `false` when both positive and negative signals exist.

**Runtime Integration (`src/daemon/runtime.rs`):**
- PRD phase runs before poll/claim at lines 546-550, preventing dual workflow ownership.
- `run_prd_phase()` at lines 592-618 correctly constructs `PrdPollConfig` and delegates via `spawn_blocking_op`.
- `has_prd_label()` filter at line 728 skips PRD-labeled issues from the normal `ralph:ready` claim path.

**Config (`src/config/global.rs`):** All 6 config fields with correct defaults (lines 732-754): `daemon_prd_enabled=true`, `question_backends=["claude","codex"]`, `writer_backend="claude"`, `reviewer_backend="codex"`, `max_revisions=3`, `backend_timeout_secs=120`. Validation at `src/config/mod.rs:421-450` enforces exactly 2 question backends and parseable backend specs.

**GitHub Helpers (`src/daemon/github.rs`):**
- `fetch_issue_comments()` (line 1172): Returns structured `IssueComment` with `id`, `author_login`, `body`, `created_at`, sorted chronologically.
- `post_comment_with_marker_metadata()` (line 1291): Idempotent — checks for existing marker before posting, fetches back metadata on success.
- `ensure_prd_labels_best_effort()` (line 1333): Called at daemon startup (verified at `src/cli/daemon.rs:153`), handles "already exists" gracefully.

**Error Handling (`src/error.rs`):** `InteractivePrdFailed` variant at line 128 with exit code 14. The `run_backend_sync()` function (lines 1210-1245) creates a `new_current_thread` tokio runtime inside `spawn_blocking`, which is safe since the blocking thread pool is not inside the async runtime.

**Testing Coverage:**
- 54 unit tests in `src/daemon/interactive_prd.rs` covering all state transitions, approval detection edge cases, comment filtering, draft boundary logic, and error accumulation.
- 12 integration tests in `tests/daemon_interactive_prd.rs` exercising persistence across restart, label conflict behavior, answer-to-draft flow, feedback revision, approval by comment, approval by label, mixed approval+feedback, multi-tick end-to-end (Pending->Done), pre-draft comment exclusion, bot-login failure exhaustion, and partial label failure recovery.
- 33 conformance tests in `src/validate/tests_interactive_prd.rs` registered in `src/validate/mod.rs:21,105`.

**Build Verification:** `cargo check` compiles cleanly. All 665 unit tests pass with 0 failures.
