---
artifact: final-review-proposals
loop: 7
project: issue-90
backend: claude
role: final_reviewer
created_at: 2026-03-03T04:03:04Z
---

# Final Review: NO AMENDMENTS

## Summary

The implementation is correct, complete, and safe. All functional requirements from the master prompt are met. Here is the detailed verification:

**Files Changed (13 total):**
- `Cargo.toml` / `Cargo.lock` — adds `toml_edit = "0.22"` dependency
- `src/cli/mod.rs` — adds `copy_files: bool` field to `InitArgs`
- `src/cli/init.rs` — core implementation: minimal/full/overlay planning and execution
- `src/config/global.rs` — `save_sparse()` function and helpers for in-place TOML patching
- `src/config/mod.rs` — re-exports `save_sparse`
- `src/cli/config.rs` — global `config set` now calls `save_sparse` instead of `workspace.save_config()`
- `src/cli/auto.rs` — test updated: asserts `!templates.exists()` for auto-init
- `src/validate/tests_init.rs` — 19 conformance tests covering all init behaviors
- `src/validate/tests_auto_init.rs` — 5 conformance tests for auto bootstrap (asserts minimal, no templates)
- `src/validate/tests_commands.rs` — 5 new conformance tests for sparse config set behavior
- `src/validate/tests_run.rs` — template fallback test updated for missing-template-by-default
- `tests/init_command.rs` — integration tests updated with `copy_files` field

**Correctness verification:**

1. **Minimal init** (`src/cli/init.rs:223-232`): `plan_minimal_actions()` creates only `projects/` dir and writes `MINIMAL_TOML`. The `MINIMAL_TOML` constant contains `[workspace]` header with comments, no explicit keys. Unit test `minimal_toml_parses_to_default()` at `src/config/global.rs:1878` confirms it deserializes to `GlobalConfig::default()`.

2. **`--copy-files` full scaffold** (`src/cli/init.rs:198-221`): `plan_full_actions()` creates `projects/`, `templates/`, writes full config, and all 11 template files using the shared `TEMPLATE_FILES` constant.

3. **`--copy-files` overlay** (`src/cli/init.rs:236-270, 275-351`): `plan_overlay_actions()` skips existing dirs/templates. `merge_overlay_config()` uses `toml_edit::DocumentMut` to merge default keys into existing doc without overwriting user values. The recursive `merge_tables()` function handles both regular tables and inline tables correctly, converting inline to regular when needed for nested merging.

4. **Error handling** for `--copy-files`: Non-workspace non-empty dir returns `RalphError::Validation(...)` → exit code 2. Malformed TOML returns `RalphError::Orchestration(...)` → exit code 1. Both messages match spec requirements.

5. **Dry-run**: Both minimal and full/overlay paths check `args.dry_run` before calling `create_workspace_from_actions()`, ensuring no filesystem writes.

6. **Bootstrap paths**: `auto.rs:110` and `daemon/bootstrap.rs:69` both call `init::create_workspace()` which uses `plan_minimal_actions()` — no templates created.

7. **`Workspace::init()`** (`workspace/mod.rs:84-95`): Unchanged — still creates `templates/` and full config. Used only by integration tests. Backward compatible.

8. **`save_sparse()`** (`src/config/global.rs:1154-1197`): Reads existing file, parses as `DocumentMut`, serializes full config to extract target value, then patches only the targeted key. Missing file falls back to full save. Error paths don't write to disk. Handles inline table conversion, intermediate table creation, and optional key removal.

9. **`sparse_key_segments()`** (`src/config/global.rs:1206-1226`): Correctly treats `backends.<backend>.env.<rest>` as a literal key. Models and role_timeouts split normally.

10. **`config set --global`** (`src/cli/config.rs:313-317`): Uses `resolve_config_alias()` for aliases, calls `set_global_value()` for in-memory mutation, then `save_sparse()` for disk write.

**Safety verification:**
- No concurrency/parallel code changes — all operations are single-threaded sequential
- No panic paths that could leave inconsistent state — error propagation via `?` prevents partial writes
- No resource leaks — all file I/O uses standard `fs::write` / `fs::read_to_string`
- `save_sparse` error path verified by test at line 3511: file unchanged on error

**Test coverage:**
- 855 unit tests, 0 failures
- All integration tests pass
- 19 init conformance tests, 5 auto-init tests, 5 sparse config-set tests
- Zero compiler warnings
- No stray files in the repo
