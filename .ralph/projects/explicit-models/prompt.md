# Explicit Model Selection

## Goal

Allow users to specify which model each backend should use, using the syntax `backend(model)` — e.g. `claude(opus)`, `codex(gpt-5.3-codex-xhigh)`. This applies everywhere a backend name is accepted: `default_backend`, `starting_backend`, and `--backend` CLI flag.

## Current State

Backend names are bare strings like `"claude"` or `"codex"`. The backend config in `ralph.toml` defines the command and args but has no model concept. The CLI backends both accept a `--model <MODEL>` flag:
- `claude --model opus` or `claude --model claude-opus-4-6`
- `codex exec --model gpt-5.3-codex-xhigh ...`

## Design

### Syntax: `backend(model)`

When a backend reference includes parentheses, parse it as `name(model)`:
- `claude` → backend="claude", model=None (use whatever default the CLI uses)
- `claude(opus)` → backend="claude", model=Some("opus")
- `codex(gpt-5.3-codex-xhigh)` → backend="codex", model=Some("gpt-5.3-codex-xhigh")

### Where model specs appear

1. **`default_backend`** in `[workspace]` section of `ralph.toml` — e.g. `default_backend = "claude(opus)"`
2. **`starting_backend`** in project config / `--backend` CLI flag — e.g. `--backend "claude(opus)"`
3. **Backend assignment** — when the orchestrator assigns backends per loop (planner, implementer, reviewer), model specs propagate from the starting backend

### Parsing

Add a helper function (e.g. in `src/backend/mod.rs` or a new small module):

```rust
pub struct BackendSpec {
    pub name: String,
    pub model: Option<String>,
}

pub fn parse_backend_spec(spec: &str) -> Result<BackendSpec>
```

Parsing rules:
- If `spec` contains `(` and ends with `)`, split into name and model
- Otherwise, name = spec, model = None
- Validate that name is non-empty
- Validate that model (if present) is non-empty

### Model injection into CLI args

When constructing a `CliBackend`, if a model is specified, prepend `--model <MODEL>` to the args list. This works for both `claude` and `codex` CLIs.

**In `src/backend/claude.rs` and `src/backend/codex.rs`**: Change `backend_from_config` to accept an optional model parameter. If a model is given, insert `["--model", model]` into the args (at the beginning, before other args).

**Alternatively**, the model injection can happen in `BackendRegistry::new()` when constructing backends, or in a wrapper. The key point: model must be part of the CLI args passed to the backend process.

### BackendRegistry changes

The registry currently hardcodes two backends: "claude" and "codex". With model specs, the same backend name (e.g. "claude") might be used with different models in different roles.

**Approach**: The registry stores backends keyed by the **full spec string** (e.g. `"claude(opus)"`), not just the name. When a spec has no model, it falls back to the base backend (e.g. `"claude"`).

Changes to `BackendRegistry`:
1. `new()` — still creates the two base backends from config (no model override)
2. Add `fn get_or_create_for_spec(&mut self, spec: &str) -> Result<Arc<dyn Backend>>` — parses the spec, and if a model is specified, creates a new `CliBackend` with the model injected into args (cloning the base config). Cache these so the same spec doesn't create duplicates.
3. `opposite()` — when called with a spec like `"claude(opus)"`, return the opposite **base name** (i.e. `"codex"`). The opposite does NOT inherit the model — it uses whatever model the other backend is configured with by default.
4. `fn name(&self)` on `CliBackend` — should return the full spec string when a model is set, so logging/state shows which model was used.

### State/artifacts

The backend names stored in `state.json` (e.g. `loop.backends.planner`) should use the full spec string when a model is specified: `"claude(opus)"` rather than just `"claude"`. This is informational — it records which model was actually used.

### Propagation through assign_feature_backends / assign_completion_backends

Currently these methods receive `starting_backend: &str` (e.g. `"claude"`) and return `FeatureLoopBackends` with planner/implementer/reviewer names.

With model specs:
- `starting_backend` can be `"claude(opus)"`
- The planner gets that spec
- The implementer gets `opposite(spec)` — which is the opposite base name WITHOUT a model override (e.g. just `"codex"`)
- The reviewer gets the same spec as the planner

This means only the starting backend's model propagates. The opposite backend uses its default model. This is the correct behavior — if users want to override the opposite backend's model too, they can change the opposite backend's default model in config.

### Config validation

In `resolve_effective_config()` (`src/config/mod.rs`), the `starting_backend` is validated against `global.backend_config()`. Update this to parse the spec first and validate the base name.

### What NOT to change

- Do not add a `model` field to `BackendConfig` in `ralph.toml` — the model is specified inline in the backend reference string
- Do not change the `Backend` trait
- Do not change `TmuxBackend` — model injection happens at the `CliBackend` level, before tmux wrapping
- Do not change template files or prompt content
- Do not break existing configs that use bare backend names like `"claude"` — these must continue to work unchanged

### Tests

- Unit test for `parse_backend_spec`: bare name, name with model, edge cases (empty, missing parens)
- Unit test that `BackendRegistry::opposite()` works with spec strings
- Integration test or unit test verifying that a model-specified backend includes `--model` in its args
- Existing integration tests must continue to pass (they use bare backend names)
