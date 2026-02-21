## Summary

Enhance the daemon's auto-rebase process to invoke an AI-powered "rebase agent" when `git rebase` fails due to merge conflicts, instead of immediately aborting and posting a failure comment. The rebase agent — defaulting to Claude Opus — will be spawned in the conflict-bearing worktree to resolve conflicts, stage resolutions, and continue the rebase. This converts a class of persistent rebase failures into automated resolutions, reducing manual intervention for daemon-managed PR branches.

Additionally, the `auto_rebase_phase` gating logic is relaxed so that PRs with `mergeable=CONFLICTING` status are no longer skipped outright — they are eligible for rebase with agent-assisted conflict resolution. This is necessary because the existing gate (`runtime.rs:1101-1112`) skips all conflicting PRs before `execute_rebase` runs, meaning without this change the agent would never trigger for the primary conflict case.

## Acceptance Criteria

- [ ] `auto_rebase_phase` in `src/daemon/runtime.rs` no longer skips PRs whose GitHub `merge_status` is `Conflicting`; instead, these PRs proceed to `execute_rebase` when the rebase agent is enabled (agent backend is not `"none"`)
- [ ] `execute_rebase` detects conflict-specific rebase failures using repository state (`git::has_conflicts()` and `git::is_rebase_in_progress()`) rather than parsing stderr text, distinguishing conflicts from other rebase errors (dirty worktree, invalid upstream)
- [ ] A new `rebase_agent` module under `src/daemon/` encapsulates the conflict resolution agent lifecycle: backend resolution, prompt construction, agent invocation, conflict verification, and `git rebase --continue` loop
- [ ] The agent is invoked automatically when `git rebase <target>` exits with conflicts — no user interaction required
- [ ] Claude Opus is the default rebase agent backend, configurable via `daemon_rebase_agent_backend` in `WorkspaceConfig` (defaults to `"claude(opus)"`)
- [ ] The rebase agent can be disabled entirely by setting `daemon_rebase_agent_backend = "none"`; validation logic in `validate_backend_spec` and `validate_backend_spec_name` explicitly allows the `"none"` sentinel for this config key
- [ ] The agent resolution loop supports multi-commit rebases: after resolving one commit's conflicts and running `git rebase --continue`, it re-checks for new conflicts from subsequent commits and re-invokes the agent as needed
- [ ] The resolution loop handles `git rebase --continue` outcomes beyond conflicts-or-success: empty commits (detected via exit code + stderr) are automatically advanced with `git rebase --skip`, and other non-conflict failures (hook errors, editor failures) trigger immediate abort
- [ ] Agent invocation respects the existing `rebase_timeout_seconds` budget — the shared deadline is passed through to the agent phase
- [ ] Agent invocation uses process-group isolation (`setsid()`) and kills the entire process group on timeout, preventing subprocess leaks; stdout/stderr are consumed asynchronously to prevent pipe-buffer deadlocks on verbose agent output
- [ ] If the agent fails to resolve conflicts (agent error, timeout, or conflicts remain after resolution), the rebase is aborted (`git rebase --abort`) and the existing failure-comment path is taken — no behavior regression
- [ ] A new `RebaseConflict` structured return type distinguishes conflict failures from non-conflict failures in `execute_rebase`
- [ ] The agent backend is resolved through the existing backend config machinery (`parse_backend_spec` → `claude::backend_from_config` / `codex::backend_from_config`), using the configured command path, args, and env from `GlobalConfig.backends` rather than hardcoding CLI invocations
- [ ] Configuration is backward-compatible: existing `ralph.toml` files without `daemon_rebase_agent_backend` use the default `"claude(opus)"`
- [ ] The full config wiring is complete: `WorkspaceConfig` field, `ProjectDaemonOverrides` field, `EffectiveDaemonConfig` field, `resolve_daemon_config()` merge logic, `DaemonRuntimeConfig` field, `cli/daemon.rs` threading, and `cli/config.rs` get/set/show mappings for both global and project scopes
- [ ] `git::conflicting_files()` is hardened to use `git status --porcelain -z` (NUL-delimited) parsing, correctly handling quoted paths, renames, and filenames with special characters
- [ ] Unit tests verify: conflict detection from repository state, agent prompt construction, timeout budget accounting, the `"none"` disable path, and `"none"` sentinel validation bypass
- [ ] Conformance tests in `src/validate/tests_daemon.rs` cover: default config value, `"none"` disable path, agent failure/timeout fallback, and the `Conflicting` merge-status gating change
- [ ] Integration-style tests verify the full resolution loop using mock agent scripts in a temp git repo with synthetic conflicts, including multi-commit rebases and `--skip` handling

## Technical Approach

### 1. Auto-Rebase Entry Condition Change

**Problem (Review Issue #1):** The current `auto_rebase_phase` (`runtime.rs:1101-1112`) skips PRs with `PrMergeStatus::Conflicting` before `execute_rebase` ever runs. If the agent is enabled, these are exactly the PRs it should attempt to resolve.

**Solution:** Change the `Conflicting` match arm to proceed when the rebase agent is enabled:

```rust
match merge_info.merge_status {
    PrMergeStatus::Conflicting => {
        if config.rebase_agent_backend == "none" {
            eprintln!("auto-rebase: skip {task_id} — PR merge status is Conflicting (agent disabled)");
            continue;
        }
        eprintln!("auto-rebase: {task_id} — PR is Conflicting, attempting rebase with agent");
        // fall through to execute_rebase
    }
    PrMergeStatus::Unknown => {
        eprintln!("auto-rebase: skip {task_id} — PR merge status is Unknown");
        continue;
    }
    PrMergeStatus::Mergeable => {}
}
```

The `Unknown` gate remains unchanged. The `Conflicting` gate now conditionally proceeds based on agent availability.

### 2. Conflict Detection via Repository State

**Problem (Review Issue #5):** The original spec mentions "exit code 1 with conflict markers" which is brittle — stderr text varies by git version and locale.

**Solution:** Conflict classification relies entirely on repository state, not message parsing:

1. Add `git::is_rebase_in_progress(workdir)` — checks for `.git/rebase-merge/` or `.git/rebase-apply/` directories (or the worktree equivalent under `.git/worktrees/<name>/`)
2. When `git rebase` fails (non-zero exit), check:
   - `git::is_rebase_in_progress(worktree_path)` AND `git::has_conflicts(worktree_path)` → conflict failure, enter agent loop
   - `git::is_rebase_in_progress(worktree_path)` AND NOT `has_conflicts` → non-conflict rebase failure (e.g., hook error), abort immediately
   - NOT `is_rebase_in_progress` → rebase never started or already aborted, return error as today

This is deterministic and locale-independent.

### 3. Hardened Conflict File Discovery

**Problem (Review Issue #6):** Current `conflicting_files()` uses `git status --porcelain` (text mode, newline-delimited) which is fragile for quoted paths, renames, and filenames with special characters.

**Solution:** Add a new `conflicting_files_z()` function in `src/git/mod.rs` that uses NUL-delimited output:

```rust
pub fn conflicting_files_z(workdir: &Path) -> Result<Vec<String>> {
    ensure_git_repo(workdir)?;
    let output = Command::new("git")
        .args(["status", "--porcelain", "-z"])
        .current_dir(workdir)
        .output()?;
    // NUL-delimited entries: "XY path\0" or "XY old\0new\0" for renames
    let raw = output.stdout;
    let mut files = Vec::new();
    let mut cursor = 0;
    while cursor < raw.len() {
        // Read XY status (2 bytes) + space (1 byte)
        if cursor + 3 > raw.len() { break; }
        let xy = &raw[cursor..cursor + 2];
        let prefix = std::str::from_utf8(xy).unwrap_or("");
        cursor += 3; // skip "XY "
        // Find NUL terminator for path
        let nul_pos = raw[cursor..].iter().position(|&b| b == 0)
            .unwrap_or(raw.len() - cursor);
        let path = String::from_utf8_lossy(&raw[cursor..cursor + nul_pos]).to_string();
        cursor += nul_pos + 1; // skip past NUL
        // For rename entries (R, C), skip the second path
        if prefix.contains('R') || prefix.contains('C') {
            if let Some(nul2) = raw[cursor..].iter().position(|&b| b == 0) {
                cursor += nul2 + 1;
            }
        }
        if matches!(prefix, "UU" | "AA" | "DD" | "AU" | "UA" | "DU" | "UD") {
            files.push(path);
        }
    }
    Ok(files)
}
```

The rebase agent module will use `conflicting_files_z()` exclusively. The existing `conflicting_files()` is left unchanged for backward compatibility with callers in `src/git/commit.rs`.

### 4. New `src/daemon/rebase_agent.rs` Module

Create a focused module with a single public entry point:

```rust
pub fn resolve_rebase_conflicts(
    worktree_path: &Path,
    rebase_target: &str,
    agent_backend: &str,     // e.g. "claude(opus)" or "none"
    global_config: &GlobalConfig,
    deadline: Instant,
) -> Result<()>
```

**Backend resolution (Review Issue #4):** Instead of hardcoding `claude -p --model opus ...`, resolve the backend command through the existing config machinery, mirroring the pattern in `src/daemon/refine.rs:50-60`:

```rust
fn create_agent_backend(backend_spec: &str, global_config: &GlobalConfig) -> Result<AgentCommand> {
    let spec = parse_backend_spec(backend_spec)?;
    let model = spec.model.as_deref();
    let backend_config = global_config.backend_config(&spec.name)
        .ok_or_else(|| RalphError::Validation(format!("unknown rebase agent backend: {backend_spec}")))?;

    // Build command from config: uses configured command path, env, base args
    let command = backend_config.command.clone();
    let env = backend_config.env.clone();

    // Build args based on backend type
    let args = match spec.name.as_str() {
        "claude" => build_claude_agent_args(model, &backend_config.args),
        "codex" => build_codex_agent_args(model, &backend_config.args),
        _ => return Err(RalphError::Validation(format!("unsupported rebase agent backend: {}", spec.name))),
    };

    Ok(AgentCommand { command, args, env })
}
```

For Claude, `build_claude_agent_args` constructs: `-p --model <model> --permission-mode acceptEdits --allowedTools "Bash,Edit,Read,Glob,Grep"`. For Codex, `build_codex_agent_args` constructs the equivalent one-shot execution form. Both use the configured `command` path (not hardcoded binary names) and merge in configured `env` vars.

**Agent process lifecycle (Review Issue #8):** Instead of using `run_command_with_timeout` (which uses piped stdout/stderr and only kills the direct child), use a purpose-built spawner that:

1. Places the child in its own process group via `setsid()` (matching the pattern in `src/daemon/process.rs:35-39` and `src/backend/mod.rs:405-415`)
2. Spawns reader threads for stdout and stderr that drain into bounded buffers, preventing pipe-buffer deadlocks on verbose agent output
3. On timeout, kills the entire process group via `kill(-(pid), SIGKILL)` (matching `kill_and_reap_child` in `src/backend/mod.rs:644-668`)
4. Uses the absolute deadline model (not activity-based) since the rebase agent shares a fixed timeout budget with fetch/rebase/push

```rust
fn run_agent_with_timeout(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    workdir: &Path,
    prompt: &str,
    deadline: Instant,
) -> Result<AgentOutput>
```

**Resolution loop (Review Issue #7):** Handles states beyond conflicts-or-success:

```
1. Invoke agent with conflicting file list from git::conflicting_files_z()
2. After agent completes, verify git::has_conflicts() is false
3. If conflicts remain → return error (agent failed to resolve)
4. Run `git rebase --continue` (with GIT_EDITOR=true to suppress editor)
5. Match on outcome:
   a. Success → return Ok (caller proceeds to push)
   b. Exit non-zero + has_conflicts() → new conflicts from next commit, goto 1
   c. Exit non-zero + stderr contains "nothing to commit" or
      "No changes" → empty commit after resolution, run `git rebase --skip`, goto 4
   d. Exit non-zero + other → non-recoverable failure, return error
6. Cap loop iterations at 10 to prevent infinite loops
```

The `GIT_EDITOR=true` environment variable is set for `git rebase --continue` to prevent it from opening an editor for commit messages. The empty-commit detection (step 5c) uses stderr content as a secondary signal only — the primary check is that `has_conflicts()` returns false while `is_rebase_in_progress()` returns true, indicating the rebase stalled on a no-op commit rather than a conflict.

### 5. Configuration Extension

**Problem (Review Issue #2):** The original spec's file list was incomplete — a new daemon config key requires updates across 7+ files.

**Full config wiring**, following the established pattern for `daemon_refinement_backend`:

**a) `src/config/global.rs` — WorkspaceConfig:**
```rust
#[serde(default = "default_daemon_rebase_agent_backend")]
pub daemon_rebase_agent_backend: String,
```
Default function: `fn default_daemon_rebase_agent_backend() -> String { "claude(opus)".to_owned() }`

**b) `src/config/project.rs` — ProjectDaemonOverrides:**
```rust
pub rebase_agent_backend: Option<String>,
```

**c) `src/config/mod.rs` — EffectiveDaemonConfig:**
```rust
pub rebase_agent_backend: String,
```
And in `resolve_daemon_config()`:
```rust
rebase_agent_backend: daemon_overrides
    .and_then(|cfg| cfg.rebase_agent_backend.clone())
    .unwrap_or_else(|| global.workspace.daemon_rebase_agent_backend.clone()),
```

**d) `src/daemon/runtime.rs` — DaemonRuntimeConfig:**
```rust
/// Backend spec for rebase conflict resolution agent ("none" to disable).
pub rebase_agent_backend: String,
```

**e) `src/cli/daemon.rs` — runtime config construction:**
```rust
rebase_agent_backend: daemon_cfg.rebase_agent_backend,
```

**f) `src/cli/config.rs` — get/set/show mappings:**
- Global scope: `"workspace.daemon_rebase_agent_backend"` ↔ `workspace.daemon_rebase_agent_backend`
- Project scope: `"daemon.rebase_agent_backend"` ↔ `daemon.rebase_agent_backend`
- Show display: include `rebase_agent_backend` in the daemon section JSON

### 6. Disable Sentinel Handling

**Problem (Review Issue #3):** `"none"` is not a valid backend spec per `parse_backend_spec` / `validate_backend_spec_name`, which only accepts `"claude"` and `"codex"`. The spec must define explicit bypass logic.

**Solution:** The `"none"` sentinel is handled at two levels:

**a) Validation bypass:** Add a `validate_rebase_agent_backend` function in `src/cli/backend_spec.rs`:
```rust
pub fn validate_rebase_agent_backend(spec: &str) -> Result<()> {
    if spec == "none" {
        return Ok(());  // "none" is a valid sentinel meaning "disabled"
    }
    validate_backend_spec_name(spec)
}
```
This is called from the config set paths in `src/cli/config.rs` when the user runs `ralph config set workspace.daemon_rebase_agent_backend <value>` or `ralph config set daemon.rebase_agent_backend <value>`.

**b) Runtime bypass:** In `resolve_rebase_conflicts`, check early:
```rust
if agent_backend == "none" {
    return Err(RalphError::Orchestration("rebase agent disabled".into()));
}
```

**c) Gating in `auto_rebase_phase`:** The `Conflicting` merge-status check (Section 1 above) checks `config.rebase_agent_backend == "none"` to decide whether to skip or proceed.

### 7. Integration into `execute_rebase`

Restructure `execute_rebase` to accept the agent backend and global config:

```rust
fn execute_rebase(
    worktree_path: &Path,
    rebase_target: &str,
    branch: &str,
    timeout: Duration,
    rebase_agent_backend: &str,
    global_config: &GlobalConfig,
) -> Result<()>
```

Flow:
1. Fetch (unchanged)
2. Rebase — on failure:
   a. Check `is_rebase_in_progress()` AND `has_conflicts()` → conflict path
   b. Otherwise → abort and return error (existing path)
3. If conflicts and agent not `"none"`: call `resolve_rebase_conflicts()`
4. If agent succeeds: proceed to push
5. If agent fails or disabled: abort rebase (`git rebase --abort`), return error (existing path)
6. Push with `--force-with-lease` (unchanged)

The abort is **not** performed before calling `resolve_rebase_conflicts` — the rebase must remain in-progress for the agent to resolve conflicts and then `git rebase --continue`.

### 8. Prompt Construction

The agent prompt includes:
- The list of conflicting files (from `git::conflicting_files_z()`)
- The rebase target branch name for context
- Instructions to read each conflicting file, resolve conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`), and `git add` each resolved file
- Explicit instruction to NOT run `git rebase --continue` (the caller handles that)
- The current working directory context (worktree path)

### 9. Error Handling

- `"none"` backend → skip agent, abort rebase, return error
- Agent process spawn failure → abort rebase, return error
- Agent timeout (deadline exceeded) → kill agent process group, abort rebase, return error
- Agent exits non-zero → abort rebase, return error
- Agent exits 0 but conflicts remain → abort rebase, return error with specific message
- `git rebase --continue` non-conflict failure (after max retries or unrecoverable) → abort rebase, return error
- Resolution loop exceeds 10 iterations → abort rebase, return error
- All error paths fall through to the existing failure-comment posting in `auto_rebase_phase`

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/rebase_agent.rs` | **New** — agent backend resolution, process spawner with `setsid()`/group-kill, prompt construction, resolution loop with `--continue`/`--skip` handling |
| `src/daemon/mod.rs` | Add `pub mod rebase_agent;` |
| `src/daemon/runtime.rs` | Modify `execute_rebase` signature to accept agent backend + global config; add conflict detection via `is_rebase_in_progress()` + `has_conflicts()`; call agent on conflict; thread `rebase_agent_backend` through `DaemonRuntimeConfig`; relax `Conflicting` gate in `auto_rebase_phase` |
| `src/config/global.rs` | Add `daemon_rebase_agent_backend` field to `WorkspaceConfig` with `default_daemon_rebase_agent_backend()` function |
| `src/config/project.rs` | Add `rebase_agent_backend: Option<String>` to `ProjectDaemonOverrides` |
| `src/config/mod.rs` | Add `rebase_agent_backend` to `EffectiveDaemonConfig` struct and `resolve_daemon_config()` merge logic |
| `src/cli/daemon.rs` | Thread `rebase_agent_backend` from effective config into `DaemonRuntimeConfig` |
| `src/cli/config.rs` | Add get/set/show mappings for `workspace.daemon_rebase_agent_backend` (global) and `daemon.rebase_agent_backend` (project) |
| `src/cli/backend_spec.rs` | Add `validate_rebase_agent_backend()` that accepts `"none"` sentinel before delegating to `validate_backend_spec_name()` |
| `src/git/mod.rs` | Add `is_rebase_in_progress()` (checks `.git/rebase-merge` and `.git/rebase-apply`); add `conflicting_files_z()` using NUL-delimited parsing |
| `src/validate/tests_daemon.rs` | Add conformance tests: default config value, `"none"` disable path, `Conflicting` merge-status gating, agent failure fallback, multi-conflict loop, timeout fallback |

## Testing Strategy

### Unit Tests (in `src/daemon/rebase_agent.rs`)
- **Prompt construction**: Verify generated prompt includes file list, target branch, and correct instructions (no `git rebase --continue`)
- **`"none"` disables agent**: `resolve_rebase_conflicts` with `"none"` returns error immediately
- **Timeout budget accounting**: Verify remaining budget is correctly computed from deadline and passed to agent spawner
- **Backend resolution**: Verify `create_agent_backend("claude(opus)", &config)` resolves to correct command/args from global config; verify unknown backend names are rejected; verify `"none"` is rejected at backend creation level

### Unit Tests (in `src/git/mod.rs`)
- **`is_rebase_in_progress`**: Create temp repo, start rebase that conflicts, verify returns true; abort rebase, verify returns false
- **`conflicting_files_z`**: Test with normal paths, paths containing spaces, quoted paths, and rename entries (if feasible with synthetic git state)

### Unit Tests (in `src/cli/backend_spec.rs`)
- **`validate_rebase_agent_backend`**: Accepts `"none"`, accepts `"claude(opus)"`, accepts `"codex"`, rejects `"gemini"`, rejects empty string

### Conformance Tests (in `src/validate/tests_daemon.rs`)

Following the project's established conformance test patterns (`RalphHarness::new_daemon`, `run_case`, mock gh scripts):

- **Default config value**: `ralph config get workspace.daemon_rebase_agent_backend` returns `"claude(opus)"`
- **Config set/get round-trip**: Set to `"none"`, verify get returns `"none"`; set to `"claude(sonnet)"`, verify get returns `"claude(sonnet)"`
- **Project override**: Set project-level `daemon.rebase_agent_backend`, verify it takes precedence over global
- **`"none"` disable path**: With agent set to `"none"` and a mock gh returning `CONFLICTING`, verify the PR is skipped (existing behavior preserved)
- **`Conflicting` gate proceeds when agent enabled**: With agent set to `"claude(opus)"` and a mock gh returning `CONFLICTING`, verify `execute_rebase` is attempted (not skipped at the gate)
- **Agent failure fallback**: With a mock agent script that exits non-zero, verify the rebase is aborted and the failure-comment path is taken
- **Agent timeout fallback**: With a mock agent that sleeps, verify timeout triggers abort and failure comment
- **Full resolution with mock agent**: Using `setup_remote_clone()` pattern, create a temp repo with synthetic conflicts, use a mock agent script that resolves conflicts by writing fixed content and running `git add`, verify rebase completes and push succeeds
- **Multi-commit rebase**: Create a branch with 2+ conflicting commits, verify the resolution loop handles `git rebase --continue` cycling through each commit via the mock agent
- **Empty commit skip**: Create a scenario where agent resolution produces a no-op commit, verify `git rebase --skip` is used and rebase proceeds

### Existing Test Compatibility
- Run the full `cargo test` suite to verify no regressions to existing rebase, daemon, and config tests
- Existing conformance tests in `src/validate/tests_daemon.rs` must pass without modification — the `Conflicting` gate change is conditional on agent config, and the default (`"claude(opus)"`) changes behavior only when the PR is actually conflicting and rebase is attempted

## Out of Scope

- **Interactive conflict resolution**: This spec covers only automated/unattended resolution within the daemon. No user-facing UI for conflict review
- **Non-daemon rebase**: The `git::branch::sync_project_branch()` path used by `ralph run` is not modified; this feature is daemon-only
- **Codex prompt tuning**: While the config accepts any backend spec and the backend is resolved through config machinery, prompt construction and testing target Claude only. Codex invocation is structurally supported but prompt compatibility is not validated
- **Conflict resolution quality metrics**: No tracking of resolution success rate, quality scoring, or agent output auditing
- **Session reuse for rebase agent**: The agent is one-shot per conflict set; no session resume across commits or rebase attempts
- **PR comment enrichment**: Failure comments are not enhanced to distinguish "agent attempted but failed" from "no agent available". This could be a follow-up
- **Rebase strategy configuration**: No support for `git rebase --strategy` or `--strategy-option` flags; uses default merge strategy
- **Activity-based inactivity timeout for agent**: The agent spawner uses an absolute deadline model (shared with fetch/rebase/push budget), not the activity-based watchdog used by `CliBackend::execute_streaming`. An inactivity timeout for the agent could be a follow-up optimization
