## Summary

Refactor `ralph init` so the default behavior creates a **minimal workspace**: only `projects/` and a sparse `ralph.toml` (no `templates/` directory, no template files). The current full-scaffold behavior (templates directory, 11 template files, fully-populated config) moves behind a `--copy-files` flag. The same minimal behavior applies to auto-bootstrap (`ensure_workspace` in `auto.rs`) and daemon bootstrap (`ensure_workspace_initialized` in `bootstrap.rs`). Separately, change `ralph config set --global` to perform **sparse TOML writes** using `toml_edit`, patching only the target key in-place and preserving comments, formatting, and unset keys.

## Acceptance Criteria

1. `ralph init` (no flags) creates only `projects/` and a minimal `ralph.toml` — no `templates/` directory or template files
2. The minimal `ralph.toml` parses successfully via `toml::from_str::<GlobalConfig>()`, yielding the same defaults as `GlobalConfig::default()`
3. `ralph init --copy-files` creates the full scaffold: `projects/`, `templates/`, all 11 template files, and a fully-populated `ralph.toml` (via `GlobalConfig::default().save()`)
4. `ralph init --copy-files` on an existing workspace (directory containing `ralph.toml`) performs value-only overlay: loads existing config into `GlobalConfig`, re-serializes via `save_config()` (filling missing defaults for known schema fields), writes only missing template files. Comments, formatting, and any unrecognized TOML keys in the original file are **not** preserved — this is a known limitation of the overlay, since it uses `toml::to_string_pretty` (the same serializer used by all other full-save paths). Users who want comment/formatting preservation should use `ralph config set` (which uses the sparse-write path) rather than re-running `init --copy-files`
5. `ralph init --copy-files --dry-run` prints overlay actions without error, including on existing workspaces (prints `skip-existing` for template files already present, `merge-config` for existing `ralph.toml`)
6. `ralph init` (no flags) on a non-empty directory still fails with the existing validation error
7. `ralph auto` bootstrap uses minimal init behavior (no templates directory or files)
8. Daemon bootstrap (`ensure_workspace_initialized`) uses minimal init behavior (no templates directory or files)
9. `ralph config set --global <key> <value>` performs sparse writes via `toml_edit`: reads the on-disk file into a `DocumentMut`, patches only the target key, writes back
10. Sparse writes preserve TOML comments, formatting, and keys not being set
11. All existing config key validation semantics preserved — every key accepted by `set_global_config_value()` works, `daemon_prd_*` keys remain rejected (they are not handled by `set_global_config_value`), aliases (`planner_backend` → `workflow.planner_backend`) resolve correctly. Dynamic suffixes under `backends.<name>.env.*`, `backends.<name>.models.*`, and `backends.<name>.role_timeouts.*` continue to accept arbitrary suffix strings including those containing dots (e.g., `backends.claude.env.FOO.BAR` sets env key `FOO.BAR`)
12. Template fallback continues to work: `load_template_source()` in `prompts/template_introspection.rs` already returns compiled-in defaults when on-disk template files are missing
13. `Workspace::load` works with the minimal `ralph.toml` (all fields have `#[serde(default)]`)
14. `Workspace::init` signature and behavior unchanged (used by 16+ test call sites)
15. Existing `GlobalConfig::save()` method preserved for full-serialization use cases (overlay merge, `Workspace::save_config()` when called outside sparse path)

## Technical Approach

### 1. Add `--copy-files` flag to `InitArgs`

In `src/cli/mod.rs`, add a `copy_files` field to `InitArgs`:

```rust
#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long, default_value = ".ralph")]
    pub dir: PathBuf,
    #[arg(short = 'n', long = "dry-run")]
    pub dry_run: bool,
    #[arg(long)]
    pub copy_files: bool,
}
```

### 2. Add `WriteMinimalConfig` action and parameterize `plan_actions`

In `src/cli/init.rs`:

**New action variant:**
```rust
enum InitAction {
    CreateDir { path: PathBuf },
    WriteConfig { path: PathBuf },           // full config via GlobalConfig::default().save()
    WriteMinimalConfig { path: PathBuf },     // sparse config with only [workspace] section
    WriteTemplate { path: PathBuf, content: &'static str },
}
```

**Minimal TOML constant** — define `MINIMAL_TOML` containing a `[workspace]` header with `version = "1.0"` (the only field needed to parse; all other fields derive from `#[serde(default)]`). Include brief comments pointing users to `ralph config set` and `ralph init --copy-files`.

**Parameterize `plan_actions`:**
```rust
pub(crate) fn plan_actions(root: &Path, copy_files: bool) -> Vec<InitAction>
```
- When `copy_files == false`: returns `[CreateDir(projects/), WriteMinimalConfig(ralph.toml)]` — 2 actions
- When `copy_files == true`: returns the current full list — `CreateDir(projects/)`, `CreateDir(templates/)`, `WriteConfig(ralph.toml)`, plus 11 `WriteTemplate` actions

**Update `execute_actions`** to handle `WriteMinimalConfig`:
```rust
InitAction::WriteMinimalConfig { path } => fs::write(path, MINIMAL_TOML)?,
```

### 3. Add overlay validation and overlay-safe `--copy-files`

Add `validate_target_for_overlay(root: &Path) -> Result<()>`:
- If directory doesn't exist or is empty → allow (same as `validate_target`)
- If directory exists and contains `ralph.toml` → allow (overlay mode)
- If directory exists, is non-empty, but has no `ralph.toml` → reject with existing error

Update `execute()` in `init.rs`:
- Non-copy-files: uses `validate_target(root)` (existing strict validation — rejects non-empty dirs)
- Copy-files: uses `validate_target_for_overlay(root)` (allows existing workspace)
- In overlay mode: load existing config via `GlobalConfig::load()`, re-save via `save()` (fills in missing defaults for known schema fields), write only templates that don't already exist

**Overlay preservation semantics:** The overlay uses `GlobalConfig::load()` → `save()`, which round-trips through `serde`. This means:
- All known config **values** set by the user are preserved (they survive the deserialize → serialize round-trip)
- Missing keys are filled in with defaults (the purpose of overlay)
- TOML comments, custom formatting, and any keys not in the `GlobalConfig` schema are **lost** — this is the same behavior as the existing `workspace.save_config()` path and is acceptable because `init --copy-files` is a deliberate scaffolding operation, not an incremental edit

For **dry-run overlay** output, `print_actions` will display:
- `merge-config .ralph/ralph.toml` instead of `write-config` when `ralph.toml` already exists
- `skip-existing .ralph/templates/foo.md` for templates already on disk
- `write-template .ralph/templates/bar.md` for missing templates

### 4. Parameterize `create_workspace`

```rust
pub(crate) fn create_workspace(root: &Path, copy_files: bool) -> Result<Workspace>
```

Update all callers:
- `auto.rs` `ensure_workspace()`: `create_workspace(&path, false)` — minimal
- `daemon/bootstrap.rs` `ensure_workspace_initialized()`: `create_workspace(&workspace_root, false)` — minimal
- `validate/harness.rs` `init_workspace_fast()`: `create_workspace(&ralph_root, false)` — minimal
- `init::execute()`: `create_workspace_from_actions(&args.dir, &actions)` — continues using the planned actions directly

### 5. Sparse config writes with `toml_edit`

**Add dependency** to `Cargo.toml`:
```toml
toml_edit = "0.22"
```

**Add `save_config_sparse()` in `src/config/global.rs`:**

```rust
pub(crate) fn save_config_sparse(
    toml_path: &Path,
    key: &str,
    raw_value: &str,
) -> Result<()>
```

Implementation:
1. **Validate** — clone current config via `GlobalConfig::load(toml_path)`, call `set_global_config_value(&mut clone, key, raw_value)?` to validate. This preserves all existing validation semantics (type parsing, backend validation, alias resolution, `daemon_prd_*` rejection). If validation fails, error propagates before any file mutation.
2. **Read document** — read the on-disk TOML file into `toml_edit::DocumentMut`.
3. **Determine typed value** — after mutation, inspect the clone to determine whether the key should be removed (e.g., `null` → `None` for optional fields) or what the new typed value is.
4. **Split key into TOML path segments** — use key-aware segment splitting (see §6). Walk the document, creating intermediate tables as needed.
5. **Patch** — set or remove the value in the `DocumentMut`.
6. **Write back** — serialize `DocumentMut` to string and write to disk.

**Update `execute_set` in `src/cli/config.rs`:**

For `ConfigScope::Global`, replace:
```rust
set_global_value(&mut workspace.config, key, raw_value)?;
workspace.save_config()?;
```
with:
```rust
let toml_path = workspace.root.join("ralph.toml");
save_config_sparse(&toml_path, key, raw_value)?;
// Reload workspace config to reflect the change in memory
workspace.config = GlobalConfig::load(&toml_path)?;
```

Project-scoped `config set` is unchanged — it continues using the existing full-serialization path via `ProjectConfig::save()`.

### 6. Key-aware segment splitting for dynamic paths

Config keys use dot-separated paths, but some segments contain dynamic sub-keys that may themselves contain dots. The three dynamic patterns are:
- `backends.<name>.env.<var>` — the `<var>` suffix can contain dots (e.g., `FOO.BAR`)
- `backends.<name>.models.<role>` — the `<role>` suffix is a fixed set (planner, implementer, etc.) and never contains dots, but uses the same splitting logic for consistency
- `backends.<name>.role_timeouts.<role>` — same as models

Because `set_global_config_value()` uses `key.starts_with("backends.claude.env.")` with `trim_start_matches` to extract the suffix (including any dots in the suffix as a single value), the segment splitter must replicate this behavior:

```rust
fn split_config_key(key: &str) -> Vec<&str>
```

Implementation:
- Check if `key` starts with any of the 9 known dynamic prefixes (`backends.{claude,codex,gemini}.{env,models,role_timeouts}.`)
- If matched: return exactly 4 segments — `["backends", "<name>", "{env|models|role_timeouts}", "<everything-after-prefix>"]`. The fourth segment preserves dots in the suffix (e.g., for `backends.claude.env.FOO.BAR`, segments are `["backends", "claude", "env", "FOO.BAR"]`)
- If not matched: simple `key.split('.')` — works for all static keys since none contain dots in their values

This approach correctly handles:
- `backends.claude.env.ANTHROPIC_API_KEY` → `["backends", "claude", "env", "ANTHROPIC_API_KEY"]`
- `backends.claude.env.FOO.BAR` → `["backends", "claude", "env", "FOO.BAR"]` (dotted env var name preserved as single segment)
- `backends.claude.models.planner` → `["backends", "claude", "models", "planner"]`
- `workflow.planner_backend` → `["workflow", "planner_backend"]` (simple split)

### 7. Leave `Workspace::init` unchanged

`Workspace::init` (in `src/workspace/mod.rs:84`) is called by 16+ test call sites. It creates `projects/`, `templates/`, and writes full config. No signature or behavior change — this avoids a large test migration. Production paths all go through `create_workspace()`.

### 8. Update `template_fallback_when_file_missing` test

The conformance test `template_fallback_when_file_missing` in `src/validate/tests_run.rs` currently calls `setup_with_standard_mock()` which calls `h.init_workspace()` (the CLI `ralph init` subprocess). With minimal init, the `templates/` directory and `qa.md` file will not exist, so the `fs::remove_file(&qa_template)` call at line 624 will fail.

**Fix:** Change the test setup to use `ralph init --copy-files` instead of bare `ralph init`, so the template files exist for removal. This is the correct approach because the test's purpose is to verify that the runtime falls back to compiled-in defaults when a previously-existing template file is deleted — it specifically needs the full scaffold to test that scenario.

Concretely, update `setup_with_standard_mock()` (or add an alternative setup helper) to run `h.ralph_ok(["init", "--copy-files"])` for tests that need template files on disk. The `template_fallback_when_file_missing` test uses this alternative setup. Other tests that use `setup_with_standard_mock()` and don't need templates can continue using bare `init` (template fallback handles missing files transparently).

In practice, because `setup_with_standard_mock` is shared by multiple `tests_run.rs` tests and most don't care about templates, the cleanest approach is:
- Keep `setup_with_standard_mock` using bare `h.init_workspace()` (minimal)
- In `template_fallback_when_file_missing` only, add a pre-step: `h.ralph_ok(["init", "--copy-files"])` before the `fs::remove_file` call. Since `init --copy-files` on an existing workspace is an overlay (merges without error), this works even after `setup_with_standard_mock` already ran `init`.

## Files & Modules

| File | Changes |
|------|---------|
| `Cargo.toml` | Add `toml_edit = "0.22"` to `[dependencies]` |
| `src/cli/mod.rs` | Add `copy_files: bool` field (with `#[arg(long)]`) to `InitArgs` |
| `src/cli/init.rs` | Add `WriteMinimalConfig` variant to `InitAction`; define `MINIMAL_TOML` constant; change `plan_actions(root)` → `plan_actions(root, copy_files)`; add `validate_target_for_overlay()`; update `execute()` to branch on `args.copy_files`; update `create_workspace()` to accept `copy_files: bool`; add overlay merge logic and dry-run overlay output (`merge-config`, `skip-existing`) for copy-files on existing workspace |
| `src/cli/auto.rs` | Change `create_workspace(&path)` → `create_workspace(&path, false)` in `ensure_workspace()` |
| `src/daemon/bootstrap.rs` | Change `create_workspace(&workspace_root)` → `create_workspace(&workspace_root, false)` in `ensure_workspace_initialized()` |
| `src/validate/harness.rs` | Change `create_workspace(&ralph_root)` → `create_workspace(&ralph_root, false)` in `init_workspace_fast()` |
| `src/config/global.rs` | Add `save_config_sparse(toml_path, key, raw_value)` function; add `split_config_key()` helper; add `toml_edit` usage; keep existing `save()` and `set_global_config_value()` unchanged |
| `src/cli/config.rs` | Update `execute_set` for `ConfigScope::Global` to call `save_config_sparse()` instead of `set_global_value()` + `save_config()`; add `GlobalConfig::load()` reload after sparse write |
| `src/validate/tests_init.rs` | Update `creates_workspace_structure` to not assert `templates/` dir; update `creates_template_files` to run `init --copy-files`; update `dry_run_prints_actions` expected output for minimal mode; update `default_config` to assert minimal TOML parses with correct defaults via `Workspace::load`; add new tests: `init_copy_files_creates_full_structure`, `init_copy_files_overlay_preserves_custom_config`, `init_copy_files_overlay_dry_run_shows_merge_and_skip`, `dry_run_copy_files_prints_full_actions` |
| `src/validate/tests_auto_init.rs` | Update `auto_initializes_workspace_when_missing` to assert only `ralph.toml` and `projects/`, no template files; remove template assertions from other auto-init tests |
| `src/validate/tests_run.rs` | Update `template_fallback_when_file_missing` to run `init --copy-files` overlay before removing `qa.md`, so template file exists for deletion |
| `src/validate/tests_daemon.rs` | No changes needed — existing daemon bootstrap tests (`daemon_bootstrap_non_git_dir`, `daemon_bootstrap_zero_commit_repo`, etc.) only assert `.ralph` directory existence, not template files |
| `tests/init_command.rs` | Update tests that use `InitArgs` struct literal to include `copy_files: false`; update `test_init_creates_template_files_with_cli_execute` to pass `copy_files: true`; update `test_init_fails_on_existing_non_empty_workspace` and `test_init_does_not_partially_overwrite_on_failure` to include `copy_files: false`; add new tests: `test_init_minimal_creates_only_projects_and_config`, `test_init_copy_files_creates_full_scaffold`, `test_init_copy_files_overlay_preserves_values` |

## Testing Strategy

### Unit Tests (in `src/cli/init.rs::tests`)

- `plan_actions_minimal_mode`: Verify `plan_actions(root, false)` returns exactly 2 actions: `CreateDir(projects/)` and `WriteMinimalConfig(ralph.toml)`
- `plan_actions_copy_files_mode`: Verify `plan_actions(root, true)` returns 14 actions (2 dirs + 1 config + 11 templates)
- `create_workspace_minimal_writes_parseable_toml`: Create minimal workspace, verify `toml::from_str::<GlobalConfig>(MINIMAL_TOML)` succeeds and `Workspace::load()` works
- `validate_target_for_overlay_allows_existing_workspace`: Directory with `ralph.toml` passes
- `validate_target_for_overlay_rejects_non_workspace_nonempty`: Non-empty dir without `ralph.toml` fails
- `validate_target_for_overlay_allows_empty_dir`: Empty dir passes

### Unit Tests (in `src/config/global.rs::tests`)

- `sparse_save_sets_simple_key`: Set `workspace.default_backend`, verify on-disk file has only that key changed
- `sparse_save_preserves_comments`: Write a file with comments, set a key, verify comments survive
- `sparse_save_preserves_unset_keys`: Start with minimal TOML, set one key, verify no other keys appear
- `sparse_save_creates_intermediate_tables`: Set `backends.claude.timeout_seconds` on minimal file, verify `[backends.claude]` table created
- `sparse_save_handles_dynamic_env_key`: Set `backends.claude.env.FOO`, verify nested path created correctly
- `sparse_save_handles_dotted_env_key`: Set `backends.claude.env.FOO.BAR`, verify the env key is stored as `FOO.BAR` (single TOML key, not nested tables). The on-disk representation must use a quoted key: `"FOO.BAR" = "value"` under `[backends.claude.env]`
- `sparse_save_handles_dynamic_model_key`: Set `backends.claude.models.planner`, verify correct nesting
- `sparse_save_handles_dynamic_role_timeout`: Set `backends.claude.role_timeouts.qa`, verify correct nesting
- `sparse_save_null_removes_optional_key`: Set optional field to `"null"`, verify key removed from document
- `sparse_save_rejects_invalid_key`: Attempt unsupported key, verify error before file mutation
- `sparse_save_rejects_daemon_prd_keys`: `daemon_prd_*` keys still fail (they are not in `set_global_config_value`'s match arms)
- `split_config_key_tests`: Cover simple paths (`workflow.qa_enabled` → `["workflow", "qa_enabled"]`), env paths (`backends.claude.env.FOO` → 4 segments), dotted env paths (`backends.claude.env.FOO.BAR` → `["backends", "claude", "env", "FOO.BAR"]`), models paths, role_timeouts paths

**Table-driven key coverage test** — `sparse_save_all_supported_keys`: A single parameterized test that iterates over every key accepted by `set_global_config_value()` (all ~102 static keys plus representative dynamic keys for each of the 9 dynamic prefixes). For each key, the test:
1. Starts from a minimal TOML file
2. Calls `save_config_sparse(path, key, sample_value)`
3. Verifies the write succeeds without error
4. Reloads via `GlobalConfig::load(path)` and verifies the value was applied

This test uses a const array of `(key, sample_value)` tuples covering:
- All static keys (e.g., `("workspace.version", "2.0")`, `("workflow.qa_enabled", "true")`, `("git.auto_branch", "false")`, etc.)
- Representative dynamic keys: `("backends.claude.env.MY_VAR", "val")`, `("backends.codex.models.planner", "gpt-4")`, `("backends.gemini.role_timeouts.qa", "300")`
- At least one dotted dynamic key: `("backends.claude.env.DOTTED.KEY", "val")`

This catches drift between `set_global_config_value()` and `save_config_sparse()` — if a new key is added to the match arms but the sparse path can't handle it, this test fails.

### Integration / Conformance Tests (in `src/validate/tests_init.rs`)

- `creates_workspace_structure`: Updated — asserts `projects/` and `ralph.toml` only, no `templates/`
- `creates_template_files`: Renamed to `copy_files_creates_template_files` — runs `init --copy-files`, asserts all 11 templates
- `default_config`: Updated — asserts minimal TOML parses with correct defaults via `Workspace::load` (no longer asserts specific TOML field values in the file, since the minimal file only contains `[workspace]` with `version`)
- `init_copy_files_creates_full_workspace_structure`: Full scaffold including templates dir
- `init_copy_files_overlay_preserves_custom_config`: Init, modify config via `config set`, re-run `--copy-files`, verify custom value preserved in reloaded config
- `init_copy_files_overlay_dry_run_shows_merge_and_skip`: Run `init`, then `init --copy-files --dry-run`, verify stdout contains `merge-config` and no errors
- `dry_run_prints_actions`: Updated expected output for minimal mode (2 lines: `create-dir .ralph/projects`, `write-minimal-config .ralph/ralph.toml`)
- `dry_run_copy_files_prints_actions`: Full dry-run output (14 lines, matching current behavior)
- Existing `rejects_nonempty_dir`, `dry_run_rejects_*` tests remain unchanged

### Conformance Tests (in `src/validate/tests_auto_init.rs`)

- `auto_initializes_workspace_when_missing`: Updated — asserts only `ralph.toml` and `projects/`, no template files, no `templates/` directory
- `auto_on_existing_workspace_with_missing_ralph_toml_reinitializes`: Updated — remove any template assertions if present (currently none)
- Other auto-init tests: no template-related assertions to remove

### Conformance Tests (in `src/validate/tests_run.rs`)

- `template_fallback_when_file_missing`: Updated — after `setup_with_standard_mock` (which now uses minimal init), run `h.ralph_ok(["init", "--copy-files"])` to overlay the full scaffold, then `fs::remove_file(&qa_template)` proceeds as before. The test continues to verify that `ralph run` uses compiled-in defaults when a template file is missing.

### Conformance Tests (in `src/validate/tests_daemon.rs`)

- No changes needed — existing daemon bootstrap tests (`daemon_bootstrap_non_git_dir`, `daemon_bootstrap_zero_commit_repo`, `daemon_bootstrap_idempotent`, `daemon_bootstrap_existing_repo_noop`) assert only `.ralph` directory existence and commit counts, not template files or directory structure details.

### Integration Tests (in `tests/init_command.rs`)

- All tests using `InitArgs` struct literal: add `copy_files: false` field
- `test_init_creates_template_files_with_cli_execute`: Updated to pass `copy_files: true` — this test specifically validates template creation
- `test_init_creates_workspace_structure`: Unchanged — uses `Workspace::init` (not affected)
- `test_init_cli_with_absolute_path`, `test_init_cli_with_relative_path`: Updated to expect no `templates/` in default mode; add `--copy-files` variant tests for full scaffold assertions
- New: `test_init_minimal_does_not_create_templates`: Verify `execute(InitArgs { copy_files: false, .. })` creates no `templates/` directory
- New: `test_init_copy_files_overlay_preserves_values`: Init, write custom value to config, re-init with `copy_files: true`, verify custom value survives

### Existing Tests Unchanged

- `Workspace::init` test call sites (16+): Unchanged — `Workspace::init` still creates `projects/`, `templates/`, and full config
- `load_template_source` tests: Unchanged — fallback behavior already tested
- Template fallback in `tests_run.rs`: Updated as described above (overlay pre-step)

## Out of Scope

- Migrating existing workspaces (users with current full scaffolds keep them as-is)
- Project-level sparse saves (`ProjectConfig::save()` continues using full serialization)
- Additional `init` flags beyond `--copy-files` (e.g., `--template-only`, `--force`)
- Template management commands (add/remove/list templates)
- Changes to `ralph config show`, `ralph config get`, or `ralph config edit`
- New `ConfigKey` enum or general-purpose key parser abstraction
- Changing `Workspace::init` signature (test-only method, unchanged to avoid 16+ call site migration)
- Sparse writes for `daemon_prd_*` keys (they are not in `set_global_config_value`'s match arms and remain unsettable via `config set`)
- TOML-preserving merge for `init --copy-files` overlay (uses `serde` round-trip, same as existing `save_config()` — comments and unknown keys are lost, which is acceptable for a scaffolding operation)
- Dotted backend names (backend names are hard-coded as `claude`, `codex`, `gemini` — no user-defined backends exist)