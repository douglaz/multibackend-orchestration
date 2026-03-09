---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T08:23:46Z
---

# Implementation: Fix Hard Rollback Stale Tracking Ref + Strengthen Session Test

## Changes Made

### 1. [P1] Fix stale tracking ref resurrection (`src/cli/rollback.rs`)

**Execution path** (line 126–137): Replaced the two-step branch recovery that trusted local `origin/<branch>` tracking refs first with a single authoritative check via `remote_branch_exists_on_remote()` (uses `git ls-remote --exit-code`). If the branch doesn't exist on the actual remote, rollback fails immediately — even if a stale local tracking ref exists.

**Dry-run path** (line 79): Removed the `remote_ref_exists()` check (local tracking ref) from the fallback condition. Now only `remote_branch_exists_on_remote()` is used, matching the execution path behavior.

**Import cleanup**: Removed unused `remote_ref_exists` import.

### 2. [P1] New validate test: `rollback_hard_stale_tracking_ref` (`src/validate/tests_commands.rs`)

Covers the exact regression scenario:
1. Run 2 loops, detach HEAD, delete local branch
2. Verify stale `origin/<branch>` tracking ref still exists
3. Delete branch from the bare remote
4. Assert `rollback --hard 1` **fails** (not silently succeeds via stale ref)
5. Assert HEAD doesn't move
6. Assert `rollback --hard --dry-run 1` also fails

### 3. [P3] Strengthen `rollback_push_failure_continues` session assertions (`src/validate/tests_commands.rs`)

Enabled `session_reuse_enabled=true` and `session_reuse_roles=implementer,reviewer` before running loops, so the session invalidation code path in rollback is exercised. Added a comment explaining the structural limitation: the standard mock doesn't emit `session_id` values, so no real `SessionRecord`s are created — real invalidation is unit-tested in `state.rs::remove_loop_clears_session_records`.
