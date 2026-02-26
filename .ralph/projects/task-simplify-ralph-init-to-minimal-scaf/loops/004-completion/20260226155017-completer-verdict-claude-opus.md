---
artifact: completer-verdict
loop: 4
project: task-simplify-ralph-init-to-minimal-scaf
backend: claude(opus)
role: completer
created_at: 2026-02-26T15:50:17Z
---

I've now thoroughly verified all source files. Let me compile my findings against each acceptance criterion.

# Verdict: COMPLETE

The project satisfies all requirements:

- **AC1** `ralph init` creates only `projects/` and minimal `ralph.toml`: Satisfied by `plan_actions_minimal()` in `src/cli/init.rs:144-153` which creates only `projects/` dir and `WriteMinimalConfig`. Unit test `create_workspace_minimal_writes_only_projects_and_minimal_config` confirms no `templates/` created. Conformance test `init::creates_minimal_workspace_structure` validates via CLI.

- **AC2** Minimal config parses to defaults-equivalent `GlobalConfig`: Satisfied by `minimal_config_toml()` at `init.rs:139-142` using `GlobalConfig::default().workspace.version` (no hardcoded version drift). Unit test `minimal_config_parses_to_default_config` and conformance test `init::default_minimal_config_equivalent_to_defaults` verify `toml::from_str::<GlobalConfig>() == GlobalConfig::default()`.

- **AC3** `ralph init --copy-files` creates full scaffold and full config: Satisfied by `plan_actions_full()` at `init.rs:155-190` which creates `projects/`, `templates/`, full `ralph.toml` via `GlobalConfig::default().save()`, and all 11 template files. Conformance tests `init::copy_files_creates_template_files` and `init::copy_files_writes_full_config` verify.

- **AC4** `ralph init --copy-files` overlays existing workspace: Satisfied by `plan_actions_full()` which detects existing `ralph.toml` and emits `MergeConfig` (loads config, re-saves through full serializer preserving user values) and `SkipExisting` for existing templates. `validate_target()` at `init.rs:114-137` allows non-empty dirs with `ralph.toml` when `copy_files=true`. Invalid config fails before writes. Conformance test `init::copy_files_overlay_existing_workspace` and `init::copy_files_overlay_invalid_config_fails_without_partial_writes` verify.

- **AC5** `ralph init --copy-files --dry-run` reports `merge-config` and `skip-existing`: Satisfied by `InitAction::describe()` at `init.rs:61-71` and `print_actions()`. Conformance test `init::dry_run_copy_files_overlay_prints_merge_and_skip_existing` verifies exact output including `merge-config` and `skip-existing` labels.

- **AC6** `ralph init` (no flag) still rejects non-empty non-workspace targets: Satisfied by `validate_target()` at `init.rs:114-137` - non-empty dir without `ralph.toml` returns `Validation` error regardless of `copy_files`. Unit test `validate_target_rejects_nonempty_directory_without_workspace_marker` and conformance test `init::rejects_nonempty_dir` verify.

- **AC7** `ralph auto` bootstrap initializes minimal workspace: Satisfied by `ensure_workspace()` in `src/cli/auto.rs:103-114` calling `init::create_workspace(&...join(".ralph"), false)`. Unit test `ensure_workspace_creates_workspace_when_missing` asserts no `templates/` created. Conformance test `auto_init::auto_initializes_workspace_when_missing` verifies.

- **AC8** Daemon bootstrap initializes minimal workspace: Satisfied by `ensure_workspace_initialized()` in `src/daemon/bootstrap.rs:63-71` calling `crate::cli::init::create_workspace(&workspace_root, false)`.

- **AC9** `config set --global` performs sparse in-place TOML edits: Satisfied by `execute_set()` in `src/cli/config.rs:301-327` calling `save_global_value_sparse()` which delegates to `save_config_sparse()` in `src/config/global.rs:1202-1225`. Uses `toml_edit::DocumentMut` for in-place mutation. Config is reloaded from disk after write.

- **AC10** Sparse edits preserve comments/format/unset keys: Satisfied by `save_config_sparse()` parsing existing TOML as `DocumentMut`, mutating only the targeted key path, then writing back. Unit test `sparse_save_preserves_comments_and_unknown_keys` and conformance test `config_set_global_sparse_preserves_comments` verify comments, unknown keys, and custom sections are preserved.

- **AC11** All currently supported keys/aliases still work; `daemon_prd_*` remains rejected: Satisfied by `set_global_config_value()` at `global.rs:1419-1695` which handles all keys and rejects unknown ones. Test `shared_mutator_rejects_daemon_prd_keys` explicitly verifies all 6 `daemon_prd_*` keys are rejected. Table-driven test `sparse_save_table_driven_key_coverage_matches_shared_mutator` exercises 85+ key/value pairs including aliases (`planner_backend`, `qa_backend`).

- **AC12** Dotted suffixes in dynamic backend keys remain supported: Satisfied by `split_global_config_key_segments()` at `global.rs:1119-1195` which preserves dotted suffixes for `backends.{claude|codex|gemini}.{env|models|role_timeouts}.<suffix>`. Unit test `split_global_config_key_segments_preserves_dotted_dynamic_suffix` and `sparse_save_supports_dotted_dynamic_suffixes` verify.

- **AC13** Template fallback to compiled defaults still works when files are absent: Satisfied because minimal init creates no template files, but `Workspace::load` succeeds (verified by `create_workspace_minimal_writes_only_projects_and_minimal_config`). Template path defaults point to `templates/*.md` which fall back to compiled defaults when files are absent (existing behavior, unchanged).

- **AC14** `Workspace::load` succeeds with minimal config: Satisfied by `Workspace::load()` at `workspace/mod.rs:29-40` which calls `GlobalConfig::load()`. Unit test `create_workspace_minimal_writes_only_projects_and_minimal_config` creates minimal workspace and verifies `Workspace::load(workspace_root)` succeeds.

- **AC15** `Workspace::init` signature/behavior unchanged: Verified at `workspace/mod.rs:84-95` - `Workspace::init()` still creates `projects/`, `templates/`, saves full config via `GlobalConfig::default().save()`. No changes to this method.

- **AC16** `GlobalConfig::save()` retained for full-serialization paths: Verified at `global.rs:1076-1080` - `GlobalConfig::save()` still exists using `toml::to_string_pretty()`. Used by `Workspace::init`, `MergeConfig` overlay action, and test helpers.

---
