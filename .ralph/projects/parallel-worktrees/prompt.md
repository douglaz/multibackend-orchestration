I now have complete knowledge of the codebase. Let me write the revised specification addressing all 8 review issues.

---

## Summary

Replace the centralized `.ralph/index.json` project registry with a decentralized, directory-scan-based design. Today, `index.json` stores a flat list of all project metadata and a workspace-wide `active_project` field. This creates two problems in multi-worktree environments: (1) concurrent writes to `index.json` from different worktrees corrupt or overwrite each other's changes, and (2) the `active_project` field is workspace-global but semantically per-checkout — changing it in one worktree silently affects all others.

The fix: eliminate `index.json` entirely. Derive the project list by scanning `.ralph/projects/*/state.json`. Store `active_project` in a worktree-local location outside of `.ralph/` (e.g., `.git/ralph-active-project`), or remove it and require explicit `--project` everywhere. Each project's `state.json` already contains most of the metadata currently duplicated in `ProjectRef`; the duplication is the root cause of the sync problem.

**Schema extension:** `ProjectState` does not currently store a top-level `created_at` timestamp. Add a `created_at: DateTime<Utc>` field to `ProjectState` (with `serde(default)` for backward compatibility). At project creation time, `create_project` sets this field. For existing `state.json` files that lack `created_at`, derive it from the earliest `started_at` across all loops, or fall back to the filesystem `mtime` of `state.json`. The `completed_at` timestamp is derivable: a project with `status == Completed` uses the latest `completed_at` across all loops and completion attempts.

**Atomic writes:** To prevent partial-read corruption during concurrent scans, `ProjectState::save` is changed to write to a temporary file in the same directory and then atomically rename over the target. The `tempfile` crate is already a dependency.

## Acceptance Criteria

1. **`index.json` is no longer required or written.** Ralph operates without `.ralph/index.json`. An existing `index.json` is silently ignored (no error if present for backward compatibility).

2. **`ralph project list` derives its output by scanning `.ralph/projects/*/state.json`.** Output columns remain identical. Active-project marker uses the new worktree-local mechanism. Results are sorted by project ID for deterministic output.

3. **`ralph project new` creates the project directory and `state.json` without updating any shared file.** No shared registry file is written. The new `state.json` includes a `created_at` field.

4. **`ralph run`, `ralph status`, `ralph history`, `ralph tail`, `ralph rollback`, and `ralph project show` resolve the project from `--project` flag or from the worktree-local active-project file.** Error message when no project is specified and none is active remains user-friendly.

5. **`ralph project use <id>` validates that the project directory exists (by checking `.ralph/projects/<id>/state.json`), then writes only to the worktree-local active-project file.** It does not write any file inside `.ralph/`.

6. **`ralph config show`, `ralph config get`, `ralph config set`, and `ralph config edit` resolve default project scope from the worktree-local active-project mechanism.** Project existence is validated by checking the project directory, not an index. MCP `config_show` follows the same resolution.

7. **Orchestrator `persist_state_and_index` no longer syncs to a shared index.** It saves `state.json` only. Rename to `persist_state`.

8. **Rollback saves `state.json` only.** No shared index sync.

9. **MCP handlers for `project_list`, `project_show`, `status`, `history`, and `config_show` work against the scan-based project list.** The `"active"` field in MCP JSON output uses the local active-project resolution. Project existence checks use directory presence, not index lookup.

10. **Two concurrent `ralph run --project X` and `ralph run --project Y` in separate worktrees of the same repo do not conflict on any shared file** (project-level `.lock` files remain per-project and already handle this).

11. **Migration:** `ralph init` no longer creates `index.json`. Existing workspaces that have `index.json` continue to work (it is simply not read). On first load after upgrade, if a worktree-local active-project file does not exist but `index.json` does exist and contains `active_project`, seed the local file from `index.json` once and log a message. This is a one-time migration that preserves the user's active project selection.

12. **`ProjectRef` is removed.** All metadata formerly in `ProjectRef` (status, loop counts, etc.) is derived from `ProjectState` on demand via a computed `ProjectSummary` struct.

13. **`ralph run --project <id>` updates the worktree-local active project** (preserving current behavior where explicit `--project` sets the active project). Stale active-project IDs (pointing to a deleted or nonexistent project directory) are treated as "no active project" — the file is not automatically cleared, but commands produce the same `ActiveProjectNotSet`-style error with a hint that the configured project no longer exists.

14. **Corrupt or invalid active-project files** (empty, whitespace-only, containing an invalid project ID format) are treated as "no active project" with a warning on stderr.

15. **`ProjectState::save` uses atomic write** (temp file + rename) to prevent partial reads by concurrent scanners.

16. **`check_parent_project_consistency` validates against the project directory** (checks `state.json` existence) rather than the index.

## Technical Approach

### 1. Add `created_at` to `ProjectState`

Add a `created_at` field to `ProjectState`:

```rust
#[serde(default = "default_created_at")]
pub created_at: DateTime<Utc>,
```

The `default_created_at` function returns `DateTime::<Utc>::MIN_UTC` as a sentinel. When `list_projects()` or `load_project_summary()` encounters this sentinel, it falls back to the earliest `started_at` across all loops, or `Utc::now()` if no loops exist. `ProjectState::new()` sets `created_at` to `Utc::now()`.

The `completed_at` value for `ProjectSummary` is computed: if `status == Completed`, take the latest `completed_at` across all `loops` and `completion_attempts`.

### 2. Make `ProjectState::save` atomic

Replace direct `fs::write` with temp-file-and-rename:

```rust
pub fn save(&self, path: &Path) -> Result<()> {
    let raw = serde_json::to_string_pretty(self)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, raw.as_bytes())?;
    tmp.persist(path)?;
    Ok(())
}
```

This ensures readers never see partial JSON. The `tempfile` crate is already in `Cargo.toml`.

### 3. Remove `WorkspaceIndex` entirely

Remove the `WorkspaceIndex` struct, `ProjectRef`, `ProjectLifecycleStatus`, and all methods. Remove `index.rs`. Remove the `index` field from `Workspace`. Remove `save_index()`.

Add a new `ProjectSummary` struct (computed, never persisted):

```rust
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub status: ProjectStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_feature_loops: u32,
    pub total_completion_attempts: u32,
    pub last_loop_number: u32,
    pub parent_project: Option<String>,
}
```

Derived from `ProjectState`:
- `total_feature_loops` = count of loops with `status == Completed`
- `total_completion_attempts` = count of completion_attempts with `status == Completed`
- `last_loop_number` = max loop_number across loops and completion_attempts
- `completed_at` = latest `completed_at` across all loops/attempts when `status == Completed`

### 4. Directory-scan project listing

Add to `Workspace`:

```rust
pub fn list_projects(&self) -> Result<Vec<ProjectSummary>>
```

Reads `.ralph/projects/`, iterates subdirectories, loads each `state.json`, and returns `ProjectSummary` entries sorted by project ID. Non-directory entries are skipped. Subdirectories missing `state.json` are skipped with a warning on stderr. Malformed `state.json` files are skipped with a warning (the scan does not fail for one bad project).

```rust
pub fn project_exists(&self, id: &str) -> bool
```

Returns `true` if `.ralph/projects/<id>/state.json` exists.

```rust
pub fn load_project_summary(&self, id: &str) -> Result<ProjectSummary>
```

Loads a single project's `state.json` and returns a computed `ProjectSummary`.

### 5. Worktree-local active project

Store the active project ID in a file that is local to the git worktree and not shared across clones/worktrees:

- **Primary location:** `.git/ralph-active-project` (a plain text file containing just the project ID, trimmed). In a worktree, `.git` is a file pointing to the actual gitdir; use `git rev-parse --git-dir` to resolve the actual per-worktree git directory.
- **Fallback (non-git repos):** `.ralph/.active-project-local` and add it to `.ralph/.gitignore` so it's never committed. This handles the case where ralph is used outside a git repository.

Add to `Workspace`:

```rust
pub fn active_project_id(&self) -> Option<String>
pub fn set_active_project_id(&self, id: &str) -> Result<()>
```

`active_project_id()` reads the worktree-local file, trims whitespace. Returns `None` if the file does not exist, is empty, or contains only whitespace. If the content fails `validate_project_id` (invalid characters), logs a warning to stderr and returns `None`.

`set_active_project_id()` validates that the project exists (via `project_exists()`) before writing. Returns `ProjectNotFound` if not.

**Behavior of `run --project`:** When `--project` is explicitly provided, the orchestrator calls `set_active_project_id()` to update the local active project, preserving current behavior.

**Stale IDs:** When `active_project_id()` returns `Some(id)` but the project directory does not exist, callers receive a `ProjectNotFound` error with a hint: `"active project '{id}' no longer exists; use 'ralph project use <id>' to set a new active project"`.

### 6. One-time migration from `index.json`

On `Workspace::load()` (called by `discover()`): after loading `ralph.toml`, if the worktree-local active-project file does not exist, check whether `.ralph/index.json` exists. If it does, attempt to parse it and extract `active_project`. If a value is found and the corresponding project directory exists, write it to the worktree-local file and log: `"migrated active project '{id}' from index.json to worktree-local storage"`. This is best-effort: parse failures or missing projects are silently ignored.

This migration runs only once per worktree because subsequent loads find the local file already present (even if empty after a `project use` cycle).

### 7. Update all consumer sites

Every location that currently reads `workspace.index.active_project` switches to `workspace.active_project_id()`. Every location that reads `workspace.index.projects` or `workspace.index.get_project()` switches to `workspace.project_exists()`, `workspace.load_project_summary()`, or `workspace.list_projects()`.

**Complete call-site migration table:**

| File | Current pattern | New pattern |
|------|----------------|-------------|
| `src/cli/project.rs` List | iterate `workspace.index.projects` | `workspace.list_projects()` |
| `src/cli/project.rs` Use | `workspace.index.set_active_project` + `save_index` | `workspace.set_active_project_id()` |
| `src/cli/project.rs` Show | `workspace.index.active_project` + `get_project` | `workspace.active_project_id()` + `workspace.load_project_summary()` |
| `src/cli/project.rs` New | `workspace.index.add_project` + `save_index` | remove index add; only write `state.json` |
| `src/cli/config.rs` resolve_scope | `workspace.index.active_project` | `workspace.active_project_id()` |
| `src/cli/config.rs` ensure_project_exists | `workspace.index.get_project()` | `workspace.project_exists()` |
| `src/cli/status.rs` | `workspace.index.active_project` | `workspace.active_project_id()` |
| `src/cli/history.rs` | `workspace.index.active_project` + `get_project` | `workspace.active_project_id()` + `workspace.project_exists()` |
| `src/cli/tail.rs` | `workspace.index.active_project` | `workspace.active_project_id()` |
| `src/cli/rollback.rs` | `workspace.index.active_project` + `get_project_mut` + `save_index` | `workspace.active_project_id()`; remove index sync |
| `src/workflow/orchestrator.rs` run | `workspace.index.active_project` + `set_active_project` + `save_index` | `workspace.active_project_id()` + `workspace.set_active_project_id()` |
| `src/workflow/orchestrator.rs` persist_state_and_index | `workspace.index.get_project_mut` + `save_index` | remove entirely; save `state.json` only; rename to `persist_state` |
| `src/workflow/orchestrator.rs` check_parent_project_consistency | `workspace.index.get_project()` | `workspace.project_exists()` + load parent `state.json` to compare `parent_project` field |
| `src/mcp/handlers.rs` resolve_project_id | `workspace.index.active_project` | `workspace.active_project_id()` |
| `src/mcp/handlers.rs` handle_project_list | `workspace.index.projects` + `workspace.index.active_project` | `workspace.list_projects()` + `workspace.active_project_id()` |
| `src/mcp/handlers.rs` handle_project_show | `workspace.index.get_project()` | `workspace.load_project_summary()` + `load_project_state()` |
| `src/mcp/handlers.rs` handle_status | `workspace.index.get_project()` for existence check | `workspace.project_exists()` |
| `src/mcp/handlers.rs` handle_history | `workspace.index.get_project()` for existence check | `workspace.project_exists()` |
| `src/mcp/handlers.rs` handle_config_show | `workspace.index.active_project` + `workspace.index.get_project()` | `workspace.active_project_id()` + `workspace.project_exists()` |
| `src/project/lifecycle.rs` create_project | `workspace.index.get_project()` for duplicate check + `add_project` + `save_index` | `workspace.project_exists()` for duplicate check; remove index manipulation; call `workspace.set_active_project_id()` for auto-activation |

### 8. Remove `check_parent_project_consistency` index dependency

The function at `orchestrator.rs:1793` currently reads the index to compare `parent_project` fields. Change it to load the project's own `state.json` (which is already loaded as the `state` parameter) — the consistency check becomes a no-op or is removed, since the single source of truth is now `state.json` alone. If cross-project parent validation is desired, check whether the parent project directory exists via `workspace.project_exists(parent_id)`.

### 9. Remove `persist_state_and_index` index sync

The function at `orchestrator.rs:2404` currently updates `ProjectRef` fields in the index and calls `save_index()`. Remove the index update portion entirely. Rename to `persist_state`. The function body becomes:

```rust
fn persist_state(project_dir: &Path, state: &ProjectState) -> Result<()> {
    save_project_state(project_dir, state)
}
```

This can be inlined at call sites if preferred.

### 10. Simplify `Workspace::init`

`Workspace::init` stops creating `index.json`. It creates `ralph.toml`, `projects/`, and `templates/` only. The `Workspace` struct no longer has an `index` field.

### 11. Simplify `Workspace::load`

`Workspace::load` stops loading `index.json`. It reads `ralph.toml` only. If `index.json` exists, it is silently ignored (no error, no read — except for the one-time active-project migration described in section 6).

### 12. `create_project` simplification

`create_project` in `lifecycle.rs` no longer calls `workspace.index.add_project()` or `workspace.save_index()`. It creates the project directory, writes `state.json` (with `created_at` set), writes `prompt.md`, and auto-activates (writes the worktree-local file via `set_active_project_id()`) if no active project is currently set. Duplicate-project detection uses `workspace.project_exists()` instead of `workspace.index.get_project()`.

### 13. Deterministic scan ordering

`list_projects()` sorts results by project ID (lexicographic, ascending). This ensures `ralph project list` and MCP `project_list` produce stable output regardless of filesystem `read_dir` ordering.

Non-directory entries in `.ralph/projects/` are silently skipped. Subdirectories without `state.json` are skipped with a stderr warning. Subdirectories with unparseable `state.json` are skipped with a stderr warning including the parse error. These decisions prevent one corrupt project from breaking the listing of all projects.

## Files & Modules

| File | Change |
|------|--------|
| `src/workspace/index.rs` | **Delete entirely.** Remove `WorkspaceIndex`, `ProjectRef`, `ProjectLifecycleStatus`, and all methods. |
| `src/workspace/mod.rs` | Remove `index` field from `Workspace`. Remove `save_index()`. Remove `active_project()`. Remove `pub mod index` and associated imports. Add `active_project_id()`, `set_active_project_id()`, `list_projects()`, `load_project_summary()`, `project_exists()`. Add `ProjectSummary` struct (or in a new `summary.rs`). Update `init()` to skip `index.json`. Update `load()` to skip `index.json` (with one-time migration). |
| `src/workspace/active.rs` | **New file.** Worktree-local active-project resolution logic: detecting git worktree dir via `git rev-parse --git-dir`, reading/writing the active-project file, fallback for non-git repos. |
| `src/workspace/discovery.rs` | No changes needed. |
| `src/project/state.rs` | Add `created_at: DateTime<Utc>` field with `serde(default)`. Change `save()` to use atomic temp-file-and-rename via `tempfile::NamedTempFile`. |
| `src/project/lifecycle.rs` | Remove `ProjectRef` creation and index manipulation from `create_project`. Set `created_at` on new `ProjectState`. Replace duplicate check with `workspace.project_exists()`. Auto-activate via `workspace.set_active_project_id()`. Remove `use crate::workspace::index::{ProjectLifecycleStatus, ProjectRef}` import. |
| `src/cli/project.rs` | Update all subcommands to use new `Workspace` API. Remove all `workspace.index` references. |
| `src/cli/config.rs` | Replace `workspace.index.active_project` with `workspace.active_project_id()` in `resolve_scope`. Replace `workspace.index.get_project()` with `workspace.project_exists()` in `ensure_project_exists`. |
| `src/cli/status.rs` | Replace `workspace.index.active_project` with `workspace.active_project_id()`. |
| `src/cli/history.rs` | Replace active-project resolution and project existence check. |
| `src/cli/tail.rs` | Replace active-project resolution. |
| `src/cli/rollback.rs` | Replace active-project resolution. Remove index sync block (the `get_project_mut` + `save_index` at lines 149-174). |
| `src/workflow/orchestrator.rs` | Replace active-project resolution in `run()`. Replace `set_active_project` + `save_index` with `set_active_project_id()`. Remove index sync from `persist_state_and_index()` (rename to `persist_state`). Update `check_parent_project_consistency` to use `workspace.project_exists()` instead of index lookup. |
| `src/mcp/handlers.rs` | Update `resolve_project_id` to use `workspace.active_project_id()`. Update `handle_project_list` to use `workspace.list_projects()` + `workspace.active_project_id()`. Update `handle_project_show` to use `workspace.load_project_summary()`. Update `handle_status` and `handle_history` to use `workspace.project_exists()` for validation. Update `handle_config_show` to use `workspace.active_project_id()` + `workspace.project_exists()`. |
| `src/error.rs` | No changes needed. `ActiveProjectNotSet` remains. |
| `src/validate/harness.rs` | Remove `load_index()` helper. Add `load_active_project()` helper that reads the worktree-local active-project file. |
| `src/validate/tests_init.rs` | Remove assertions about `index.json` existence and content. Add assertion that `index.json` is NOT created. Verify `projects/` and `ralph.toml` are created. |
| `src/validate/tests_project.rs` | Replace `load_index()` calls with `load_state()` and `load_active_project()`. Verify project creation writes `state.json` with `created_at`. Verify `project use` writes to worktree-local file. Verify `project list` output is sorted by ID. |
| `src/validate/tests_commands.rs` | Update config tests that may indirectly depend on index-based project resolution. Update status/history tests to not reference index. |
| `src/validate/tests_mcp.rs` | Update `project_list`, `project_show`, `status`, `history`, `config_show` tests. Verify `active` field uses local resolution. |
| `src/validate/tests_run.rs` | Remove any implicit index.json assertions from loop execution tests. |
| `tests/init_command.rs` | Remove assertions about `index.json` existence and content. Replace `workspace.index.active_project` assertions with worktree-local file checks. |
| `tests/git.rs` | Remove `index.json` references from git-related test setup and gitignore assertions. Add `.git/ralph-active-project` to gitignore expectations if applicable. |

## Testing Strategy

### Conformance tests (src/validate/)

1. **`tests_init.rs` updates:**
   - `init::creates_workspace_structure`: Assert `.ralph/projects/` and `ralph.toml` exist. Assert `index.json` is NOT created.
   - `init::default_index`: Remove or replace. New test `init::no_index_json` verifies no `index.json` after init.
   - Other init tests: Remove `index.json` assertions.

2. **`tests_project.rs` updates:**
   - `project::new_updates_index`: Replace with `project::new_creates_state_with_created_at` — verify `state.json` contains `created_at` field. Verify `load_active_project()` returns the project ID (auto-activation).
   - `project::use_switches_active`: Verify writes to worktree-local file (via `load_active_project()`), not `index.json`. Verify no `index.json` modification.
   - `project::list_shows_project`: Verify deterministic sorted output. Verify active marker matches worktree-local file.
   - `project::show_displays_info` / `project::show_json`: Verify output includes `created_at` derived from state.
   - **New:** `project::use_validates_existence` — `project use nonexistent` fails with exit code 2.
   - **New:** `project::list_sorted_deterministic` — create projects `c`, `a`, `b`; verify list output is sorted `a`, `b`, `c`.
   - **New:** `project::stale_active_project` — set active to a project, delete its directory, verify `ralph status` fails with descriptive error.

3. **`tests_commands.rs` updates:**
   - `config_show_project` / `config_get` / `config_set`: Verify these resolve default project from worktree-local active file.
   - `status_no_active_project`: Verify behavior unchanged (exit code 2).
   - **New:** `config_default_scope_uses_active` — set active project, run `config show` without `--project`, verify project scope is used.

4. **`tests_mcp.rs` updates:**
   - `project_list`: Verify scan-based output with `active` field from local resolution.
   - `project_show`: Verify works without index.
   - `status`, `history`: Verify existence checks use directory presence.
   - `config_show`: Verify active-project fallback uses local mechanism.

5. **`tests_run.rs` updates:**
   - Remove any implicit `index.json` assertions from loop execution tests.
   - Verify `persist_state` (renamed) saves `state.json` correctly.

6. **New conformance tests:**
   - **`project::backward_compat_ignores_index`:** Create workspace, manually create `index.json`, verify all commands work and ignore it.
   - **`project::concurrent_project_creation`:** Two sequential `project new` calls succeed without shared file conflicts (validates no index contention).
   - **`project::active_migration_from_index`:** Create workspace with legacy `index.json` containing `active_project`, verify first command migrates active to local file.

### Harness updates

- Remove `load_index()` from `RalphHarness`.
- Add `load_active_project(&self) -> Result<Option<String>>` that reads the worktree-local active-project file from the test's git directory.
- Add `assert_no_index_json(&self)` convenience assertion.

### Unit tests (in-module #[cfg(test)])

7. **`list_projects()` unit tests:** Create a temp workspace with multiple project dirs containing `state.json`. Verify the scan returns all projects with correct metadata sorted by ID. Verify empty projects dir returns empty list. Verify malformed `state.json` is skipped with warning. Verify non-directory entries are skipped.

8. **`active_project_id()` / `set_active_project_id()` unit tests:** Test read/write roundtrip. Test missing file returns `None`. Test empty/whitespace file returns `None`. Test invalid characters in file triggers warning and returns `None`. Test in a git-repo context (with `.git` directory). Test in a non-git context (fallback path).

9. **`ProjectState` `created_at` tests:** Test `new()` sets `created_at`. Test deserialization of legacy state without `created_at` gets sentinel default. Test `ProjectSummary` derivation falls back correctly.

10. **`ProjectState::save` atomicity test:** Write state, verify file exists and is valid JSON. (Crash-during-write testing is not feasible in unit tests but the temp+rename pattern is a well-known correctness guarantee.)

### Existing test file updates

11. **`tests/init_command.rs`:** Remove all `index.json` existence and content assertions. Replace `workspace.index.active_project.is_none()` assertions with worktree-local file absence checks.

12. **`tests/git.rs`:** Remove `fs::write` of `index.json` in test setup. Remove `index.json` gitignore assertions. Update gitignore tests if `.git/ralph-active-project` needs coverage.

### Integration / regression

13. **Backward compatibility:** Create a workspace with an existing `index.json`. Verify `Workspace::load()` succeeds and ignores it. Verify all commands work. Verify the one-time migration seeds the local active-project file.

14. **Rollback regression:** Verify `rollback` saves `state.json` correctly without writing `index.json`.

## Out of Scope

- **Deleting existing `index.json` files:** Existing files are silently ignored. Users can delete them manually if desired. A future `ralph upgrade` command could handle cleanup.

- **Multi-workspace support:** This spec addresses multi-worktree within a single repo. Multi-workspace (separate `.ralph/` roots) is a separate concern.

- **Locking for `list_projects()` scan:** The scan reads `state.json` files that are now written atomically (temp+rename). Per-project `ProjectLock` already protects concurrent orchestration writes. Scan-time reads that race with an in-progress rename will either see the old file or the new file, never a partial write. No additional locking is needed.

- **Active-project syncing across worktrees:** By design, active project is worktree-local and intentionally not shared.

- **`ralph.toml` changes:** The workspace config file is not modified in this spec, though `workspace_version` could be moved there in a follow-up.

- **CLI UX for the `project use` command regarding branch checkout:** Branch checkout behavior is unchanged.

- **Automatic cleanup of stale active-project references:** If a project is deleted but the active-project file still references it, the file is not auto-cleared. Commands fail with a descriptive error directing the user to `ralph project use`.

- **Atomic writes for `GlobalConfig::save` or `ProjectConfig::save`:** Only `ProjectState::save` is made atomic in this spec, since it is the file read by concurrent scanners. Config files are not scanned concurrently.