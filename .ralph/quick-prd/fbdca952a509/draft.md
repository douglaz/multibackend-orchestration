Now I have a thorough understanding of the codebase. Let me write the specification.

---

## Summary

Replace the daemon's implicit `Workspace::discover()` startup with an explicit `--data-dir` flag that designates a standalone directory (outside any git repo) where the daemon manages one or more repositories. Each `--repo owner/repo` gets its own subdirectory at `<data-dir>/owner/repo/`, auto-cloned from GitHub when absent, and each repo carries its own `.ralph/` workspace with independent `daemon/tasks.json`. The daemon spawns a parallel `runtime::run()` tokio task per repo and aggregates status/abort across all repos in the data-dir.

## Acceptance Criteria

1. `ralph daemon start` requires `--data-dir <path>` (mandatory) and at least one `--repo owner/repo` (repeatable).
2. Startup aborts with a clear error if `--data-dir` resolves to a path inside a git working tree.
3. For each `--repo`, the daemon creates `<data-dir>/owner/repo/` if missing, clones it via `gh repo clone`, and runs `bootstrap::ensure_repo_ready` (idempotent).
4. Each repo gets its own `TaskStore` rooted at `<data-dir>/owner/repo/.ralph/`.
5. One `runtime::run()` tokio task runs per repo; the daemon waits on all and propagates the first error.
6. `ralph daemon status --data-dir <path>` scans `<data-dir>/*/*/.ralph/daemon/tasks.json`, prints a combined table with a REPO column.
7. `ralph daemon abort --data-dir <path> <task-id>` scans all repo task stores under `--data-dir` to find the matching task.
8. `--repo` no longer accepts `Option<String>`; the old fallback paths (`daemon_repo` config key, `gh repo view`) are removed from the start flow.
9. All existing daemon conformance tests pass after migration to the new harness methods.
10. New tests cover: empty-dir clone+bootstrap, and git-repo-as-data-dir rejection.

## Technical Approach

### CLI argument changes (`src/cli/daemon.rs`)

**DaemonStartArgs**: Add `--data-dir: PathBuf` (required, `#[arg(long)]`). Change `--repo: Option<String>` to `--repo: Vec<String>` (`#[arg(long = "repo")]`, at minimum one required — validated at the top of `execute_start`).

**DaemonCommand::Status**: Change from unit variant to `Status(DaemonStatusArgs)` with a new `DaemonStatusArgs` struct containing `--data-dir: PathBuf`.

**DaemonAbortArgs**: Add `--data-dir: PathBuf`.

### Guard: data-dir must not be inside a git repo

New helper `guard_not_git_repo(data_dir: &Path) -> Result<()>`:
- If `data_dir` doesn't exist yet, walk parents until one exists and check that ancestor.
- Run `git rev-parse --show-toplevel` with `current_dir` set to the nearest existing ancestor.
- If it succeeds (exit 0), return `Err(RalphError::Validation("--data-dir must not be inside a git repository"))`.

### Clone-or-bootstrap helper

New helper `clone_or_bootstrap(owner: &str, repo: &str, repo_dir: &Path) -> Result<()>`:
1. If `repo_dir/.git/` exists → skip clone (already set up).
2. Otherwise, run `gh repo clone owner/repo <repo_dir>`. If clone fails and `repo_dir` is empty, propagate the error (we need the real repo). If clone succeeds, proceed.
3. Call `bootstrap::ensure_repo_ready_sync(repo_dir)` unconditionally (idempotent — ensures `.ralph/` workspace exists).

### execute_start rewrite

```
1. preflight_check_gh()
2. guard_not_git_repo(&data_dir)
3. fs::create_dir_all(&data_dir)
4. For each --repo slug:
   a. validate_repo_slug(&slug)
   b. (owner, repo_name) = parse_repo_slug(&slug)
   c. repo_dir = data_dir.join(owner).join(repo_name)
   d. clone_or_bootstrap(owner, repo_name, &repo_dir)
   e. workspace = Workspace::load(repo_dir.join(".ralph"))
   f. store = TaskStore::new(&workspace.root)
   g. Build DaemonRuntimeConfig (repo_root = repo_dir)
   h. Collect (store, runtime_config) into vec
5. Spawn tokio task per (store, config) calling runtime::run()
6. tokio::select! on JoinSet — first error propagated, all tasks cancelled
```

The ralph_bin resolution and daemon config loading remain per-repo (each repo's `.ralph/ralph.toml` may differ). The `resolve_repo_from_gh()` and `resolve_git_root()` helpers are deleted — no longer needed.

### execute_status rewrite

Scan `<data-dir>/*/*/.ralph/daemon/tasks.json` using `fs::read_dir` two levels deep. For each found `tasks.json`, load via `TaskStore`. Print combined table. No `Workspace::discover()` call.

### execute_abort rewrite

Same directory scan. Collect all tasks across all stores. Use `resolve_task_index` against the combined list. Call `abort_task` on the correct store.

### bootstrap.rs visibility

Change `fn ensure_repo_ready_sync` to `pub fn ensure_repo_ready_sync` so `clone_or_bootstrap` (and tests) can call it directly without the async wrapper.

### Harness additions (`src/validate/harness.rs`)

**`new_daemon(bin, owner, repo)`**: Creates `TempDir` with repo at `temp_dir/owner/repo/` (git init + initial commit), so `temp_dir` itself acts as the data-dir. Returns `RalphHarness` with `repo_root = temp_dir/owner/repo/`.

**`data_dir(&self) -> &Path`**: Returns `self.temp_dir.path()` — the parent directory above `owner/repo/`.

**`daemon_env(args, env_vars)`**: Like `ralph_env` but sets `current_dir` to `self.temp_dir.path()` (data-dir) instead of `repo_root`.

### Test migration (`src/validate/tests_daemon.rs`)

Every test that calls `RalphHarness::new(bin)` for daemon purposes switches to `RalphHarness::new_daemon(bin, "acme", "widgets")`. Every `h.ralph_env(["daemon", "start", ...], ...)` becomes `h.daemon_env(["daemon", "start", "--data-dir", h.data_dir().to_str().unwrap(), "--repo", "acme/widgets", ...], ...)`. Same pattern for status and abort invocations. The `write_tasks` helper writes to the per-repo `.ralph/daemon/tasks.json` path (unchanged since `repo_root` still points to `data_dir/acme/widgets/`).

## Files & Modules

| File | Change |
|---|---|
| `src/cli/daemon.rs` | Add `--data-dir` to start/status/abort args. Change `--repo` to `Vec<String>`. Rewrite `execute_start` with guard, multi-repo loop, and `JoinSet`. Rewrite `execute_status` and `execute_abort` for directory scanning. Add `guard_not_git_repo()` and `clone_or_bootstrap()`. Remove `resolve_repo_from_gh()`, `resolve_git_root()`, `effective_daemon_config()` (config loaded per-repo inline). |
| `src/daemon/bootstrap.rs` | Change `fn ensure_repo_ready_sync` to `pub fn ensure_repo_ready_sync`. |
| `src/validate/harness.rs` | Add `new_daemon()` constructor, `data_dir()` accessor, `daemon_env()` method. |
| `src/validate/tests_daemon.rs` | Migrate all daemon tests to `new_daemon` + `daemon_env` pattern. Add `daemon_start_bootstraps_empty_dir` and `daemon_start_rejects_git_data_dir` tests. |
| `src/daemon/mod.rs` | No changes — `DaemonTask` already has `owner`/`repo` fields; `TaskStore` is instantiated per-repo. |
| `src/daemon/runtime.rs` | No changes — `run()` signature and behavior unchanged; called once per repo. |

## Testing Strategy

**Migrated tests (mechanical update, same assertions)**:
- All existing daemon conformance tests switch to `new_daemon("acme", "widgets")` harness and pass `--data-dir` / `--repo` flags. No behavioral changes expected — tests exercise the same runtime paths through the new CLI surface.

**New conformance tests**:

1. **`daemon_start_bootstraps_empty_dir`**: Create empty data-dir. Run `daemon start --data-dir <dir> --repo acme/widgets --single-iteration` with a mock `gh` that simulates clone (creates repo dir + git init + commit). Assert: repo dir exists at `<data-dir>/acme/widgets/`, `.ralph/` workspace initialized, `tasks.json` written, daemon completes without error.

2. **`daemon_start_rejects_git_data_dir`**: Create data-dir *inside* an existing git repo. Run `daemon start --data-dir <git-repo-subdir> --repo acme/widgets`. Assert: non-zero exit, stderr contains "must not be inside a git repository".

3. **`daemon_status_multi_repo`** (optional, if time permits): Pre-populate tasks for two repos under a data-dir. Run `daemon status --data-dir <dir>`. Assert: both repos' tasks appear in output.

**Verification commands**:
- `nix develop -c cargo check` — compilation
- `nix build` — full unit + conformance test suite

## Out of Scope

- Multi-repo concurrency limits (global `--max-concurrent` across repos) — each repo gets its own independent concurrency limit for now.
- Hot-reloading repo list while daemon is running — restart required to add/remove repos.
- Daemon config merging across repos (each repo uses its own `.ralph/ralph.toml`).
- Removing the legacy `resolve_repo_from_gh()` fallback from non-daemon commands — only the daemon start path changes.
- Persistent daemon process management (pidfile, systemd integration, `daemon stop` command).
- Shared cross-repo task deduplication (same issue in multiple repos is treated independently).
- Migration path from old single-repo daemon invocations — this is a breaking CLI change.