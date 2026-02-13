---
artifact: acceptance-pass
loop: 7
project: parallel-worktrees
backend: claude(opus)
role: qa
created_at: 2026-02-13T17:36:48Z
---

All validation is complete. Here is the acceptance QA report:

# QA: PASS
## Tests Run
- **Compilation**: `cargo check` and `cargo build` both succeed cleanly (only "dirty git tree" warning)
- **Unit & integration tests**: 498 passed, 0 failed, 1 ignored (the ignored test is a subprocess helper `subprocess_helper_emits_empty_active_project_warning` used by another test)
- **Conformance tests** (`ralph validate`): 98/98 passed, 0 failed, 0 skipped
- **Test suites covered**: 15 separate test binaries including `init_command`, `git`, `mcp_handlers`, `templates`, `validate_cli`, `tail_tmux`, and all in-crate modules

## Verification Summary

All 16 acceptance criteria verified as PASS:

1. **index.json no longer required or written**: `Workspace::init()` creates only `ralph.toml`, `projects/`, and `templates/`. `Workspace::load()` reads only `ralph.toml`. No code path writes `index.json`.
2. **Project list from directory scan**: `list_projects()` scans `.ralph/projects/*/state.json`, sorts by project ID, skips invalid entries with warnings.
3. **project new creates state.json only**: `create_project()` writes `state.json` with `created_at`, auto-activates via worktree-local file. No shared registry written.
4. **CLI commands resolve via --project or worktree-local**: `resolve_project_id()` used by `status`, `history`, `tail`, `rollback` commands. Stale IDs produce descriptive error with hint.
5. **project use writes worktree-local only**: `set_active_project_id()` writes to `.git/ralph-active-project` (or `.ralph/.active-project-local` for non-git).
6. **Config resolves from worktree-local**: `resolve_scope()` uses `active_project_id()`. Stale active projects fall back to global scope with warning.
7. **persist_state_and_index renamed to persist_state**: Function renamed and simplified to `save_project_state()` only. No index sync.
8. **Rollback saves state.json only**: Removed 27-line index sync block from rollback. Only `save_project_state()` called.
9. **MCP handlers scan-based**: `handle_project_list` uses `workspace.list_projects()`. `handle_project_show` uses `load_project_summary()`. All existence checks use `project_exists()`.
10. **No shared file conflicts**: No writes to any shared file. Per-project `state.json` uses atomic writes. Active project stored worktree-locally.
11. **Migration from index.json**: One-time migration in `Workspace::load()` seeds worktree-local file from legacy `index.json` `active_project`. Conformance test `migration_from_legacy_index` passes.
12. **ProjectRef removed**: No `ProjectRef`, `WorkspaceIndex`, or `ProjectLifecycleStatus` types remain. `src/workspace/index.rs` deleted. `ProjectSummary` computed on-demand.
13. **--project updates worktree-local active**: `orchestrator.rs run()` calls `set_active_project_id()` when `explicit_project` is true.
14. **Corrupt active-project files handled**: `read_active_project()` returns `None` with warning for empty, whitespace-only, or invalid-character content. Conformance test `corrupt_active_project` verifies exit code 2.
15. **Atomic ProjectState::save**: Uses `tempfile::NamedTempFile::new_in()` + `write_all()` + `persist()` for crash-safe writes. Unit test `save_is_atomic_and_roundtrips` passes.
16. **check_parent_project_consistency uses project dir**: Checks `workspace.project_exists(parent_id)` instead of index lookup. Emits warning if parent project directory missing.

**Additional validations**:
- `src/workspace/index.rs` confirmed deleted (file does not exist)
- No remaining references to `workspace.index`, `save_index`, `WorkspaceIndex`, `ProjectRef`, or `ProjectLifecycleStatus` in production code
- `tempfile` dependency confirmed in `Cargo.toml`
- `created_at` field added to `ProjectState` with `serde(default)` for backward compatibility
- Remaining `index.json` references in `tests/git.rs` are test fixtures for git diff/commit exclusion behavior (not functional dependencies)
