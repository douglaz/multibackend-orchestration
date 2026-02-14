Now I have a complete understanding of the codebase. Here's the engineering specification:

---

## Summary

When running `ralph auto`, if no `.ralph` directory exists in the current directory or its ancestors, the command fails with `WorkspaceNotFound`. Users must first run `ralph init` manually. This feature auto-creates the `.ralph` workspace when `ralph auto` is invoked and no workspace exists, and makes `ralph.toml` fully optional by allowing every field to fall back to its hardcoded default when the file is empty or missing fields.

## Acceptance Criteria

1. **Auto-init on `ralph auto`**: If `discover_workspace_root()` returns `WorkspaceNotFound`, `ralph auto` creates `.ralph/` in the current working directory (equivalent to `ralph init .ralph`) and proceeds without error.
2. **Printed notice**: When auto-creating, print `initialized workspace at .ralph` to stderr so the user knows it happened.
3. **Empty `ralph.toml` parses to defaults**: An empty file (`""`) or a file containing only whitespace produces a valid `GlobalConfig` identical to `GlobalConfig::default()`.
4. **Partially populated `ralph.toml`**: Any subset of top-level sections (`[workspace]`, `[backends]`, `[workflow]`, `[templates]`, `[git]`) can be omitted; missing sections fall back to their defaults.
5. **Missing `ralph.toml` file**: `GlobalConfig::load()` returns `GlobalConfig::default()` when the file does not exist on disk (distinct from parse failure).
6. **No behavior change for other commands**: Commands other than `auto` continue to fail with `WorkspaceNotFound` if `.ralph` is missing — the auto-init is scoped to `auto` only.
7. **No behavior change for `ralph init`**: `ralph init` still rejects non-empty directories and creates the full workspace structure including templates.

## Technical Approach

### A. Make `GlobalConfig` deserializable from empty TOML

Add `#[serde(default)]` to every field on `GlobalConfig`, and add `Default` implementations (or `#[serde(default = "...")]`) to every struct that currently lacks one: `WorkspaceConfig`, `BackendConfigs`, `BackendConfig`, `WorkflowConfig`, `TemplateConfig`, `GitConfig`. The existing `GlobalConfig::default()` already defines the correct values — wire the serde defaults to use it.

Concrete changes in `src/config/global.rs`:
- Add `#[serde(default)]` to all five fields of `GlobalConfig`.
- Implement `Default` for `WorkspaceConfig`, `BackendConfigs`, `BackendConfig`, `WorkflowConfig`, `TemplateConfig`, `GitConfig` by extracting the corresponding values from the existing `GlobalConfig::default()`.
- For `BackendConfigs::default()`, construct both `claude` and `codex` with their respective defaults (command, args, timeout, models).

### B. Handle missing `ralph.toml` in `GlobalConfig::load()`

In `GlobalConfig::load()`, if `fs::read_to_string` returns `io::ErrorKind::NotFound`, return `Ok(GlobalConfig::default())` with model fill applied. All other IO errors continue to propagate.

### C. Auto-init in `ralph auto`

In `src/cli/auto.rs::execute()`, replace the direct `Workspace::discover()?` call with:

```rust
let workspace = match Workspace::discover() {
    Ok(ws) => ws,
    Err(RalphError::WorkspaceNotFound) => {
        let root = std::env::current_dir()?.join(".ralph");
        init::execute_at(&root)?;
        eprintln!("initialized workspace at {}", root.display());
        Workspace::load(root)?
    }
    Err(e) => return Err(e),
};
```

Extract the directory/template creation logic from `init::execute()` into a reusable `init::execute_at(root: &Path) -> Result<()>` function. The existing `init::execute(args)` calls `execute_at(&args.dir)` and prints the success message.

## Files & Modules

| File | Change |
|---|---|
| `src/config/global.rs` | Add `#[serde(default)]` to `GlobalConfig` fields. Implement `Default` for `WorkspaceConfig`, `BackendConfigs`, `BackendConfig`, `WorkflowConfig`, `TemplateConfig`, `GitConfig`. Handle `NotFound` in `load()`. |
| `src/cli/auto.rs` | Catch `WorkspaceNotFound` and auto-init before proceeding. Import `init` module. |
| `src/cli/init.rs` | Extract `execute_at(root: &Path) -> Result<()>` from `execute()`. Make it `pub(crate)`. |

## Testing Strategy

### Unit tests in `src/config/global.rs`

1. **`empty_toml_deserializes_to_defaults`**: `toml::from_str::<GlobalConfig>("")` succeeds and all fields match `GlobalConfig::default()`.
2. **`partial_toml_deserializes_missing_sections_to_defaults`**: TOML with only `[workspace]` section produces correct defaults for `backends`, `workflow`, `templates`, `git`.
3. **`load_returns_defaults_when_file_missing`**: Call `GlobalConfig::load()` on a nonexistent path and assert result equals default config with models filled.

### Unit tests in `src/cli/auto.rs`

4. **`auto_creates_workspace_when_missing`**: Run `execute()` in a temp dir with no `.ralph` — assert `.ralph/` directory, `ralph.toml`, `projects/`, `templates/` all exist after the call (will fail at backend health check, but workspace creation can be verified by checking the directory before the error).

### Existing tests

5. Verify existing `init_command` tests and `GlobalConfig` deserialization tests continue to pass unchanged.

## Out of Scope

- Auto-init for commands other than `auto` (e.g., `run`, `status`, `daemon`).
- Config file migration or versioning (e.g., upgrading old `ralph.toml` schemas).
- Interactive prompting before auto-creating the workspace.
- Gitignore management (adding `.ralph` to `.gitignore`).
- Validating that auto-created templates match the latest embedded defaults after upgrades.