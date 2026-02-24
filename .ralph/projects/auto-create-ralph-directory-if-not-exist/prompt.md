### Feature
Implement two changes:

1. `ralph auto` must auto-initialize a workspace when no `.ralph` exists in the current directory or any ancestor.
2. `ralph.toml` parsing must support empty and partial files by falling back to defaults.

### Required Behavior

#### 1) Auto-init only for `ralph auto`
- In `ralph auto`, when `Workspace::discover()` returns `RalphError::WorkspaceNotFound`, create a workspace at `<current_dir>/.ralph` and continue execution in the same invocation.
- Auto-init must produce the same workspace structure and default templates as `ralph init .ralph`.
- Print exactly `initialized workspace at .ralph` to **stderr** when auto-init occurs.
- If `Workspace::discover()` returns any error other than `WorkspaceNotFound`, propagate it unchanged.
- Scope is strictly `ralph auto`; other commands must keep current behavior (fail with `WorkspaceNotFound` when no workspace exists).

#### 2) Config defaults for empty/partial TOML
- `toml::from_str::<GlobalConfig>("")` and whitespace-only TOML must succeed and equal `GlobalConfig::default()`.
- Missing top-level sections must default correctly: `workspace`, `backends`, `workflow`, `templates`, `git`.
- Missing fields inside present sections must also default correctly.
- Missing backend blocks must use real backend-specific defaults (same values as `GlobalConfig::default()`), not inert empty backend values.
- Preserve current load semantics: if `.ralph` exists and `ralph.toml` is missing, loading still errors (do not silently fallback in `load()`).

### Implementation Requirements

#### `src/cli/init.rs`
- Extract reusable `pub(crate) fn create_workspace(root: &Path) -> Result<Workspace>`.
- It must:
  - call `Workspace::init(root)`,
  - write all default templates,
  - return the initialized `Workspace`.
- Keep `init::execute()` behavior unchanged except using `create_workspace()`.

#### `src/cli/auto.rs`
- Add helper `ensure_workspace() -> Result<Workspace>`:
  - `discover()` success => return workspace.
  - `WorkspaceNotFound` => create `<cwd>/.ralph` via `init::create_workspace`.
  - print stderr line exactly `initialized workspace at .ralph`.
- Replace initial `Workspace::discover()?` in `auto` flow with `ensure_workspace()?`.
- Keep all non-`auto` command behavior untouched.

#### `src/config/global.rs`
- Ensure `GlobalConfig` supports empty/partial deserialization through serde defaults.
- Add/confirm `Default` impls for nested config types and enums so section omission works.
- Align `GlobalConfig::default()` and serde-driven defaults to avoid divergence.
- For `BackendConfigs`, ensure missing `claude` or `codex` entries deserialize to backend-specific defaults matching `GlobalConfig::default()`.

### Tests (Required)

#### Unit tests
- `empty_toml_deserializes_to_defaults`
- `whitespace_toml_deserializes_to_defaults`
- `partial_sections_default_correctly`
- `missing_single_backend_uses_backend_specific_default`
- `ensure_workspace_creates_workspace_when_missing` (verifies structure + `Workspace` config)
- `create_workspace_writes_all_templates`

#### Validate conformance tests (mandatory for this feature)
Add a new validate module (for example `src/validate/tests_auto_init.rs`) and register it in `src/validate/mod.rs`:
- `auto_initializes_workspace_when_missing`
- `auto_init_prints_stderr_notice`
- `auto_does_not_change_other_commands_workspace_not_found_behavior`
- `auto_on_existing_workspace_with_missing_ralph_toml_still_errors`
- `init_behavior_unchanged_for_non_empty_target`

### Constraints
- Do not modify `GlobalConfig::load()` behavior for missing files.
- Do not add auto-init behavior to commands other than `auto`.
- Do not change `ralph init` validation behavior for non-empty directories.
- Keep error handling through existing `RalphError` patterns and exit code mapping.

### Definition of Done
- All new unit tests pass.
- Validate suite includes and passes new auto-init conformance tests.
- Existing tests continue to pass without behavioral regressions.