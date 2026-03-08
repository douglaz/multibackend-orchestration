---
artifact: quick-dev-plan-implement
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T09:43:02Z
---

All review feedback items are now addressed. Here's a summary of the complete changeset:

**Functional fixes:**
1. **Backoff cancellation** (P1): Wrapped retry backoff `sleep()` in `tokio::select!` with `cancel.cancelled()` in both `orchestrator.rs` and `quick_dev_orchestrator.rs`
2. **KillOnDrop reaper** (P2): Added reaper thread for zombie cleanup when `WNOHANG` returns 0, with comment about tokio reaper race
3. **Tmux env sanitization** (P1): Applied `SANITIZED_ENV_VARS` via `unset` commands in `build_shell_command()`

**Code quality:**
4. **`run_in()` → `run()`** (P2): Renamed in `prd/quick.rs` and updated 6 call sites
5. **`max_backend_retries()` dedup** (P2): Extracted to `workflow/mod.rs`, removed duplicates from both orchestrators
6. **Config plumbing**: Added `daemon_max_backend_retries` to `WorkspaceConfig`, plumbed through `EffectiveDaemonConfig` → `DaemonRuntimeConfig` → task params

**Test fixes:**
7. **Daemon concurrency assertions**: Tightened 4 loose OR-pattern assertions to require specific strings
8. **Interactive PRD tests**: Replaced child-arg capture assertions with in-process dispatch log checks
9. **PR runtime tests**: Migrated `pr_url_plumbed_through_child_args` to assert dispatch log instead of child args; removed `e2e_draft_create_via_binary` (relied on `RALPH_DAEMON_BIN` subprocess dispatch, draft-PR watcher is already covered by dedicated tests)

**Files changed:** 19 files, +135/-183 lines. `cargo check --tests` passes clean.
