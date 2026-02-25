---
artifact: final-review-proposals
loop: 15
project: implement-an-event-driven-multi-turn-prd
backend: claude
role: final_reviewer
created_at: 2026-02-22T10:10:30Z
---

I'll conduct a thorough final review of the interactive PRD implementation. Let me start by reading the core source files and tracing dependencies across the codebase.


Let me also start reading the test files and github.rs in parallel.


Good, agents are still working. Let me also verify the build compiles and tests pass while waiting.
Let me also start reading some critical cross-cutting concerns directly.
Good, let me read the core implementation directly while agents finish.
Let me continue reading the rest of the unit tests and check the remaining files.
Now let me check the agents and read the error.rs file.
Good. Let me now check the runtime.rs integration with PRD. Let me also check the agent outputs.
Agent a51e0b3 has completed with a comprehensive report. Let me now check the runtime.rs integration and verify the build compiles.
The project compiles cleanly. Let me now run the unit tests and verify key cross-file dependencies.
Now let me check the config fields and validation.
Good. Now let me check the config validation (question backends must contain exactly 2, backend spec parsing).
Let me now check on the test results and also verify some important cross-file concerns.
All 57 interactive_prd unit tests pass. Let me now do the critical cross-file dependency audit. Let me check some specific edge cases I noticed.
Now let me verify the startup label ensure is called.
Good. Let me verify the validate/mod.rs registration and check one more critical concern - the `data_dir` construction in runtime.rs to make sure the persistence path is correct.
Let me verify a few more cross-file concerns: the `data_dir` path logic, the `DaemonRuntimeConfig` plumbing from config, and the comment-at-cursor boundary ordering assumption.
Now let me check how the DaemonRuntimeConfig is populated from config.
Let me look at the specific DaemonRuntimeConfig construction to verify PRD fields are plumbed.
Let me check the `resolve_daemon_config` function to verify PRD fields are resolved from workspace config.
Now let me check one more potential edge case - the IssueComment struct definition and how comment IDs are used for ordering (GitHub comment IDs are monotonically increasing, so using `id <= boundary` for filtering is correct).
Let me also verify the `/ Interactive PRD phase:` line 543-544 in runtime.rs — I noticed a missing `/` in the comment.
OK, the comment at line 544 is fine — my earlier grep output had an artifact with `/ Interactive PRD phase:` but reading it directly shows it's actually `// Interactive PRD phase:` which is correct.

Let me now verify the `EffectiveDaemonConfig` struct has the PRD fields.
Everything checks out. Let me now do a final verification that all unit tests in the project still pass (not just interactive_prd).
All 668 unit tests pass. Let me complete the cross-file audit and write the final review.
# Final Review: NO AMENDMENTS

## Summary

The interactive PRD workflow implementation is complete, correct, and thoroughly tested. I verified this by:

**State Machine (src/daemon/interactive_prd.rs, ~2700 lines):**
- All 5 states (`Pending`, `AwaitingAnswers`, `AwaitingFeedback`, `Done`, `Failed`) are implemented with correct transitions at lines 383-399.
- `Pending→AwaitingAnswers` (line 412): Label swap (add `ralph:prd-active` before removing `ralph:prd`), `ralph:ready` conflict removal, dual-backend question generation with synthesis, idempotent marker posting via bot-scoped helpers.
- `AwaitingAnswers→AwaitingFeedback` (line 554): First non-bot answer detection after `questions_posted_at`, draft generation via writer/reviewer pipeline with `check_spec_sections()`, idempotent draft marker.
- `AwaitingFeedback→Done` (line 679): Approval by comment or `ralph:prd-approved` label, terminal state persisted BEFORE label removal (line 867), safe rollback on save failure.
- `AwaitingFeedback→AwaitingFeedback` revision (line 756): Aggregated feedback, revision generation, incremented draft marker.
- `Any→Failed` (line 1382): Error accumulation with `error_count >= 3` threshold, persistence-safe terminal state, best-effort error comment.

**Persistence (line 78-122):** Atomic write via `tempfile::NamedTempFile` + `sync_all()` + `persist()`. Path: `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json`. All minimum persisted fields present in `InteractivePrdState` struct (lines 39-55).

**Approval Detection (line 132-158):** Code blocks stripped, negative phrases checked first, positive word-boundary patterns (`approved`, `lgtm`, `ship it`, `looks good`), mixed signals return false. All per spec.

**Comment Processing:** Bot filtering by `author_login` (not marker), draft boundary enforced in `find_new_feedback_comments` (line 895-922), bot-scoped marker posting prevents user spoofs.

**Config (src/config/global.rs, lines 61-72):** All 6 `daemon_prd_*` fields with correct types and defaults. Validation at startup in `validate_interactive_prd_workspace_config` (src/config/mod.rs:421) enforces exactly 2 question backends and valid backend specs. Fast-fail called from `src/cli/daemon.rs:179`.

**Runtime Integration (src/daemon/runtime.rs):** PRD phase runs before claim/dispatch (line 544-550) via `run_prd_phase` → `spawn_blocking_op` → `poll_and_advance_prd`. `has_prd_label()` filters PRD issues from normal claim flow (line 728). All 7 PRD config fields plumbed through `DaemonRuntimeConfig` (lines 61-72) → `PrdPollConfig` (lines 603-615).

**Lifecycle Labels (src/daemon/github.rs:1463):** `ensure_prd_labels_best_effort()` creates all 5 labels idempotently at startup, called from `src/cli/daemon.rs:153`.

**Error Handling (src/error.rs:128):** `InteractivePrdFailed(String)` variant with exit code 14.

**Test Coverage:**
- 57 unit tests in `src/daemon/interactive_prd.rs` (all passing)
- 24 integration tests in `tests/daemon_interactive_prd.rs` covering persistence, label conflicts, end-to-end flows, bot login failures, save failures, marker spoofing, draft boundary filtering
- 41 conformance tests in `src/validate/tests_interactive_prd.rs` registered via `pub fn tests()` in `src/validate/mod.rs:105`
- 668 total library unit tests pass with 0 failures

**Cross-file Dependencies Verified:**
- `data_dir` construction in `run_prd_phase` (runtime.rs:596-601) correctly strips owner/repo from repo_root to avoid path duplication
- `state_path()` (interactive_prd.rs:195-201) constructs `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json` — compatible with data_dir derivation
- GitHub comment IDs are monotonically increasing (u64), so `id <= boundary` filtering in `find_new_feedback_comments` and `find_first_answer_comment` is correct
- `comments.sort_by_key(|c| c.created_at)` in `fetch_issue_comments` (github.rs:1274) ensures chronological ordering
- Nested tokio runtime in `run_backend_sync` (line 1347) is safe — runs inside `spawn_blocking` thread pool, not nested in async context
