### Title
AI-Assisted Conflict Recovery for Daemon Auto-Rebase

### Objective
Implement automated conflict resolution in daemon-managed rebases. When `git rebase <target>` fails due to merge conflicts, invoke a configurable AI “rebase agent” in the worktree to resolve conflicts, stage files, and let the daemon continue the rebase loop. Preserve current failure behavior for non-conflict errors and unresolved conflicts.

### In Scope
- Daemon auto-rebase path only (`execute_rebase` and related runtime/config plumbing).
- New `src/daemon/rebase_agent.rs` module for agent lifecycle.
- Config support for enabling/disabling and selecting backend.
- Unit, integration-style, and validate conformance coverage.

### Out of Scope
- Interactive/manual conflict resolution UI.
- Non-daemon rebase flows (for example `ralph run` project sync path).
- Resolution quality scoring, analytics, or PR comment redesign.
- Rebase strategy flags (`--strategy`, `--strategy-option`).

### Functional Requirements
1. Update `execute_rebase` in `src/daemon/runtime.rs` to classify rebase failures:
1. `git rebase <target>` success: return success unchanged.
2. Rebase conflict failure: only when all are true:
- exit code is `1`
- stderr contains conflict indicators (`CONFLICT` or `could not apply`)
- `git::has_conflicts(worktree_path)` is `true`
3. Any other failure is non-conflict and follows current abort/failure-comment path.

2. Add `src/daemon/rebase_agent.rs` with a single orchestration entrypoint:
```rust
pub fn resolve_rebase_conflicts(
    worktree_path: &Path,
    rebase_target: &str,
    agent_backend: &str,
    deadline: Instant,
) -> Result<(), RalphError>
```
Use a dedicated error enum internally, mapped into `RalphError` with clear messages.

3. Rebase-agent loop behavior (required, deterministic):
1. Max iterations: `10` (constant).
2. On each iteration:
- Read conflicting files via `git::conflicting_files()`.
- Build prompt from a fixed template (see Prompt Contract).
- Invoke agent command with remaining deadline budget.
- Verify conflicts cleared with `git::has_conflicts() == false`.
- Run `git rebase --continue` with remaining deadline budget.
3. If `--continue` introduces new conflicts, repeat iteration.
4. Exit success only when `git rebase --continue` succeeds and no conflicts remain.
5. If max iterations reached, return failure.

4. Timeout budget requirements:
- Use one shared `deadline` for initial rebase + all agent/continue steps.
- Before each subprocess call, compute remaining duration; if zero/negative, fail with timeout error.
- No step may run without bounded timeout.

5. Cleanup and fallback requirements:
- On agent failure (spawn error, timeout, non-zero exit, unresolved conflicts, loop cap), run `git rebase --abort` if rebase is in progress.
- Return error so existing daemon failure comment path executes unchanged.
- Non-conflict rebase failures keep existing behavior.

6. Configuration requirements:
- Add to `WorkspaceConfig` in `src/config/global.rs`:
```rust
#[serde(default = "default_daemon_rebase_agent_backend")]
pub daemon_rebase_agent_backend: String
```
Default: `"claude(opus)"`.
- Add project override in `src/config/project.rs`:
```rust
pub rebase_agent_backend: Option<String>
```
- Thread resolved value into `DaemonRuntimeConfig` and runtime callsites.
- Special value `"none"` disables agent path entirely and uses existing failure behavior.
- Backward compatibility: missing config key must behave as default `"claude(opus)"`.

7. Backend parsing/execution rules:
- Supported values for this feature: `"none"`, `"claude"`, `"claude(<model>)"`.
- `"claude"` means model default `"opus"`.
- Unsupported backend strings must produce a clear configuration/runtime error.
- Invoke via `process::run_command_with_timeout` in worktree directory.

8. Prompt Contract (must be fixed template with placeholders):
- Include rebase target branch.
- Include explicit conflicting file list.
- Require resolving markers and staging each resolved file (`git add`).
- Forbid `git rebase --continue` and `git rebase --abort`.
- Instruct agent not to modify unrelated files.

### Implementation Files
- `src/daemon/rebase_agent.rs` (new)
- `src/daemon/mod.rs` (export module)
- `src/daemon/runtime.rs` (failure classification, invocation, config threading)
- `src/config/global.rs` (new config key + default)
- `src/config/project.rs` (project override)
- `src/cli/daemon.rs` (thread resolved config into runtime)
- `src/git/mod.rs` (reuse existing conflict helpers; add helper only if needed)
- `tests/daemon_rebase_agent.rs` (integration-style tests)
- `src/validate/tests_daemon_rebase.rs` (new conformance tests) and `src/validate/mod.rs` registration

### Acceptance Criteria
- [ ] `execute_rebase` distinguishes conflict failures from non-conflict failures using explicit criteria above.
- [ ] Conflict failures invoke `resolve_rebase_conflicts` automatically unless backend is `"none"`.
- [ ] Multi-commit conflict rebases are handled via iterative resolve/continue loop up to 10 iterations.
- [ ] Shared deadline is enforced across agent and `rebase --continue` subprocesses.
- [ ] Any unresolved/failing agent path aborts rebase (if in progress) and triggers existing failure-comment flow.
- [ ] Config key `daemon_rebase_agent_backend` defaults to `"claude(opus)"` and is backward-compatible.
- [ ] `"none"` disables agent resolution and preserves prior behavior.
- [ ] Unsupported backend strings fail clearly.
- [ ] Unit tests cover classifier logic, prompt construction, timeout accounting, backend parsing, and disable path.
- [ ] Integration-style tests cover successful conflict resolution, multi-commit conflicts, non-zero agent exit, unresolved conflicts, and timeout.
- [ ] Validate conformance tests are added and registered for daemon rebase-agent behavior.

### Testing Requirements
1. Unit tests:
- Conflict classification function.
- Prompt template rendering with target + files.
- Remaining-time calculation and timeout error path.
- Backend parsing: `none`, `claude`, `claude(opus)`, unsupported value.
- Disabled path behavior.

2. Integration-style tests (`tests/daemon_rebase_agent.rs`):
- Synthetic conflict repo with mock `claude` executable resolving and staging files.
- Multi-commit rebase conflicts requiring multiple loop iterations.
- Mock agent exits non-zero.
- Mock agent exits 0 but leaves conflicts.
- Mock agent exceeds deadline.

3. Validate tests (`src/validate/tests_daemon_rebase.rs`):
- Agent-enabled conflict recovery path.
- Agent-disabled (`none`) fallback path.
- Agent failure fallback path.

### Non-Functional Constraints
- No regressions to existing daemon rebase success/failure behavior outside conflict-specific handling.
- Keep error messages actionable and include whether agent was attempted.
- Keep changes localized; avoid refactoring unrelated daemon or backend systems.