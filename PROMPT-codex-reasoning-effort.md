# Codex Reasoning Effort Suffix Decomposition

## Overview

The codex CLI `model_reasoning_effort` config controls how much reasoning the model applies. Ralph's per-role model config already encodes effort in the model name suffix (e.g., `gpt-5.3-codex-xhigh`, `gpt-5.3-codex-high`, `gpt-5.3-codex-medium`). However, ChatGPT accounts don't support these suffixed model names — only the base `gpt-5.3-codex` works. The effort must be passed separately via `-c model_reasoning_effort="xhigh"`.

This feature makes ralph **decompose** suffixed codex model names at invocation time: strip the known effort suffix from the model name and pass it as a separate `-c model_reasoning_effort="..."` CLI arg. This way the existing config (`models.planner = "gpt-5.3-codex-xhigh"`) keeps working unchanged — ralph just interprets the suffix intelligently.

## Background

The codex CLI accepts: `codex exec -c model_reasoning_effort="xhigh" --model gpt-5.3-codex ...`

Known effort suffixes: `-low`, `-medium`, `-high`, `-xhigh`. These are appended to a base codex model name.

The claude CLI does NOT have reasoning effort — this feature is codex-only.

## Design

### Suffix Parsing

Add a helper function (in `src/backend/codex.rs` or a shared utility):

```rust
/// Recognized codex reasoning-effort suffixes, longest-first for greedy matching.
const CODEX_EFFORT_SUFFIXES: &[(&str, &str)] = &[
    ("-xhigh", "xhigh"),
    ("-medium", "medium"),
    ("-high", "high"),
    ("-low", "low"),
];

/// If `model_name` ends with a known effort suffix, return `(base_model, Some(effort))`.
/// Otherwise return `(model_name, None)`.
pub fn parse_codex_model_effort(model_name: &str) -> (&str, Option<&str>) {
    for &(suffix, effort) in CODEX_EFFORT_SUFFIXES {
        if let Some(base) = model_name.strip_suffix(suffix) {
            return (base, Some(effort));
        }
    }
    (model_name, None)
}
```

Note: `-xhigh` must be checked before `-high` to avoid `-xhigh` matching as `-high` with leftover `x`. The ordering above handles this.

### CLI Arg Injection

Modify `src/backend/codex.rs` `backend_from_config()` to decompose the model name:

```rust
pub fn backend_from_config(config: &GlobalConfig, model: Option<&str>) -> CliBackend {
    let backend = &config.backends.codex;
    let mut args = backend.args.clone();
    let name = if let Some(model_name) = model {
        let (base_model, effort) = parse_codex_model_effort(model_name);
        args.splice(0..0, ["--model".to_owned(), base_model.to_owned()]);
        if let Some(effort_level) = effort {
            args.splice(0..0, [
                "-c".to_owned(),
                format!("model_reasoning_effort=\"{effort_level}\""),
            ]);
        }
        // The backend name preserves the ORIGINAL suffixed model for display/state.json
        format!("codex({model_name})")
    } else {
        "codex".to_owned()
    };

    CliBackend::new(
        &name,
        backend.command.clone(),
        args,
        Duration::from_secs(backend.timeout_seconds),
        backend.env.clone(),
    )
}
```

Key points:
- The **display name** keeps the original suffixed model (e.g., `codex(gpt-5.3-codex-xhigh)`) so state.json and logs show exactly what was configured.
- The **CLI args** use the base model name (`--model gpt-5.3-codex`) plus the extracted effort (`-c model_reasoning_effort="xhigh"`).
- If the model has no known suffix, no `-c model_reasoning_effort` arg is injected (the codex CLI will use its own config default).
- The `claude.rs` `backend_from_config()` is NOT modified — suffix decomposition is codex-specific.

### What NOT to Change

- **No new config structs** — no `BackendRoleReasoningEfforts`. The existing `BackendRoleModels` with suffixed model names is the configuration mechanism.
- **No changes to `BackendConfig`** — the models config stays as-is.
- **No changes to `BackendRegistry`**, `resolve_backend_for_role()`, `get_or_create_for_spec()`, or the orchestrator — all the decomposition happens inside `codex::backend_from_config()`.
- **No changes to default model names** — `gpt-5.3-codex-xhigh`, `gpt-5.3-codex-high`, `gpt-5.3-codex-medium` stay as defaults.
- **No changes to `parse_backend_spec()`** — the spec format is unchanged.
- **No changes to `ralph.toml` live config** — user file, not touched.

## Files to Modify

| File | Change |
|------|--------|
| `src/backend/codex.rs` | Add `parse_codex_model_effort()` function; modify `backend_from_config()` to decompose model name and inject `-c model_reasoning_effort` arg |
| `tests/backend.rs` | Add tests verifying the decomposition: suffixed model produces correct CLI args, non-suffixed model passes through unchanged |

## Unit Tests

```rust
// In src/backend/codex.rs #[cfg(test)] module:

#[test]
fn parse_codex_model_effort_strips_xhigh() {
    let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-xhigh");
    assert_eq!(base, "gpt-5.3-codex");
    assert_eq!(effort, Some("xhigh"));
}

#[test]
fn parse_codex_model_effort_strips_high() {
    let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-high");
    assert_eq!(base, "gpt-5.3-codex");
    assert_eq!(effort, Some("high"));
}

#[test]
fn parse_codex_model_effort_strips_medium() {
    let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-medium");
    assert_eq!(base, "gpt-5.3-codex");
    assert_eq!(effort, Some("medium"));
}

#[test]
fn parse_codex_model_effort_strips_low() {
    let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-low");
    assert_eq!(base, "gpt-5.3-codex");
    assert_eq!(effort, Some("low"));
}

#[test]
fn parse_codex_model_effort_no_suffix() {
    let (base, effort) = parse_codex_model_effort("gpt-5.3-codex");
    assert_eq!(base, "gpt-5.3-codex");
    assert_eq!(effort, None);
}

#[test]
fn parse_codex_model_effort_unknown_suffix() {
    let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-turbo");
    assert_eq!(base, "gpt-5.3-codex-turbo");
    assert_eq!(effort, None);
}
```

Integration test in `tests/backend.rs`: Create a codex backend with a suffixed model via `get_or_create_for_spec("codex(gpt-5.3-codex-xhigh)")`, execute it against a mock script that prints its args, and verify the output contains `--model gpt-5.3-codex` and `-c model_reasoning_effort="xhigh"` (not `--model gpt-5.3-codex-xhigh`).

## Verification

`nix build` passes. A codex backend configured with model `gpt-5.3-codex-xhigh` should produce:
```
codex -c model_reasoning_effort="xhigh" --model gpt-5.3-codex exec --dangerously-bypass-approvals-and-sandbox -
```

A codex backend with model `gpt-5.3-codex` (no suffix) should produce:
```
codex --model gpt-5.3-codex exec --dangerously-bypass-approvals-and-sandbox -
```

## Scope Boundaries

- Do NOT add new config structs or fields
- Do NOT change default model names — keep the existing suffixed defaults
- Do NOT modify `claude.rs` — this is codex-specific
- Do NOT modify `BackendRegistry`, `resolve_backend_for_role()`, or the orchestrator
- Do NOT modify `ralph.toml` live config
- Do NOT add CLI flags for reasoning effort
