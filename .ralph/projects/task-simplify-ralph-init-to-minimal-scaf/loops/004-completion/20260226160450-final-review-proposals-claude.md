---
artifact: final-review-proposals
loop: 4
project: task-simplify-ralph-init-to-minimal-scaf
backend: claude
role: final_reviewer
created_at: 2026-02-26T16:04:50Z
---

I see — `setup_with_standard_mock` calls `h.init_workspace()` which now does minimal init (no templates). The `template_fallback_when_file_missing` test was changed to call `h.ralph_ok(["init", "--copy-files"])` instead, because it needs templates to exist so it can delete one and test fallback. This is correct.

I've now comprehensively reviewed all the changes. Let me summarize my findings.

# Final Review: NO AMENDMENTS

## Summary

I reviewed all source changes across 13 files (2034 diff lines) against the master branch. The implementation is correct, safe, and complete. Here are the key verifications:

**`ralph init` minimal scaffold** (`src/cli/init.rs`):
- `plan_actions_minimal()` correctly creates only `projects/` dir and writes minimal `ralph.toml` (lines 144-153).
- `minimal_config_toml()` (line 139-142) uses `GlobalConfig::default().workspace.version` with `{:?}` formatting, producing valid TOML that parses back to defaults. Test at line 352-360 confirms round-trip equivalence.
- `plan_actions_full()` correctly handles overlay mode: loads existing config via `GlobalConfig::load()` before any fs mutations, producing `MergeConfig` action for existing configs and `SkipExisting` for existing templates (lines 155-190). This ensures invalid TOML fails early before any partial writes.
- `validate_target()` logic (line 124-126) correctly allows `--copy-files` overlay only when `ralph.toml` exists, and rejects all non-empty dirs for minimal init. Verified through boolean trace of all 4 cases.

**Bootstrap call sites** (`src/cli/auto.rs:105`, `src/daemon/bootstrap.rs:69`, `src/validate/harness.rs:342`):
- All three correctly pass `copy_files: false` for minimal init behavior.

**Sparse config writes** (`src/config/global.rs:1202-1224`):
- `save_config_sparse()` validates via `set_global_config_value()` on a cloned config *before* any file mutation. Only writes on success. The validation-before-mutation pattern ensures no partial writes on failure.
- `key_becomes_none()` (lines 1227-1247) correctly handles all optional fields that can be set to `null`/`none`.
- `split_global_config_key_segments()` (lines 1119-1194) preserves dotted dynamic suffixes for all 9 backend dynamic key patterns (`env`, `models`, `role_timeouts` × 3 backends).
- `ensure_table_path_in_table`/`ensure_table_path_in_inline_table` correctly handle both standard and inline TOML tables, preserving existing table types during navigation.
- Triple alias normalization in the `execute_set` flow (config.rs → save_config_sparse → set_global_config_value) is safe because `normalize_global_config_key_alias` is idempotent.

**CLI changes** (`src/cli/mod.rs:53-54`, `src/cli/config.rs:307-314`):
- `copy_files: bool` added to `InitArgs` with `#[arg(long)]` (defaults false).
- `execute_set` for global scope correctly uses sparse path and reloads config from disk after write.
- `set_global_value` correctly marked `#[cfg(test)]` since production path now uses `save_global_value_sparse`.

**`Workspace::init`** (`src/workspace/mod.rs`): Zero changes — signature and behavior preserved.

**`GlobalConfig::save()`** (`src/config/global.rs:1076-1080`): Retained, unchanged.

**Tracing writer change** (`src/main.rs:10`): `.with_writer(std::io::stderr)` is a necessary fix to prevent tracing logs from contaminating stdout, which conformance tests check via `assert_stdout_eq()`.

**Test coverage**: Comprehensive across unit, integration, and conformance tests:
- Minimal vs copy-files action plans and filesystem results (8 tests in `init.rs`)
- Overlay validation with edge cases including invalid TOML (4 tests)
- Dry-run output contract for both modes (3 conformance tests in `tests_init.rs`)
- Sparse write preservation of comments/format/unknown keys (1 unit + 1 conformance test)
- Rejection without mutation on invalid key/value/TOML (3 unit + 1 conformance test)
- Table-driven key coverage test exercising all keys against sparse save (1 comprehensive test)
- Dotted dynamic suffix handling (2 unit + 1 conformance test)
- Alias path correctness (1 unit test)
- Template fallback test correctly updated to use `--copy-files` for setup

**No stray files**: `git status` shows clean working tree.
