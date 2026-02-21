The revised specification has been written to `.ralph/quick-prd/22efc61a4bf6/draft.md`. Here's a summary of how each review issue was addressed:

### Review Issue Resolutions

1. **Auto-rebase Entry Conditions** — The `Conflicting` match arm in `auto_rebase_phase` (`runtime.rs:1101-1112`) now conditionally proceeds when the agent is enabled instead of unconditionally skipping. `Unknown` gate is unchanged.

2. **Config Surface Completeness** — Full 7-file wiring specified: `WorkspaceConfig`, `ProjectDaemonOverrides`, `EffectiveDaemonConfig` + `resolve_daemon_config()`, `DaemonRuntimeConfig`, `cli/daemon.rs` threading, and `cli/config.rs` get/set/show mappings for both global and project scopes.

3. **Disable Sentinel Handling** — New `validate_rebase_agent_backend()` function in `backend_spec.rs` explicitly allows `"none"` before delegating to `validate_backend_spec_name()`. Runtime checks at both the gating level and the agent entry point.

4. **Backend Invocation Design** — Replaced hardcoded `claude -p` with resolution through `parse_backend_spec` → `backend_config()` → configured command/args/env, mirroring the `refine.rs:50-60` pattern. Backend-specific arg builders for Claude and Codex.

5. **Conflict Detection Robustness** — Classification now uses `is_rebase_in_progress()` (checks `.git/rebase-merge` and `.git/rebase-apply`) combined with `has_conflicts()`, not stderr text parsing.

6. **Conflict File Parsing Edge Cases** — New `conflicting_files_z()` using `git status --porcelain -z` with NUL-delimited parsing, correctly handling renames and special characters. Existing `conflicting_files()` left unchanged for backward compatibility.

7. **Rebase Continue Edge Cases** — Resolution loop now handles: new conflicts (re-invoke agent), empty commits (detect and `git rebase --skip`), and non-recoverable failures (abort immediately). `GIT_EDITOR=true` prevents editor invocation.

8. **Timeout/Process Lifecycle** — Agent spawner uses `setsid()` for process-group isolation and `kill(-(pid), SIGKILL)` for group termination (matching existing patterns in `process.rs` and `backend/mod.rs`). Stdout/stderr consumed asynchronously to prevent pipe deadlocks.

9. **Testing Coverage** — Tests now span: unit tests in `rebase_agent.rs`, `git/mod.rs`, and `backend_spec.rs`; conformance tests in `src/validate/tests_daemon.rs` (not just a standalone `tests/` file) covering default values, disable path, gating change, failure/timeout fallback, full resolution flow, multi-commit, and empty-commit skip.