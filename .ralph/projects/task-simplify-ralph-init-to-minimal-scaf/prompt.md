Implement two behavior changes in `ralph` with conformance-first coverage.

### Objective
1. Change `ralph init` default behavior to create a **minimal workspace**.
2. Change `ralph config set --global` to perform **sparse TOML edits** that preserve formatting/comments/unrelated keys.

### Functional Requirements

#### 1) `ralph init` modes
- Add `--copy-files` to `InitArgs`.
- Default (`ralph init`, no flag):
  - Create only:
    - `.ralph/projects/`
    - `.ralph/ralph.toml` (minimal content)
  - Do **not** create `.ralph/templates/` or template files.
- `ralph init --copy-files`:
  - Create full scaffold:
    - `projects/`
    - `templates/`
    - all 11 template files
    - full `ralph.toml` via existing full serialization path (`GlobalConfig::default().save()` or equivalent).
- `ralph init --copy-files --dry-run`:
  - Must print planned overlay/scaffold actions without mutating disk.

#### 2) Minimal config correctness
- Minimal `ralph.toml` must parse via `toml::from_str::<GlobalConfig>()`.
- Parsed value must be equivalent to `GlobalConfig::default()`.
- Do not hardcode a version that can drift; use the current default workspace version value.

#### 3) Overlay behavior for `--copy-files`
- Validation rules:
  - Nonexistent or empty target dir: allowed.
  - Non-empty dir with `ralph.toml`: allowed (overlay mode).
  - Non-empty dir without `ralph.toml`: reject with existing non-empty validation error.
- Overlay semantics on existing workspace:
  - Load current config into `GlobalConfig`.
  - Re-save through full serializer (fills missing known defaults).
  - Preserve user-set values for known schema fields.
  - Write only missing template files (`skip-existing` for existing ones).
  - Comments/formatting/unknown TOML keys are not preserved (explicitly accepted limitation).
- If existing `ralph.toml` is invalid TOML/schema, fail with error and no partial writes.

#### 4) Bootstrap behavior
- `auto` bootstrap (`ensure_workspace`) must use **minimal** init behavior.
- daemon bootstrap (`ensure_workspace_initialized`) must use **minimal** init behavior.
- Keep `Workspace::init` signature/behavior unchanged.

#### 5) Sparse global config writes
- Add `toml_edit` dependency.
- Implement `save_config_sparse(toml_path, key, raw_value) -> Result<()>`:
  - Validate using existing `set_global_config_value()` semantics on a cloned `GlobalConfig`.
  - Use alias-normalized key path (e.g. `planner_backend` → `workflow.planner_backend`) for document mutation.
  - Edit `toml_edit::DocumentMut` in place, creating intermediate tables as needed.
  - Remove key when value semantically becomes `None` (e.g. `null` for optional fields).
  - Preserve comments, formatting, and unrelated keys.
  - On any validation/parse failure: **no file mutation**.
- Update `config set --global` flow:
  - Replace full-save path with sparse-save path.
  - Reload `workspace.config` from disk after successful sparse write.
- Project-scoped `config set` remains unchanged.

#### 6) Key splitting for dynamic suffixes
- Support all keys accepted by `set_global_config_value()`, including aliases and existing rejections (`daemon_prd_*` still rejected).
- For keys matching:
  - `backends.{claude|codex|gemini}.env.<suffix>`
  - `backends.{claude|codex|gemini}.models.<suffix>`
  - `backends.{claude|codex|gemini}.role_timeouts.<suffix>`
- Treat `<suffix>` as one terminal segment, preserving dots (e.g. `FOO.BAR` remains a single key segment).

### Required File-Level Changes
- `Cargo.toml`: add `toml_edit`.
- `src/cli/mod.rs`: add `copy_files: bool` to `InitArgs`.
- `src/cli/init.rs`:
  - support minimal vs full action planning.
  - add overlay validation mode for `--copy-files`.
  - add minimal config write action.
  - add dry-run action labels including `merge-config` and `skip-existing` where applicable.
  - parameterize `create_workspace(root, copy_files)`.
- `src/cli/auto.rs`: call `create_workspace(..., false)`.
- `src/daemon/bootstrap.rs`: call `create_workspace(..., false)`.
- `src/validate/harness.rs`: call `create_workspace(..., false)` where fast init is used.
- `src/config/global.rs`: add sparse save implementation + key splitting/normalization helper(s).
- `src/cli/config.rs`: use sparse global save path + reload config.
- Keep `GlobalConfig::save()` and `Workspace::init` available and behaviorally unchanged.

### Acceptance Criteria
1. `ralph init` creates only `projects/` and minimal `ralph.toml`.
2. Minimal config parses to defaults-equivalent `GlobalConfig`.
3. `ralph init --copy-files` creates full scaffold and full config.
4. `ralph init --copy-files` overlays existing workspace (merge config + only missing templates).
5. `ralph init --copy-files --dry-run` reports `merge-config` and `skip-existing` where applicable.
6. `ralph init` (no flag) still rejects non-empty non-workspace targets.
7. `ralph auto` bootstrap initializes minimal workspace.
8. daemon bootstrap initializes minimal workspace.
9. `config set --global` performs sparse in-place TOML edits.
10. Sparse edits preserve comments/format/unset keys.
11. All currently supported keys/aliases still work; `daemon_prd_*` remains rejected.
12. Dotted suffixes in dynamic backend keys remain supported.
13. Template fallback to compiled defaults still works when files are absent.
14. `Workspace::load` succeeds with minimal config.
15. `Workspace::init` signature/behavior unchanged.
16. `GlobalConfig::save()` retained for full-serialization paths.

### Test Requirements
- Add/update unit + conformance tests for:
  - minimal vs copy-files action plans and filesystem results,
  - overlay validation behavior,
  - dry-run outputs for both modes,
  - sparse write preservation (comments/format/unrelated keys),
  - dotted dynamic suffix handling,
  - alias path correctness,
  - rejection behavior with no file mutation on failure,
  - template fallback test setup (`init --copy-files` only where file deletion is required).
- Include a table-driven key coverage test that exercises all keys accepted by `set_global_config_value()` against sparse save path.
- Run:
  - `nix develop -c cargo check`
  - `nix develop -c cargo test`
  - `nix build -L`
  - `./result/bin/ralph validate --bin ./result/bin/ralph`