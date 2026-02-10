# Per-Backend Role-Specific Model Defaults

## Goal

Allow each backend (claude, codex) to specify which model to use for each orchestration role (planner, implementer, reviewer, completer, reformatter). This works with alternation — when a backend is selected for a role via alternation, the role-specific model is automatically applied. This is distinct from per-role backend overrides (which pin a specific backend to a role); this feature controls **which model** a backend uses **when it's selected**.

## Current Behavior

When alternation selects e.g. "claude" as the planner, it creates a bare `CliBackend` with no `--model` flag. The explicit-models feature allows specs like `claude(opus)` but only through manual configuration — there's no automatic per-role model selection.

## Desired Behavior

Each backend config gains a `[backends.<name>.models]` table mapping roles to model names:

```toml
[backends.claude.models]
planner = "claude-sonnet-4-5-20250929"
implementer = "claude-sonnet-4-5-20250929"
reviewer = "claude-sonnet-4-5-20250929"
completer = "claude-sonnet-4-5-20250929"
reformatter = "claude-sonnet-4-5-20250929"

[backends.codex.models]
planner = "o3"
implementer = "o3"
reviewer = "o3"
completer = "o3"
reformatter = "o3"
```

When alternation selects "claude" for the planner role, the registry looks up `backends.claude.models.planner`. If set (e.g. `"claude-sonnet-4-5-20250929"`), it returns the spec `"claude(claude-sonnet-4-5-20250929)"` so a model-injected backend is used. If not set, it returns bare `"claude"` (current behavior).

Per-role backend overrides (from the per-role-models feature) take precedence — if `planner_backend = "claude(opus)"` is set, the role-models config is ignored for that role.

## Implementation

### 1. Config: add BackendRoleModels

**`src/config/global.rs`:**

Add a new struct:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendRoleModels {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub completer: Option<String>,
    pub reformatter: Option<String>,
}
```

Add to `BackendConfig`:
```rust
pub struct BackendConfig {
    pub command: String,
    pub args: Vec<String>,
    pub timeout_seconds: u64,
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub models: BackendRoleModels,
}
```

Update `GlobalConfig::default()` with the desired model defaults:
- Claude: planner/reviewer/completer = `"claude-sonnet-4-5-20250929"`, implementer/reformatter = `"claude-sonnet-4-5-20250929"`
- Codex: planner/reviewer/completer = `"o3"`, implementer/reformatter = `"o3"`

Note: Use model IDs that the respective CLIs accept via `--model`. Verify the exact model ID strings work with the claude and codex CLIs.

### 2. BackendRegistry: resolve role-specific models

**`src/backend/mod.rs`:**

Add a method to `BackendRegistry`:
```rust
pub fn resolve_backend_for_role(&self, base_backend: &str, role: &str) -> String
```

Logic:
1. Parse `base_backend` with `parse_backend_spec()` — if it already has a model (e.g. from a per-role override), return it as-is
2. Look up `self.config.backend_config(&parsed.name)` to get the `BackendConfig`
3. Look up the role in `backend_config.models` (match on role string: "planner", "implementer", "reviewer", "completer", "reformatter")
4. If a model is found, return `format!("{}({})", parsed.name, model)`
5. Otherwise return `base_backend.to_owned()`

Update `assign_feature_backends()`: after computing each role's backend (via alternation + per-role override), call `resolve_backend_for_role(backend, role)` for planner, implementer, and reviewer.

Update `assign_completion_backends()`: same for planner and completer.

### 3. Orchestrator: reformatter role model

**`src/workflow/orchestrator.rs`:**

In `execute_with_parse_retries()`, when resolving the reformatter backend (the opposite backend used for attempt 2), apply the "reformatter" role model. Currently at line ~1658:

```rust
let reformatter_backend = registry
    .opposite(backend.name())
    .ok()
    .and_then(|opposite_name| registry.get(opposite_name))
    .unwrap_or_else(|| backend.clone());
```

Change to use `resolve_backend_for_role` on the opposite backend name before looking it up:

```rust
let reformatter_spec = registry
    .opposite(backend.name())
    .map(|name| registry.resolve_backend_for_role(name, "reformatter"))
    .unwrap_or_else(|_| backend.name().to_owned());
let reformatter_backend = registry
    .get_or_create_for_spec(&reformatter_spec)
    .unwrap_or_else(|_| backend.clone());
```

Note: `execute_with_parse_retries` currently takes `&BackendRegistry` (not `&mut`). Since `get_or_create_for_spec` takes `&mut self`, this will need adjustment — either change the signature to take `&mut BackendRegistry`, use interior mutability (e.g. `RwLock` on the backends map), or pre-create the reformatter backends during startup. Choose the approach that best fits the existing architecture.

### 4. Update .ralph/ralph.toml

Add models tables with the actual model IDs:

```toml
[backends.claude.models]
planner = "claude-sonnet-4-5-20250929"
implementer = "claude-sonnet-4-5-20250929"
reviewer = "claude-sonnet-4-5-20250929"
completer = "claude-sonnet-4-5-20250929"
reformatter = "claude-sonnet-4-5-20250929"

[backends.codex.models]
planner = "o3"
implementer = "o3"
reviewer = "o3"
completer = "o3"
reformatter = "o3"
```

### 5. Health checks

During startup, iterate over all role models for each backend and call `get_or_create_for_spec()` to pre-create and health-check all model-specific backends that will be used. This ensures early failure if a model ID is invalid.

## Acceptance Criteria

- [ ] `BackendRoleModels` struct with 5 optional string fields (planner, implementer, reviewer, completer, reformatter)
- [ ] `BackendConfig.models` field (serde-default to empty/None)
- [ ] `BackendRegistry::resolve_backend_for_role()` returns model-injected spec when role model is configured
- [ ] `resolve_backend_for_role()` returns input unchanged when no role model is configured
- [ ] `resolve_backend_for_role()` returns input unchanged when input already has a model spec (per-role override takes precedence)
- [ ] `assign_feature_backends()` applies role models to planner, implementer, reviewer
- [ ] `assign_completion_backends()` applies role models to planner, completer
- [ ] Reformatter in `execute_with_parse_retries()` uses "reformatter" role model from the opposite backend
- [ ] Per-role backend overrides still take precedence over role models
- [ ] `.ralph/ralph.toml` updated with model tables
- [ ] `GlobalConfig::default()` includes role model defaults
- [ ] Health checks cover all configured role models
- [ ] Bare backend names without role models continue to work (backward compatible)
- [ ] All existing tests pass
- [ ] Unit tests for `resolve_backend_for_role` (with model, without model, with existing spec)
- [ ] Unit tests for `assign_feature_backends` verifying role models are applied

## Files to Modify

- `src/config/global.rs` — Add `BackendRoleModels`, add `models` to `BackendConfig`, update defaults
- `src/backend/mod.rs` — Add `resolve_backend_for_role()`, update `assign_feature_backends()` and `assign_completion_backends()`
- `src/workflow/orchestrator.rs` — Update reformatter backend resolution to use role model
- `.ralph/ralph.toml` — Add `[backends.claude.models]` and `[backends.codex.models]` tables
