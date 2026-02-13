Good — `StateLocked` exits with code 3. Now I have all the context needed to produce the revised spec.

Here is the revised specification addressing all three review issues:

---

## Summary

Add a `ralph project delete <id>` CLI subcommand that removes a project's directory tree (`.ralph/projects/<id>/`) after validating the project ID format, confirming the project exists, verifying it is not the currently active project, and acquiring its file lock to ensure no concurrent operation is in progress. If the target is the active project, the command exits with an error directing the user to switch first. If the project is locked by another process, the command exits with a `StateLocked` error (exit code 3). On success, the lock is released, the entire project directory is deleted via `fs::remove_dir_all`, and a confirmation message is printed.

## Acceptance Criteria

1. `ralph project delete my-proj` removes `.ralph/projects/my-proj/` (including `state.json`, `prompt.md`, `loops/`, `.lock`, and any other contents) and prints `project 'my-proj' deleted`.
2. If `my-proj` does not exist (no `state.json`), the command exits with code 2 and prints a `ProjectNotFound` error.
3. If `my-proj` is the currently active project, the command exits with code 2 and prints a `Validation` error: `"cannot delete the active project 'my-proj'; run \`ralph project use <other-id>\` first"`.
4. If no project is active, the delete command proceeds normally (no active-project guard fires).
5. If the project ID contains characters outside `[a-zA-Z0-9_-]` or is empty, the command exits with code 2 and prints a `Validation` error before any filesystem access occurs.
6. If the project's `.lock` file is held by another process, the command exits with code 3 and prints a `StateLocked` error.
7. Conformance test `project::delete_removes_directory` passes under `ralph validate`.
8. Conformance test `project::delete_refuses_active` passes under `ralph validate`.
9. Conformance test `project::delete_nonexistent_fails` passes under `ralph validate`.
10. Conformance test `project::delete_no_active_project` passes under `ralph validate`.

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

### Project ID validation (`src/project/lifecycle.rs`)

Make the existing `validate_project_id` function `pub(crate)` (it is currently private) so the delete handler can reuse it without duplicating the `[a-zA-Z0-9_-]` check. No changes to the validation logic itself.

### Command handler (`src/cli/project.rs`)

Add a `ProjectCommand::Delete` match arm:

1. Call `validate_project_id(&id)` — return `RalphError::Validation` if the ID contains invalid characters or is empty. This prevents path-traversal attacks (e.g., `../../../etc`) before any filesystem path is constructed.
2. `Workspace::discover()?`
3. Check `workspace.project_exists(&id)` — return `RalphError::ProjectNotFound` if false.
4. Check `workspace.active_project_id() == Some(id)` — return `RalphError::Validation` with the "switch first" message if true.
5. Acquire the project lock via `ProjectLock::acquire(&workspace.project_dir(&id), &id)`. If this returns `Err(RalphError::StateLocked { .. })`, propagate the error (exit code 3). This prevents deleting a project that is mid-operation in another process.
6. Drop the lock (explicitly or via scope exit) — the lock file is inside the directory about to be removed, so holding an open file descriptor during `remove_dir_all` could fail on some platforms. Acquire-then-drop proves no other process holds the lock at the moment of deletion; a concurrent process attempting to lock after this point will fail because the directory no longer exists.
7. Call `fs::remove_dir_all(workspace.project_dir(&id))?`.
8. Print `project '{id}' deleted`.

### Conformance tests (`src/validate/tests_project.rs`)

Add four test functions to the existing `tests()` vec:

- **`project::delete_removes_directory`**: `init` → `create_project("del-test", ...)` → create a second project and `project use` it (so `del-test` is not active) → `ralph_ok(["project", "delete", "del-test"])` → assert `project_dir("del-test")` no longer exists → assert stdout contains `deleted`.
- **`project::delete_refuses_active`**: `init` → `create_project("active-del", ...)` (auto-activated as first project) → `ralph_exit(["project", "delete", "active-del"], 2)` → assert stderr contains `cannot delete the active project` → assert project directory still exists.
- **`project::delete_nonexistent_fails`**: `init` → `ralph_exit(["project", "delete", "no-such-proj"], 2)` → assert stderr contains `not found`.
- **`project::delete_no_active_project`**: `init` → `create_project("orphan-del", ...)` → clear the active-project file by writing `"\n"` to `<repo>/.git/ralph-active-project` → assert `load_active_project()` returns `None` → `ralph_ok(["project", "delete", "orphan-del"])` → assert `project_dir("orphan-del")` no longer exists → assert stdout contains `deleted`.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/mod.rs` | Add `Delete(ProjectDeleteArgs)` to `ProjectCommand` enum; add `ProjectDeleteArgs` struct |
| `src/cli/project.rs` | Add `ProjectCommand::Delete` match arm with ID validation, exists/active/lock guards, and `remove_dir_all` |
| `src/project/lifecycle.rs` | Change `validate_project_id` visibility from `fn` to `pub(crate) fn` |
| `src/validate/tests_project.rs` | Add four `ConformanceTest` entries and their test functions |

No new files. No changes to `Workspace`, `active.rs`, `error.rs`, `lock.rs`, or `state.rs`.

## Testing Strategy

1. **Conformance tests** (described above) exercise the command end-to-end via the `RalphHarness` subprocess harness, matching the established pattern in `tests_project.rs`. They cover the happy path, active-project refusal, nonexistent-project error, and the no-active-project scenario.
2. **Lock contention** is not tested via conformance tests because reliably holding a file lock from the harness while spawning the delete subprocess is fragile and platform-dependent. The lock path is validated by code review and manual testing.
3. **ID validation** is already unit-tested by the existing `validate_project_id` tests in `lifecycle.rs`. The delete handler reuses the same function, so no additional unit tests are needed for the validation logic itself. The `delete_nonexistent_fails` conformance test implicitly exercises the validation pass (valid ID, project not found).
4. **Existing tests** remain unchanged — `cargo test` must still pass. The new `Delete` variant in `ProjectCommand` is exhaustively matched, so existing match arms are unaffected.
5. **Manual smoke test**: `nix develop -c cargo build && ralph project delete <id>` against a real workspace.

## Out of Scope

- Interactive confirmation prompt (e.g., "are you sure?") or `--force` flag. The command is analogous to `rm -rf` on a known path; a `--force` flag can be added later if needed.
- Cascading cleanup of child projects that reference the deleted project as `parent_project`. Parent references are informational only and do not break child projects.
- Git branch cleanup (deleting the `ralph/<id>` branch). Branch management is a separate concern and can be addressed independently.
- Clearing the active-project pointer if it happens to reference the deleted project via a stale file (the guard prevents this case).
- Conformance testing of lock contention (described above in Testing Strategy).