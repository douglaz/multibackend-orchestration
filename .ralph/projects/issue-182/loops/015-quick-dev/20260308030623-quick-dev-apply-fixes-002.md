---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 2
project: issue-182
backend: claude
role: implementer
created_at: 2026-03-08T03:06:23Z
---

# Apply-Fixes: Reviewer Changes Applied

## Change 1 (High): Fixed orphan terminalization branch logic

**File:** `src/daemon/runtime.rs:1084-1113`

The `poll_adopted_orphans` function was using `format!("ralph/daemon/{}", orphan.task_id)` which doesn't match the actual task branch pattern. Fixed to:

1. **First** check `TaskMetadata.pr_url` — parse the PR number via `github::extract_pr_number()`, then query PR state via `github::query_pr_merge_info()`. If `state == "MERGED"` → `ralph:completed`, otherwise → `ralph:failed`.
2. **Fallback** (no `pr_url` persisted) — use the correct project branch `ralph/issue-{issue_number}` instead of the incorrect `ralph/daemon/{task_id}`.

## Change 2 (Medium): Added 3 missing conformance tests

**File:** `src/validate/tests_daemon_orphan.rs` — now registers 10 tests (up from 7).

| Test | Spec path covered |
|------|------------------|
| `orphan_terminalization_routes_through_complete_task` | Adopts a dead-PID orphan, asserts `complete_task` side effects fire (comment posted, label swapped to terminal, PID/PGID cleared) |
| `crash_after_spawn_before_stage3` | Spawns a live child, persists PID/PGID but skips `children.insert` (crash window), asserts reconciliation adopts the orphan and does NOT reset the label |
| `dispatch_failure_clears_pid` | Pre-sets PID/PGID, triggers dispatch failure via a failing mock ralph, asserts PID/PGID are defensively cleared |
