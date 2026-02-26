The revised specification has been written. Here's a summary of how each review issue was addressed:

**Issue 1 — Callsite completeness:** Instead of changing `get_or_create_for_spec`'s signature (which would break 6 non-orchestrator callsites in `prd.rs`, `quick_prd.rs`, `auto.rs`, `mcp/handlers.rs`, and preload helpers), the spec now adds a **new** `get_or_create_for_role(spec, role)` method alongside the unchanged existing method. Section 8 explicitly inventories all non-orchestrator callsites with justification for why they're safe with the default timeout.

**Issue 2 — Reformatter path:** Section 6 now explicitly fixes the reformatter acquisition to use `get_or_create_for_role(&reformatter_spec, "reformatter")` instead of `registry.get()`, ensuring the reformatter timeout actually applies and the cache key works correctly with role-suffixed entries.

**Issue 3 — Role coverage ambiguity (acceptance_qa):** The Summary and Section 1 now explicitly state that `acceptance_qa` does **not** get its own timeout field. The orchestrator already passes `"qa"` as the role label for acceptance QA (the `"acceptance_qa"` string is only used for model selection via `BackendRoleModels`). Adding a timeout field would be dead config. This is also listed in Out of Scope with the rationale.

**Issue 4 — Config UX/migration behavior:** Section 2 now specifies deprecation warnings in **two** places: config load time AND `config set` time. When a user runs `ralph config set backends.claude.timeout_seconds 3600`, the value is persisted but a warning is immediately emitted. The spec explicitly states there is no precedence — the old field is purely vestigial and never used for execution.

**Issue 5 — Config surface completeness:** Section 7 now covers `config show`, `config get`, and `config set` for the new `timeouts.*` keys in both CLI (`src/cli/config.rs`) and MCP (`src/mcp/handlers.rs` `config_show`). Both files are in the Files & Modules table. Acceptance criteria include config visibility requirements.

**Issue 6 — Testing coverage gap:** The Testing Strategy now includes explicit migration plans for `tests/orchestrator.rs` (updating `timeout_seconds = 30` to `RoleTimeouts` with all fields at 30), `tests/init_command.rs` (asserting `RoleTimeouts::default()`), config CLI tests (set/get/show for `timeouts.*`), and deprecation warning tests.

**Issue 7 — Scope clarity (project-level overrides):** `ProjectTimeoutOverrides` has been **removed** entirely. The spec now targets workspace-level `ralph.toml` configuration only, matching the original requirements. Project-level timeout overrides are explicitly listed in Out of Scope with justification.