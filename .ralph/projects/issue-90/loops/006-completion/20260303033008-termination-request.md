---
artifact: termination-request
loop: 6
project: issue-90
backend: codex(gpt-5.3-codex-spark-xhigh)
role: planner
created_at: 2026-03-03T03:30:08Z
---

I’ll determine whether any requirements from `prompt.md` plus the latest review amendments remain unimplemented by checking the relevant implementation/test files, then I’ll return a single feature spec (or completion request) in the required format.
I found explicit existing tests and functions for inline-table handling already, so I’m now checking whether they satisfy the exact open issues (error behavior vs silent no-ops, physical insertion verification, CLI regression coverage).
I’ve confirmed many core tests for minimal init, `--copy-files`, and inline-table merges exist; next I’m checking bootstrap and fallback coverage to see whether any requirement is still uncovered.
I verified the core init/config files look substantially updated. I’ll quickly inspect `tests_auto_init.rs` and the bootstrap helper to confirm minimal bootstrap expectations are already covered.
Bootstrap and auto paths are routed through `create_workspace`, so I’m doing a final check for residual gaps in `init.rs` action planning and CLI arg wiring to ensure nothing from AM-002/FR items remains unimplemented.
I confirmed `copy-files` CLI flag and workspace bootstrap wiring are in place. I’ll verify `config set --global` now writes via sparse save and still handles aliases/clear semantics.
I found tests indicating inline-table handling and key aliasing were already strengthened, so I’m checking the exact matcher in `set_global_config_value` to verify daemon PRD keys are still rejected and not silently accepted.
I found no lingering `plan_actions` dead-code artifact in `init.rs`; just one existing non-related dead-code allowance in `config.rs`. I’ll now finalize whether that affects planned scope.
# Project Completion Request

## Rationale
All implementation and conformance requirements from `prompt.md` are already satisfied in the current codebase, and the previously identified gaps (AM-001/2/3 and FR-001/2/3) have corresponding implementations and regression coverage in the relevant modules:

- `ralph init` default is minimal (only `projects/` + minimal `ralph.toml`) and supports `--copy-files` overlay/new behavior.
- `--copy-files` overlay and validation semantics (new/empty vs non-empty workspace vs malformed TOML) are implemented with required error messages and dry-run behavior.
- Copy-files merge preserves existing values, fills missing keys, and handles inline-table configs safely.
- `config set --global` now routes to sparse in-place writes using `toml_edit`, with alias handling and daemon PRD rejection preserved.
- Clear semantics (`null`) remove optional keys; dotted env keys are treated as literal segments for `backends.<backend>.env.<rest>`.
- `auto`/`daemon` bootstrap paths use minimal init behavior.
- Stray implementation artifact is no longer present, and obsolete `plan_actions` dead code path has been removed.
- Unit and validate coverage has already been expanded to exercise both normal and inline-table edge cases.

## Summary of Work
- `src/cli/init.rs`: minimal/copy-files plan splitting, overlay merge, and dry-run action planning completed; inline-table-aware overlay merge implemented.
- `src/cli/mod.rs`: `InitArgs.copy_files` support in place.
- `src/config/global.rs`: `save_sparse` + robust table traversal (`ensure_tables`/`navigate_tables_mut`) implemented.
- `src/cli/config.rs`: `config set --global` uses sparse persistence path.
- `Cargo.toml`: `toml_edit` dependency present.
- `src/validate/tests_init.rs`, `src/validate/tests_commands.rs`, `src/validate/tests_auto_init.rs`, `src/validate/tests_run.rs` updated/extended for minimal bootstrap, copy-files behaviors, inline-table cases, and fallback/template behavior.
- `src/daemon/bootstrap.rs` and auto init path continue to initialize via minimal workspace behavior.

## Remaining Items
- None
