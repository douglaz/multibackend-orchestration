---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 3
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T10:35:39Z
---

## Summary of changes

### P1 Fix 1: Sanitized env vars re-introduced in tmux backend

**File:** `src/backend/tmux_backend.rs`

**Problem:** `build_shell_command()` emitted `unset CLAUDECODE;` then `export CLAUDECODE=...;` in sequence if the backend had `CLAUDECODE` configured in its env map, re-introducing the sanitized var.

**Fix:** Added a `SANITIZED_ENV_VARS.contains()` check in the export loop (line 133) to skip any env var that appears in the sanitized list. The `unset` block remains as a defense-in-depth measure against process-level inheritance.

**Test:** Added `build_shell_command_filters_sanitized_env_vars` test that configures a backend with both `CLAUDECODE` and `SAFE_VAR` in its env, then asserts:
- `unset 'CLAUDECODE'` is present
- `export 'CLAUDECODE'` is NOT present
- `export 'SAFE_VAR'='ok'` IS present

### P1 Fix 2: `eprintln!` bypasses per-task `WithSubscriber` dispatch

**Files changed:**
- `src/prd/quick.rs` — 1 `eprintln!` → `warn!` (added `use tracing::warn`)
- `src/workflow/orchestrator.rs` — 1 `eprintln!` → `warn!` (already had `warn` imported)
- `src/git/branch.rs` — 5 `eprintln!` → `warn!` (added `use tracing::warn`). All are in `sync_project_branch`, which is task-executed.
- `src/workspace/mod.rs` — 2 `eprintln!` → `info!`/`warn!` (added `use tracing::{info, warn}`). The migration message uses `info!`; the skip-on-error message uses `warn!`.

**Rationale:** `eprintln!` writes directly to stderr, bypassing the per-task `tracing::Dispatch` set via `WithSubscriber`. Converting to tracing macros ensures output routes through each task's file subscriber into `.ralph/tmp/logs/<task_id>.log`, preventing interleaving across concurrent tasks.
