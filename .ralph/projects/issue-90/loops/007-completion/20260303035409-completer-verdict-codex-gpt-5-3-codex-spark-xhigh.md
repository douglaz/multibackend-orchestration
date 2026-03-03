---
artifact: completer-verdict
loop: 7
project: issue-90
backend: codex(gpt-5.3-codex-spark-xhigh)
role: completer
created_at: 2026-03-03T03:54:09Z
---

I’ll verify each requirement directly in code and tests by tracing init/config paths end-to-end, with emphasis on overlay/dry-run behavior and sparse TOML writes. I’ll start by checking the relevant CLI, config, and validate files in parallel, then I’ll assess any gaps against the master requirements.
I found the key modules and tests that should cover this: `cli/init.rs`, `cli/mod.rs`, `cli/config.rs`, `config/global.rs`, bootstrap entrypoints (`cli/auto.rs`, `daemon/bootstrap.rs`), and conformance tests in `validate/tests_init.rs` plus sparse-write assertions in `validate/tests_commands.rs`. Next I’m reading those files to verify behavior against every requirement and cross-module assumptions.
I’ve confirmed the init/config entrypoints exist and are wired. I’ll now inspect `config/global.rs` and `workspace/mod.rs` in the exact parser/serializer/sparse-write paths to validate alias handling, dotted keys, and compatibility semantics.
I’ve validated core parser and sparse persistence code paths are present. Next I’ll inspect conformance tests for the new assertions (`init` defaults, `--copy-files`, bootstrap behavior) and a couple of behavior tests that changed as a consequence (`run` fallback/template assumptions).
I’ve confirmed the init conformance suite now explicitly includes minimal/fulls/overlay/dry-run/error cases. I’ll verify `auto` and daemon bootstrap tests plus `run` template fallback assumptions to ensure defaults remain compatible with minimal init.
I’m checking the `template_fallback_when_file_missing` behavior to ensure minimal init (no default templates on disk) still uses fallback defaults as required. I’ll read that test and the fallback utility implementation.
I’ve confirmed template fallback implementation is path-based fallback (no side effects), and I’ll now validate command-level conformance tests around sparse set behavior, aliasing, and clear semantics to ensure no regression remains in CLI semantics.
I’ve confirmed conformance cases exist for comment preservation and null clears in `config --global`. I’ll do one last pass on `cli/config.rs` internals and `set_global_value` unit tests for compatibility of alias parsing and any validation edge changes.
I still need to verify exit-code mapping for the new `--copy-files` failure modes. I’ll inspect the error/runner path to ensure non-workspace vs malformed TOML map to the intended codes.
I’ll quickly inspect `cfg_set_*` helpers to ensure role-timeout/map/env parsing is still strict (including unknown role rejection) and that clear semantics for `null` values are as intended for both `models` and `role_timeouts`.
# Verdict: COMPLETE

The project satisfies all requirements:
- `1) init` minimal default: `src/cli/init.rs` defines `MINIMAL_TOML` with guidance comments + `[workspace]` only, and planner/actions split minimal vs full paths (`plan_minimal_actions`, `plan_full_actions`), with tests asserting minimal init writes only `projects/` and `ralph.toml`.
- `2) --copy-files behavior`: `src/cli/init.rs` implements `validate_copy_files_target()` and explicit branches for non-empty non-workspace (exit via `Validation`), malformed `ralph.toml` (exit via `Orchestration` with message containing `failed to parse ralph.toml`), and new/empty targets.
- `3) Overlay semantics`: `merge_overlay_config()` and `merge_tables()` use `toml_edit::DocumentMut`, merge defaults into existing docs only where keys are missing, preserve unknown keys/comments/formatting, and avoid overwriting user values.
- `4) Dry-run`: `src/cli/init.rs` action printer emits minimal-only actions by default and full/overlay actions with templates when `--copy-files` is set; `execute()` skips writes in dry-run mode.
- `5) Bootstrap minimal`: both `src/cli/auto.rs` and `src/daemon/bootstrap.rs` call the minimal init path (`create_workspace` without `--copy-files`), while `Workspace::init()` remains unchanged for compatibility.
- `6) Sparse persistence`: `src/config/global.rs` adds `save_sparse()` built on `toml_edit`, and `src/cli/config.rs` routes global set to sparse save while retaining existing parse/validation/mutation flow in `set_global_config_value()`.
- `7) Key resolution & aliases`: canonicalization remains wired (`planner_backend` → `workflow.planner_backend`, `qa_backend` → `workflow.qa_backend`) and unsupported keys (including `workspace.daemon_prd_*`) continue to be rejected by existing global key checks.
- `8) Clearing semantics`: `save_sparse()` removes keys when the canonical key resolves to `None` (clear), while required non-optional values are written explicitly through existing mutation logic even when equal to defaults.
- `9) Dynamic dotted keys`: `sparse_key_segments()` handles `backends.<backend>.env.<rest>` as a literal rest-key (supports dots inside env var names), while `backends.*.models.<role>` and `...role_timeouts.<role>` keep normal dotted path behavior and clear to remove on `null`.
- `10) Fallback/template behavior`: template loading fallback remains unchanged (`src/prompts/templates.rs`), and `Workspace::load()` resolves minimal TOML via `GlobalConfig::load`, matching requirement for minimal bootstrap compatibility.
- Conformance/unit verification scaffolding present:
  - Cargo dependency includes `toml_edit` in `Cargo.toml`.
  - `src/validate/tests_init.rs` covers minimal default, `--copy-files` new/overlay/error/dry-run scenarios, and template merge behavior.
  - `src/validate/tests_commands.rs` covers sparse-set behavior, comment/format preservation, and clear semantics.
  - `src/validate/tests_auto_init.rs` and daemon init-related tests assert minimal bootstrap behavior.
  - `src/config/global.rs` contains dedicated unit tests for sparse writes, comment preservation, missing-table creation, optional clear semantics, dotted env keys, and model/role timeout clears.
- Required cross-file compatibility checked:
  - CLI wiring in `src/cli/mod.rs` adds `copy_files`.
  - Bootstrap call sites and fallback behavior are wired to minimal init path.
  - No project-scoped config persistence path changes were introduced for project scope (`save_sparse` is global-only).
