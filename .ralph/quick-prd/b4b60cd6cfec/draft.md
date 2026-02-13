I now have a thorough understanding of the codebase. Let me write the spec.

## Summary

A `ralph daemon` command that runs as a long-lived process, polling GitHub repositories for issues matching configurable label filters. When a matching issue is found, the daemon creates a ralph project from the issue body, runs the orchestration pipeline in a managed subprocess, and reports results back to the issue via comments, labels, and pull requests. Task state is persisted to disk for crash recovery.

## Acceptance Criteria

1. **`ralph daemon start`** launches a foreground process that polls GitHub for issues at a configurable interval using `gh issue list`. It writes a PID file to `.ralph/daemon/daemon.pid` and refuses to start if another daemon is already running (PID file exists and process is alive).

2. **`ralph daemon status`** reads `.ralph/daemon/tasks.json` and prints a table of tracked tasks with columns: issue number, repo, ralph project ID, status (`pending | in_progress | completed | failed | aborted`), and elapsed time.

3. **`ralph daemon abort <issue-number>`** kills the subprocess for the given issue, sets its task status to `aborted`, comments on the GitHub issue with an abort notice, and applies the `ralph:aborted` label.

4. **Configuration** is read from a new `[daemon]` section in `ralph.toml`:
   ```toml
   [daemon]
   repos = ["owner/repo"]
   labels = ["ralph"]
   poll_interval_seconds = 60
   max_concurrent = 1
   ```
   All fields have defaults (`repos = []`, `labels = ["ralph"]`, `poll_interval_seconds = 60`, `max_concurrent = 1`). The daemon refuses to start if `repos` is empty.

5. **Issue lifecycle**: When a new matching issue is detected:
   - Daemon comments: `> Ralph is working on this...` and adds `ralph:in-progress` label.
   - Creates a ralph project via `create_project()` with `id = "github-<issue-number>"` and `PromptSource::File` pointing to a temp file containing the issue body.
   - Spawns `Orchestrator::run()` in a `tokio::spawn` task with `until_complete: true`.
   - On success: comments with a summary, adds `ralph:completed` label, removes `ralph:in-progress`, and runs `gh pr create` referencing the issue.
   - On failure: comments with error details and adds `ralph:failed` label.

6. **Persistence**: `.ralph/daemon/tasks.json` is an array of task records. On daemon startup, any `in_progress` tasks are checked: if the PID is dead, the task is restarted. Pending tasks are re-enqueued.

7. **Concurrency**: At most `max_concurrent` orchestration tasks run simultaneously. Additional issues are queued as `pending` and started in FIFO order as slots open.

8. **Signal handling**: On SIGINT/SIGTERM, the daemon stops polling, waits for in-progress tasks to checkpoint (up to 30s), updates their status to `pending` in tasks.json (so they resume on next start), and exits cleanly.

9. **Existing configs without `[daemon]`** continue to deserialize without error (serde `default`).

## Technical Approach

### CLI layer (`src/cli/daemon.rs`)

Add a `Daemon(DaemonArgs)` variant to the `Commands` enum in `src/cli/mod.rs`. `DaemonArgs` has a subcommand enum:

```rust
#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    Start,
    Status,
    Abort(DaemonAbortArgs),
}
```

`start` calls `daemon::run_daemon()`. `status` and `abort` are synchronous reads/writes against `tasks.json` and process signals — they do not need the daemon to be running (they operate on the persisted file directly, though `abort` also sends SIGTERM to the subprocess).

### Config layer (`src/config/global.rs`)

Add `DaemonConfig` struct and a `#[serde(default)]` field on `GlobalConfig`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub repos: Vec<String>,           // e.g. ["owner/repo"]
    pub labels: Vec<String>,          // issue labels to match
    pub poll_interval_seconds: u64,   // default 60
    pub max_concurrent: usize,        // default 1
}
```

### Daemon core (`src/daemon/mod.rs`, new module)

**`DaemonState`** — in-memory representation of `tasks.json`:

```rust
struct DaemonTask {
    issue_number: u64,
    repo: String,
    project_id: String,
    status: TaskStatus,        // Pending, InProgress, Completed, Failed, Aborted
    pid: Option<u32>,          // OS PID of the tokio task (not used for kill — use JoinHandle)
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    error: Option<String>,
}
```

State is loaded on startup, saved atomically (write-to-temp + rename) after every mutation.

**Poll loop** (`run_daemon`):
1. Load workspace and config.
2. Write PID file. Check for existing daemon via PID liveness.
3. Load or create `tasks.json`.
4. Resume any `in_progress` tasks whose subprocess died.
5. Enter `loop` with `tokio::time::interval(poll_interval)`:
   - For each repo, run `gh issue list --repo <repo> --label <labels> --json number,title,body,labels --state open`.
   - Parse JSON output. Filter out issues already tracked in `tasks.json` or already bearing `ralph:in-progress`/`ralph:completed`/`ralph:failed` labels.
   - For new issues: create a `DaemonTask` with status `Pending`, save state.
   - Start pending tasks up to `max_concurrent` capacity.
6. On SIGINT/SIGTERM (via `tokio::signal`), break out of loop, run shutdown sequence.

**Task execution** (spawned per task):
1. Comment on issue + add `ralph:in-progress` label via `gh`.
2. Write issue body to a temp file.
3. Call `create_project()` with the workspace.
4. Create `Orchestrator` and call `.run()` with `until_complete: true`.
5. On `Ok(result)`: comment summary, add `ralph:completed`, remove `ralph:in-progress`, run `gh pr create --title "Fix #N: <title>" --body "<summary>" --repo <repo>`.
6. On `Err(e)`: comment error, add `ralph:failed`, remove `ralph:in-progress`.
7. Update `DaemonTask` status and save state.

Each spawned task communicates completion back to the main loop via a `tokio::sync::mpsc` channel so the main loop can update concurrency counts and start queued tasks.

### GitHub interaction

All GitHub interaction goes through `gh` CLI (already a project dependency pattern — backends use CLI tools). This avoids adding an HTTP client dependency. Commands:

- `gh issue list --repo R --label L --json number,title,body,labels --state open`
- `gh issue comment N --repo R --body "..."`
- `gh issue edit N --repo R --add-label "ralph:in-progress"`
- `gh issue edit N --repo R --remove-label "ralph:in-progress" --add-label "ralph:completed"`
- `gh pr create --repo R --title "..." --body "..." --head <branch>`

These are wrapped in a small `GhCli` helper struct in `src/daemon/gh.rs` that shells out via `tokio::process::Command`, parses JSON output, and returns typed results. Errors from `gh` are mapped to a new `RalphError::DaemonGhError { details: String }` variant.

### PID file management

Write `std::process::id()` to `.ralph/daemon/daemon.pid` on start. On `daemon start`, if the file exists, read the PID and check `/proc/<pid>/` (Linux) or `kill(pid, 0)` (portable). If alive, exit with error "daemon already running (PID N)". Remove PID file on clean shutdown.

### New dependencies

- None required. `tokio` (already `features = ["full"]`) provides `signal`, `process`, `sync`, `time`. `serde_json` handles tasks.json. `chrono` handles timestamps.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/mod.rs` | Add `Daemon(DaemonArgs)` to `Commands` enum, add `mod daemon`, add dispatch arm |
| `src/cli/daemon.rs` | **New.** `DaemonArgs`, `DaemonCommand` enum, `execute()` routing to `start`/`status`/`abort` |
| `src/daemon/mod.rs` | **New.** `DaemonTask`, `TaskStatus`, `DaemonState` (load/save tasks.json), `run_daemon()` main loop |
| `src/daemon/gh.rs` | **New.** `GhCli` struct wrapping `gh` CLI calls: `list_issues()`, `comment()`, `add_label()`, `remove_label()`, `create_pr()` |
| `src/daemon/task_runner.rs` | **New.** `spawn_task()` — creates project, runs orchestrator, updates GitHub issue. Returns result via mpsc channel |
| `src/config/global.rs` | Add `DaemonConfig` struct, add `#[serde(default)] pub daemon: DaemonConfig` field to `GlobalConfig`, add `Default` impl |
| `src/error.rs` | Add `DaemonGhError { details: String }` and `DaemonError(String)` variants to `RalphError` |
| `src/main.rs` | Add `mod daemon` |
| `src/lib.rs` (or wherever modules are declared) | Add `pub mod daemon` |

## Testing Strategy

1. **Unit tests for `DaemonConfig` deserialization** (`src/config/global.rs`): Verify that existing configs without `[daemon]` still parse. Verify configs with `[daemon]` parse all fields. Verify defaults.

2. **Unit tests for `DaemonState` persistence** (`src/daemon/mod.rs`): Create `DaemonState`, add/update tasks, save to temp file, reload, assert equality. Test atomic write (content matches after reload). Test that loading a missing file returns empty state.

3. **Unit tests for `GhCli` output parsing** (`src/daemon/gh.rs`): Mock JSON output from `gh issue list` and verify `list_issues()` returns correctly typed structs. Test malformed output returns error.

4. **Unit tests for task lifecycle state transitions** (`src/daemon/mod.rs`): Verify `Pending → InProgress → Completed`, `Pending → InProgress → Failed`, `InProgress → Aborted` transitions. Verify that `max_concurrent` is respected (cannot start more than N tasks).

5. **Integration test for `daemon status`/`abort`** (`tests/`): Create a tasks.json fixture, run `ralph daemon status`, verify tabular output. Run `ralph daemon abort <N>` on a non-running task, verify status update.

6. **CLI parse tests** (`src/cli/mod.rs`): Verify `ralph daemon start`, `ralph daemon status`, `ralph daemon abort 42` parse correctly.

7. **No live GitHub tests**: All `gh` interactions are behind the `GhCli` struct. Integration tests use a `MockGhCli` trait object or skip the `gh` calls by checking `--dry-run` (add `--dry-run` flag to `daemon start` for testing the poll loop without GitHub side effects).

## Out of Scope

- **Webhooks / push-based GitHub events**: This spec uses polling only. A webhook server could replace or supplement polling in a future iteration.
- **Multi-workspace support**: The daemon operates on a single workspace (the one discovered by `Workspace::discover()`).
- **Web UI or HTTP API**: The daemon is CLI-only. No REST endpoints.
- **Authentication or multi-user access control**: Relies on `gh auth` being already configured.
- **PR review / merge automation**: The daemon creates PRs but does not auto-merge them.
- **Rate limiting / GitHub API quota management**: The `gh` CLI handles token-based rate limiting internally. No additional backoff logic.
- **Daemonization (background fork)**: `ralph daemon start` runs in the foreground. Users can use `nohup`, `systemd`, `tmux`, etc. to background it.
- **Log rotation or structured log output to file**: Uses existing `tracing` infrastructure; log file management is left to the operator.
- **Watching for issue edits / re-runs**: Once an issue is picked up, edits to the issue body are ignored. A new issue or manual re-trigger would be needed.