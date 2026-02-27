Now I have a thorough understanding of the codebase. Let me produce the revised specification.

---

## Summary

When running `ralph auto`, if no `.ralph` directory exists in the current directory or its ancestors, the command fails with `WorkspaceNotFound`. Users must first run `ralph init` manually. This feature auto-creates the `.ralph` workspace when `ralph auto` is invoked and no workspace exists, and makes `ralph.toml` fully optional by allowing every field to fall back to its hardcoded default when the file is empty or missing fields.

## Acceptance Criteria

1. **Auto-init on `ralph auto`**: If `Workspace::discover()` returns `WorkspaceNotFound`, `ralph auto` creates `.ralph/` in the current working directory (equivalent to `ralph init .ralph`) and proceeds without error.
2. **Printed notice**: When auto-creating, print `initialized workspace at .ralph` to stderr so the user knows it happened.
3. **Empty `ralph.toml` parses to defaults**: An empty file (`""`) or a file containing only whitespace produces a valid `GlobalConfig` identical to `GlobalConfig::default()`. This requires `#[serde(default)]` on every field of `GlobalConfig` and `Default` impls on all nested structs and enums.
4. **Partially populated `ralph.toml`**: Any subset of top-level sections (`[workspace]`, `[backends]`, `[workflow]`, `[templates]`, `[git]`) can be omitted; missing sections fall back to their defaults. Within each section, any subset of fields can be omitted with the same fallback behavior.
5. **Missing `ralph.toml` during auto-init only**: The auto-init path constructs `GlobalConfig::default()` in-memory via `Workspace::init()` — it never reads from disk. The existing `Workspace::load()` / `GlobalConfig::load()` behavior is unchanged: a missing `ralph.toml` on an already-existing `.ralph` directory remains an error, preserving detection of configuration corruption.
6. **No behavior change for other commands**: Commands other than `auto` continue to fail with `WorkspaceNotFound` if `.ralph` is missing — the auto-init is scoped to `auto` only.
7. **No behavior change for `ralph init`**: `ralph init` still rejects non-empty directories and creates the full workspace structure including templates.

## Technical Approach

### A. Make `GlobalConfig` deserializable from empty TOML

Add `#[serde(default)]` to every field on `GlobalConfig`. Add `Default` implementations for every nested struct and enum that currently lacks one, wired to return the same values as the existing `GlobalConfig::default()`.

Concrete changes in `src/config/global.rs`:

- Add `#[serde(default)]` to all five fields of `GlobalConfig` (`workspace`, `backends`, `workflow`, `templates`, `git`).
- Implement `Default` for `WorkspaceConfig` — extract values from the existing `GlobalConfig::default()`.
- Implement `Default` for `BackendConfigs` — construct both `claude` and `codex` with their respective default commands, args, timeouts, and models.
- Implement `Default` for `BackendConfig` — this is the per-backend default (empty command/args, zero timeout). `BackendConfigs::default()` constructs each backend explicitly rather than relying on `BackendConfig::default()`, but the impl is needed for serde when a partial `[backends]` section omits one backend.
- Implement `Default` for `WorkflowConfig` — extract values from the existing `GlobalConfig::default()`.
- Implement `Default` for `CommitMessageStyle` — returns `CommitMessageStyle::Conventional`, matching `GlobalConfig::default()`.
- Implement `Default` for `PromptChangeAction` — returns `PromptChangeAction::Abort`, matching `GlobalConfig::default()`.
- Implement `Default` for `TemplateConfig` — extract template paths from the existing `GlobalConfig::default()`.
- Implement `Default` for `GitConfig` — extract values from the existing `GlobalConfig::default()`.

The existing field-level `#[serde(default)]` and `#[serde(default = "...")]` annotations on individual struct fields (e.g., `tmux`, `daemon_poll_seconds`, `qa_enabled`) remain unchanged. The new struct-level `Default` impls provide the top-level fallback when an entire section is missing from the TOML.

**Note on `BackendConfig::default()`**: Since `BackendConfigs::default()` explicitly constructs each backend with the correct command/args/timeout, the `BackendConfig::default()` only needs to serve as a serde fallback. It should use empty/zero values (`command: ""`, `args: vec![]`, `timeout_seconds: 0`) — this is never hit in practice because `BackendConfigs::default()` provides the real defaults. If a user writes `[backends]` with only `[backends.claude]`, the codex backend gets `BackendConfig::default()` which is inert.

### B. `GlobalConfig::load()` — no changes

`GlobalConfig::load()` is **not modified**. It continues to propagate `io::ErrorKind::NotFound` as an error when `ralph.toml` is missing on disk. This preserves the invariant that an existing `.ralph` directory with a deleted `ralph.toml` is an error, not a silent fallback to defaults. The auto-init path (Section C) avoids this codepath entirely by using the `Workspace` returned from `Workspace::init()`.

### C. Auto-init in `ralph auto`

Extract the workspace creation + template writing logic from `init::execute()` into a reusable function:

```rust
// src/cli/init.rs
pub(crate) fn create_workspace(root: &Path) -> Result<Workspace> {
    let workspace = Workspace::init(root)?;
    let templates_dir = workspace.root.join("templates");
    fs::write(templates_dir.join("spec.md"), default_planner_template())?;
    fs::write(templates_dir.join("implementation.md"), default_implementer_template())?;
    fs::write(templates_dir.join("review.md"), default_reviewer_template())?;
    fs::write(templates_dir.join("prompt_reviewer.md"), default_prompt_reviewer_template())?;
    fs::write(templates_dir.join("completion.md"), default_completer_template())?;
    fs::write(templates_dir.join("qa.md"), default_qa_template())?;
    Ok(workspace)
}

pub fn execute(args: InitArgs) -> Result<()> {
    let workspace = create_workspace(&args.dir)?;
    println!("initialized workspace at {}", workspace.root.display());
    Ok(())
}
```

`create_workspace` returns the `Workspace` returned by `Workspace::init()` directly, avoiding the redundant write-then-read cycle of calling `Workspace::load()` after `Workspace::init()`. The `Workspace` from `init()` already holds the correct `GlobalConfig::default()` in memory.

In `src/cli/auto.rs`, extract a helper function:

```rust
fn ensure_workspace() -> Result<Workspace> {
    match Workspace::discover() {
        Ok(ws) => Ok(ws),
        Err(RalphError::WorkspaceNotFound) => {
            let root = std::env::current_dir()?.join(".ralph");
            let ws = init::create_workspace(&root)?;
            eprintln!("initialized workspace at {}", root.display());
            Ok(ws)
        }
        Err(e) => Err(e),
    }
}
```

Replace the first `Workspace::discover()?` call (line 129) with `ensure_workspace()?`.

The second `Workspace::discover()` call (line 240, after `create_project()`) is left unchanged. It is safe because: (a) the first call either discovered or created `.ralph` in `current_dir()`, so the workspace is guaranteed to exist; (b) `create_project()` writes into the existing workspace directory structure; (c) `discover()` walks up from `current_dir()` which is the same directory where `.ralph` was just created/found. The second call intentionally re-loads from disk to pick up the freshly-created project state.

### D. Refactor `GlobalConfig::default()` to use new `Default` impls

After adding `Default` impls for all nested structs, refactor `GlobalConfig::default()` to delegate:

```rust
impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            workspace: WorkspaceConfig::default(),
            backends: BackendConfigs::default(),
            workflow: WorkflowConfig::default(),
            templates: TemplateConfig::default(),
            git: GitConfig::default(),
        }
    }
}
```

This ensures the struct-level defaults and `GlobalConfig::default()` are always in sync.

## Files & Modules

| File | Change |
|---|---|
| `src/config/global.rs` | Add `#[serde(default)]` to all five `GlobalConfig` fields. Implement `Default` for `WorkspaceConfig`, `BackendConfigs`, `BackendConfig`, `WorkflowConfig`, `CommitMessageStyle`, `PromptChangeAction`, `TemplateConfig`, `GitConfig`. Refactor `GlobalConfig::default()` to delegate to the new `Default` impls. No changes to `load()` or `save()`. |
| `src/cli/auto.rs` | Add `ensure_workspace() -> Result<Workspace>` helper. Replace first `Workspace::discover()?` with `ensure_workspace()?`. Second `Workspace::discover()` unchanged. Import `init` module. |
| `src/cli/init.rs` | Extract `pub(crate) fn create_workspace(root: &Path) -> Result<Workspace>` from `execute()`. `execute()` calls `create_workspace()` then prints the success message. |

## Testing Strategy

### Unit tests in `src/config/global.rs`

1. **`empty_toml_deserializes_to_defaults`**: `toml::from_str::<GlobalConfig>("")` succeeds and all fields match `GlobalConfig::default()`.
2. **`partial_toml_deserializes_missing_sections_to_defaults`**: TOML with only `[workspace]` section (and its required fields) produces correct defaults for `backends`, `workflow`, `templates`, `git`.
3. **`partial_backend_section_uses_defaults`**: TOML with `[backends.claude]` only (no `[backends.codex]`) produces the default codex config for the missing backend.

### Unit test for `ensure_workspace` in `src/cli/auto.rs`

4. **`ensure_workspace_creates_workspace_when_missing`**: Set `current_dir` to a temp directory with no `.ralph`. Call `ensure_workspace()`. Assert `.ralph/` directory, `ralph.toml`, `projects/`, `templates/` all exist. Assert the returned `Workspace` has `config` equal to `GlobalConfig::default()`. This test exercises only the `ensure_workspace()` helper, not the full async `execute()` pipeline, making it reliable and fast.

### Unit test for `create_workspace` in `src/cli/init.rs`

5. **`create_workspace_writes_templates`**: Call `create_workspace()` on a temp directory. Assert all six template files exist under `templates/` and that the returned `Workspace` is valid.

### Existing tests

6. Verify existing `init_command` tests and `GlobalConfig` deserialization tests continue to pass unchanged.

## Out of Scope

- Auto-init for commands other than `auto` (e.g., `run`, `status`, `daemon`).
- Config file migration or versioning (e.g., upgrading old `ralph.toml` schemas).
- Interactive prompting before auto-creating the workspace.
- Gitignore management (adding `.ralph` to `.gitignore`).
- Validating that auto-created templates match the latest embedded defaults after upgrades.
- Concurrent `ralph auto` race conditions: two simultaneous invocations in the same directory may both attempt auto-init. `fs::create_dir_all` is idempotent for directories, but `Workspace::init()` rejects non-empty directories, so the second invocation will fail with a validation error. This is acceptable — concurrent workspace initialization is not a supported use case.
- Making `GlobalConfig::load()` tolerant of missing `ralph.toml` on existing workspaces. A missing `ralph.toml` in an existing `.ralph` directory remains an error to surface configuration corruption rather than silently falling back to defaults.