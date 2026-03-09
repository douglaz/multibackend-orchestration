---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T17:07:25Z
---

The `current_dir()` calls now only appear in `execute()` (the CLI boundary), not in `ensure_workspace()`. The `ensure_workspace` function receives the resolved path via the `fallback_cwd` parameter.

# Applied Reviewer Fixes

## Fix 1: CLI backward-compatibility for interactive tracing output

**Problem:** CLI callers (`auto`, `run`, `quick-dev-auto`, `quick-dev-run`) invoked library tasks without a per-task tracing subscriber, so `tracing::info!` progress messages from the library entry points were only captured by the global subscriber (if present).

**Changes:**
- `src/daemon/tasks.rs`: Added `cli_stderr_dispatch()` — builds a `tracing::Dispatch` writing to stderr with `EnvFilter` (matching `main.rs` config), no timestamps, no target prefix.
- `src/cli/auto.rs:170-192`: Wraps `run_auto_task(...)` with `.with_subscriber(dispatch)`.
- `src/cli/quick_dev_auto.rs:94-108`: Wraps `run_quick_dev_auto_task(...)` with `.with_subscriber(dispatch)`.
- `src/cli/run.rs:26-48`: Wraps `run_run_task(...)` with `.with_subscriber(dispatch)`.
- `src/cli/quick_dev_run.rs:57-70`: Wraps `run_quick_dev_run_task(...)` with `.with_subscriber(dispatch)`.

All four CLI callers now explicitly attach a stderr tracing subscriber to their task futures via `WithSubscriber`, ensuring interactive users see progress output.

## Fix 2: CWD-safety — remove `current_dir()` from `ensure_workspace`

**Problem:** `ensure_workspace()` in `auto.rs` and `quick_dev_auto.rs` called `std::env::current_dir()` directly in the workspace-not-found fallback path.

**Changes:**
- `src/cli/auto.rs:116`: `ensure_workspace` signature changed to accept `fallback_cwd: &Path`. The fallback path uses `fallback_cwd.join(".ralph")` instead of `std::env::current_dir()?.join(".ralph")`.
- `src/cli/quick_dev_auto.rs:51`: Same signature change and fallback replacement.
- `src/cli/auto.rs:147`: CWD resolved once at the CLI boundary (`let cwd = std::env::current_dir()?;`) and passed to `ensure_workspace(args.workspace_root.as_ref(), &cwd)`.
- `src/cli/quick_dev_auto.rs:82`: Same boundary-level CWD resolution.
- Tests updated: removed `CwdGuard`/`cwd_lock` helpers from `auto.rs` tests; `ensure_workspace` test calls now pass an explicit `fallback_cwd` from `tempdir()` instead of mutating process-global CWD.
