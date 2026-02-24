---
artifact: prompt-review
project: task-summary-replace-the-current-backend
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-16T16:17:56Z
---

# Prompt Review

## Issues Found
- The provided prompt is a summary of edits, not an executable implementation spec. It references “Section 1/2/6/7/8” but does not include the actual requirements text, so downstream loops cannot implement deterministically.
- The timeout schema is still underspecified. Exact key names, role list, defaults, and fallback behavior are not fully enumerated, which risks inconsistent config parsing and runtime behavior.
- Runtime mapping is ambiguous for some paths. It states orchestrator role behavior and reformatter fix, but does not define precise `get_or_create_for_role` cache-key behavior or what `get_or_create_for_spec` should do after migration.
- Deprecation behavior is not testable as written. Warning trigger rules, stream (`stderr`), and deterministic message format are not specified, making automated tests brittle.
- MCP scope is unclear. Current MCP only has `config_show`; requiring MCP `get/set` would be inconsistent unless explicitly declared out of scope.
- Test plan is incomplete for this repo’s conformance standards. Existing validate tests currently set deprecated timeout keys and must be migrated; this needs explicit requirements.

## Refined Prompt
# Feature: Role-Based Backend Timeouts with Deprecated Global Timeout Compatibility

### Goal
Implement role-specific backend execution timeouts for orchestration while keeping backward-compatible parsing and persistence for deprecated `backends.<backend>.timeout_seconds` keys.

### Scope
- Workspace-level config only (`.ralph/ralph.toml`).
- No project-level timeout overrides.
- No new MCP tools; only existing `config_show` must expose new fields.

### Required Behavior
1. Add role-specific timeout config under each backend:
   - `backends.claude.timeouts.planner`
   - `backends.claude.timeouts.implementer`
   - `backends.claude.timeouts.reviewer`
   - `backends.claude.timeouts.qa`
   - `backends.claude.timeouts.completer`
   - `backends.claude.timeouts.reformatter`
   - Same keys under `backends.codex.timeouts.*`
2. Default each role timeout to `7200` seconds.
3. Keep deprecated keys:
   - `backends.claude.timeout_seconds`
   - `backends.codex.timeout_seconds`
4. Deprecated keys must be parseable and persistable but never used for runtime execution timeout selection.
5. No precedence rules are needed because deprecated keys are vestigial for execution.

### Data Model Changes
Update `src/config/global.rs`:
1. Add `RoleTimeouts` struct with fields:
   - `planner: u64`
   - `implementer: u64`
   - `reviewer: u64`
   - `qa: u64`
   - `completer: u64`
   - `reformatter: u64`
2. `RoleTimeouts::default()` sets all fields to `7200`.
3. Add helper `for_role(&self, role: &str) -> Option<u64>` for the six roles above.
4. Add `timeouts: RoleTimeouts` to `BackendConfig`.
5. Keep `timeout_seconds` in `BackendConfig` as deprecated storage (optional, serializable only when present), and never read it in execution code.
6. Ensure TOML deserialization supports partial `[backends.<backend>.timeouts]` blocks by defaulting missing role fields.

### Backend Registry API
Update `src/backend/mod.rs`:
1. Keep existing `get_or_create_for_spec(&str)` signature unchanged.
2. Add `get_or_create_for_role(&mut self, spec: &str, role: &str) -> Result<Arc<dyn Backend>>`.
3. `get_or_create_for_role` must:
   - Parse backend spec exactly like `get_or_create_for_spec`.
   - Resolve timeout from `backends.<backend>.timeouts.<role>`.
   - Fall back to default `7200` if role is unknown.
   - Use a role-aware cache key so the same spec can exist with different role timeouts.
4. `get_or_create_for_spec` must continue using a non-role cache key and default timeout behavior (non-role callsites remain on default timeout).
5. Add/adjust backend constructors (`src/backend/claude.rs`, `src/backend/codex.rs`) to accept an explicit timeout value while preserving all existing model/args behavior.

### Orchestrator Callsite Migration
Update `src/workflow/orchestrator.rs`:
1. Use `get_or_create_for_role` for orchestrator execution callsites:
   - planning: role `"planner"`
   - implementing: role `"implementer"`
   - review: role `"reviewer"`
   - qa phase: role `"qa"`
   - completer: role `"completer"`
   - parse reformatter: role `"reformatter"`
2. Reformatter path must acquire backend via `get_or_create_for_role(&reformatter_spec, "reformatter")`, not `registry.get()`.
3. Acceptance QA must keep using role label `"qa"` for timeout selection.
4. Do not add `timeouts.acceptance_qa`.
5. Non-orchestrator callsites remain on `get_or_create_for_spec`:
   - `src/cli/prd.rs`
   - `src/cli/quick_prd.rs`
   - `src/cli/auto.rs`
   - `src/mcp/handlers.rs`
   - preload helpers in orchestrator

### Deprecated Key UX
Implement warnings in two places:
1. Config load time (`GlobalConfig::load`): if deprecated key exists for a backend, emit a warning to `stderr`.
2. `ralph config set backends.<backend>.timeout_seconds <value>`: persist value and emit warning to `stderr` immediately.

Use deterministic warning text:
`warning: backends.<backend>.timeout_seconds is deprecated and ignored at runtime; use backends.<backend>.timeouts.<role>`

### CLI and MCP Surface
1. `src/cli/config.rs`:
   - Add `config set` support for all `backends.<backend>.timeouts.<role>` keys.
   - Keep deprecated `backends.<backend>.timeout_seconds` set behavior with warning.
   - Ensure `config get` and `config show` can read/print new keys.
2. `src/mcp/handlers.rs`:
   - `config_show` global and project responses must include new timeout fields under `backends`.

### Tests (Mandatory)
Update and add tests so behavior is unambiguous and enforceable.

1. Unit/integration:
   - `src/config/global.rs` tests for defaults, partial timeout table parsing, deprecated key parsing/persistence.
   - `tests/backend.rs` tests for `get_or_create_for_role` cache separation and timeout selection.
   - `tests/orchestrator.rs` migrate timeout setup from `timeout_seconds` to role timeout fields.
   - `tests/init_command.rs` assert default role timeout structure exists.
   - `src/cli/config.rs` tests for setting/getting new timeout keys and deprecated warning behavior.
2. Conformance (`src/validate/`):
   - Migrate validate tests that currently set `backends.*.timeout_seconds` to set relevant `timeouts.*` keys (`tests_streaming.rs`, `tests_e2e_conformance.rs`).
   - Extend `tests_commands.rs` to verify `config set/get/show` for `backends.*.timeouts.*`.
   - Extend `tests_mcp.rs` `config_show` assertions to include timeout objects.
   - Add a validate case that deprecated `config set` succeeds and emits warning on `stderr`.

### Acceptance Criteria
- Orchestrator roles use role-specific timeouts.
- Reformatter timeout is applied through role-aware backend acquisition.
- Acceptance QA uses `qa` timeout and no `acceptance_qa` timeout key exists.
- Deprecated timeout keys are loadable/settable with warnings but never affect execution.
- CLI and MCP config outputs expose `timeouts.*`.
- All tests pass:
  - `nix develop -c cargo check`
  - `nix develop -c cargo test`
  - `nix build -L`
  - `./result/bin/ralph validate --bin ./result/bin/ralph`

### Out of Scope
- Project-level timeout overrides.
- New MCP `config_get` or `config_set` tools.
- Changes to backend/model selection semantics outside timeout handling.
