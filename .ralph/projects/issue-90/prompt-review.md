---
artifact: prompt-review
project: issue-90
backend: codex
role: prompt_reviewer
created_at: 2026-03-03T02:13:57Z
---

# Prompt Review

## Issues Found
- The target path is ambiguous (`.ralph/...` in requirements vs `root.join("ralph.toml")` in approach), which can lead to writing files in the wrong directory.
- `--copy-files` mode selection is underspecified: checking only `ralph.toml.exists()` can produce wrong behavior/error text for non-empty non-workspace directories.
- Overlay merge rules are incomplete for partially present nested tables and dynamic maps, so different implementations may overwrite user values differently.
- Alias handling is not wired to sparse persistence (`planner_backend`, `qa_backend`), risking in-memory updates that are not written to the canonical TOML key.
- The proposed `serde(skip_serializing_if)` changes can alter full-config serialization and conflict with “full scaffold writes fully populated config.”
- “Null/default-sentinel/None” clearing semantics are mixed and not normalized, which makes `config set` behavior hard to test consistently.
- Error message expectations are inconsistent (some exact, some contains), creating flaky conformance tests.
- “All 102+ keys continue to work” is stated, but no explicit regression mechanism is defined.
- Dry-run output expectations do not fully pin ordering and path format, which can cause false negatives in validate tests.

## Refined Prompt
## Goal
Implement two behavior changes with strict backward compatibility:
1. `ralph init` becomes minimal by default.
2. `ralph config set --global` writes sparse in-place TOML updates using `toml_edit` while preserving formatting/comments.

## Definitions
- `ROOT` = directory passed to `ralph init` (`InitArgs.dir`).
- Workspace files are under `ROOT`: `ROOT/ralph.toml`, `ROOT/projects/`, `ROOT/templates/`.
- “Full scaffold” = projects dir + templates dir + 11 template files + fully populated config file.
- “Minimal scaffold” = `ROOT/projects/` + minimal `ROOT/ralph.toml` only.

## Functional Requirements

### 1) `ralph init` default behavior (minimal)
- `ralph init` must create only:
  - `ROOT/projects/`
  - `ROOT/ralph.toml` containing:
    - guidance comments
    - `[workspace]`
    - no explicit config keys
- `ROOT/templates/` and template files must not be created.
- Minimal TOML must deserialize via `toml::from_str::<GlobalConfig>()` to `GlobalConfig::default()`.

### 2) `ralph init --copy-files` behavior
- On new/empty target: create full scaffold.
- On existing workspace (`ROOT/ralph.toml` exists and is a regular file): perform overlay.
- On existing non-empty directory without `ROOT/ralph.toml`: exit code `2` with message exactly:
  - `directory exists but is not a ralph workspace (no ralph.toml found)`
- If `ROOT/ralph.toml` exists but is not parseable TOML for `GlobalConfig`: exit code `1`, message must include:
  - `failed to parse ralph.toml`

### 3) Overlay semantics (`ralph init --copy-files` on existing workspace)
- Templates:
  - Ensure `ROOT/templates/` exists.
  - Create only missing template files.
  - Do not overwrite existing template files.
- Config merge:
  - Parse existing config with `toml_edit::DocumentMut`.
  - Build a default reference document from `GlobalConfig::default()` using existing full serialization format.
  - Insert only missing keys from default into existing doc.
  - Never overwrite existing user-provided values.
  - Preserve unknown user keys.
  - Preserve comments/formatting where possible through `toml_edit`.
- Resulting effective config (after deserialization) must match full defaults plus user overrides.

### 4) Dry-run behavior
- `ralph init --dry-run` shows only minimal actions (`create-dir projects`, `write-config ralph.toml`).
- `ralph init --copy-files --dry-run` shows full/overlay actions including template-related actions.
- Dry-run must execute no filesystem writes.

### 5) Bootstrap behavior
- `ralph auto` workspace bootstrap must use minimal init path (no template creation).
- Daemon bootstrap must use minimal init path (no template creation).
- `Workspace::init()` signature and behavior stay unchanged (test-only compatibility).

## Global Config Sparse Writes (`ralph config set --global`)

### 6) Persistence method
- Keep existing key parsing/validation/mutation in `set_global_config_value()`.
- Replace global-save call path with `save_sparse(path, key, config)` using `toml_edit`.
- `save_sparse` must patch only the targeted key in place.

### 7) Key resolution and aliases
- Sparse writer must use canonical key paths.
- Aliases must continue to work:
  - `planner_backend` -> `workflow.planner_backend`
  - `qa_backend` -> `workflow.qa_backend`
- Rejected keys must remain rejected:
  - `workspace.daemon_prd_*`

### 8) Clearing semantics
- For clearable optional fields, when user sets `null`, remove the TOML key from disk (do not write empty value).
- For non-optional fields, always write the explicit value even if equal to default.
- Do not require global serde serialization changes that would alter full scaffold output format.

### 9) Dynamic dotted key handling
- `backends.<backend>.env.<rest>`:
  - Treat `<rest>` as one literal map key, even if it contains dots.
  - Example: `backends.claude.env.MY.DOTTED.KEY` writes key `"MY.DOTTED.KEY"` under `[backends.claude.env]`.
- `backends.<backend>.models.<role>` and `backends.<backend>.role_timeouts.<role>`:
  - Split as normal dotted path.
  - Remove key when set to `null`.

### 10) Fallback/template behavior
- `render_template_with_fallback()` behavior must remain unchanged and continue to work when template files are absent.
- `Workspace::load()` must work with minimal `ROOT/ralph.toml`.

## Implementation Targets
- `Cargo.toml`: add `toml_edit`.
- `src/cli/mod.rs`: add `InitArgs.copy_files`.
- `src/cli/init.rs`:
  - add minimal TOML constant
  - split minimal/full planning
  - add overlay validation/planning
  - add action variants for minimal/overlay config writes
  - update execute branching and dry-run handling
  - update bootstrap helper to minimal plan
- `src/config/global.rs`:
  - add `save_sparse`
  - keep full-save path intact for callers that need full serialization
  - add unit tests for sparse write behavior and minimal parse
- `src/cli/config.rs`: global `config set` path calls sparse save.
- Validate tests:
  - update init tests for minimal default
  - add/adjust `--copy-files` new/overlay/error/dry-run tests
  - update auto-init expectations (no templates by default)
  - update run fallback test to not delete non-existent template

## Test Requirements

### Unit tests
- Minimal TOML parses to `GlobalConfig::default()`.
- Sparse write preserves comments/unrelated formatting.
- Sparse write creates intermediate tables as needed.
- Sparse write removes optional key on `null`.
- Sparse write handles `backends.*.env.*` dotted literal keys correctly.
- Sparse write handles models/role_timeouts clear semantics correctly.

### Conformance tests
- `init` default is minimal and dry-run reflects minimal actions.
- `init --copy-files` full scaffold on new/empty target.
- `init --copy-files` overlay:
  - preserves customized values
  - fills missing keys
  - creates missing templates only
- Correct error code/message for:
  - non-workspace non-empty directory
  - malformed `ralph.toml`
- Auto/daemon bootstrap remains minimal.
- Template fallback test still passes with missing files by default.

## Non-Goals
- No behavior changes for project-scoped config persistence.
- No changes to template contents.
- No migration tool for existing workspaces.
- No changes to `config show/get/edit` semantics.

## Done Criteria
- All acceptance criteria above are met.
- New/updated conformance tests pass.
- Existing `cargo test` and validate suite pass.
- No regression in config key support or alias handling.
