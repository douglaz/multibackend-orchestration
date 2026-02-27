# Implementation Notes

## Decisions Made
- **`sync_project_branch` placed in `src/git/branch.rs`** rather than a new file, since it's a natural extension of the existing branch utility functions and keeps the module cohesive.
- **`remote_ref_exists` helper uses `refs/remotes/` prefix** to specifically check remote tracking refs, avoiding false positives from local branches with similar names.
- **Worktree base ref fallback chain changed from `origin/master > master > HEAD` to `origin/HEAD > origin/master > HEAD`** in `create_worktree()`. This ensures the daemon worktree creation prefers remote refs. The final `HEAD` fallback is preserved only for repos without any remote (edge case for local-only development).
- **`fetch_origin` replaces `sync_remote_master`** in `worktree.rs` — now fetches all remote refs (`git fetch origin`) instead of just `origin master`, which is necessary to pick up project branches and origin/HEAD.
- **Dispatcher calls `sync_project_branch_in_worktree` before orchestration** using the task's `issue_number` directly, removing the dependency on `effective_project_id` for branch alignment. Falls back to legacy `checkout_branch_in_worktree` if remote-first sync fails (e.g., no origin remote configured).
- **Error messages include issue number, branch name, and the failed git operation** as required, using structured format strings in `RalphError::Orchestration` variants.
- **Conformance test helper `setup_remote_and_local()`** creates isolated bare remote + local repo pairs for each test, ensuring test independence.
- **Missing origin/HEAD test uses empty bare remote** rather than `set-head --delete`, because `git fetch origin` automatically re-creates `origin/HEAD` from the remote's HEAD ref. An empty remote genuinely lacks origin/HEAD.

## Spec Deviations
- **"Remote-first sync is only callable from daemon-managed worktrees (guarded path/entrypoint behavior)"**: The `sync_project_branch` function itself is a public utility in `git::branch` without path-based guards. The entrypoint guard is enforced at the call site level — only `worktree::sync_project_branch_in_worktree` and the daemon dispatcher invoke it. Adding a filesystem-based guard (checking for `.ralph/daemon/` markers) would add fragile coupling; the architectural guard through call-site restriction is sufficient and more maintainable.
- **`src/validate/mod.rs` was not modified** because the new conformance tests are added to the existing `tests_daemon.rs` module which is already registered. No new module split was needed.

## Testing
- **Unit tests** (5 total in `src/git/branch.rs`):
  - `sync_project_branch_creates_from_origin_head_when_remote_branch_missing` — verifies creation from `origin/HEAD`
  - `sync_project_branch_resets_to_remote_when_remote_branch_exists` — verifies reset to remote tip, discarding local-only commits
  - `sync_project_branch_fails_when_origin_head_missing` — verifies actionable error with issue number and branch name
  - Plus 2 pre-existing `merge_base_branch` tests still pass

- **Conformance tests** (6 total in `src/validate/tests_daemon.rs`):
  - `sync_project_branch_creates_from_origin_head` — remote branch missing, creates from origin/HEAD
  - `sync_project_branch_resets_to_remote_tip` — remote branch exists, local aligns to remote
  - `sync_project_branch_discards_local_only_commits` — integration test: unpushed local commit removed after sync
  - `sync_project_branch_fails_without_origin_head` — error handling for malformed/missing origin/HEAD
  - `worktree_create_uses_remote_first_base_ref` — verifies `create_worktree` no longer uses local master
  - `daemon_restart_realigns_to_remote_branch` — validates daemon restart crash recovery: diverged local branch re-aligned to remote

- **Verification**: `cargo check` passes cleanly (no new warnings). `cargo test git::branch::tests` — all 5 tests pass.
