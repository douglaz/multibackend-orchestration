I now have a thorough understanding of the codebase. Here's the spec:

---

## Summary

Add a `ralph project delete <id>` CLI subcommand that removes a project's directory tree (`.ralph/projects/<id>/`) after validating the project exists and is not the currently active project. If the target is the active project, the command exits with an error directing the user to switch first. On success, the entire project directory is deleted via `fs::remove_dir_all` and a confirmation message is printed.

## Acceptance Criteria

1. `ralph project delete my-proj` removes `.ralph/projects/my-proj/` (including `state.json`, `prompt.md`, `loops/`, `.lock`, and any other contents) and prints `project 'my-proj' deleted`.
2. If `my-proj` does not exist (no `state.json`), the command exits with code 2 and prints a `ProjectNotFound` error.
3. If `my-proj` is the currently active project, the command exits with code 2 and prints a `Validation` error: `"cannot delete the active project 'my-proj'; run \`ralph project use <other-id>\` first"`.
4. If no project is active, the delete command proceeds normally (no active-project guard fires).
5. Conformance test `project::delete_removes_directory` passes under `ralph validate`.
6. Conformance test `project::delete_refuses_active` passes under `ralph validate`.
7. Conformance test `project::delete_nonexistent_fails` passes under `ralph validate`.

## Technical Approach

### CLI layer (`src/cli/mod.rs`)

Add a `Delete` variant to `ProjectCommand`:

```rust
pub enum ProjectCommand {
    New(ProjectNewArgs),
    List,
    Use(ProjectUseArgs),
    Show(ProjectShowArgs),
    Delete(ProjectDeleteArgs),
}

pub struct ProjectDeleteArgs {
    pub project_id: String,
}
```

The positional `project_id` argument mirrors the existing `ProjectUseArgs` pattern.

### Command handler (`src/cli/project.rs`)

Add a `ProjectCommand::Delete` match arm:

1. `Workspace::discover()?`
2. Check `workspace.project_exists(&id)` — return `RalphError::ProjectNotFound` if false.
3. Check `workspace.active_project_id() == Some(id)` — return `RalphError::Validation` with the "switch first" message if true.
4. Call `fs::remove_dir_all(workspace.project_dir(&id))?`.
5. Print `project '{id}' deleted`.

No new methods on `Workspace` are needed; the existing `project_exists`, `active_project_id`, and `project_dir` methods suffice. The delete operation is a simple `fs::remove_dir_all` — no lock acquisition is necessary since we are removing the entire directory (including `.lock`), and the active-project guard already prevents deleting a project that could be mid-run.

### Conformance tests (`src/validate/tests_project.rs`)

Add three test functions to the existing `tests()` vec:

- **`project::delete_removes_directory`**: `init` → `create_project("del-test", ...)` → create a second project and `project use` it (so `del-test` is not active) → `ralph_ok(["project", "delete", "del-test"])` → assert `project_dir("del-test")` no longer exists → assert stdout contains `deleted`.
- **`project::delete_refuses_active`**: `init` → `create_project("active-del", ...)` (auto-activated as first project) → `ralph_exit(["project", "delete", "active-del"], 2)` → assert stderr contains `cannot delete the active project` → assert project directory still exists.
- **`project::delete_nonexistent_fails`**: `init` → `ralph_exit(["project", "delete", "no-such-proj"], 2)` → assert stderr contains `not found`.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/mod.rs` | Add `Delete(ProjectDeleteArgs)` to `ProjectCommand` enum; add `ProjectDeleteArgs` struct |
| `src/cli/project.rs` | Add `ProjectCommand::Delete` match arm with exists/active guards and `remove_dir_all` |
| `src/validate/tests_project.rs` | Add three `ConformanceTest` entries and their test functions |

No new files. No changes to `Workspace`, `active.rs`, `error.rs`, or `state.rs`.

## Testing Strategy

1. **Conformance tests** (described above) exercise the command end-to-end via the `RalphHarness` subprocess harness, matching the established pattern in `tests_project.rs`. They cover the happy path, active-project refusal, and nonexistent-project error.
2. **Existing tests** remain unchanged — `cargo test` must still pass. The new `Delete` variant in `ProjectCommand` is exhaustively matched, so existing match arms are unaffected.
3. **Manual smoke test**: `nix develop -c cargo build && ralph project delete <id>` against a real workspace.

## Out of Scope

- Interactive confirmation prompt (e.g., "are you sure?") or `--force` flag. The command is idempotent-safe and analogous to `rm -rf` on a known path; a `--force` flag can be added later if needed.
- Cascading cleanup of child projects that reference the deleted project as `parent_project`. Parent references are informational only and do not break child projects.
- Git branch cleanup (deleting the `ralph/<id>` branch). Branch management is a separate concern and can be addressed independently.
- Clearing the active-project pointer if it happens to reference the deleted project via a stale file (the guard prevents this case).
- Lock-aware deletion (checking `.lock` before removing). The active-project guard is sufficient; a locked project is necessarily the active project or being run, which the guard already blocks.