I now have a comprehensive understanding of the entire system. Let me write the specification.

---

## Summary

Replace the centralized `.ralph/index.json` project registry with a decentralized, directory-scan-based design. Today, `index.json` stores a flat list of all project metadata and a workspace-wide `active_project` field. This creates two problems in multi-worktree environments: (1) concurrent writes to `index.json` from different worktrees corrupt or overwrite each other's changes, and (2) the `active_project` field is workspace-global but semantically per-checkout — changing it in one worktree silently affects all others.

The fix: eliminate `index.json` entirely. Derive the project list by scanning `.ralph/projects/*/state.json`. Store `active_project` (if retained at all) in a worktree-local location outside of `.ralph/` (e.g., `.git/ralph-active-project`), or remove it and require explicit `--project` everywhere. Each project's `state.json` already contains all the metadata currently duplicated in `ProjectRef`; the duplication is the root cause of the sync problem.

## Acceptance Criteria

1. **`index.json` is no longer required or written.** Ralph operates without `.ralph/index.json`. An existing `index.json` is silently ignored (no error if present for backward compatibility).

2. **`ralph project list` derives its output by scanning `.ralph/projects/*/state.json`.** Output columns remain identical. Active-project marker uses the new local mechanism.

3. **`ralph project new` creates the project directory and `state.json` without updating any shared file.** No shared registry file is written.

4. **`ralph run`, `ralph status`, `ralph history`, `ralph tail`, `ralph rollback`, and `ralph project show` resolve the project from `--project` flag or from the worktree-local active-project file.** Error message when no project is specified and none is active remains user-friendly.

5. **`ralph project use <id>` writes only to the worktree-local active-project file.** It does not write any file inside `.ralph/`.

6. **Orchestrator `persist_state_and_index` no longer syncs to a shared index.** It saves `state.json` only.

7. **Rollback saves `state.json` only.** No shared index sync.

8. **MCP handlers for `project_list` and `project_show` work against the scan-based project list.** The `"active"` field in MCP JSON output uses the local active-project resolution.

9. **Two concurrent `ralph run --project X` and `ralph run --project Y` in separate worktrees of the same repo do not conflict on any shared file** (project-level `.lock` files remain per-project and already handle this).

10. **Migration:** `ralph init` no longer creates `index.json`. Existing workspaces that have `index.json` continue to work (it is simply not read).

11. **`ProjectRef` metadata (status, loop counts, etc.) is derived from `state.json` on demand** rather than cached in a shared file.

## Technical Approach

### 1. Remove `WorkspaceIndex.projects` and `WorkspaceIndex.active_project`

Strip the `projects` vector and `active_project` field from `WorkspaceIndex`. The struct retains only `workspace_version` and `created_at` (or is removed entirely if those are moved to `ralph.toml`). All methods on `WorkspaceIndex` that manipulate projects or active-project are deleted.

If `WorkspaceIndex` would become empty, remove it and `index.json` entirely, storing `workspace_version` in `ralph.toml` (it's already there as `config.workspace.version`).

### 2. Directory-scan project listing

Add a function to `Workspace`:

```rust
pub fn list_projects(&self) -> Result<Vec<ProjectSummary>>
```

This reads `.ralph/projects/`, iterates subdirectories, loads each `state.json`, and returns a `ProjectSummary` (the replacement for `ProjectRef`) derived from the state. The `ProjectSummary` struct is computed, not persisted.

For project existence checks (e.g., `get_project`), add:

```rust
pub fn project_exists(&self, id: &str) -> bool
pub fn load_project_summary(&self, id: &str) -> Result<ProjectSummary>
```

### 3. Worktree-local active project

Store the active project ID in a file that is local to the git worktree and not shared across clones/worktrees:

- **Primary location:** `.git/ralph-active-project` (a plain text file containing just the project ID). In a worktree, `.git` is a file pointing to the actual gitdir; use `git rev-parse --git-dir` to resolve the actual per-worktree git directory.
- **Fallback (non-git repos):** `.ralph/.active-project-local` and add it to `.ralph/.gitignore` so it's never committed. This handles the case where ralph is used outside a git repository.

Add to `Workspace`:

```rust
pub fn active_project_id(&self) -> Option<String>
pub fn set_active_project_id(&self, id: &str) -> Result<()>
```

These read/write the worktree-local file. The `Workspace` struct drops its `index` field or retains a simplified version without project data.

### 4. Update all consumer sites

Every location that currently reads `workspace.index.active_project` switches to `workspace.active_project_id()`. Every location that reads `workspace.index.projects` or `workspace.index.get_project()` switches to `workspace.load_project_summary()` or `workspace.list_projects()`.

**Specific call sites to update:**

| File | Current pattern | New pattern |
|------|----------------|-------------|
| `src/cli/project.rs` List | iterate `workspace.index.projects` | call `workspace.list_projects()` |
| `src/cli/project.rs` Use | `workspace.index.set_active_project` + `save_index` | `workspace.set_active_project_id()` |
| `src/cli/project.rs` Show | `workspace.index.active_project` + `get_project` | `workspace.active_project_id()` + `load_project_summary()` |
| `src/cli/project.rs` New | `workspace.index.add_project` + `save_index` | remove index add; only write `state.json` |
| `src/cli/status.rs` | `workspace.index.active_project` | `workspace.active_project_id()` |
| `src/cli/history.rs` | `workspace.index.active_project` + `get_project` | `workspace.active_project_id()` + `load_project_summary()` |
| `src/cli/tail.rs` | `workspace.index.active_project` | `workspace.active_project_id()` |
| `src/cli/rollback.rs` | `workspace.index.active_project` + index sync | `workspace.active_project_id()`; remove index sync |
| `src/workflow/orchestrator.rs` run | `workspace.index.active_project` + `set_active_project` + `save_index` | `workspace.active_project_id()` + `set_active_project_id()` |
| `src/workflow/orchestrator.rs` persist | `workspace.index.get_project_mut` + `save_index` | remove index sync; save `state.json` only |
| `src/mcp/handlers.rs` | `workspace.index.active_project` + `workspace.index.projects` | `workspace.active_project_id()` + `workspace.list_projects()` |

### 5. Remove `ActiveProjectNotSet` error handling change

Keep the `RalphError::ActiveProjectNotSet` error variant. It is still raised when no `--project` is provided and no worktree-local active project is set.

### 6. Remove `persist_state_and_index` index sync

The function `persist_state_and_index` in `orchestrator.rs` currently updates the `ProjectRef` in the index and calls `save_index()`. Remove the index update portion entirely. The function becomes just `save_project_state(project_dir, state)` — it could be inlined or renamed to `persist_state`.

### 7. Simplify `Workspace::init`

`Workspace::init` stops creating `index.json`. It creates `ralph.toml`, `projects/`, and `templates/` only.

### 8. Simplify `Workspace::load`

`Workspace::load` stops loading `index.json`. It reads `ralph.toml` only. If `index.json` exists, it is silently ignored (no error, no read).

### 9. `create_project` simplification

`create_project` in `lifecycle.rs` no longer calls `workspace.index.add_project()` or `workspace.save_index()`. It creates the project directory, writes `state.json` and `prompt.md`, and optionally auto-activates (writes the worktree-local file) if no active project is set.

## Files & Modules

| File | Change |
|------|--------|
| `src/workspace/index.rs` | Remove `ProjectRef`, `projects`, `active_project` from `WorkspaceIndex`. Remove `add_project`, `set_active_project`, `active_project_ref`, `get_project`, `get_project_mut`. If struct becomes trivial, consider removing entirely. |
| `src/workspace/mod.rs` | Remove `index` field from `Workspace` (or simplify). Add `active_project_id()`, `set_active_project_id()`, `list_projects()`, `load_project_summary()`, `project_exists()`. Update `init()` and `load()` to skip `index.json`. |
| `src/workspace/discovery.rs` | No changes needed. |
| `src/project/lifecycle.rs` | Remove `ProjectRef` creation and index manipulation from `create_project`. Auto-activate via new worktree-local method. |
| `src/cli/project.rs` | Update all subcommands to use new `Workspace` API. Remove all `workspace.index` references. |
| `src/cli/status.rs` | Replace `workspace.index.active_project` with `workspace.active_project_id()`. Replace `workspace.index.get_project()` with `workspace.load_project_summary()`. |
| `src/cli/history.rs` | Same pattern as status. |
| `src/cli/tail.rs` | Replace active-project resolution. |
| `src/cli/rollback.rs` | Replace active-project resolution. Remove index sync block (lines 149-174). |
| `src/workflow/orchestrator.rs` | Replace active-project resolution in `run()`. Remove index sync from `persist_state_and_index()`. Remove `set_active_project` + `save_index` on explicit `--project`. |
| `src/mcp/handlers.rs` | Update `handle_project_list` to use `workspace.list_projects()`. Update `resolve_project_id` to use `workspace.active_project_id()`. |
| `src/error.rs` | No changes needed. `ActiveProjectNotSet` remains. |
| `tests/init_command.rs` | Remove assertions about `index.json` existence and content. |
| `tests/git.rs` | Remove `index.json` references from git-related test setup. |

New file (optional):
| `src/workspace/active.rs` | Worktree-local active-project resolution logic (detecting git worktree dir, reading/writing the file). Could also live in `mod.rs`. |

## Testing Strategy

1. **Unit tests for `list_projects()`:** Create a temp workspace with multiple project dirs containing `state.json`. Verify the scan returns all projects with correct metadata. Verify empty projects dir returns empty list. Verify malformed `state.json` is skipped or errors gracefully.

2. **Unit tests for `active_project_id()` / `set_active_project_id()`:** Test read/write roundtrip. Test missing file returns `None`. Test in a git-repo context (mock `git rev-parse --git-dir`). Test in a non-git context (fallback path).

3. **Unit tests for `project_exists()` / `load_project_summary()`:** Test with existing and non-existing project IDs. Test that summary fields match state.json content.

4. **Integration test for `project new`:** Verify project dir and `state.json` are created. Verify no `index.json` is created or modified. Verify auto-activation writes to the local active-project file.

5. **Integration test for `project list`:** Create multiple projects. Verify all appear in scan output. Verify active marker matches the local active-project file.

6. **Integration test for `project use`:** Verify writes to worktree-local file. Verify does not modify any file in `.ralph/`.

7. **Integration test for concurrent access:** Two processes each create a different project simultaneously. Verify both projects exist afterward and no corruption occurs (no shared file to corrupt).

8. **Regression tests for orchestrator:** Verify `persist_state_and_index` (renamed) saves `state.json` correctly without writing `index.json`.

9. **Backward compatibility test:** Create a workspace with an existing `index.json`. Verify `Workspace::load()` succeeds and ignores it. Verify all commands work.

10. **MCP handler tests:** Verify `project_list` returns scan-based results. Verify `project_show` resolves via scan. Verify `active` field uses local resolution.

## Out of Scope

- **Migrating data from `index.json` to `state.json`:** `state.json` already contains all the data that matters. `ProjectRef` fields like `total_feature_loops` and `last_loop_number` are already derivable from `state.json` (the orchestrator computes them during sync today). No migration needed.
- **Deleting existing `index.json` files:** Existing files are silently ignored. Users can delete them manually if desired.
- **Multi-workspace support:** This spec addresses multi-worktree within a single repo. Multi-workspace (separate `.ralph/` roots) is a separate concern.
- **Locking for `list_projects()` scan:** The scan reads `state.json` files that may be concurrently written. Since these are per-project and already protected by `ProjectLock` during writes, and reads are tolerant of slightly stale data, no additional locking is needed for the listing operation.
- **Active-project syncing across worktrees:** By design, active project is worktree-local and intentionally not shared.
- **`ralph.toml` changes:** The workspace config file is not modified in this spec, though `workspace_version` could be moved there in a follow-up.
- **CLI UX for the `project use` command regarding branch checkout:** Branch checkout behavior is unchanged.