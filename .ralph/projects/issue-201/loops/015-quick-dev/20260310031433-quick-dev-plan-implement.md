---
artifact: quick-dev-plan-implement
loop: 15
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T03:14:33Z
---

All tasks are complete. Here's a summary of all changes made:

## Files Modified (9) + 1 New File

### `src/project/amendments.rs`
- Added `PrReview` variant to `AmendmentSource` enum with kebab-case serialization as `"pr-review"`

### `src/config/global.rs`
- Added `daemon_pr_review_whitelist: Vec<String>` to `WorkspaceConfig` (with `#[serde(default)]`)
- Added corresponding entry in `Default` impl

### `src/config/mod.rs`
- Added `pr_review_whitelist: Vec<String>` to `EffectiveDaemonConfig`
- Added threading in `resolve_daemon_config()`: `pr_review_whitelist: global.workspace.daemon_pr_review_whitelist.clone()`

### `src/cli/daemon.rs`
- Threaded `pr_review_whitelist` from `EffectiveDaemonConfig` into `DaemonRuntimeConfig` construction

### `src/daemon/runtime.rs`
- Added `pr_review_whitelist: Vec<String>` field to `DaemonRuntimeConfig`
- Added `pr_review_phase()` call in main loop after `auto_rebase_phase` and before `poll_and_claim`
- Added `pr_review_phase()` function: polls for new comments, capacity checks, label swap, dispatches completed tasks
- Added staged amendment drain + project state reset in `dispatch_task()` after worktree creation
- Updated test config constructions with new field

### `src/daemon/github.rs`
- Added `CommentEndpoint` enum (PullComment, IssueComment, Review) with serde support
- Added `PrReviewComment` struct with `dedup_key()` method
- Added `fetch_pr_review_comments()` - fetches from all 3 GitHub PR comment endpoints with `--paginate`
- Added `is_pr_open()` - checks PR state via API
- Added `fetch_endpoint_json()` - paginated API helper
- Added `merge_paginated_json_arrays()` - merges `[...][...]` output from `gh api --paginate`
- Added unit tests for JSON parsing, endpoint serialization, pagination merging

### `src/daemon/pr_review.rs` (NEW)
- `PrReviewState` - dedup state with `load()`/`save()` persistence to `.ralph/daemon/pr-review-state/`
- `stage_amendment()` / `drain_staged_amendments()` - two-phase amendment staging outside worktree
- `has_staged_amendments()` - check for pending staged amendments
- `reset_project_state_for_resume()` - resets `status` to `InProgress` and `quick_dev_phase` to `CodexReview` for quick-dev projects
- `comment_to_amendment()` - converts PR comments to `AmendmentRequest` with proper formatting
- `discover_tasks_with_prs()` - scans task metadata for PR-backed tasks
- `poll_pr_reviews()` - main polling function: discovers tasks, filters by whitelist, deduplicates, stages amendments
- Comprehensive unit tests (17 tests): dedup, whitelist filtering, self-comment filtering, staging roundtrip, state persistence, project state reset, task discovery

### `src/daemon/mod.rs`
- Added `pub mod pr_review;`

### `src/validate/tests_pr_lifecycle.rs` + `src/workflow/quick_dev_orchestrator.rs`
- Updated test config constructions with `pr_review_whitelist: vec![]`
