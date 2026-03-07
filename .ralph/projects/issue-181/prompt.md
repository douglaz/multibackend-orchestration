## Summary

The draft PR watcher in the daemon uses a hardcoded `ralph/daemon/{task_id}` branch name, but `sync_project_branch` immediately switches the worktree to `ralph/issue-{N}`. This mismatch means the watcher pushes a stale/empty branch and `gh pr create` fails. Additionally, on daemon restart, `verify_worktree_branch` force-checks out back to the stale daemon branch before `sync_project_branch` switches it again, causing an unnecessary round-trip.

The fix eliminates the `ralph/daemon/{task_id}` branch format entirely. `create_worktree` will accept the target branch name as a parameter, and `dispatch_task` will derive it via `resolve_branch_name` (reusing the already-validated `git.branch_format` config), ensuring a single unified branch name throughout the task lifecycle. A migration fallback in `verify_worktree_branch` handles pre-existing worktrees still on the old daemon branch.

## Acceptance Criteria

- The draft PR watcher uses the same branch name the orchestrator commits to (`ralph/issue-{N}`)
- `gh pr create` no longer fails with "No commits between master and ralph/daemon/..."
- Draft PRs are successfully created for in-progress tasks
- No remaining hardcoded references to `ralph/daemon/{task_id}` branch format in `dispatch_task` or the watcher path
- The `find_existing_pr` lookup at dispatch time queries the correct branch (`ralph/issue-{N}`)
- The `ChildHandle.branch` field carries the correct branch for the rebase agent
- `verify_worktree_branch` on worktree reuse checks for the project branch, not the stale daemon branch; pre-existing worktrees still on `ralph/daemon/{task_id}` are migrated without error
- Existing `sync_project_branch` behavior is unaffected (it still handles remote-first sync)

## Technical Approach

### 1. Add `branch_name` parameter to `create_worktree` (`src/daemon/worktree.rs`)

Change the signature of `create_worktree` to accept `branch_name: &str` instead of computing it internally from `task_id`:

```rust
pub fn create_worktree(
    repo_root: &Path,
    workspace_root: &Path,
    task_id: &str,
    branch_name: &str,              // NEW: replaces internal format!("ralph/daemon/{task_id}")
    _repo_root_lock: Option<Arc<Semaphore>>,
) -> Result<PathBuf> {
```

Remove line 33 (`let branch_name = format!("ralph/daemon/{task_id}");`) and use the parameter throughout. The worktree directory path (`task_worktree_path`) remains based on `task_id` — only the git branch changes. All internal callers of `branch_name` (the `verify_worktree_branch` check on line 36, the `branch_exists` check on line 103, the `git worktree add -b` on line 122) will now use the caller-supplied value.

Update the doc comment (lines 22–25) to reflect the new parameter instead of referencing `ralph/daemon/<task_id>`.

### 2. Migration fallback in `verify_worktree_branch` (`src/daemon/worktree.rs:234`)

When reusing a pre-existing worktree that is still on `ralph/daemon/{task_id}` (from a previous daemon version), `verify_worktree_branch` will attempt to force-checkout the new expected branch (`ralph/issue-{N}`). If this branch does not yet exist locally (e.g., the daemon crashed before `sync_project_branch` ever ran), the bare `git checkout --force` will fail.

Fix: change the checkout command (line 270) from `git checkout --force <expected_branch>` to `git checkout --force -B <expected_branch>`. The `-B` flag creates the branch at the current HEAD if it does not already exist, and resets it to HEAD if it does. This is safe because `sync_project_branch` runs immediately after `create_worktree` returns and will force-reset the branch to the correct remote tracking state.

```rust
// Before (line 270):
let checkout = Command::new("git")
    .args(["checkout", "--force", expected_branch])
    .current_dir(wt_path)

// After:
let checkout = Command::new("git")
    .args(["checkout", "--force", "-B", expected_branch])
    .current_dir(wt_path)
```

### 3. Compute `branch_name` early in `dispatch_task` via `resolve_branch_name` (`src/daemon/runtime.rs`)

Derive the branch name using the existing `resolve_branch_name` utility and the already-validated `git.branch_format` config, rather than hardcoding the format string. This reuses the same branch-rendering logic used elsewhere in the codebase and stays consistent with the `validate_daemon_branch_format` check at startup (line 800).

Move the `branch_name` computation to before the `create_worktree` call (around line 1423, right after `project_id` is computed):

```rust
let branch_name = crate::git::branch::resolve_branch_name(
    &config.global_config.git.branch_format,
    &project_id,
);
```

Pass `&branch_name` to `create_worktree`:

```rust
worktree::create_worktree(&repo_root, &ws_root, &tid, &branch_name_clone, lock)
```

Remove the old line 1583 (`let branch_name = format!("ralph/daemon/{task_id}");`).

All downstream consumers — `find_existing_pr` (line 1603), `draft_pr_watcher` (line 1703), and `ChildHandle { branch: branch_name }` (line 1735) — now automatically use the correct `ralph/issue-{N}` value with no further changes.

### 4. `sync_project_branch` remains unchanged

`sync_project_branch` (`src/git/branch.rs:112`) still runs after `create_worktree`. Since the worktree is already on `ralph/issue-{N}`, `sync_project_branch` will:
- Fetch origin (still needed)
- Force-sync the local base branch to remote (still needed)
- Find the branch already checked out and force-reset it to the remote tracking branch if it exists, or leave it as-is if not

No changes required to `sync_project_branch`. The only behavioral difference is that the worktree starts on the correct branch, so steps 3/4 of `sync_project_branch` operate on an already-matching branch instead of switching from a daemon branch.

### 5. Do NOT modify `detect_legacy_slug_branch` (`src/daemon/runtime.rs:1359`)

The `ralph/daemon/` prefix in the exclusion filter at line 1391 is inside `detect_legacy_slug_branch`, not stale-branch cleanup. This filter prevents leftover daemon branches from being misclassified as legacy slug branches (which would trigger false resume warnings). Removing it is unnecessary for the watcher fix and would introduce false positives. Leave line 1391 unchanged:

```rust
if branch.starts_with("ralph/issue-") || branch.starts_with("ralph/daemon/") {
    continue;
}
```

### 6. Update doc comment in `src/daemon/github.rs:885`

The `current_branch` function doc comment references `ralph/daemon/{task_id}` as the daemon-created branch. Update it to reflect the new unified branch naming.

### 7. Update tests

All call sites of `create_worktree` must be updated for the new `branch_name: &str` parameter. Complete call-site inventory:

**`src/daemon/worktree.rs` (unit tests):**
- Line 673: `create_worktree_returns_expected_worktree_path` — add branch param, update path-only assertion (no branch assertion to change)
- Line 684: `verify_worktree_branch_returns_ok_for_matching_branch` — add branch param, change line 685 `format!("ralph/daemon/{task_id}")` expectation to the passed branch
- Line 699: `verify_worktree_branch_returns_error_for_missing_expected_branch` — add branch param

**`src/validate/tests_daemon.rs` (integration tests):**
- Line 2163: `create_worktree_reuses_existing_branch` — add branch param, update branch-list assertions on lines 2170, 2173, 2184, 2187 from `ralph/daemon/acme-widgets-99` to the test branch name
- Line 2192: same test, second call — add branch param
- Line 2206: `clean_worktree_removes_dirty_files` — add branch param
- Line 2274: `runtime_create_worktree_handles_stale_metadata` — add branch param
- Line 2293: same test, second call — add branch param
- Line 2306: `runtime_reuse_worktree_corrects_branch_mismatch` — add branch param, update `expected_branch` on line 2306 from `format!("ralph/daemon/{task_id}")` to the new format
- Line 2309: same test — add branch param
- Line 2320: same test, reuse call — add branch param
- Line 2758: `dispatch_ignores_legacy_slug_project_fallback` — add branch param
- Line 3257: `worktree_uses_origin_head_not_local_refs` — add branch param
- Line 3369: `worktree_falls_back_when_origin_head_missing` — add branch param
- Line 3449: `worktree_falls_back_to_head_for_empty_remote` — add branch param

**`src/validate/tests_pr_runtime.rs`:**
- Line 257: `pr_url_plumbed_through_child_args` — update `expected_branch` from `format!("ralph/daemon/{task_id}")` to `format!("ralph/issue-{issue_number}")`

**`src/validate/mock_scripts.rs`:**
- Line 1438: update doc comment referencing `ralph/daemon/{task_id}`

**`src/git/branch.rs` (unit test):**
- Line 787: `git checkout -b ralph/daemon/test-task` — update to `ralph/issue-1` to match the `sync_project_branch` test setup (this is a test that simulates a daemon worktree; the test still works because `sync_project_branch` will switch to `ralph/issue-1` regardless of the starting branch)

**New regression test — daemon-restart reuse of old daemon-branch worktree:**

Add a test in `src/validate/tests_daemon.rs` that simulates the migration edge case:
1. Create a worktree using the old format (manually create with `git worktree add -b ralph/daemon/{task_id}`)
2. Call `create_worktree` with the new `branch_name = "ralph/issue-{N}"` parameter
3. Assert that the worktree reuse path succeeds (the migration fallback in `verify_worktree_branch` creates and checks out `ralph/issue-{N}`)
4. Assert the worktree is now on `ralph/issue-{N}`

## Files & Modules

| File | Change |
|---|---|
| `src/daemon/worktree.rs:22-33` | Add `branch_name: &str` param to `create_worktree`, remove hardcoded format string, update doc comment |
| `src/daemon/worktree.rs:270` | Migration fallback: change `git checkout --force` to `git checkout --force -B` in `verify_worktree_branch` |
| `src/daemon/runtime.rs:~1423` | Compute `branch_name` via `resolve_branch_name(&config.global_config.git.branch_format, &project_id)` |
| `src/daemon/runtime.rs:1442` | Pass `&branch_name` to `create_worktree` |
| `src/daemon/runtime.rs:1583` | Remove stale `let branch_name = format!("ralph/daemon/{task_id}")` |
| `src/daemon/github.rs:885` | Update doc comment referencing `ralph/daemon/{task_id}` |
| `src/daemon/worktree.rs:667-708` | Update unit test call sites and branch expectations |
| `src/validate/tests_daemon.rs:2163,2192,2206,2274,2293,2309,2320,2758,3257,3369,3449` | Update all `create_worktree` call sites for new signature and branch assertions |
| `src/validate/tests_daemon.rs` (new) | Add regression test for daemon-restart reuse of old `ralph/daemon/` worktree |
| `src/validate/tests_pr_runtime.rs:257` | Update `expected_branch` from daemon format to issue format |
| `src/validate/mock_scripts.rs:1438` | Update doc comment |
| `src/git/branch.rs:787` | Update test setup branch name |

## Testing Strategy

1. **Unit tests in `worktree.rs`**: Verify `create_worktree` creates the worktree on the caller-supplied branch name, and `verify_worktree_branch` correctly validates it on reuse.

2. **Migration regression test** (new, in `tests_daemon.rs`): Manually create a worktree on `ralph/daemon/{task_id}` (simulating a pre-upgrade worktree), then call `create_worktree` with `branch_name = "ralph/issue-{N}"`. Assert that the reuse path succeeds via the `-B` fallback and the worktree ends up on the expected branch. Also test the case where `ralph/issue-{N}` does not exist locally yet (the critical migration edge case).

3. **Existing `draft_pr_watcher` tests** (`draft_pr_watcher_single_iteration_for_test`): Pass `ralph/issue-{N}` as the branch argument and verify the push and PR creation calls use that branch.

4. **PR URL plumbing test** (`tests_pr_runtime.rs`): Updated to expect `ralph/issue-{N}` as the head branch in `gh pr list --head` calls.

5. **Integration tests in `tests_daemon.rs`**: Verify end-to-end that `dispatch_task` creates a worktree, `sync_project_branch` runs, and the `ChildHandle.branch` is `ralph/issue-{N}`.

6. **Manual smoke test**: Run the daemon against a test repo with a GitHub issue, confirm a draft PR is created on `ralph/issue-{N}` and the PR URL is persisted to task metadata.

7. **Regression check**: Verify the rebase agent (`runtime.rs:2540`) still receives the correct branch via `ChildHandle.branch` and `create_worktree_on_branch` works with `ralph/issue-{N}`.

## Out of Scope

- Changing the worktree directory naming scheme (`task_worktree_path` still uses `task_id` for the filesystem path — this is orthogonal to the branch name)
- Refactoring `sync_project_branch` to skip work when the branch already matches (it's idempotent and the overhead is negligible)
- Cleaning up leftover `ralph/daemon/*` branches from previous runs on existing repos (users can prune manually with `git branch -D`)
- Changing `create_worktree_on_branch` (used by the rebase agent) — it already accepts an explicit branch parameter and is not affected
- Modifying the `detect_legacy_slug_branch` exclusion filter (line 1391) — the `ralph/daemon/` exclusion must remain to prevent false legacy-slug classification of leftover daemon branches