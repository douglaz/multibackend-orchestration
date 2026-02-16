Now I have a thorough understanding of the entire codebase. Let me write the spec.

---

## Summary

Replace the single `timeout_seconds` field on `BackendConfig` (currently applied uniformly to all agent roles) with a per-role `role_timeouts` map. Each agent role (planner, implementer, reviewer, qa, completer, acceptance_qa, reformatter) gets its own optional timeout override, falling back to the existing `timeout_seconds` as the default. The default value remains 2 hours (7200 seconds). This change flows through `CliBackend` construction in `claude.rs`/`codex.rs`, the `TmuxBackend` timeout path, and the `execute_with_timeout_retries` orchestrator loop.

## Acceptance Criteria

- [ ] `BackendConfig` gains a `role_timeouts: RoleTimeouts` field with optional per-role `u64` values (planner, implementer, reviewer, qa, completer, acceptance_qa, reformatter)
- [ ] `BackendConfig.timeout_seconds` is retained as the fallback default; existing configs with only `timeout_seconds` continue to work unchanged
- [ ] `PartialBackendConfig` and its `into_backend_config_with_defaults` handle merging `role_timeouts` from user TOML with coded defaults
- [ ] `CliBackend` no longer bakes in a single `Duration` at construction; instead it accepts a role-keyed timeout resolver or carries the full timeout config
- [ ] `backend_from_config` in `claude.rs` and `codex.rs` passes role-timeout information through to `CliBackend`
- [ ] `execute_with_timeout_retries` in the orchestrator resolves the effective timeout for the current role before invoking the backend
- [ ] `TmuxBackend` uses role-aware timeout via `TmuxExecutionContext.role` when calling `tmux::wait_for_exit`
- [ ] TOML config `[backends.claude.role_timeouts]` and `[backends.codex.role_timeouts]` sections are supported and optional
- [ ] Default timeout for all roles is 7200 seconds when no override is specified
- [ ] `BackendTimeout` error in `error.rs` is enriched with `role` and `timeout_secs` for debuggability
- [ ] E2E conformance test `backend_timeout_exhausted_fails_task` updated for new config shape
- [ ] Unit tests verify `RoleTimeouts` resolution, TOML deserialization, and fallback behavior

## Technical Approach

### 1. New `RoleTimeouts` struct (`src/config/global.rs`)

Add a struct mirroring `BackendRoleModels` but for timeouts:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct RoleTimeouts {
    pub planner: Option<u64>,
    pub implementer: Option<u64>,
    pub reviewer: Option<u64>,
    pub qa: Option<u64>,
    pub completer: Option<u64>,
    pub acceptance_qa: Option<u64>,
    pub reformatter: Option<u64>,
}

impl RoleTimeouts {
    pub fn for_role(&self, role: &str) -> Option<u64> {
        match role {
            "planner" => self.planner,
            "implementer" => self.implementer,
            "reviewer" => self.reviewer,
            "qa" => self.qa,
            "completer" => self.completer,
            "acceptance_qa" => self.acceptance_qa,
            "reformatter" => self.reformatter,
            _ => None,
        }
    }

    pub fn fill_from(&mut self, defaults: &RoleTimeouts) {
        // Same pattern as BackendRoleModels::fill_from
    }
}
```

Add `role_timeouts: RoleTimeouts` to `BackendConfig`. Add it to `PartialBackendConfig` as `Option<RoleTimeouts>` and merge in `into_backend_config_with_defaults` using the `fill_from` pattern.

### 2. Timeout resolution method on `BackendConfig`

```rust
impl BackendConfig {
    pub fn timeout_for_role(&self, role: &str) -> Duration {
        let secs = self.role_timeouts.for_role(role).unwrap_or(self.timeout_seconds);
        Duration::from_secs(secs)
    }
}
```

### 3. Refactor `CliBackend` to carry role-timeout config (`src/backend/mod.rs`)

Two options exist; the simpler approach is chosen:

**Option A (chosen):** Keep `CliBackend` carrying a single `timeout: Duration` but change `backend_from_config` to accept a role parameter. The orchestrator already knows the role at backend-construction time via `get_or_create_for_spec` and `resolve_backend_for_role`. However, backends are cached by spec key (e.g. `claude(opus)`) and reused across roles, which means the timeout baked into `CliBackend` at construction time would be wrong for subsequent roles.

**Option B (chosen instead):** Change `CliBackend` to carry `default_timeout: Duration` plus an `Arc<dyn Fn(&str) -> Duration>` role resolver, or more simply, store the entire `BackendConfig` reference. But the simplest approach is:

**Option C (actual implementation):** Pass the role-specific timeout into the `Backend::execute_with_log` call chain. This requires the least structural change:

1. Add a `timeout_override: Option<Duration>` parameter to `execute_streaming` (private method on `CliBackend`). When set, it overrides `self.timeout`.
2. Add `execute_with_log_and_timeout` to the `Backend` trait with a default impl that delegates to `execute_with_log`, and override it in `CliBackend` and `TmuxBackend`.
3. In the orchestrator's `execute_with_timeout_retries`, resolve the timeout from `GlobalConfig` via `BackendRegistry` and pass it through.

**Simplest path (final choice):** Add a method to `BackendRegistry` that resolves timeout for a `(backend_spec, role)` pair:

```rust
impl BackendRegistry {
    pub fn timeout_for_role(&self, backend_name: &str, role: &str) -> Duration {
        let parsed = parse_backend_spec(backend_name).ok();
        let base_name = parsed.as_ref().map(|p| p.name.as_str()).unwrap_or(backend_name);
        self.config.backend_config(base_name)
            .map(|bc| bc.timeout_for_role(role))
            .unwrap_or(Duration::from_secs(7200))
    }
}
```

Then modify `execute_with_timeout_retries` to accept a `timeout: Duration` parameter and pass it to a new `execute_with_timeout` method on the `Backend` trait:

```rust
// New trait method with default
async fn execute_with_log_timeout(
    &self,
    prompt: &str,
    log_writer: Option<&mut LogWriter>,
    timeout: Duration,
) -> Result<String>;
```

`CliBackend::execute_streaming` already takes `self.timeout` — the override replaces it. `TmuxBackend` passes it to `wait_for_exit`.

### 4. Thread timeout through the orchestrator (`src/workflow/orchestrator.rs`)

`execute_with_timeout_retries` gains a `timeout: Duration` parameter. All call sites in `execute_with_parse_retries` resolve it via `registry.timeout_for_role(backend_name, role)` before passing it down.

The `BackendRegistry` reference must be available at call sites. It already is — the orchestrator holds `&mut BackendRegistry` and passes individual `Arc<dyn Backend>` instances to these functions. We add the registry (or just the resolved timeout) as an argument.

### 5. Enrich error types (`src/error.rs`)

```rust
BackendTimeout {
    backend: String,
    role: String,
    timeout_secs: u64,
},
```

Update all match arms in `orchestrator.rs`, `cli/run.rs`, and `tmux_backend.rs`.

### 6. Update TOML config file

`.ralph/ralph.toml` can optionally include:
```toml
[backends.claude.role_timeouts]
# All optional; falls back to timeout_seconds (7200)
# planner = 3600
# implementer = 7200
```

No changes needed to the default file — `role_timeouts` defaults to all-`None`.

## Files & Modules

| File | Change |
|---|---|
| `src/config/global.rs` | Add `RoleTimeouts` struct; add `role_timeouts` field to `BackendConfig`; add `timeout_for_role` method; update `PartialBackendConfig` to handle merging; update `Default` impls |
| `src/config/mod.rs` | No structural changes needed (timeout resolution stays on `BackendConfig`) |
| `src/backend/mod.rs` | Add `execute_with_log_timeout` to `Backend` trait; update `CliBackend::execute_streaming` to accept timeout param; add `BackendRegistry::timeout_for_role` method |
| `src/backend/claude.rs` | No change needed (timeout is resolved at execution time, not construction time) |
| `src/backend/codex.rs` | No change needed (same reason) |
| `src/backend/tmux_backend.rs` | Override `execute_with_log_timeout`; pass timeout to `wait_for_exit` instead of `self.inner.timeout()` |
| `src/workflow/orchestrator.rs` | Pass resolved timeout to `execute_with_timeout_retries`; resolve via `BackendRegistry::timeout_for_role` at each call site |
| `src/error.rs` | Add `role: String` and `timeout_secs: u64` fields to `BackendTimeout` variant |
| `src/cli/run.rs` | Update `BackendTimeout` match destructuring |
| `.ralph/ralph.toml` | No mandatory changes; optionally add `[backends.*.role_timeouts]` examples |
| `src/validate/tests_e2e_conformance.rs` | Update timeout config test to use either `timeout_seconds` (backward compat) or `role_timeouts` |

## Testing Strategy

**Unit tests (`src/config/global.rs`):**
- `RoleTimeouts::for_role` returns correct value for each role and `None` for unknown
- `RoleTimeouts::fill_from` merges partial overrides with defaults
- `BackendConfig::timeout_for_role` falls back to `timeout_seconds` when role override is `None`
- TOML deserialization with `[backends.claude.role_timeouts]` section parses correctly
- TOML deserialization without `role_timeouts` section defaults to all-`None` (backward compat)
- Partial `role_timeouts` (only some roles set) deserializes correctly

**Unit tests (`src/backend/mod.rs`):**
- `BackendRegistry::timeout_for_role` resolves from config for known backends
- `BackendRegistry::timeout_for_role` returns 7200s default for unknown backends
- `BackendRegistry::timeout_for_role` parses backend specs like `claude(opus)` to resolve against base `claude` config
- `CliBackend` timeout test verifies the override `Duration` is used (existing test `cli_backend_timeout_kills_and_reaps_child_and_writes_footer` adapted)

**Integration / E2E tests (`src/validate/tests_e2e_conformance.rs`):**
- Existing `backend_timeout_exhausted_fails_task` continues to pass (backward compat: `timeout_seconds` still works)
- New test: role-specific timeout override is respected (e.g., set `role_timeouts.planner = 2` and verify planner times out while a longer default would not)

**Error message tests:**
- `BackendTimeout` error string includes role name and timeout value

## Out of Scope

- **Per-project role timeout overrides**: The project-level config (`ProjectConfig`) does not gain `role_timeouts` in this change. Project-level overrides could be added later following the same `ProjectDaemonOverrides` pattern.
- **CLI flags for role timeouts**: No `--planner-timeout` or similar CLI arguments. Configuration is TOML-only.
- **Removing `timeout_seconds` entirely**: It is retained as the fallback default. Full deprecation/removal is a follow-up if desired.
- **Dynamic timeout adjustment**: No runtime tuning based on prompt size, historical execution time, or other heuristics.
- **Timeout configuration for the refinement/rebase backend**: The `daemon_rebase_timeout_seconds` is a separate concern and remains unchanged.
- **Validation bounds on timeout values**: No minimum/maximum enforcement beyond Rust's `u64` range. Could be added as a follow-up.