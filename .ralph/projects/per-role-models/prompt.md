# Per-Role Backend/Model Overrides

## Goal

Allow users to specify which backend (and optionally which model) to use for each orchestration role (planner, implementer, reviewer, completer) independently. When a per-role override is set, it bypasses the normal alternation logic for that role. Roles without overrides continue to use the default alternation behavior.

## Current Behavior

Ralph alternates backends between roles based on the `starting_backend` and the loop number:
- Odd loops: planner = starting_backend, implementer = opposite, reviewer = starting_backend
- Even loops: planner = opposite, implementer = starting_backend, reviewer = opposite

The `starting_backend` can be specified as a backend spec like `claude(opus)` thanks to the existing explicit-models feature. But there is no way to say "always use claude(sonnet) for the implementer" independently.

## Desired Behavior

Users can set per-role backend overrides at three levels (highest precedence first):
1. **CLI flags** on `ralph run`: `--planner-backend`, `--implementer-backend`, `--reviewer-backend`, `--completer-backend`
2. **Project config** (`config.toml`): `planner_backend`, `implementer_backend`, `reviewer_backend`, `completer_backend` in `[workflow]`
3. **Global config** (`ralph.toml`): same fields in `[workflow]`

When a per-role override is set, that role ALWAYS uses the specified backend/model regardless of loop number or alternation. Roles without overrides still alternate normally.

Example: `ralph run --implementer-backend "claude(sonnet)"` means every loop's implementer uses claude with the sonnet model, while planner and reviewer still alternate between claude and codex.

## Implementation

### 1. Config changes

**`src/config/global.rs` — `WorkflowConfig`:**
Add optional per-role override fields:
```rust
pub struct WorkflowConfig {
    // existing fields...
    #[serde(default)]
    pub planner_backend: Option<String>,
    #[serde(default)]
    pub implementer_backend: Option<String>,
    #[serde(default)]
    pub reviewer_backend: Option<String>,
    #[serde(default)]
    pub completer_backend: Option<String>,
}
```

**`src/config/project.rs` — `ProjectWorkflowOverrides`:**
Add the same four optional fields.

**`src/config/mod.rs` — `EffectiveWorkflowConfig`:**
Add four `Option<String>` fields for the per-role overrides. Resolution: CLI > project > global > None.

Validate each override using `parse_backend_spec()` and check that the base name is a known backend (same validation as `starting_backend`).

### 2. CLI changes

**`src/cli/mod.rs` — `RunArgs`:**
Add four new `--planner-backend`, `--implementer-backend`, `--reviewer-backend`, `--completer-backend` flags (all `Option<String>`).

### 3. BackendRegistry changes

**`src/backend/mod.rs` — `assign_feature_backends`:**
Accept a new parameter for the per-role overrides (or accept the EffectiveWorkflowConfig). For each role:
- If a per-role override is set, use it directly (the full spec string, e.g. `"claude(sonnet)"`)
- Otherwise, use the current alternation logic

Same for `assign_completion_backends` (planner + completer roles).

The method signature change could look like:
```rust
pub fn assign_feature_backends(
    &self,
    loop_number: u32,
    starting_backend: &str,
    role_overrides: &RoleOverrides,
) -> Result<FeatureLoopBackends>
```

Where `RoleOverrides` is a simple struct:
```rust
pub struct RoleOverrides {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub completer: Option<String>,
}
```

Or alternatively, just pass the four `Option<&str>` values directly.

### 4. Orchestrator changes

**`src/workflow/orchestrator.rs`:**
Pass the per-role overrides from `EffectiveWorkflowConfig` into `assign_feature_backends()` and `assign_completion_backends()` calls. The overrides are already resolved by config resolution, so the orchestrator just forwards them.

### 5. Health checks

When per-role overrides are set, ensure those backends are included in the health check. The `get_or_create_for_spec()` method already handles creating backends for specs not in the initial registry, so calling it for each override during startup and including the result in health checks should suffice.

### 6. State recording

No changes needed — `FeatureLoopBackends` already stores backend strings, so `"claude(sonnet)"` will naturally be recorded in state.json.

## Acceptance Criteria

- [ ] Four new optional fields (`planner_backend`, `implementer_backend`, `reviewer_backend`, `completer_backend`) in `WorkflowConfig`, `ProjectWorkflowOverrides`, and `EffectiveWorkflowConfig`
- [ ] Four new CLI flags on `ralph run`: `--planner-backend`, `--implementer-backend`, `--reviewer-backend`, `--completer-backend`
- [ ] Config resolution: CLI > project > global > None (same precedence as other workflow fields)
- [ ] Each per-role override is validated using `parse_backend_spec()` + known backend check
- [ ] `assign_feature_backends()` uses override for each role when set, falls back to alternation when not
- [ ] `assign_completion_backends()` uses override for planner/completer roles when set
- [ ] Health checks include per-role override backends
- [ ] Bare names like `"claude"` and spec strings like `"claude(sonnet)"` both work as overrides
- [ ] Roles without overrides continue to alternate normally
- [ ] State.json records the actual backend used (override or alternation result)
- [ ] All existing tests continue to pass
- [ ] Unit tests for `assign_feature_backends` with various override combinations
- [ ] Unit tests for config resolution with per-role overrides

## Files to Modify

- `src/config/global.rs` — Add per-role fields to `WorkflowConfig`
- `src/config/project.rs` — Add per-role fields to `ProjectWorkflowOverrides`
- `src/config/mod.rs` — Add per-role fields to `EffectiveWorkflowConfig`, resolve them, validate them
- `src/cli/mod.rs` — Add CLI flags to `RunArgs`
- `src/backend/mod.rs` — Update `assign_feature_backends()` and `assign_completion_backends()` to accept and apply overrides
- `src/workflow/orchestrator.rs` — Pass overrides from effective config into backend assignment calls
