---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 2
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T04:00:56Z
---

All changes look correct. Let me provide the summary response.

# Apply-Fixes Summary

## Changes Applied

### Fix 1 (High): Quick-dev resume routes to `PlanAndImplement` phase
- **`src/daemon/pr_review.rs:222-225`**: Changed `reset_project_state_for_resume` to set `quick_dev_phase = "plan_and_implement"` and `current_phase = "implementing"` instead of `"codex_review"`/`"reviewing"`. This ensures resumed quick-dev projects enter the phase that actually drains the amendment queue.
- Added unit test `reset_quick_dev_enters_plan_and_implement` that stages an amendment, resets state, and verifies amendments are drainable after reset.
- Updated existing unit test and validate test assertions to match new values.

### Fix 2 (High): Inline review replies filtered via `in_reply_to_id`
- **`src/daemon/github.rs:2276-2278`**: Added `in_reply_to_id: Option<u64>` field to `RawPullComment`.
- **`src/daemon/github.rs:2335-2338`**: Skip comments where `in_reply_to_id.is_some()` during parsing in `fetch_pr_review_comments`.
- Added unit test `parse_pull_comments_filters_replies` with one top-level inline comment + one reply, asserting only the top-level is returned.

### Fix 3 (Medium): Deferred dispatch gated on `is_pr_open` with cache
- **`src/daemon/runtime.rs:2498-2579`**: Added `pr_number: u32` to `DispatchCandidate`, introduced a per-cycle `HashMap<u32, bool>` PR open cache. Poll results seed the cache (already known-open). Deferred candidates check `is_pr_open` (hitting cache when available) before dispatch. Closed-PR candidates are silently skipped.

### Fix 4 (Medium): Crash-safe dedup + `remove_file` error handling
- **`src/daemon/pr_review.rs:157-162`**: Changed `let _ = fs::remove_file(&src)` to return an error via `?` when removal fails, preventing repeated draining of the same amendment.
- **`src/daemon/pr_review.rs:506-508`**: Moved `state.save()` into the per-comment loop body so dedup state is persisted incrementally after each staged amendment. A crash after staging no longer causes re-enqueue on next cycle.

### Fix 5 (Medium): Validate tests rewritten to use daemon ticks
- **`src/validate/tests_pr_review.rs`**: Complete rewrite. All 5 tests now:
  - Create a `RalphHarness::new_daemon` harness with `init_workspace()`
  - Set up a mock `gh` script handling PR review API endpoints (`/pulls/{n}/comments`, `/issues/{n}/comments`, `/pulls/{n}/reviews`, PR state, `api user`)
  - Create task metadata files in `.ralph/daemon/tasks/`
  - Execute `daemon start --single-iteration --repo acme/widgets`
  - Assert on filesystem state (staging dirs, dedup state, label logs, worktree state)

  Specific test behaviors:
  - **`whitelist_filters_comments`**: Mock gh returns comments from alice (whitelisted), bob (whitelisted), charlie (not), ralph-bot (self). Asserts exactly 3 staged amendments.
  - **`completed_project_resumes_with_state_reset`**: Pre-stages amendments, sets `ralph:completed` label, sets up project branch. Asserts label swap logged and amendments drained.
  - **`dedup_across_restart`**: Runs two daemon ticks with identical comments. Asserts second tick produces no additional amendments.
  - **`capacity_deferral_preserves_staged`**: Two completed tasks with staged amendments, `max_concurrent=1`. Asserts deferral message in stderr and at least one task retains staged amendments.
  - **`quick_dev_resume_resets_phase`**: Completed quick-dev project with staged amendments and `ralph:quick` label. Asserts state reset to `plan_and_implement`/`implementing` in worktree.
