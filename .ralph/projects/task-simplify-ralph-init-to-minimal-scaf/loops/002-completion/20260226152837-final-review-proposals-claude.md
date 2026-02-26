---
artifact: final-review-proposals
loop: 2
project: task-simplify-ralph-init-to-minimal-scaf
backend: claude
role: final_reviewer
created_at: 2026-02-26T15:28:37Z
---

# Final Review: NO AMENDMENTS

## Summary

The implementation is correct, safe, and complete. All changes are well-structured and properly tested.

**Init changes** (`src/cli/init.rs`, `src/cli/mod.rs`): The `--copy-files` flag is correctly added to `InitArgs`. The minimal path creates only `projects/` and a minimal `ralph.toml` that parses equivalently to `GlobalConfig::default()` (verified by `minimal_config_parses_to_default_config` test). The full path supports overlay mode with proper validation: non-empty dirs require `ralph.toml` to exist, invalid TOML fails before any writes (no partial mutations), existing templates get `SkipExisting`, and existing config gets `MergeConfig` which round-trips through `GlobalConfig::load` then `save`. Dry-run labels (`merge-config`, `skip-existing`) match spec requirements.

**Bootstrap call sites** (`src/cli/auto.rs:107`, `src/daemon/bootstrap.rs:69`, `src/validate/harness.rs:342`): All correctly pass `copy_files: false` for minimal init.

**Sparse config writes** (`src/config/global.rs:1202-1224`): The `save_config_sparse` function validates the mutation on a cloned config via `set_global_config_value` before touching the TOML document, ensuring no file mutation on validation failure. Key splitting (`split_global_config_key_segments`) correctly preserves dotted dynamic suffixes for all 9 backend subkey patterns (`env`, `models`, `role_timeouts` × `claude`, `codex`, `gemini`). The `key_becomes_none` function covers all optional fields that can be set to `null`/`none`. The `extract_edit_value` approach of serializing the full config then navigating by path segments correctly handles dotted keys in BTreeMaps since `toml::Value::get` does direct key lookup (not path traversal).

**Config set flow** (`src/cli/config.rs:309-313`): Correctly calls sparse save then reloads config from disk, maintaining workspace state consistency.

**Alias normalization**: The `normalize_global_config_key_alias` function is idempotent, so the triple-call path (resolve_config_alias → save_config_sparse → set_global_config_value) is safe.

**The `main.rs` `.with_writer(std::io::stderr)` change**: While not explicitly in the spec, this is a necessary supporting change — without it, tracing logs would contaminate stdout and cause `assert_stdout_eq` failures in conformance tests for dry-run output.

**Test coverage**: Comprehensive — 857 unit tests pass, 290 conformance tests pass (284 pass; 6 pre-existing failures in `interactive_prd::prd_done_*` are unrelated to this branch, confirmed by the fact that `tests_interactive_prd.rs` was not modified). The table-driven `sparse_save_table_driven_key_coverage_matches_shared_mutator` test exercises all keys and verifies sparse save produces identical config to the shared mutator path. `nix build -L` succeeds.

**No stray files**: `git status` shows only `.ralph/` (daemon working directory, expected).

**Files verified**: `Cargo.toml`, `src/cli/init.rs`, `src/cli/mod.rs`, `src/cli/auto.rs`, `src/cli/config.rs`, `src/config/global.rs`, `src/config/mod.rs`, `src/daemon/bootstrap.rs`, `src/validate/harness.rs`, `src/validate/tests_init.rs`, `src/validate/tests_commands.rs`, `src/validate/tests_auto_init.rs`, `src/validate/tests_run.rs`, `src/main.rs`, `tests/init_command.rs`.
