Now I have a complete picture. Let me write the revised specification.

---

## Summary

Replace the daemon's implicit `Workspace::discover()` startup with an explicit `--data-dir` flag that designates a standalone directory (outside any git repo) where the daemon manages one or more repositories. Each `--repo owner/repo` gets its own subdirectory at `<data-dir>/owner/repo/`, auto-cloned from GitHub when absent, and each repo carries its own `.ralph/` workspace with independent `daemon/tasks.json`. The daemon spawns a parallel `runtime::run()` tokio task per repo and aggregates status/abort across all repos in the data-dir.

## Acceptance Criteria

1. `ralph daemon start` requires `--data-dir <path>` (mandatory) and at least one `--repo owner/repo` (repeatable). Duplicate `--repo` values are rejected with a clear error before any I/O.
2. Startup aborts with a clear error if `--data-dir` resolves to a path inside a git working tree (checked by walking to the nearest existing ancestor and running `git rev-parse --show-toplevel`).
3. For each `--repo`, the daemon creates `<data-dir>/owner/repo/` (including the intermediate `owner/` directory) if missing, clones it via `gh repo clone`, and then runs `bootstrap::ensure_repo_ready_sync` (idempotent). If the clone fails and the directory is empty or absent, the error is propagated immediately — bootstrap is **not** used as a fallback for a failed clone.
4. `--repo` values are validated against a strict GitHub slug pattern (`^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$`); any component that is `.`, `..`, or contains `/` or path separators beyond the single dividing slash is rejected. This prevents path-traversal attacks when joining to `--data-dir`.
5. Each repo gets its own `TaskStore` rooted at `<data-dir>/owner/repo/.ralph/`.
6. One `runtime::run()` tokio task runs per repo via a `JoinSet`. On success, the daemon waits for all tasks to complete. On error, remaining tasks are aborted (via `JoinSet::abort_all()`) and the first error is propagated.
7. `ralph daemon status --data-dir <path>` scans `<data-dir>/*/*/.ralph/daemon/tasks.json`, prints a combined table with a REPO column.
8. `ralph daemon abort --data-dir <path> <task-id>` scans all repo task stores under `--data-dir` to find the matching task. A bare issue number that matches tasks in multiple repos produces an error listing the ambiguous matches.
9. `--repo` no longer accepts `Option<String>`; the old fallback paths (`daemon_repo` config key, `resolve_repo_from_gh()`) are removed from the daemon start flow. The `workspace.daemon_repo` and project-level `daemon.repo` config keys remain in the config model and `config set`/`config show` paths but are **ignored by `daemon start`**. A deprecation warning is printed to stderr if `daemon.repo` is set in config when `daemon start` runs, informing the user to use `--repo` instead.
10. All existing daemon conformance tests pass after migration to the new harness pattern.
11. New required tests cover: empty-dir clone+bootstrap, git-repo-as-data-dir rejection, multi-repo status aggregation, abort across repos (including ambiguous bare issue number), duplicate `--repo` rejection, and clone failure propagation.

## Technical Approach

### CLI argument changes (`src/cli/daemon.rs`)

**DaemonStartArgs**: Add `--data-dir: PathBuf` (required, `#[arg(long)]`). Change `--repo: Option<String>` to `--repo: Vec<String>` (`#[arg(long = "repo")]`). Validation at the top of `execute_start` rejects empty `--repo` vec with a usage error.

**DaemonCommand::Status**: Change from unit variant to `Status(DaemonStatusArgs)` with a new `DaemonStatusArgs { data_dir: PathBuf }`.

**DaemonAbortArgs**: Add `--data-dir: PathBuf` (required).

**Dispatch in `execute()`**: Update the `Status` match arm from `DaemonCommand::Status => ...` to `DaemonCommand::Status(status_args) => ...`.

### Repo slug validation hardening

Replace current `validate_repo_slug()` with a stricter version:
- Split on exactly one `/`.
- Each component must match `^[a-zA-Z0-9._-]+$` (GitHub's actual allowed characters for owner/repo names).
- Explicitly reject `.` and `..` as either component.
- Error message unchanged: `"invalid repo '{slug}': expected owner/repo"`.

### Duplicate `--repo` rejection

After collecting and validating all `--repo` slugs, normalize them (lowercase, trim) and check for duplicates. If any slug appears more than once, return `Err(RalphError::Validation("duplicate --repo: {slug}"))`.

### Guard: data-dir must not be inside a git repo

New helper `guard_not_git_repo(data_dir: &Path) -> Result<()>`:
- Walk up from `data_dir` to find the nearest existing ancestor (handles the case where `data_dir` doesn't exist yet — e.g., `--data-dir /tmp/new/path` where `/tmp/new/` exists).
- Run `git rev-parse --show-toplevel` with `current_dir` set to that ancestor.
- If it succeeds (exit 0), return `Err(RalphError::Validation("--data-dir must not be inside a git repository"))`.
- If it fails (exit non-zero, or command not found), the guard passes.

### Clone-or-bootstrap helper

New helper `clone_or_bootstrap(owner: &str, repo: &str, repo_dir: &Path) -> Result<()>`:
1. If `repo_dir/.git/` exists → skip clone, go to step 3.
2. Create parent directory: `fs::create_dir_all(repo_dir.parent().unwrap())`. Then run `gh repo clone {owner}/{repo} {repo_dir}`. If clone fails, propagate the error immediately — **do not fall back to bootstrap**. A clone failure means the repo doesn't exist on GitHub or `gh` auth is broken; silently bootstrapping an empty local repo would cause the daemon to run against a wrong/empty codebase.
3. Call `bootstrap::ensure_repo_ready_sync(repo_dir)` unconditionally (idempotent — ensures `.ralph/` workspace, initial commit if HEAD is unborn, etc.).

### execute_start rewrite

```
1. preflight_check_gh()
2. Reject if args.repo is empty
3. Validate + deduplicate all --repo slugs (strict pattern, no duplicates)
4. guard_not_git_repo(&args.data_dir)
5. fs::create_dir_all(&args.data_dir)
6. Deprecation check: for each repo, load config; if daemon.repo is set, eprintln deprecation warning (once)
7. For each --repo slug:
   a. (owner, repo_name) = parse_repo_slug(&slug)
   b. repo_dir = data_dir.join(owner).join(repo_name)
   c. clone_or_bootstrap(owner, repo_name, &repo_dir)
   d. workspace = Workspace::load(repo_dir.join(".ralph"))
   e. daemon_cfg = resolve_daemon_config(&workspace.config, project_config)
   f. store = TaskStore::new(&workspace.root)
   g. ralph_bin = resolve ralph binary (RALPH_DAEMON_BIN env or current_exe)
   h. Build DaemonRuntimeConfig { owner, repo: repo_name, repo_root: repo_dir, ... }
   i. Collect (store, runtime_config) into vec
8. Create tokio::task::JoinSet
9. For each (store, config): spawn runtime::run(store, config) on the JoinSet
10. Loop on join_set.join_next():
    - If a task returns Err: call join_set.abort_all(), return the error
    - If a task returns Ok(Err(e)): call join_set.abort_all(), return Err(e)
    - If all tasks complete Ok(Ok(())): return Ok(())
```

The `resolve_repo_from_gh()` and `resolve_git_root()` helpers are deleted — no longer needed. `effective_daemon_config()` is inlined per-repo (each repo's `.ralph/ralph.toml` may differ).

### execute_status rewrite

Takes `DaemonStatusArgs { data_dir }`. Scan `<data-dir>` two levels deep (`read_dir` on data_dir, then `read_dir` on each child) looking for `.ralph/daemon/tasks.json`. For each found, load via `TaskStore`. Print combined table with existing columns. No `Workspace::discover()` call.

### execute_abort rewrite

Takes modified `DaemonAbortArgs { data_dir, task_id_or_number }`. Same directory scan as status. Collect all tasks across all stores. If `task_id_or_number` is a bare issue number that matches tasks in multiple repos, return an error listing the ambiguous matches (with full task IDs and repo slugs). Otherwise call `abort_task` on the correct store.

### bootstrap.rs visibility

Change `fn ensure_repo_ready_sync` to `pub fn ensure_repo_ready_sync` so `clone_or_bootstrap` (and tests) can call it directly without the async wrapper.

### Harness additions (`src/validate/harness.rs`)

**`new_daemon(bin, owner, repo)`**: Creates `TempDir`. Sets `repo_root = temp_dir.path().join(owner).join(repo)`. Creates `repo_root` with `create_dir_all`, runs `git init` + initial commit inside it (same logic as `new()`). The `temp_dir` root acts as the data-dir (parent of `owner/repo/`). Returns `RalphHarness { temp_dir, repo_root, ralph_bin }`.

**`data_dir(&self) -> &Path`**: Returns `self.temp_dir.path()` — the parent directory above `owner/repo/`.

**`data_dir_str(&self) -> String`**: Convenience: `self.temp_dir.path().to_str().unwrap().to_owned()`.

**`daemon_env(args, env_vars)`**: Like `ralph_env` but sets `current_dir` to `self.temp_dir.path()` (data-dir) instead of `repo_root`. This is important because `--data-dir` is an explicit flag, not cwd-dependent, but some tests may rely on cwd for `gh` mock resolution.

### Test migration (`src/validate/tests_daemon.rs`)

**Migration pattern**: Because the test runner always constructs `RalphHarness::new(&self.ralph_bin)` and passes it to every test function, daemon tests cannot change the runner-provided harness. Instead, each daemon test that needs the new data-dir layout will construct its own `RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets")` inside the test body (the same pattern already used by `daemon_bootstrap_zero_commit_repo` which calls `RalphHarness::new_zero_commit_repo(&h.ralph_bin)`). The runner-provided `h` is used solely to obtain `ralph_bin`.

Every daemon test that invokes `daemon start`, `daemon status`, or `daemon abort` will be updated to:
1. `let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets")?;`
2. Replace `h.ralph_env(["daemon", "start", ...], ...)` with `dh.daemon_env(["daemon", "start", "--data-dir", &dh.data_dir_str(), "--repo", "acme/widgets", ...], ...)`.
3. Same pattern for status (`--data-dir`) and abort (`--data-dir`) invocations.
4. `write_tasks` and other helpers that write to `.ralph/daemon/tasks.json` continue to work unchanged since `dh.repo_root` still points to `data_dir/acme/widgets/`.

### Parse test updates (`src/cli/mod.rs`)

The clap `Cli::parse_from()` tests in `src/cli/mod.rs` that verify `daemon start` and `daemon abort` argument parsing must be updated to include `--data-dir` in the argument list. The `DaemonCommand::Status` match arms must change from unit variant to struct destructuring. Specifically:
- `daemon start --repo acme/widgets ...` becomes `daemon start --data-dir /tmp/test --repo acme/widgets ...`
- `daemon abort <id>` becomes `daemon abort --data-dir /tmp/test <id>`
- Any assertion on `DaemonCommand::Status` changes from unit variant match to struct match.

### Config compatibility

The `workspace.daemon_repo` key in `GlobalConfig` and the project-level `daemon.repo` key in `ProjectDaemonConfig` remain in the config model. The `config set` and `config show` commands continue to read/write these keys. However, `execute_start()` no longer reads `daemon_cfg.repo` for repo resolution. Instead, if a loaded config has `repo` set, a one-time deprecation warning is emitted to stderr:
```
warning: daemon.repo config key is ignored by `daemon start`; use --repo flag instead
```
This avoids a breaking change in config parsing while making the behavioral change explicit.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/daemon.rs` | Add `--data-dir` to start/status/abort args. Change `--repo` to `Vec<String>`. Add `DaemonStatusArgs` struct. Rewrite `execute_start` with guard, strict slug validation, duplicate rejection, clone-or-bootstrap per-repo loop, `JoinSet` spawn, and abort-all-on-error semantics. Rewrite `execute_status` and `execute_abort` for directory scanning (with ambiguous bare-issue-number error). Add `guard_not_git_repo()` and `clone_or_bootstrap()`. Harden `validate_repo_slug()` to reject `.`/`..` and non-GitHub characters. Remove `resolve_repo_from_gh()`, `resolve_git_root()`. Inline `effective_daemon_config()` per-repo. Add deprecation warning for `daemon.repo` config key. |
| `src/daemon/bootstrap.rs` | Change `fn ensure_repo_ready_sync` to `pub fn ensure_repo_ready_sync`. |
| `src/validate/harness.rs` | Add `new_daemon(bin, owner, repo)` constructor, `data_dir()` accessor, `data_dir_str()` convenience method, `daemon_env(args, env_vars)` method. |
| `src/validate/tests_daemon.rs` | Migrate all daemon tests to construct `new_daemon` harness inside test body + use `daemon_env` with `--data-dir`/`--repo` flags. Add 6 new required tests (see Testing Strategy). |
| `src/cli/mod.rs` | Update clap parse tests for `daemon start` (add `--data-dir`), `daemon abort` (add `--data-dir`), and `daemon status` (struct variant match). |
| `src/daemon/mod.rs` | No changes — `DaemonTask` already has `owner`/`repo` fields; `TaskStore` is instantiated per-repo. |
| `src/daemon/runtime.rs` | No changes — `run()` signature and behavior unchanged; called once per repo. |
| `src/config/mod.rs` | No changes — `daemon_repo` / `daemon.repo` keys remain; only daemon start stops reading them. |
| `src/config/global.rs` | No changes — `daemon_repo` field retained for backwards compatibility. |

## Testing Strategy

### Migrated tests (mechanical update, same assertions)

All existing daemon conformance tests switch to the in-body `new_daemon("acme", "widgets")` harness pattern and pass `--data-dir` / `--repo` flags. The runner-provided `h` is used only for `ralph_bin`. No behavioral changes expected — tests exercise the same runtime paths through the new CLI surface. The `write_tasks` helper writes to `dh.repo_root/.ralph/daemon/tasks.json` which is `<data-dir>/acme/widgets/.ralph/daemon/tasks.json`, consistent with the new layout.

### New required conformance tests

1. **`daemon_start_bootstraps_empty_dir`**: Create empty data-dir via `TempDir`. Run `daemon start --data-dir <dir> --repo acme/widgets --single-iteration` with a mock `gh` that simulates clone (creates `<data-dir>/acme/widgets/`, runs `git init` + commit inside it). Assert: repo dir exists, `.ralph/` workspace initialized, `tasks.json` written, daemon completes exit 0.

2. **`daemon_start_rejects_git_data_dir`**: Create data-dir *inside* an existing git repo (use the runner-provided `h.repo_root` as the git repo, create a subdir). Run `daemon start --data-dir <git-repo-subdir> --repo acme/widgets`. Assert: non-zero exit, stderr contains "must not be inside a git repository".

3. **`daemon_status_multi_repo`**: Pre-populate `tasks.json` for two repos under a data-dir (`acme/widgets` and `acme/gadgets`). Run `daemon status --data-dir <dir>`. Assert: both repos' tasks appear in output, each with correct REPO column.

4. **`daemon_abort_cross_repo`**: Pre-populate tasks for two repos under a data-dir with different issue numbers. Run `daemon abort --data-dir <dir> <task-id>` using a full task ID from one repo. Assert: correct task aborted in correct repo's store.

5. **`daemon_abort_ambiguous_bare_number`**: Pre-populate tasks in two repos with the same bare issue number (e.g., issue 42 in both `acme/widgets` and `acme/gadgets`). Run `daemon abort --data-dir <dir> 42`. Assert: non-zero exit, stderr contains both repo names listing the ambiguous matches.

6. **`daemon_start_rejects_duplicate_repo`**: Run `daemon start --data-dir <dir> --repo acme/widgets --repo acme/widgets`. Assert: non-zero exit, stderr contains "duplicate --repo".

7. **`daemon_start_clone_failure_propagates`**: Run `daemon start --data-dir <dir> --repo acme/nonexistent --single-iteration` with a mock `gh` that exits non-zero for clone. Assert: non-zero exit, stderr contains clone failure message. Assert: no `.ralph/` directory created (bootstrap did not silently run).

### Parse tests (`src/cli/mod.rs`)

Update existing `Cli::parse_from` tests:
- `daemon start` test: add `--data-dir /tmp/test`, verify `data_dir` field parsed.
- `daemon abort` test: add `--data-dir /tmp/test`, verify `data_dir` field parsed.
- Add assertion that `DaemonCommand::Status(args)` now carries `data_dir`.

### Verification commands

- `nix develop -c cargo check` — compilation
- `nix build` — full unit + conformance test suite

## Out of Scope

- Multi-repo concurrency limits (global `--max-concurrent` across repos) — each repo gets its own independent concurrency limit from its own config.
- Hot-reloading repo list while daemon is running — restart required to add/remove repos.
- Daemon config merging across repos (each repo uses its own `.ralph/ralph.toml`).
- Removing `workspace.daemon_repo` / `daemon.repo` config keys from the config model or `config set`/`config show` — keys are retained but ignored by `daemon start`, with a deprecation warning.
- Removing the legacy `resolve_repo_from_gh()` fallback from non-daemon commands — only the daemon start path changes.
- Persistent daemon process management (pidfile, systemd integration, `daemon stop` command).
- Shared cross-repo task deduplication (same issue in multiple repos is treated independently).
- Migration path from old single-repo daemon invocations — this is a breaking CLI change for `daemon start`; `daemon status` and `daemon abort` also now require `--data-dir`.