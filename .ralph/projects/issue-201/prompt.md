## Summary

Extend Ralph's daemon to automatically monitor open PRs it created for review comments from whitelisted GitHub users, convert those comments into `AmendmentRequest`s, and resume the completed project's orchestration to push fix commits to the existing PR branch. This closes the feedback loop between human reviewers and Ralph's amendment infrastructure (issue #194) for post-completion projects.

## Acceptance Criteria

- [ ] Daemon polls open Ralph PRs and detects review comments (inline, top-level, and review summary) from whitelisted reviewers
- [ ] Each qualifying comment is converted to an `AmendmentRequest` and enqueued exactly once (deduplication via persisted composite `{endpoint}:{github_comment_id}` keys across poll cycles)
- [ ] Reviewer whitelist is configurable via `[workspace]` section in `.ralph/ralph.toml` (e.g. `daemon_pr_review_whitelist = ["user1", "user2"]`)
- [ ] A completed project (`ProjectStatus::Completed`) with an open PR can be resumed when new amendments are enqueued — including both regular and quick-dev projects
- [ ] Worktree recreation and amendment staging are race-safe: amendments are staged outside the worktree, then copied into the project's amendment queue after the worktree is (re)created during dispatch
- [ ] Orchestrator pushes amendment fixes as new commits to the existing PR branch (PR stays open, no new PR created)
- [ ] Non-whitelisted comments are silently ignored and never trigger amendments
- [ ] Ralph's own comments (matching `authenticated_login()`) are silently ignored
- [ ] PR comment polling is disabled when the whitelist is empty (default)
- [ ] Deduplication state survives daemon restarts (persisted to `.ralph/daemon/pr-review-state/`)
- [ ] Resume dispatch respects `max_concurrent` capacity — if all slots are occupied, re-activation is deferred to the next poll cycle
- [ ] All GitHub API calls use pagination (`--paginate`) to avoid silently dropping comments beyond the default page size

## Technical Approach

### 1. Configuration

**`src/config/global.rs`** — Add to `WorkspaceConfig`:

```rust
#[serde(default)]
pub daemon_pr_review_whitelist: Vec<String>,
```

Default: empty vec (feature disabled).

**`src/config/mod.rs`** — Add to `EffectiveDaemonConfig`:

```rust
pub pr_review_whitelist: Vec<String>,
```

Update `resolve_daemon_config()` to thread it through:

```rust
pr_review_whitelist: workspace.daemon_pr_review_whitelist.clone(),
```

**`src/cli/daemon.rs`** — Thread into `DaemonRuntimeConfig` construction (around line 232–261 where the struct is built from `EffectiveDaemonConfig`):

```rust
pr_review_whitelist: effective.pr_review_whitelist.clone(),
```

**`src/daemon/runtime.rs`** — Add to `DaemonRuntimeConfig`:

```rust
pub pr_review_whitelist: Vec<String>,
```

### 2. PR Discovery via Task Metadata

Instead of using `gh pr list --head "ralph/"` (which is an unreliable prefix filter), reuse persisted task metadata at `.ralph/daemon/tasks/{task_id}.json` (which already stores `pr_url`). This is more reliable, avoids false matches, and reuses existing infrastructure.

**Discovery algorithm in `pr_review_phase()`:**

1. Scan all `.ralph/daemon/tasks/*.json` files via `glob`
2. For each file with a non-null `pr_url`, extract the PR number via existing `github::extract_pr_number()`
3. Extract the issue number from the task_id (format: `{owner}-{repo}-{N}`)
4. Query `gh api repos/{owner}/{repo}/pulls/{pr_number} --jq '.state'` to confirm the PR is still open (skip closed/merged PRs)
5. Cache open PR state for the poll cycle to avoid redundant API calls

This eliminates the `list_open_ralph_prs()` function entirely and avoids the prefix-matching problem.

### 3. PR Review Comment Fetching (`src/daemon/github.rs`)

Add a new function that fetches comments from all three relevant GitHub API endpoints:

**`fetch_pr_review_comments(owner, repo, pr_number, gh_bin) -> Vec<PrReviewComment>`**

Calls three endpoints with `--paginate` to handle overflow:

1. **Inline review comments**: `gh api repos/{owner}/{repo}/pulls/{pr_number}/comments --paginate` — line-level code review comments
2. **Top-level PR comments**: `gh api repos/{owner}/{repo}/issues/{pr_number}/comments --paginate` — general discussion comments on the PR
3. **Review summary comments**: `gh api repos/{owner}/{repo}/pulls/{pr_number}/reviews --paginate` — the body text submitted with a review (approve/request changes/comment); filter to only include reviews where `body` is non-empty

Define structs:

```rust
/// Source endpoint for dedup key namespacing
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CommentEndpoint {
    PullComment,   // /pulls/{n}/comments
    IssueComment,  // /issues/{n}/comments
    Review,        // /pulls/{n}/reviews
}

pub struct PrReviewComment {
    pub id: u64,
    pub endpoint: CommentEndpoint,
    pub author: String,
    pub body: String,
    pub path: Option<String>,    // inline comments only
    pub line: Option<u32>,       // inline comments only
    pub created_at: String,
}
```

The composite dedup key is `(endpoint, id)` — serialized as `"pull_comment:12345"`, `"issue_comment:67890"`, `"review:11111"`. This prevents cross-endpoint ID collisions since GitHub uses separate ID sequences per endpoint.

### 4. Amendment Source Extension (`src/project/amendments.rs`)

Add a new variant to `AmendmentSource`:

```rust
pub enum AmendmentSource {
    Cli,
    FinalReview,
    File,
    PrReview,  // new
}
```

### 5. Amendment Staging Area

**Problem:** Completed tasks have their worktree removed (`should_cleanup_worktree` returns `true` for `ralph:completed`), and the amendment queue lives inside the worktree at `{worktree}/.ralph/projects/{project_id}/amendment-queue/`. Enqueueing directly into a non-existent worktree path would fail.

**Solution:** A two-phase approach using a staging area outside any worktree:

**Phase 1 — Stage:** `pr_review_phase()` writes amendment files to a daemon-level staging directory:
```
.ralph/daemon/pr-review-amendments/{task_id}/{timestamp}-{sanitized_id}.json
```
This path exists in the workspace root (not inside any worktree) and survives worktree deletion.

**Phase 2 — Drain into worktree:** After `dispatch_task()` creates/recreates the worktree (via `create_worktree()`), a new helper `drain_staged_amendments(workspace_root, task_id, project_dir)` moves all files from the staging directory into the project's `amendment-queue/` directory inside the worktree. This is called in `dispatch_task()` after worktree creation and before spawning the child process.

**Race safety:** The staging directory is written only by the main daemon loop (single-threaded per task_id at the phase level), and drained only during dispatch (also single-threaded per task_id). No concurrent access.

### 6. Deduplication State (`src/daemon/pr_review.rs` — new module)

Create a new module for PR review polling logic:

- **Dedup store**: A JSON file per task at `.ralph/daemon/pr-review-state/{task_id}.json` containing a `HashSet<String>` of processed composite comment keys (e.g. `"pull_comment:12345"`, `"issue_comment:67890"`, `"review:11111"`).
- **`PrReviewState`** struct with `load()` / `save()` methods.
- **`poll_pr_reviews()`** — Core function that:
  1. Scans `.ralph/daemon/tasks/*.json` for tasks with a `pr_url`
  2. For each task, checks if the PR is still open (caching per cycle)
  3. Fetches comments via `fetch_pr_review_comments()` (all three endpoints, with `--paginate`)
  4. Filters out comments where `author` matches the authenticated GitHub user (via existing `github::fetch_authenticated_login_with_gh_bin()`)
  5. Filters by whitelist
  6. Deduplicates against persisted composite comment keys
  7. Converts new comments to `AmendmentRequest` with `source: AmendmentSource::PrReview` and `source_detail: Some("pr#{pr_number}/{endpoint}#{comment_id}")`
  8. Writes amendment files to the staging area (`.ralph/daemon/pr-review-amendments/{task_id}/`)
  9. Persists updated comment key set
  10. Returns a list of `(task_id, issue_number)` pairs that received new amendments

Comment-to-amendment body format:

For inline comments (with path/line):
```
PR review comment by @{author} on {path}:{line}:

{body}
```

For top-level comments (no path/line):
```
PR review comment by @{author}:

{body}
```

For review summary comments:
```
PR review summary by @{author}:

{body}
```

Amendment ID: `PR-{pr_number}-{endpoint}-{comment_id}` (e.g. `PR-42-pull_comment-12345`)

### 7. Daemon Integration (`src/daemon/runtime.rs`)

Add a **`pr_review_phase()`** function called in the main loop (after `auto_rebase_phase` and before `poll_and_claim`):

```rust
// PR review polling phase
if !config.pr_review_whitelist.is_empty() {
    if let Err(err) = pr_review_phase(config, children).await {
        eprintln!("warning: PR review polling failed: {err}");
    }
}
```

`pr_review_phase()` calls `poll_pr_reviews()` and then, for each task that received new amendments, checks whether the task needs to be re-dispatched (see §8).

### 8. Completed Project Resumption

When `pr_review_phase()` identifies tasks that received new amendments and whose issues are labeled `ralph:completed` (not currently in `children`):

**Step 1 — Capacity check:** Count available slots: `config.max_concurrent.saturating_sub(children.len())`. If zero slots are available, defer all re-activations to the next poll cycle (amendments remain safely staged). Log a message: `"PR review amendments pending for {task_id} but no capacity slots available; deferring"`.

**Step 2 — Fetch issue labels:** For each candidate task, fetch the full set of issue labels via `gh api repos/{owner}/{repo}/issues/{issue_number} --jq '.labels[].name'`. This provides the labels needed for correct dispatch behavior (`ralph:quick`, `ralph:prd-done`, etc.).

**Step 3 — Label swap:** `ralph:completed` → `ralph:in-progress` (via existing `swap_lifecycle_label()`).

**Step 4 — Dispatch:** Call existing `dispatch_task()` logic. The worktree is recreated by `create_worktree()` (which reuses the existing branch). After worktree creation, `drain_staged_amendments()` copies staged amendment files into the project's amendment queue inside the worktree. Then `should_resume_issue_project()` returns `true` (since `prompt.md` exists on the branch from the previous run), triggering the resume path (`run_task` or `quick_dev_run_task`).

**Step 5 — Quick-dev handling:** For quick-dev projects (identified by `ralph:quick` label in fetched labels), the short-circuit at `quick_dev_orchestrator.rs:139-145` fires when `status == Completed && quick_dev_phase.is_none()`. To avoid this, `drain_staged_amendments()` must also reset the project state: set `status` back to `InProgress` and set `quick_dev_phase` to `Some(QuickDevPhase::ReviewAndIterate)` so the orchestrator re-enters the review loop where amendments are processed. For regular (non-quick-dev) projects, only `status` is reset to `InProgress` — the existing amendment drain logic at `orchestrator.rs:605-626` handles the rest.

**Step 6 — PR URL:** The existing `pr_url` from task metadata (`.ralph/daemon/tasks/{task_id}.json`) is passed to `dispatch_task()`, so `handle_pr_flow` will find the existing PR and update it rather than creating a new one.

This reuses the entire existing dispatch/resume infrastructure. The new logic is limited to: detecting completed issues that need re-activation, capacity gating, label fetching, staging drain with state reset, and dispatching.

### 9. Ignoring Ralph's Own Comments

Resolve the authenticated GitHub user once per poll cycle via existing `github::fetch_authenticated_login_with_gh_bin(config.gh_bin)`. Filter out comments where `author` matches this login. This prevents Ralph's own status comments and completion comments from triggering amendments.

## Files & Modules

| File | Change |
|------|--------|
| `src/config/global.rs` | Add `daemon_pr_review_whitelist: Vec<String>` to `WorkspaceConfig` |
| `src/config/mod.rs` | Add `pr_review_whitelist: Vec<String>` to `EffectiveDaemonConfig`; update `resolve_daemon_config()` to thread it through |
| `src/cli/daemon.rs` | Thread `pr_review_whitelist` from `EffectiveDaemonConfig` into `DaemonRuntimeConfig` construction |
| `src/daemon/runtime.rs` | Add `pr_review_whitelist: Vec<String>` to `DaemonRuntimeConfig`; add `pr_review_phase()` call in main loop; add re-dispatch logic for completed issues with capacity gating; add `drain_staged_amendments()` call in `dispatch_task()` after worktree creation |
| `src/daemon/github.rs` | Add `fetch_pr_review_comments()`, `CommentEndpoint`, `PrReviewComment` structs |
| `src/daemon/pr_review.rs` | **New** — PR review polling, dedup state, comment→amendment conversion, amendment staging, project state reset for quick-dev |
| `src/daemon/mod.rs` | Add `pub mod pr_review;` |
| `src/project/amendments.rs` | Add `PrReview` variant to `AmendmentSource` |

## Testing Strategy

### Unit tests in `src/daemon/pr_review.rs`

- Comment filtering by whitelist (include/exclude cases)
- Deduplication: same composite key not enqueued twice; distinct keys across endpoints with same numeric ID are treated as separate comments
- Comment-to-`AmendmentRequest` conversion (inline with path/line, top-level, and review summary)
- State serialization/deserialization roundtrip
- Self-comment filtering (Ralph's own comments ignored)
- Empty whitelist → no polling
- Staging directory write/drain roundtrip
- Project state reset: quick-dev projects get `status=InProgress` + `quick_dev_phase=Some(ReviewAndIterate)`; regular projects get `status=InProgress` only
- Capacity gating: when `max_concurrent` slots are exhausted, amendments are staged but dispatch is deferred

### Unit tests in `src/daemon/github.rs`

- JSON parsing for `fetch_pr_review_comments` output from all three endpoints (`/pulls/{n}/comments`, `/issues/{n}/comments`, `/pulls/{n}/reviews`)
- Reviews with empty body are filtered out
- `CommentEndpoint` serialization roundtrip

### Unit tests in `src/project/amendments.rs`

- `PrReview` source serialization roundtrip

### Validate conformance tests (integration)

Following the project's existing `ConformanceTest` / `RalphHarness` pattern in `src/validate/`:

- **`pr_review::whitelist_filters_comments`** — Set up a project with `ralph:completed` label and an open PR. Inject mock review comments from both whitelisted and non-whitelisted users. Run single-iteration daemon tick. Assert: only whitelisted comments produce amendments in staging area; non-whitelisted are absent.
- **`pr_review::completed_project_resumes`** — Set up a completed project with an open PR and staged amendments. Run single-iteration daemon tick. Assert: label swapped to `ralph:in-progress`, orchestrator spawned, amendments drained from staging into worktree.
- **`pr_review::dedup_across_restarts`** — Process a comment, persist state, simulate restart, re-poll with same comment. Assert: no duplicate amendment created.
- **`pr_review::capacity_gating`** — Fill all `max_concurrent` slots. Enqueue PR review amendments for a completed project. Run single-iteration daemon tick. Assert: amendments remain staged, no dispatch attempted, no label change.
- **`pr_review::quick_dev_resume_no_short_circuit`** — Set up a completed quick-dev project with staged amendments. Run single-iteration daemon tick. Assert: project state reset to `InProgress` with `quick_dev_phase=ReviewAndIterate`, orchestrator does not short-circuit.

Register all new tests in `src/validate/mod.rs` `register_tests()`.

## Out of Scope

- **PR review replies/reactions** — only inline review comments, top-level PR comments, and review summary comments are processed; threaded reply chains and emoji reactions are not converted to amendments
- **Batch/grouped amendments** — each comment becomes one amendment; no grouping of multiple comments into a single amendment
- **Review dismissal/resolution** — no tracking of whether amendments "resolve" specific review comments on GitHub; no automatic request for re-review after pushing amendment commits
- **Non-Ralph PRs** — only PRs tracked in `.ralph/daemon/tasks/*.json` are polled
- **Custom rate limiting** — relies on `gh` CLI's built-in rate limit handling; no custom backoff for the GitHub API. The `--paginate` flag handles multi-page responses but does not add custom throttling
- **Comment editing** — if a reviewer edits a comment after it's been processed, the edit is not re-processed (dedup by composite key is permanent)
- **PR review request assignment** — no automatic request for re-review after pushing amendment commits
- **Multi-repo support** — polling is scoped to the single `{owner}/{repo}` configured for the daemon instance