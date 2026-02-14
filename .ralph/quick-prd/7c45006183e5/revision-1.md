The revised specification has been written. Here's a summary of how each review issue was addressed:

**Issue 1 — Conflict detection API inconsistency**: Locked to `--json mergeable,state,baseRefName,headRefOid`. Uses the `mergeable` field (values: `CONFLICTING`/`MERGEABLE`/`UNKNOWN`) consistently throughout — the `mergeStateStatus` field is dropped. Exact mapping from field values to `PrMergeStatus` enum is specified.

**Issue 2 — Rebase target branch source**: The `baseRefName` field is now fetched in the PR query and returned in `PrMergeInfo.base_branch`. The rebase always targets `origin/<baseRefName>` from PR metadata, never a hardcoded default branch. `DaemonTask` does not need to persist it since it's fetched fresh each cycle.

**Issue 3 — Branch/worktree feasibility**: Added a new `create_worktree_on_branch(repo_root, workspace_root, task_id, branch)` function that creates a worktree at `rebase-{task_id}` on the actual PR branch (which may differ from `ralph/daemon/{task_id}`). The existing `create_worktree` is untouched.

**Issue 4 — Requirement alignment (PR vs issue comments)**: Rebase comments are now posted on the PR via `gh pr comment`, not on the linked issue. A new `post_pr_comment` function is added alongside the existing `post_idempotent_comment`.

**Issue 5 — Failure retry and idempotency**: Failure comment markers are keyed by `head_sha` (`<!-- ralph:rebase:{task_id}:failed:{head_sha} -->`), not by timestamp. Same head SHA → no duplicate comment. New head SHA → new comment. This eliminates the failure comment spam problem.

**Issue 6 — Non-interference with active workflows**: Added `MAX_REBASES_PER_CYCLE` (3) cap and `REBASE_TIMEOUT_SECONDS` (120) per-rebase deadline. Worst-case rebase phase is ~6 minutes. Runs after `collect_children` to ensure children are reaped promptly.

**Issue 7 — Rate-limit handling**: If `gh pr view` exits non-zero (including 403/rate-limit), the daemon logs a warning including stderr and breaks out of the per-PR loop for the current cycle. Combined with the rebase interval, per-cycle cap, and break-on-error, this provides adequate throttling without new infrastructure.

**Issue 8 — Testing completeness**: Added 15 validate conformance tests in `tests_daemon.rs` covering: config defaults/overrides, disabled skip, conflict detection, merged/closed skip, unknown skip, branch-switched tasks, base-branch resolution, PR comment posting, failure deduplication, force-with-lease rejection, rate-limit handling, per-cycle cap, recently-rebased skip, status output, and backwards-compatible serialization. Also added `ralph config get/set/show` plumbing for both global and project scope.

**Issue 9 — State and observability clarity**: Defined exact skip policy — tasks are skipped if `last_rebase_at` is within the rebase interval. Added `last_rebase_head_sha` for failure deduplication. Specified `LAST REBASE` column in `ralph daemon status` output with exact format.