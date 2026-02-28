---
artifact: final-review-proposals
loop: 7
project: implement-an-event-driven-multi-turn-prd
backend: claude
role: final_reviewer
created_at: 2026-02-22T06:41:39Z
---

I'll conduct a thorough cross-file audit of the interactive PRD implementation. Let me start by reading the core source files.
Good. Config, error, and module exports look correct. Now let me read the core implementation file and the runtime integration in parallel.
Let me continue reading the key parts of the interactive_prd.rs and also search for the runtime integration point where PRD is called.
Let me read the rest of interactive_prd.rs (tests section), the github.rs helpers, and the runtime PRD startup labels section.
The `ensure_prd_labels_best_effort` isn't called from `runtime.rs`. Let me check where it's called from.
Good. Now let me check the validation function and look at the test files.
Let me continue reading the conformance tests and check the `validate/mod.rs` registration.
Let me now read the remaining conformance tests and the rest of the integration tests to complete the audit.
Now let me check a few remaining things — the `DaemonRuntimeConfig` plumbing and whether tests compile/pass.
Let me verify a few cross-file concerns: the `DaemonRuntimeConfig` plumbing from CLI to runtime, and the `run_backend_sync` nested tokio runtime issue.
Good, the plumbing is complete. Now let me verify the `resolve_daemon_config` function to make sure `daemon_cfg` is populated with PRD fields.
Now let me verify the `data_dir` path computation in `run_prd_phase` is correct by understanding the `repo_root` structure.
So `repo_root` = `{data_dir}/{owner}/{repo}`, and `run_prd_phase` does:
```
data_dir = repo_root.parent().parent() // = {data_dir}
```

Then `state_path` = `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue}.json`, which is correct.

Let me now check the `don't` vs `don't` in approval detection — the spec mentions `don't` but I want to verify the regex handles both curly and straight quotes.
Good. The `['']` character class handles both straight and curly quotes. Now let me verify one final concern — the nested tokio runtime in `run_backend_sync` and `run_review_with_retry_sync`. These create a `new_current_thread` runtime inside what may already be a tokio context (since `poll_and_advance_prd` is called from `spawn_blocking`). Let me verify this doesn't cause a nested runtime panic.
This is safe. `spawn_blocking` runs the closure on a dedicated blocking thread pool (not within a tokio context), so creating a `new_current_thread` runtime there won't cause a nested runtime panic. The pattern is correct.

Let me do one final check — the conformance tests are all listed as 23 tests, let me verify the remaining tests complete the spec coverage.
Now let me also verify the `ralph_bin_absolute` helper used in integration tests exists.
Good. Now I have a comprehensive understanding of the implementation. Let me finalize my review.
# Final Review: NO AMENDMENTS

## Summary

The interactive PRD workflow implementation is complete, correct, and well-structured. I conducted a thorough cross-file audit covering all modified source files, their dependencies, and the full test suite. Here are the specifics:

**State Machine (`src/daemon/interactive_prd.rs`, 1821 lines):**
- All 5 states implemented: `Pending`, `AwaitingAnswers`, `AwaitingFeedback`, `Done`, `Failed` (lines 29-35).
- All transition paths verified: Pending->AwaitingAnswers (line 396), AwaitingAnswers->AwaitingFeedback (line 532), AwaitingFeedback->Done (line 775), AwaitingFeedback->AwaitingFeedback revision loop (line 726-771), Any->Failed (line 1208).
- "At most one state transition per issue per tick" enforced via `HashSet<u32>` deduplication (line 319).
- Approval detection correctly strips fenced and inline code before matching, handles mixed positive/negative signals (lines 119-146). Curly and straight quotes handled in `don['']t` pattern (line 124).
- Idempotent marker detection via `find_comment_with_marker` before posting (line 453-503).
- Error accumulation: `error_count >= 3` triggers failure transition (line 946).
- Atomic state persistence: `tempfile::NamedTempFile` + `persist()` pattern (lines 83-89).

**Configuration (`src/config/global.rs`):**
- All 6 `daemon_prd_*` fields on `WorkspaceConfig` with correct defaults (lines 61-72, 732-754): `prd_enabled=true`, `question_backends=["claude","codex"]`, `writer_backend="claude"`, `reviewer_backend="codex"`, `max_revisions=3`, `backend_timeout_secs=120`.
- Validation at startup: `validate_interactive_prd_workspace_config` checks exactly 2 question backends and valid specs (lines 421-449). Called from `src/cli/daemon.rs:179`.

**Runtime Integration (`src/daemon/runtime.rs`):**
- PRD phase runs before claim/dispatch loop (line 546), preventing dual workflow ownership.
- `run_prd_phase` correctly wraps blocking `poll_and_advance_prd` in `spawn_blocking_op` (line 617).
- `has_prd_label()` guard at line 728 in `runtime.rs` correctly skips PRD-labeled issues from the normal `ralph:ready` claim path.
- `data_dir` computation (`repo_root.parent().parent()`) is correct for the `{data_dir}/{owner}/{repo}` structure (lines 596-601).
- Nested tokio runtime in `run_backend_sync` is safe — `spawn_blocking` runs on a dedicated thread pool, so `new_current_thread()` doesn't cause a nested runtime panic.

**GitHub Helpers (`src/daemon/github.rs`):**
- `ensure_prd_labels_best_effort` creates all 5 labels idempotently (lines 1333-1381), called at daemon startup from `src/cli/daemon.rs:153`.
- `fetch_issue_comments`, `fetch_authenticated_login`, `fetch_issue_labels`, `find_comment_with_marker`, `post_comment_with_marker`, `post_comment_with_marker_metadata`, `add_label_with_retry`, `remove_label_with_retry` — all verified present and correctly implemented.

**Error Handling (`src/error.rs`):**
- `InteractivePrdFailed(String)` variant with exit code 14 (lines 127-128, 150).

**Module Export (`src/daemon/mod.rs`):**
- `pub mod interactive_prd;` correctly exported (line 3).

**DaemonRuntimeConfig Plumbing (`src/cli/daemon.rs`):**
- All 7 PRD fields (`prd_enabled`, `prd_question_backends`, `prd_writer_backend`, `prd_reviewer_backend`, `prd_max_revisions`, `prd_backend_timeout_secs`) plumbed from `daemon_cfg` to `DaemonRuntimeConfig` (lines 228-233).

**Test Coverage:**
- **Unit tests** in `src/daemon/interactive_prd.rs`: 28 tests covering state serialization, approval detection (positive/negative/mixed/code-stripping/word-boundaries), marker generation, terminal state detection, save/load roundtrip, comment extraction, bot filtering, feedback comment filtering with draft boundary, error accumulation, and all transition decision paths.
- **Integration tests** in `tests/daemon_interactive_prd.rs`: 12 tests covering persistence across restart, label conflict behavior, approval detection edge cases, marker format, PRD labels integrity, terminal state detection, error count persistence, and an answer-to-draft transition with mock backends.
- **Conformance tests** in `src/validate/tests_interactive_prd.rs`: 23 tests (registered in `src/validate/mod.rs:105`) covering state serialization roundtrip, label completeness, label detection, approval detection, marker format, state persistence survives restart, failed state persistence, terminal state idempotence, PRD/ready label conflict, startup label creation, claim-path conflict prevention, idempotent reprocessing, pickup & question posting, answer-to-draft, feedback revision, approval by comment, approval by label, failure labeling, mixed comments approval, approval-path GitHub failure, approval failure exhaustion, draft boundary filtering (pre-draft approval & revision), and restart continuity marker timestamp hydration.

**Cross-File Concerns Verified:**
- No race conditions: PRD phase runs synchronously within `spawn_blocking`, one state transition per issue per tick, `HashSet` deduplication across both poll passes.
- No timing issues: `questions_posted_at` is hydrated from GitHub's `created_at` timestamp (not local clock) to handle restart scenarios correctly.
- Draft boundary filtering: `find_new_feedback_comments` uses `max(last_processed_comment_id, latest_draft_comment_id)` as boundary, correctly excluding pre-draft comments from both approval detection and revision aggregation.
- Cursor advancement: On approval success, cursor advances only after successful `do_approval_transition` (line 718), ensuring approval comments remain visible for retry on failure.
