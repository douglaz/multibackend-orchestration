---
artifact: final-review-votes
loop: 4
project: issue-149
backend: claude
role: final_reviewer
created_at: 2026-03-04T02:52:09Z
---

# Vote Results

## Amendment: FR-20260304-01

### Vote
ACCEPT

### Rationale
I verified the defect independently. The classifier at `github.rs:911-912` operates on `err.to_string().to_ascii_lowercase()`, and the error constructed at `github.rs:1007-1010` embeds the branch name directly into that string (`"git push failed for branch {branch}: {stderr}"`). The pattern lists include bare numeric tokens like `"403"` (line 921, non-retryable) and `"500"` (line 933, retryable). A branch named e.g. `feature/fix-500-timeout` would match the retryable pattern regardless of the actual stderr content, and `hotfix/403-auth` would match the non-retryable pattern. This is a real classification bug.

The default-to-retry at line 954-955 compounds the issue: any truly permanent error not matching an explicit pattern will be needlessly retried 3 times with exponential backoff (10s, 20s, 40s per `push_branch_with_retry` at line 960), delaying failure by over a minute. Defaulting unknown errors to non-retryable is safer — the worst case is a single failure on a genuinely transient error that doesn't match any pattern, which is far less disruptive than retrying permanent failures.

The existing tests (lines 2212-2278) only pass simple literal strings directly to `is_retryable_push_error` and never construct errors that simulate the real formatted error shape with embedded branch names. The proposed test additions for branch-name collision and unknown-permanent cases fill genuine coverage gaps.

The proposed fix is well-scoped: extract raw stderr for classification, flip the default, and add targeted tests. Low risk, high value.

## Amendment: FR-20260304-02

### Vote
ACCEPT

### Rationale
I confirmed the inconsistency directly. In `drain_all_children()` (lines 2014-2023), both watcher joins use bare `join_handle.await` without any timeout. In contrast, `collect_children()` (lines 1800-1805) and `kill_aborted_children()` (lines 1967-1971) both use `await_watcher_with_timeout()`, which applies `WATCHER_TEARDOWN_TIMEOUT` and aborts the task if it exceeds that bound (lines 1723-1738).

The `drain_all_children()` force-kill path is specifically the *last resort* shutdown path — the one that runs when the graceful drain has already timed out. It is precisely this path that must not hang, yet it is the one path that lacks the timeout guard. If a watcher task is stuck in a `spawn_blocking_op` (lines 85-93, no internal timeout), the unbounded await will block indefinitely, preventing `complete_task(..., "ralph:failed", ...)` at line 2026-2033 from ever executing for that task.

The fix is mechanical: replace the two bare `join_handle.await` calls with `await_watcher_with_timeout(join_handle, "artifact watcher", &task_id).await` and `await_watcher_with_timeout(join_handle, "draft PR watcher", &task_id).await` respectively, matching the pattern already established in the other teardown paths. Zero architectural risk, clear correctness improvement.
