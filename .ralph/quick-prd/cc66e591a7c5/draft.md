## Summary

Replace the per-backend `timeout_seconds` field on `BackendConfig` with a per-agent-role timeout system. Currently, each backend (claude, codex) carries a single `timeout_seconds` value applied uniformly to all role invocations. The new system introduces a `[timeouts]` section in `ralph.toml` (workspace-level only) that maps agent roles (`planner`, `implementer`, `reviewer`, `qa`, `completer`, `reformatter`, `prompt_reviewer`) to individual timeout values in seconds. The per-backend `timeout_seconds` field is deprecated (still parsed but ignored with a warning). Default timeout for all roles is 7200 seconds (2 hours). Role-based timeouts are threaded through to `CliBackend` creation by passing the role name alongside the config, so the correct `Duration` is applied at the `tokio::time::timeout` call site.

The `acceptance_qa` role does **not** get its own timeout field. Acceptance QA already executes under the `"qa"` role label in the orchestrator and uses `resolve_backend_for_role(family, "acceptance_qa")` only for model selection. For timeout purposes, acceptance QA uses the `qa` timeout — consistent with how the orchestrator currently treats it as a QA phase.

## Acceptance Criteria

- [ ] `BackendConfig.timeout_seconds` is deprecated: still deserializes without error, but is no longer used for execution timeouts. A `tracing::warn!` is emitted at config load time when a non-default value is detected.
- [ ] `config set backends.*.timeout_seconds <value>` emits a `tracing::warn!` deprecation message after writing the value (value is still persisted for backward compatibility, but unused for execution).
- [ ] New `RoleTimeouts` struct added to `src/config/global.rs` with fields for each agent role (`planner`, `implementer`, `reviewer`, `qa`, `completer`, `reformatter`, `prompt_reviewer`), each defaulting to 7200 seconds.
- [ ] `GlobalConfig` gains a `timeouts: RoleTimeouts` field, deserializable from `[timeouts]` in `ralph.toml`.
- [ ] `EffectiveConfig` carries resolved `RoleTimeouts` (directly from `GlobalConfig`, no project-level overrides).
- [ ] `backend_from_config` functions in `claude.rs` and `codex.rs` accept a `timeout: Duration` parameter instead of reading `backend.timeout_seconds`.
- [ ] `BackendRegistry` provides a new `get_or_create_for_role(&mut self, spec: &str, role: &str)` method that creates backends with role-specific timeouts, while the existing `get_or_create_for_spec(&mut self, spec: &str)` continues to work with the default timeout for non-orchestrator callers.
- [ ] Orchestrator passes the agent role when obtaining backends, ensuring each phase uses its role-specific timeout.
- [ ] Reformatter parse-retry path explicitly acquires the reformatter backend via `get_or_create_for_role(spec, "reformatter")`.
- [ ] `config show` and `config get` display effective timeout values in both CLI and MCP `config_show` outputs.
- [ ] `config set timeouts.<role> <value>` is supported for all role fields.
- [ ] `[timeouts]` section in `ralph.toml` is configurable per-role (e.g., `implementer = 10800` gives implementer 3 hours).
- [ ] `nix develop -c cargo build` compiles successfully.
- [ ] Existing tests pass (including conformance/integration tests migrated to use the new timeout path); new unit tests cover timeout resolution, deprecation warning, and cache-key separation.

## Technical Approach

### 1. Add `RoleTimeouts` config struct (`src/config/global.rs`)

Add a new struct alongside `BackendRoleModels`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RoleTimeouts {
    #[serde(default = "default_role_timeout_seconds")]
    pub planner: u64,
    #[serde(default = "default_role_timeout_seconds")]
    pub implementer: u64,
    #[serde(default = "default_role_timeout_seconds")]
    pub reviewer: u64,
    #[serde(default = "default_role_timeout_seconds")]
    pub qa: u64,
    #[serde(default = "default_role_timeout_seconds")]
    pub completer: u64,
    #[serde(default = "default_role_timeout_seconds")]
    pub reformatter: u64,
    #[serde(default = "default_role_timeout_seconds")]
    pub prompt_reviewer: u64,
}

fn default_role_timeout_seconds() -> u64 {
    7200
}
```

Add a `for_role(&self, role: &str) -> Duration` method mirroring `BackendRoleModels::for_role`, returning `Duration::from_secs(self.<field>)`. Unknown roles (including `"acceptance_qa"`) return the `qa` timeout for any role containing `"qa"`, otherwise the default (7200s). In practice, the orchestrator will always pass a known role name; this is a safety fallback.

**No `acceptance_qa` field.** The orchestrator passes `"qa"` as the role for both regular QA and acceptance QA (the `"acceptance_qa"` string is only used for model resolution via `BackendRoleModels`). Adding a separate timeout field would be dead config — the orchestrator would never look it up. If a future change splits acceptance QA into its own execution role, a timeout field can be added then.

Add `timeouts: RoleTimeouts` field to `GlobalConfig`.

### 2. Deprecate `BackendConfig.timeout_seconds`

Keep the field for backward-compatible deserialization. Add deprecation warnings in two places:

**At config load time:** After parsing `GlobalConfig`, check if either backend's `timeout_seconds` differs from the default 7200. If so, emit `tracing::warn!("backend-level timeout_seconds is deprecated and ignored; use [timeouts] section instead")`.

**At `config set` time:** In `set_global_value()` (`src/cli/config.rs`), when the key matches `backends.claude.timeout_seconds` or `backends.codex.timeout_seconds`, still write the value (for backward-compatible serialization), but also emit `tracing::warn!("backends.*.timeout_seconds is deprecated and ignored; use `ralph config set timeouts.<role> <value>` instead")`. This ensures users who attempt `ralph config set backends.claude.timeout_seconds 3600` are immediately told it has no effect.

Do **not** use the backend `timeout_seconds` value for execution, even as a fallback. When both deprecated and new configs are present, the `[timeouts]` section wins unconditionally. There is no precedence — the old field is purely vestigial.

### 3. Thread role into backend creation (`src/backend/`)

Change `claude::backend_from_config` and `codex::backend_from_config` signatures to accept an explicit `timeout: Duration` parameter:

```rust
pub fn backend_from_config(config: &GlobalConfig, model: Option<&str>, timeout: Duration) -> CliBackend
```

**Add a new `get_or_create_for_role` method** to `BackendRegistry` rather than changing the signature of `get_or_create_for_spec`. This avoids breaking the 6 non-orchestrator callsites:

```rust
impl BackendRegistry {
    /// Existing method — unchanged signature, uses default timeout (7200s).
    /// Used by: prd.rs, quick_prd.rs, auto.rs, mcp/handlers.rs, preload helpers.
    pub fn get_or_create_for_spec(&mut self, spec: &str) -> Result<Arc<dyn Backend>> {
        // Unchanged: creates backend with default_role_timeout_seconds() duration.
        // Cache key: "claude(opus)" (no role suffix).
    }

    /// New method — creates backend with role-specific timeout.
    /// Used by: orchestrator.rs for all phase-specific backend acquisition.
    pub fn get_or_create_for_role(&mut self, spec: &str, role: &str) -> Result<Arc<dyn Backend>> {
        let parsed = parse_backend_spec(spec)?;
        let cache_key = format!("{}:{role}", backend_spec_key(&parsed));

        if let Some(backend) = self.backends.get(&cache_key) {
            return Ok(backend.clone());
        }

        let timeout = self.role_timeouts.for_role(role);
        let backend = backend_with_optional_tmux(
            self.create_cli_backend_for_spec(&parsed, timeout)?,
            &self.tmux,
            self.tmux_context.clone(),
        );
        self.backends.insert(cache_key, backend.clone());
        Ok(backend)
    }
}
```

**Update `BackendRegistry` struct** to store `RoleTimeouts`:

```rust
pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn Backend>>,
    default_backend: String,
    tmux_context: SharedTmuxContext,
    config: GlobalConfig,
    tmux: BackendRegistryTmuxConfig,
    role_timeouts: RoleTimeouts,  // NEW
}
```

`BackendRegistry::new` accepts `RoleTimeouts` (from `EffectiveConfig` or directly from `GlobalConfig`). The default backends created in `new()` continue to use the default timeout (7200s) since they're role-agnostic cache entries. Role-specific entries are created lazily via `get_or_create_for_role`.

**Update `create_cli_backend_for_spec`** (private) to accept a `timeout: Duration` parameter:

```rust
fn create_cli_backend_for_spec(&self, spec: &BackendSpec, timeout: Duration) -> Result<CliBackend> {
    let model = spec.model.as_deref();
    match spec.name.as_str() {
        "claude" => Ok(claude::backend_from_config(&self.config, model, timeout)),
        "codex" => Ok(codex::backend_from_config(&self.config, model, timeout)),
        _ => Err(...)
    }
}
```

The existing `get_or_create_for_spec` calls `create_cli_backend_for_spec` with `Duration::from_secs(default_role_timeout_seconds())`.

### 4. Resolve timeouts in `EffectiveConfig` (`src/config/mod.rs`)

Add `timeouts: RoleTimeouts` to `EffectiveConfig`. In `resolve_effective_config`, copy `global.timeouts` directly (no project-level merging).

```rust
pub struct EffectiveConfig {
    pub workflow: EffectiveWorkflowConfig,
    pub templates: EffectiveTemplateConfig,
    pub daemon: EffectiveDaemonConfig,
    pub timeouts: RoleTimeouts,  // NEW
    pub global: GlobalConfig,
    pub project: Option<ProjectConfig>,
}
```

### 5. Orchestrator passes role context (`src/workflow/orchestrator.rs`)

Replace `registry.get_or_create_for_spec(backend_spec)` with `registry.get_or_create_for_role(backend_spec, role)` at all orchestrator callsites where the role is known. Specifically:

| Callsite | Role string |
|---|---|
| Prompt review (line ~248) | `"prompt_reviewer"` |
| Planning phase (line ~363) | `"planner"` |
| Implementation phase | `"implementer"` |
| Review phase | `"reviewer"` |
| QA phase | `"qa"` |
| Completion phase | `"completer"` |
| Acceptance QA (line ~1420) | `"qa"` |

### 6. Fix reformatter backend acquisition (`src/workflow/orchestrator.rs`)

In `execute_with_parse_retries`, the current code acquires the reformatter backend via:
```rust
let reformatter_backend = registry
    .get(&reformatter_spec)
    .unwrap_or_else(|| backend.clone());
```

This uses `registry.get()` (direct HashMap lookup) which will miss role-keyed entries. Change this to:
```rust
let reformatter_backend = registry
    .get_or_create_for_role(&reformatter_spec, "reformatter")
    .unwrap_or_else(|_| backend.clone());
```

This ensures: (a) the reformatter backend is created with the `reformatter` timeout if not already cached, and (b) the cache key `"codex(gpt-5.3-codex-medium):reformatter"` correctly maps to a backend with the reformatter-specific timeout.

### 7. Update config CLI and MCP config display (`src/cli/config.rs`, `src/mcp/handlers.rs`)

**`config show`** (both CLI and MCP `config_show`): Add a `"timeouts"` section to the effective config JSON output showing the resolved `RoleTimeouts` values:
```json
{
  "timeouts": {
    "planner": 7200,
    "implementer": 10800,
    "reviewer": 7200,
    "qa": 3600,
    "completer": 7200,
    "reformatter": 1800,
    "prompt_reviewer": 3600
  }
}
```

**`config get`**: Support dotted keys `timeouts.planner`, `timeouts.implementer`, etc. These read from `GlobalConfig.timeouts.<field>`.

**`config set`**: Add cases in `set_global_value()` for `timeouts.planner`, `timeouts.implementer`, `timeouts.reviewer`, `timeouts.qa`, `timeouts.completer`, `timeouts.reformatter`, `timeouts.prompt_reviewer`. Each parses a `u64` value.

### 8. Non-orchestrator callsites — no changes required

The following callsites use `get_or_create_for_spec` (the unchanged method) and continue to work with the default timeout:

| File | Usage | Why default timeout is correct |
|---|---|---|
| `src/cli/prd.rs` | Single PRD backend | PRD is a standalone pipeline; no role-based phases. Default 2h is appropriate. |
| `src/cli/quick_prd.rs` | Writer + reviewer backends | Quick PRD writer/reviewer are spec-generation roles, not workflow execution roles. |
| `src/cli/auto.rs` | Writer + reviewer backends | Same as quick_prd — spec generation pipeline. |
| `src/mcp/handlers.rs` | Writer + reviewer for `quick_prd` handler | MCP-initiated quick PRD; same reasoning. |
| `preload_override_backends()` | Preload cache warming | Preloading creates default-timeout entries. The orchestrator later calls `get_or_create_for_role` which creates role-specific entries under separate cache keys. No conflict. |
| `preload_role_model_backends()` | Preload cache warming | Same — preload entries and role-keyed entries coexist with different cache keys. |

### 9. Config example in `ralph.toml`

```toml
[timeouts]
planner = 7200
implementer = 10800   # 3 hours for complex implementations
reviewer = 7200
qa = 3600             # 1 hour for QA
completer = 7200
reformatter = 1800    # 30 min for reformatting
prompt_reviewer = 3600
```

## Files & Modules

| File | Change |
|---|---|
| `src/config/global.rs` | Add `RoleTimeouts` struct with `for_role()` method and `Default` impl. Add `default_role_timeout_seconds()` helper. Add `timeouts: RoleTimeouts` to `GlobalConfig`. Add deprecation warning logic in config load for `BackendConfig.timeout_seconds`. |
| `src/config/mod.rs` | Add `timeouts: RoleTimeouts` to `EffectiveConfig`. Set it from `global.timeouts` in `resolve_effective_config`. |
| `src/backend/claude.rs` | Change `backend_from_config` to accept `timeout: Duration` parameter instead of reading `backend.timeout_seconds`. |
| `src/backend/codex.rs` | Same change as `claude.rs`. |
| `src/backend/mod.rs` | Add `role_timeouts: RoleTimeouts` field to `BackendRegistry`. Update `BackendRegistry::new` to accept and store `RoleTimeouts`. Add `get_or_create_for_role(&mut self, spec: &str, role: &str)` method with role-suffixed cache keys. Update `create_cli_backend_for_spec` to accept `timeout: Duration`. Update existing `get_or_create_for_spec` to pass default timeout to `create_cli_backend_for_spec`. |
| `src/workflow/orchestrator.rs` | Replace `get_or_create_for_spec` with `get_or_create_for_role` at all phase-specific callsites (planner, implementer, reviewer, qa, completer, prompt_reviewer, acceptance_qa). Fix reformatter path in `execute_with_parse_retries` to use `get_or_create_for_role(spec, "reformatter")` instead of `registry.get()`. Pass `RoleTimeouts` when constructing `BackendRegistry`. |
| `src/cli/config.rs` | Add `config set` cases for `timeouts.*` keys. Add deprecation warning in `config set` for `backends.*.timeout_seconds`. Add `timeouts` section to `config show` and `config get` output (both global and effective views). |
| `src/mcp/handlers.rs` | Add `timeouts` section to `handle_config_show` JSON output for both global and effective config views. |
| `tests/init_command.rs` | Update `test_init_generates_valid_config` to verify `timeouts` defaults alongside (or instead of) `timeout_seconds` assertions. |
| `tests/orchestrator.rs` | Migrate test setup from `workspace.config.backends.*.timeout_seconds = 30` to `workspace.config.timeouts = RoleTimeouts { planner: 30, implementer: 30, ... }`. Ensure all orchestrator integration tests use the new config path. |

## Testing Strategy

1. **Unit tests in `src/config/global.rs`**:
   - `RoleTimeouts::for_role` returns correct `Duration` for each known role and default for unknown roles.
   - TOML deserialization of `[timeouts]` section with partial fields fills defaults for omitted roles.
   - Empty/missing `[timeouts]` section defaults all roles to 7200s.
   - Existing `BackendConfig` tests still pass (backward-compatible deserialization of `timeout_seconds`).
   - Deprecation warning is emitted when `timeout_seconds` is non-default (test via `tracing_subscriber` test utilities or by checking the warn logic path).

2. **Unit tests in `src/config/mod.rs`**:
   - `resolve_effective_config` copies global `RoleTimeouts` into `EffectiveConfig.timeouts`.
   - All default values are 7200s when no `[timeouts]` section is present.

3. **Unit tests in `src/backend/mod.rs`**:
   - `get_or_create_for_role("claude(opus)", "planner")` and `get_or_create_for_role("claude(opus)", "implementer")` produce distinct cache entries with different timeouts when `RoleTimeouts` differs for those roles.
   - `get_or_create_for_spec("claude(opus)")` continues to work with default timeout and does not collide with role-keyed entries.
   - Calling `get_or_create_for_role` twice with the same spec and role returns the cached instance (same `Arc`).

4. **Existing integration tests — migration**:
   - `tests/orchestrator.rs`: All test helpers that set `workspace.config.backends.*.timeout_seconds = 30` are updated to also set `workspace.config.timeouts` to a `RoleTimeouts` with all fields set to 30. The `timeout_seconds` field remains set for backward compat (it's still deserialized), but the effective timeout comes from `RoleTimeouts`.
   - `tests/init_command.rs`: `test_init_generates_valid_config` updated to also assert `workspace.config.timeouts.planner == 7200` (etc.) or to assert the struct equals `RoleTimeouts::default()`.
   - `cli_backend_timeout_kills_and_reaps_child_and_writes_footer` (in `tests/backend_tmux.rs` or similar) continues to pass since `CliBackend::new` still accepts a `Duration` — the change is only where that `Duration` comes from.

5. **Config CLI tests**:
   - `config set timeouts.implementer 10800` followed by `config get timeouts.implementer` returns `10800`.
   - `config show` output includes `"timeouts"` section with all role fields.
   - `config set backends.claude.timeout_seconds 3600` succeeds but produces a deprecation warning.

6. **Build verification**: `nix develop -c cargo build` and `nix develop -c cargo test` pass.

## Out of Scope

- **Project-level timeout overrides**: The original spec included `ProjectTimeoutOverrides` in `ProjectConfig`, but this is scope expansion beyond the requirements (which specify workspace-level `ralph.toml` configuration only). Removing project-level overrides reduces implementation surface and risk. If per-project timeout customization is needed later, it can be added as a follow-up with its own acceptance criteria.
- **CLI flag overrides for timeouts**: No `--timeout-planner=3600` flags. Role timeouts are config-file-only for now.
- **Per-backend-per-role timeouts**: The timeout is purely role-based, not backend-specific. A planner gets the same timeout whether running on claude or codex. If needed later, this can be extended to `[backends.claude.timeouts]`.
- **Runtime timeout adjustment**: No mechanism to change timeouts mid-workflow.
- **Removing `timeout_seconds` from `BackendConfig`**: The field is deprecated but kept for backward compatibility. Full removal is a future breaking change.
- **Separate `acceptance_qa` timeout**: Acceptance QA executes under the `"qa"` role label and uses the `qa` timeout. A separate `acceptance_qa` timeout field would be unused dead config. If the orchestrator is refactored to give acceptance QA its own role label, a timeout field can be added then.
- **Tmux backend timeout changes**: `TmuxBackend` delegates to `CliBackend` which already uses the `timeout` field; no separate tmux timeout work needed.
- **Daemon rebase timeout**: The `daemon_rebase_timeout_seconds` field on `WorkspaceConfig` is unrelated and unchanged.
