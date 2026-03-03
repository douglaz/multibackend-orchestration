## Summary

Simplify `ralph init` from a full-scaffold operation (config + `templates/` + 11 template files) to a minimal operation that creates only `.ralph/projects/` and a minimal `.ralph/ralph.toml`. The full scaffold moves behind `ralph init --copy-files`, which also supports overlay merging on existing workspaces. Separately, `ralph config set --global` is changed to use `toml_edit` for sparse in-place writes instead of the current full-file serialization via `toml::to_string_pretty()`.

## Acceptance Criteria

- `ralph init` creates only `.ralph/projects/` and a minimal `.ralph/ralph.toml` (guidance comments + `[workspace]` header, no explicit keys)
- Minimal `ralph.toml` parses via `toml::from_str::<GlobalConfig>()` with all fields resolving to serde defaults
- No `templates/` directory or template files created by default `ralph init`
- `ralph init --copy-files` on empty/new target creates full scaffold: projects dir, templates dir, 11 template files, fully populated config
- `ralph init --copy-files` on existing workspace (detected by `ralph.toml` being a regular file) performs overlay: writes missing template files, **key-level** merges config filling in all missing keys from the default config without overwriting user-customized values
- `ralph init --copy-files` on a non-empty directory that is not a workspace (no `ralph.toml` file) exits with error code 2 and message "directory exists but is not a ralph workspace (no ralph.toml found)"
- `ralph init --copy-files` on a directory with a `ralph.toml` that fails TOML parsing exits with error code 1 and message including "failed to parse ralph.toml"
- `ralph init --copy-files --dry-run` shows all actions (including templates) without executing
- `ralph init --dry-run` (no `--copy-files`) shows only minimal actions (create-dir, write-config)
- `ralph auto` bootstrap uses minimal init behavior (no templates)
- Daemon bootstrap uses minimal init behavior (no templates)
- `ralph config set` for global scope performs sparse writes via `toml_edit` (in-place key patching)
- Sparse writes preserve comments, formatting, and keys not being set
- All 102+ keys currently handled by `set_global_config_value()` continue to work
- `workspace.daemon_prd_*` keys remain rejected by `set_global_config_value()`
- Aliases (`planner_backend` → `workflow.planner_backend`, `qa_backend` → `workflow.qa_backend`) continue to resolve
- Null/clearing semantics: when `set_global_config_value()` sets a field to its `None`/default-sentinel value, the key is removed from disk rather than written as empty
- Template fallback mechanism (`render_template_with_fallback()` in `src/prompts/templates.rs`) continues to work for absent template files
- `Workspace::load()` works correctly with minimal `ralph.toml`
- `Workspace::init()` signature and behavior unchanged (used by tests only)

## Technical Approach

### 1. Add `toml_edit` dependency

Add `toml_edit = "0.22"` to `Cargo.toml` dependencies. This crate parses TOML into a document model that preserves comments and formatting and supports in-place mutation.

### 2. Minimal `ralph.toml` content

Define a constant `MINIMAL_RALPH_TOML` in `src/cli/init.rs`:

```toml
# Ralph workspace configuration
# Run `ralph init --copy-files` to populate templates and fill in all defaults.
# Run `ralph config set <key> <value>` to configure individual settings.
# Run `ralph config show` to see the full effective configuration.

[workspace]
```

This is the minimal file written by default `ralph init`. It must parse via `toml::from_str::<GlobalConfig>()` — verified by a unit test.

### 3. Modify `ralph init` default behavior (`src/cli/init.rs`)

**Add `--copy-files` flag to `InitArgs`** in `src/cli/mod.rs`:
```rust
#[arg(long)]
pub copy_files: bool,
```

**Split `plan_actions()` into two variants:**

- `plan_minimal_actions(root: &Path) -> Vec<InitAction>` — creates `projects/` dir and writes minimal `ralph.toml` via a new `InitAction::WriteMinimalConfig` variant.
- `plan_full_actions(root: &Path) -> Vec<InitAction>` — the current behavior (projects, templates dir, full config, 11 template files). Rename existing `plan_actions()` to this.

**Add overlay planning:**

- `plan_overlay_actions(root: &Path) -> Result<Vec<InitAction>>` — for `--copy-files` on an existing workspace.
  - **Templates:** Checks which template files under `templates/` are missing and emits `WriteTemplate` for those. Always emits `CreateDir` for `templates/` (idempotent via `create_dir_all`).
  - **Config:** Performs a **key-level merge** using `toml_edit`. Loads the existing `ralph.toml` as a `toml_edit::DocumentMut`, serializes `GlobalConfig::default()` to a second `toml_edit::DocumentMut`, then walks every key in the default document. For each key absent from the existing document, inserts the default value. Keys already present in the existing document are left untouched. The result is written back via a new `InitAction::WriteOverlayConfig { path }` variant that carries the merged document text. This guarantees that running `ralph init` minimal followed by `ralph init --copy-files` produces a config with all default keys filled in, equivalent to a full-scaffold config.

**Add `InitAction::WriteMinimalConfig` and `InitAction::WriteOverlayConfig` variants:**

```rust
WriteMinimalConfig { path: PathBuf },
WriteOverlayConfig { path: PathBuf, content: String },
```

Handle `WriteMinimalConfig` in `execute_actions()` by writing `MINIMAL_RALPH_TOML`. Handle `WriteOverlayConfig` by writing the pre-computed merged content. Both describe as `write-config` in `describe()`.

**Modify `validate_target()`:**

Current behavior is unchanged: rejects non-empty directories, used for both plain `ralph init` and `ralph init --copy-files` on new targets.

**Add `validate_overlay_target(root: &Path) -> Result<()>`:**

For `--copy-files` on an existing directory:
1. Check `root` exists and is a directory (else: `InitTargetInvalid` error).
2. Check `root.join("ralph.toml")` exists **and is a regular file** via `metadata.is_file()` (else: error code 2 with "directory exists but is not a ralph workspace (no ralph.toml found)").
3. Attempt to read and parse `ralph.toml` via `toml::from_str::<GlobalConfig>()` (failure: error code 1 with "failed to parse ralph.toml: {err}").

**Update `execute()`:**

```rust
pub fn execute(args: InitArgs) -> Result<()> {
    if args.copy_files {
        let ralph_toml = args.dir.join("ralph.toml");
        if ralph_toml.exists() {
            // Overlay mode
            validate_overlay_target(&args.dir)?;
            let actions = plan_overlay_actions(&args.dir)?;
            if args.dry_run { print_actions(&actions); return Ok(()); }
            execute_actions(&actions)?;
            println!("overlay applied to {}", args.dir.display());
            Ok(())
        } else {
            // Full scaffold on new/empty dir
            validate_target(&args.dir)?;
            let actions = plan_full_actions(&args.dir);
            if args.dry_run { print_actions(&actions); return Ok(()); }
            let workspace = create_workspace_from_actions(&args.dir, &actions)?;
            println!("initialized workspace at {}", workspace.root.display());
            Ok(())
        }
    } else {
        validate_target(&args.dir)?;
        let actions = plan_minimal_actions(&args.dir);
        if args.dry_run { print_actions(&actions); return Ok(()); }
        let workspace = create_workspace_from_actions(&args.dir, &actions)?;
        println!("initialized workspace at {}", workspace.root.display());
        Ok(())
    }
}
```

**Update `create_workspace()` (used by `auto` and daemon bootstrap):**

Change to call `plan_minimal_actions()` instead of `plan_actions()`:
```rust
pub(crate) fn create_workspace(root: &Path) -> Result<Workspace> {
    validate_target(root)?;
    let actions = plan_minimal_actions(root);
    create_workspace_from_actions(root, &actions)
}
```

### 4. Sparse config writes (`src/config/global.rs`)

**Add `save_sparse()` function:**

```rust
pub fn save_sparse(path: &Path, key: &str, config: &GlobalConfig) -> Result<()>
```

This function:
1. Reads the existing `ralph.toml` (or starts from `MINIMAL_RALPH_TOML` if missing).
2. Parses it with `toml_edit::DocumentMut`.
3. Serializes the full `config` to a temporary TOML string via `toml::to_string_pretty()`, then parses that as a second `toml_edit::DocumentMut` (the "reference document").
4. Navigates the reference document using the structured key path to extract the current typed value for the key being set.
5. If the value should be removed (see clearing semantics below), removes the key from the existing document.
6. Otherwise, navigates the existing document to the same path, creating intermediate `toml_edit` tables as needed, and sets the value from the reference document.
7. Writes the existing document back to disk.

**Why serialize-then-extract instead of `extract_toml_value()`:**

This approach uses `toml::to_string_pretty()` → `toml_edit::DocumentMut` as the single source of truth for converting typed `GlobalConfig` fields to TOML values. There is no need for a parallel `extract_toml_value()` function mirroring the 102+ match arms — the key-to-field mapping lives only in `set_global_config_value()` and serde's `Serialize` impl. The serialized reference document is navigated using the same dotted key path, ensuring one source of truth that cannot drift.

**Structured key navigation:**

The dotted config key (e.g., `workflow.qa_backend`) is split into path segments to navigate `toml_edit` tables. However, for `backends.<backend>.env.<rest>` keys, the `<rest>` portion must be treated as a **single literal map key** rather than being further split on `.`. The navigation logic uses the known key prefix structure to determine the split boundary:

- Static keys (e.g., `workflow.qa_backend`): split on `.` entirely → `["workflow", "qa_backend"]`.
- `backends.<backend>.env.<rest>`: split into `["backends", backend, "env"]` as the table path, with everything after `env.` as a single literal key. This prevents `backends.claude.env.MY.DOTTED.KEY` from creating nested tables `MY` → `DOTTED` → `KEY`.
- `backends.<backend>.models.<role>`: split on `.` entirely → `["backends", backend, "models", role]` (role names contain no dots).
- `backends.<backend>.role_timeouts.<role>`: same as models.

**Clearing semantics:**

After `set_global_config_value()` mutates the in-memory config, `save_sparse()` checks whether the value should be removed from disk by comparing the relevant field against its typed default:
- For `Option<T>` fields (e.g., `workflow.planner_backend`, `backends.*.models.*`, `backends.*.role_timeouts.*`, `workspace.daemon_repo`): if the field is `None` after mutation, the key is removed from the document.
- For other fields: the key is always written (even if it matches the default), because `config set` is an explicit user action and the value should be persisted.

Detection of `Option<T>` fields is done by checking whether the key is absent in the reference document after serialization with `#[serde(skip_serializing_if = "Option::is_none")]`. If the key does not appear in the serialized reference, it means the field is `None` and should be removed from disk. For this to work, `Option<T>` fields in `GlobalConfig` that represent clearable values must have `#[serde(skip_serializing_if = "Option::is_none")]`. Audit and add this attribute to: `WorkflowConfig::{planner_backend, implementer_backend, reviewer_backend, qa_backend, completer_backend, prompt_review_backends, planner_max_prior_loops}`, `WorkspaceConfig::daemon_repo`, `BackendRoleModels::*`, `RoleTimeouts::*`.

**Modify `execute_set()` in `src/cli/config.rs`:**

For `ConfigScope::Global`, replace `workspace.save_config()` with `save_sparse()`:

```rust
ConfigScope::Global => {
    set_global_value(&mut workspace.config, key, raw_value)?;
    GlobalConfig::save_sparse(&workspace.root.join("ralph.toml"), key, &workspace.config)?;
    println!("updated global config: {key}");
    Ok(())
}
```

**Retain `save_config()` / `save()`**: The existing full-serialize path remains available for callers that need complete config writes (e.g., `Workspace::init()` which is test-only, and the `WriteConfig` action for `--copy-files` full scaffold).

### 5. Dynamic suffix keys in sparse writes

For keys with dynamic suffixes, `save_sparse()` uses prefix-aware splitting as described in §4:

- **`backends.<backend>.env.<dotted_key>`**: The table path is `["backends", backend, "env"]`. The env key (`<dotted_key>` = everything after `backends.<backend>.env.`) is used as a single literal TOML key via `table["env"][dotted_key] = value`. This preserves the flat map structure and avoids creating unintended nested tables for keys like `MY.DOTTED.VAR`.
- **`backends.<backend>.models.<role>`**: Table path is `["backends", backend, "models"]`, leaf key is `role`. For clearing: if the role's `Option<String>` is `None`, the key is removed.
- **`backends.<backend>.role_timeouts.<role>`**: Table path is `["backends", backend, "role_timeouts"]`, leaf key is `role`. For clearing: if the role's `Option<u64>` is `None`, the key is removed.

### 6. Update conformance tests (`src/validate/tests_init.rs`)

**Modify existing tests for minimal init:**

- `creates_workspace_structure`: Assert `projects/` exists and `ralph.toml` exists. Assert `templates/` directory does **not** exist.
- `creates_template_files`: Change to verify `templates/` dir does NOT exist after plain `ralph init`. Rename to `no_templates_by_default`.
- `default_config`: Change to verify minimal `ralph.toml` parses as valid `GlobalConfig` with expected defaults (check `config.workspace.default_backend == "claude"` via deserialization), without asserting specific keys are present in the raw TOML file.
- `dry_run_prints_actions`: Update expected output to show only minimal actions: `create-dir .ralph/projects` and `write-config .ralph/ralph.toml`.
- `dry_run_short_flag`: No change (compares `-n` vs `--dry-run` outputs, will auto-reflect minimal output).
- `dry_run_rejects_nonempty_dir`: No change.
- `dry_run_rejects_file_target`: No change.
- `dry_run_rejects_unreadable_target`: No change.

**Add new tests:**

- `copy_files_creates_full_scaffold`: `ralph init --copy-files` on new dir creates templates dir, all 11 template files, and fully populated config.
- `copy_files_dry_run_prints_all_actions`: `ralph init --copy-files --dry-run` output shows template actions.
- `copy_files_overlay_preserves_user_config`: Init minimal, run `ralph config set workspace.default_backend aider`, run `ralph init --copy-files`, verify `default_backend` is still `aider` in the TOML file.
- `copy_files_overlay_writes_missing_templates`: Init minimal (no templates), run `ralph init --copy-files`, verify all 11 templates created.
- `copy_files_overlay_fills_missing_keys`: Init minimal, run `ralph init --copy-files`, verify the resulting `ralph.toml` contains all default keys (e.g., `backends.claude.command`, `workflow.max_review_iterations`).
- `copy_files_rejects_non_workspace_dir`: Create a non-empty dir without `ralph.toml`, run `ralph init --copy-files`, assert exit code 2 and error message "not a ralph workspace".
- `copy_files_rejects_malformed_config`: Create a dir with an invalid `ralph.toml` (e.g., `[[[bad`), run `ralph init --copy-files`, assert exit code 1 and error message contains "failed to parse ralph.toml".

### 7. Update auto-init conformance tests (`src/validate/tests_auto_init.rs`)

**Modify existing tests:**

- `auto_initializes_workspace_when_missing`: Remove assertions for `templates/` directory and all 11 template file assertions. Assert only `ralph.toml` and `projects/` exist.
- `auto_on_existing_workspace_with_missing_ralph_toml_reinitializes`: No change needed (already only asserts `ralph.toml` exists).
- `init_behavior_unchanged_for_non_empty_target`: No change needed.
- `auto_init_prints_stderr_notice`: No change needed.
- `auto_does_not_change_other_commands_workspace_not_found_behavior`: No change needed.

### 8. Update run conformance tests (`src/validate/tests_run.rs`)

**Modify existing test:**

- `template_fallback_when_file_missing`: Currently calls `h.init_workspace()` which will now produce a minimal workspace without templates. The test then attempts `fs::remove_file(&qa_template)` which would fail since the file no longer exists. Fix: remove the `fs::remove_file` call (the template is already absent with minimal init). The test now naturally exercises the fallback path — `render_template_with_fallback()` returns the embedded default when the file is not found.

### 9. Unit test: minimal TOML roundtrip

In `src/config/global.rs` tests, add:

```rust
#[test]
fn minimal_toml_parses_to_defaults() {
    use crate::cli::init::MINIMAL_RALPH_TOML;
    let config: GlobalConfig = toml::from_str(MINIMAL_RALPH_TOML).unwrap();
    assert_eq!(config, GlobalConfig::default());
}
```

This ensures the minimal config is always parseable and equivalent to defaults.

## Files & Modules

| File | Changes |
|------|---------|
| `Cargo.toml` | Add `toml_edit = "0.22"` dependency |
| `src/cli/mod.rs:51-57` | Add `copy_files: bool` field to `InitArgs` |
| `src/cli/init.rs` | Add `MINIMAL_RALPH_TOML` constant (make `pub(crate)`); add `WriteMinimalConfig` and `WriteOverlayConfig` variants to `InitAction`; split `plan_actions()` into `plan_minimal_actions()` and `plan_full_actions()`; add `plan_overlay_actions()` and `validate_overlay_target()`; update `create_workspace()` to use minimal; update `execute()` for `--copy-files` and overlay logic; update unit tests |
| `src/config/global.rs` | Add `save_sparse()` function; add `#[serde(skip_serializing_if = "Option::is_none")]` to clearable `Option<T>` fields; add `minimal_toml_parses_to_defaults` test; add `save_sparse_*` unit tests |
| `src/cli/config.rs:311-317` | Change `execute_set()` global path to call `save_sparse()` instead of `save_config()` |
| `src/validate/tests_init.rs` | Update `creates_workspace_structure`, `creates_template_files` (rename to `no_templates_by_default`), `default_config`, `dry_run_prints_actions`; add 7 new tests for `--copy-files`, overlay, and error semantics |
| `src/validate/tests_auto_init.rs` | Update `auto_initializes_workspace_when_missing` to remove template assertions |
| `src/validate/tests_run.rs` | Update `template_fallback_when_file_missing` to remove `fs::remove_file` call |

## Testing Strategy

1. **Unit tests** (`src/config/global.rs`):
   - `minimal_toml_parses_to_defaults`: Verify `MINIMAL_RALPH_TOML` deserializes to `GlobalConfig::default()`.
   - `save_sparse_preserves_comments`: Write a TOML with comments, sparse-set a key, verify comments survive roundtrip.
   - `save_sparse_removes_none_key`: Set an `Option<T>` field to `None` (e.g., `workflow.planner_backend` → `"null"`), verify key is removed from the document.
   - `save_sparse_creates_intermediate_tables`: Sparse-set a nested key (e.g., `workflow.qa_backend`) on a minimal file, verify intermediate `[workflow]` table is created.
   - `save_sparse_handles_dynamic_env_key`: Set `backends.claude.env.MY_VAR`, verify it appears as a flat key under `[backends.claude.env]`.
   - `save_sparse_handles_dotted_env_key`: Set `backends.claude.env.MY.DOTTED.VAR` to `"x"`, verify the TOML contains a single key `"MY.DOTTED.VAR" = "x"` under `[backends.claude.env]` — **not** nested tables `MY.DOTTED.VAR`.
   - `save_sparse_clears_role_timeout`: Set `backends.claude.role_timeouts.qa` to `"null"`, verify the key is removed from disk.
   - `save_sparse_clears_backend_model`: Set `backends.claude.models.planner` to `"null"`, verify the key is removed from disk.

2. **Unit tests** (`src/cli/init.rs`):
   - Update `create_workspace_writes_all_templates` → rename to `create_workspace_writes_minimal_config` and verify minimal behavior (no templates dir, minimal TOML written).
   - Add `full_scaffold_writes_all_templates`: Call `plan_full_actions()` → `execute_actions()`, verify all 11 templates.
   - Update `plan_actions_uses_shared_constants_in_stable_order` to test both `plan_minimal_actions` (2 actions: create-dir + write-config) and `plan_full_actions` (14 actions: 2 dirs + config + 11 templates).
   - Add `validate_overlay_target_rejects_missing_toml`: Dir exists but no `ralph.toml` → error.
   - Add `validate_overlay_target_rejects_malformed_toml`: Dir with invalid TOML → error.
   - Add `validate_overlay_target_rejects_toml_as_directory`: `ralph.toml` exists as directory, not file → error.

3. **Conformance tests** (`src/validate/tests_init.rs`):
   - Update 4 existing tests for minimal init behavior: `creates_workspace_structure`, `creates_template_files` (→ `no_templates_by_default`), `default_config`, `dry_run_prints_actions`.
   - Add 7 new tests: `copy_files_creates_full_scaffold`, `copy_files_dry_run_prints_all_actions`, `copy_files_overlay_preserves_user_config`, `copy_files_overlay_writes_missing_templates`, `copy_files_overlay_fills_missing_keys`, `copy_files_rejects_non_workspace_dir`, `copy_files_rejects_malformed_config`.

4. **Conformance tests** (`src/validate/tests_auto_init.rs`):
   - Update `auto_initializes_workspace_when_missing` to remove template directory and file assertions.

5. **Conformance tests** (`src/validate/tests_run.rs`):
   - Update `template_fallback_when_file_missing` to remove `fs::remove_file` call (file already absent with minimal init).

6. **Existing test suite**: Run `cargo test` to verify no regressions in workspace loading, config set/get, auto bootstrap, daemon bootstrap, and template rendering.

## Out of Scope

- Project-scoped `config set` sparse writes (project configs already use a sparse `ProjectConfig` struct with `Option` fields — no behavior change needed).
- Changes to `config show`, `config get`, or `config edit` commands.
- Any changes to template content or the template rendering/fallback mechanism itself.
- Migration tooling for existing workspaces (existing full `ralph.toml` files remain valid).
- Changes to `Workspace::init()` (test-only API, kept as-is for backward compatibility).
- Interactive prompts during init or overlay.