I now have a thorough understanding of the codebase. Let me write the engineering specification.

---

## Summary

Enhance the daemon's auto-rebase process to invoke an AI-powered "rebase agent" when `git rebase` fails due to merge conflicts, instead of immediately aborting and posting a failure comment. The rebase agent — defaulting to Claude Opus — will be spawned in the conflict-bearing worktree to resolve conflicts, stage resolutions, and continue the rebase. This converts a class of persistent rebase failures into automated resolutions, reducing manual intervention for daemon-managed PR branches.

## Acceptance Criteria

- [ ] Current `execute_rebase` in `src/daemon/runtime.rs` detects conflict-specific rebase failures (exit code 1 with conflict markers) and distinguishes them from other rebase errors (e.g., dirty worktree, invalid upstream)
- [ ] A new `rebase_agent` module under `src/daemon/` encapsulates the conflict resolution agent lifecycle: prompt construction, agent invocation, conflict verification, and `git rebase --continue` loop
- [ ] The agent is invoked automatically when `git rebase <target>` exits with conflicts — no user interaction required
- [ ] Claude Opus is the default rebase agent backend, configurable via `daemon_rebase_agent_backend` in `WorkspaceConfig` (defaults to `"claude(opus)"`)
- [ ] The agent resolution loop supports multi-commit rebases: after resolving one commit's conflicts and running `git rebase --continue`, it re-checks for new conflicts from subsequent commits and re-invokes the agent as needed
- [ ] Agent invocation respects the existing `rebase_timeout_seconds` budget — the shared deadline is passed through to the agent phase
- [ ] If the agent fails to resolve conflicts (agent error, timeout, or conflicts remain after resolution), the rebase is aborted (`git rebase --abort`) and the existing failure-comment path is taken — no behavior regression
- [ ] A new `RebaseConflict` error variant or structured return type distinguishes conflict failures from non-conflict failures in `execute_rebase`
- [ ] Configuration is backward-compatible: existing `ralph.toml` files without `daemon_rebase_agent_backend` use the default `"claude(opus)"`
- [ ] The rebase agent can be disabled entirely by setting `daemon_rebase_agent_backend = "none"`
- [ ] Unit tests verify: conflict detection from rebase stderr, agent prompt construction, timeout budget accounting, and the disable path
- [ ] Integration-style tests verify the full flow using mock backends in a temp git repo with synthetic conflicts

## Technical Approach

### 1. Conflict Detection in `execute_rebase`

Modify `execute_rebase` (`src/daemon/runtime.rs:1217-1291`) to distinguish conflict failures from other rebase failures. When `git rebase` fails:
- Check if `git::has_conflicts(worktree_path)` returns `true` (uses existing porcelain status parser in `src/git/mod.rs:59-69`)
- If conflicts exist, enter the agent resolution loop instead of immediately aborting
- If no conflicts (other failure reason), abort and return error as today

### 2. New `src/daemon/rebase_agent.rs` Module

Create a focused module with a single public function:

```rust
pub fn resolve_rebase_conflicts(
    worktree_path: &Path,
    rebase_target: &str,
    agent_backend: &str,  // e.g. "claude(opus)"
    deadline: Instant,
) -> Result<()>
```

**Agent invocation strategy**: Spawn the configured backend CLI directly (not through the `Backend` trait, which is async and session-aware). The rebase agent is a one-shot, stateless invocation:

```
claude -p --model opus --permission-mode acceptEdits \
  --allowedTools "Bash,Edit,Read,Glob,Grep" \
  "Resolve the following git merge conflicts in this worktree. [list of files]. 
   After resolving each file, run `git add <file>`. Do not run git rebase --continue."
```

Use `process::run_command_with_timeout` (existing in `src/daemon/process.rs:163-203`) to enforce the remaining timeout budget.

**Resolution loop** (handles multi-commit rebases):
1. Invoke agent with conflicting file list from `git::conflicting_files()`
2. After agent completes, verify `git::has_conflicts()` is false
3. If conflicts remain, return error (agent failed to resolve)
4. Run `git rebase --continue`
5. If `--continue` fails with new conflicts (next commit), goto 1
6. If `--continue` succeeds, return Ok — caller proceeds to push
7. Cap loop iterations (e.g., 10) to prevent infinite loops on adversarial repos

### 3. Configuration Extension

Add to `WorkspaceConfig` in `src/config/global.rs`:

```rust
#[serde(default = "default_daemon_rebase_agent_backend")]
pub daemon_rebase_agent_backend: String,
```

Default: `"claude(opus)"`. Special value `"none"` disables agent resolution.

Add corresponding `rebase_agent_backend: Option<String>` to `ProjectDaemonOverrides` in `src/config/project.rs`.

Thread the resolved value into `DaemonRuntimeConfig` in `src/daemon/runtime.rs`.

### 4. Integration into `execute_rebase`

Restructure `execute_rebase` to:
1. Fetch (unchanged)
2. Rebase — on failure, check for conflicts
3. If conflicts and agent enabled: call `resolve_rebase_conflicts()` 
4. If agent succeeds: proceed to push
5. If agent fails or disabled: abort rebase, return error (existing path)

### 5. Prompt Construction

The agent prompt will include:
- The list of conflicting files (from `git::conflicting_files()`)
- The rebase target branch name for context
- Instructions to read each conflicting file, resolve conflict markers, and `git add` each resolved file
- Explicit instruction to NOT run `git rebase --continue` (the caller handles that)

### 6. Error Handling

- Agent process spawn failure → abort rebase, return error
- Agent timeout (deadline exceeded) → kill agent process, abort rebase, return error
- Agent exits non-zero → abort rebase, return error
- Agent exits 0 but conflicts remain → abort rebase, return error with specific message
- All error paths fall through to the existing failure-comment posting in `auto_rebase_phase`

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/rebase_agent.rs` | **New** — agent invocation, prompt construction, resolution loop |
| `src/daemon/mod.rs` | Add `pub mod rebase_agent;` |
| `src/daemon/runtime.rs` | Modify `execute_rebase` to call agent on conflict; thread `rebase_agent_backend` through `DaemonRuntimeConfig` |
| `src/config/global.rs` | Add `daemon_rebase_agent_backend` to `WorkspaceConfig` with default |
| `src/config/project.rs` | Add `rebase_agent_backend` to `ProjectDaemonOverrides` |
| `src/cli/daemon.rs` | Thread new config field from `GlobalConfig` into `DaemonRuntimeConfig` |
| `src/git/mod.rs` | No changes needed — existing `has_conflicts()` and `conflicting_files()` are sufficient |
| `tests/daemon_rebase_agent.rs` | **New** — unit and integration tests |

## Testing Strategy

### Unit Tests (in `src/daemon/rebase_agent.rs`)
- **Prompt construction**: Verify generated prompt includes file list and correct instructions
- **`"none"` disables agent**: When backend is `"none"`, `resolve_rebase_conflicts` returns an error immediately indicating agent is disabled
- **Timeout budget accounting**: Verify remaining budget is correctly computed from deadline

### Integration Tests (in `tests/daemon_rebase_agent.rs`)
- **Synthetic conflict resolution**: Create a temp git repo, create a branch with a conflicting change, attempt rebase, verify conflict detection triggers agent path. Use a mock script as the "agent backend" that resolves conflicts by writing fixed content and running `git add`
- **Multi-commit rebase**: Create a branch with 2+ conflicting commits, verify the resolution loop handles `git rebase --continue` cycling through each commit
- **Agent failure fallback**: Use a mock agent that exits non-zero, verify the rebase is aborted and error is returned
- **Unresolved conflicts fallback**: Use a mock agent that exits 0 without resolving conflicts, verify abort path
- **Timeout enforcement**: Use a mock agent that sleeps past the deadline, verify timeout and abort

### Existing Test Compatibility
- Run the full `cargo test` suite to verify no regressions to existing rebase, daemon, and config tests
- The conformance validation suite (`src/validate/`) should pass without modification since it tests the orchestrator, not the daemon rebase path

## Out of Scope

- **Interactive conflict resolution**: This spec covers only automated/unattended resolution within the daemon. No user-facing UI for conflict review
- **Non-daemon rebase**: The `git::branch::sync_project_branch()` path used by `ralph run` is not modified; this feature is daemon-only
- **Codex as rebase agent**: While the config supports any backend string, testing and prompt tuning targets Claude only. Codex compatibility is not validated
- **Conflict resolution quality metrics**: No tracking of resolution success rate, quality scoring, or agent output auditing
- **Session reuse for rebase agent**: The agent is one-shot per conflict set; no session resume across commits or rebase attempts
- **PR comment enrichment**: Failure comments are not enhanced to distinguish "agent attempted but failed" from "no agent available". This could be a follow-up
- **Rebase strategy configuration**: No support for `git rebase --strategy` or `--strategy-option` flags; uses default merge strategy