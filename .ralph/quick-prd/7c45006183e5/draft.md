I now have a comprehensive understanding of the codebase. Let me write the engineering specification.

---

## Summary

Add a PR conflict monitor to the daemon's main polling loop that detects merge conflicts on open PRs created by the daemon and automatically rebases them onto the target branch. The monitor queries the GitHub API for mergeable state on completed tasks that have a `pr_url`, and when conflicts are detected, creates (or reuses) a worktree for the branch, runs `git rebase`, force-pushes the result, and posts a status comment on the associated issue. Failed rebases abort cleanly, post a diagnostic comment, and do not retry until the next polling cycle.

## Acceptance Criteria

- Daemon checks all completed tasks with a `pr_url` for merge conflicts each polling cycle
- Conflict detection uses `gh pr view --json mergeable` (or equivalent) to check PR mergeability without local git operations
- When a conflict is detected, the daemon rebases the PR branch onto the target branch and force-pushes
- Successful rebases are logged to stderr and a comment is posted on the linked issue
- Failed rebases abort the rebase (`git rebase --abort`), log the failure, and post a diagnostic comment on the issue (idempotent, no duplicate comments)
- Rebase operations do not interfere with active task child processes (enforced by concurrency slot accounting)
- The feature is gated behind a `daemon_rebase_enabled` config flag (default: `false`)
- A `daemon_rebase_interval_seconds` config option controls how often conflict checks run (default: `300`, independent of `poll_seconds`)
- Only PRs from tasks owned by this daemon instance (matched by `owner`/`repo`) are considered
- PRs that have already been merged or closed are skipped

## Technical Approach

### 1. PR Mergeability Check via `gh` CLI

Add `check_pr_mergeable(owner, repo, branch) -> Result<PrMergeStatus>` to `daemon/github.rs`. This runs:

```
gh pr view <branch> --repo <owner/repo> --json mergeStateStatus,state,url
```

`mergeStateStatus` returns `"CONFLICTING"`, `"MERGEABLE"`, `"UNKNOWN"`, or `"BLOCKED"`. The function returns a struct:

```rust
pub enum PrMergeStatus {
    Mergeable,
    Conflicting,
    Unknown,
    Closed,    // PR state is CLOSED or MERGED
}
```

This avoids local git operations for the detection phase and works without a worktree.

### 2. Rebase Execution

Add `rebase_branch(repo_root, workspace_root, task_id, branch, target_branch) -> Result<()>` to a new section in `daemon/runtime.rs` (or a new `daemon/rebase.rs` module). The flow:

1. **Create or reuse worktree** via `worktree::create_worktree()` — the existing function already handles branch reuse
2. **Fetch latest** — `git fetch origin` in the worktree
3. **Attempt rebase** — `git rebase origin/<target_branch>` in the worktree
4. **On success**: `git push --force-with-lease origin <branch>` and clean up
5. **On failure**: `git rebase --abort`, post failure comment, clean up
6. **Remove worktree** via `worktree::remove_worktree()` — worktrees for rebases are ephemeral

`--force-with-lease` is used instead of `--force` to prevent overwriting concurrent pushes.

### 3. Integration with Main Loop

The rebase monitor runs as a separate phase in the existing `run()` loop, gated by a timestamp check:

```rust
// In the main loop, after collect_children and before sleep:
if config.rebase_enabled && last_rebase_check.elapsed() >= rebase_interval {
    check_and_rebase_conflicted_prs(store, config).await;
    last_rebase_check = Instant::now();
}
```

`check_and_rebase_conflicted_prs` loads all tasks from the store, filters to `Completed` state with a `pr_url` and matching `owner`/`repo`, checks mergeability via the `gh` API, and rebases any conflicting PRs sequentially. This runs on the main task, not as a child process, because it is short-lived and must not consume concurrency slots meant for task execution.

### 4. Idempotent Comments

Rebase activity comments use the existing `post_idempotent_comment` infrastructure with new phase markers:
- `ralph:task:{task_id}:rebase-success` — posted after a successful rebase
- `ralph:task:{task_id}:rebase-failed:{timestamp}` — posted on failure (timestamp makes each failure unique to avoid suppressing subsequent reports)

### 5. Configuration

Add to `WorkspaceConfig` in `config/global.rs`:
- `daemon_rebase_enabled: bool` (default `false`)
- `daemon_rebase_interval_seconds: u64` (default `300`)

Add corresponding fields to `ProjectDaemonOverrides` in `config/project.rs`.

Plumb through `EffectiveDaemonConfig` → `DaemonRuntimeConfig`.

### 6. State Tracking

Add an optional `last_rebase_at: Option<String>` field to `DaemonTask` (ISO 8601 timestamp, `#[serde(default)]` for backwards compatibility). Updated after each successful rebase. This allows the daemon to skip recently-rebased PRs and provides observability via `ralph daemon status`.

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/github.rs` | Add `check_pr_mergeable()`, `PrMergeStatus` enum, `force_push_branch()` |
| `src/daemon/runtime.rs` | Add `check_and_rebase_conflicted_prs()`, rebase interval tracking in `run()`, helper `rebase_single_pr()` |
| `src/daemon/worktree.rs` | No changes — existing `create_worktree`/`remove_worktree` suffice |
| `src/daemon/mod.rs` | Add `last_rebase_at: Option<String>` to `DaemonTask` |
| `src/config/global.rs` | Add `daemon_rebase_enabled`, `daemon_rebase_interval_seconds` to `WorkspaceConfig` with defaults |
| `src/config/project.rs` | Add `rebase_enabled`, `rebase_interval_seconds` to `ProjectDaemonOverrides` |
| `src/config/mod.rs` | Add rebase fields to `EffectiveDaemonConfig`, wire into `resolve_daemon_config()` |
| `src/cli/daemon.rs` | Plumb new config fields into `DaemonRuntimeConfig` |

## Testing Strategy

**Unit tests (in-module `#[cfg(test)]`):**
- `daemon/github.rs`: Test `PrMergeStatus` deserialization from raw JSON responses (Mergeable, Conflicting, Unknown, closed PR). Pattern: existing `parse_issue_list` tests
- `daemon/mod.rs`: Test `DaemonTask` round-trip serialization with/without `last_rebase_at` (backwards compatibility). Pattern: existing `daemon_task_deserializes_without_raw_idea` tests
- `config/mod.rs`: Test `resolve_daemon_config` applies rebase overrides with correct precedence. Pattern: existing `resolve_daemon_config_applies_project_overrides` test

**Integration tests (in `tests/`):**
- `git/branch.rs` already tests `merge_base_branch`. Add a test for rebase-and-force-push flow using a local bare repo (no network): create a branch with conflicts, run the rebase function, verify the branch is updated and force-pushed. Uses `tempfile::TempDir` like existing git tests
- Main loop integration: extend the `--single-iteration` test pattern to verify that the rebase phase runs (or is skipped when disabled). This can use a mock task store with a completed task having a `pr_url`

**Manual/CI validation:**
- Create a test repo with a daemon-created PR, push a conflicting commit to master, verify the daemon detects and rebases it within one rebase interval

## Out of Scope

- **AI-assisted conflict resolution**: When a rebase fails due to conflicts that cannot be auto-resolved, the daemon posts a comment and moves on. It does not invoke an AI backend to resolve conflicts
- **Rebase strategy configuration** (e.g., merge vs rebase vs squash): Only `git rebase` is supported; merge-based conflict resolution is not offered as an alternative
- **Rate limiting infrastructure**: No new rate limiting system is introduced. The rebase interval and sequential processing provide implicit throttling. A general-purpose GitHub API rate limiter is a separate concern
- **PR status checks or CI re-trigger**: The daemon does not wait for or manage CI status after rebasing
- **Notification systems**: No Slack, email, or external notification beyond GitHub issue comments
- **Retry logic with backoff for failed rebases**: Failed rebases are retried on the next rebase interval naturally; no exponential backoff is implemented
- **Rebasing PRs not created by the daemon**: Only tasks in the daemon's `TaskStore` with `state == Completed` and a `pr_url` are eligible