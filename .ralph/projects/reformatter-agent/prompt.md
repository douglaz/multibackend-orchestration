# Reformatter Agent

## Goal

Change the parse-retry strategy in `execute_with_parse_retries()` (`src/workflow/orchestrator.rs`) so that when a backend produces unparseable output, the **opposite backend** is used as a "reformatter agent" to fix the output before retrying with the original backend.

## Current Behavior (3-attempt retry in `execute_with_parse_retries`)

1. **Attempt 1**: Send original prompt to the assigned backend → parse fails
2. **Attempt 2**: Send a reformat prompt (with the bad output + error + expected structure) to the **same** backend → parse fails
3. **Attempt 3**: Send the original prompt with a format reminder prepended to the **same** backend → parse fails → `ParseRetriesExhausted`

## New Behavior (3-attempt retry)

1. **Attempt 1**: Send original prompt to the assigned backend → parse fails
2. **Attempt 2 (reformatter agent)**: Send the reformat prompt to the **opposite backend** (obtained via `BackendRegistry::opposite()` + `BackendRegistry::get()`) → if parse succeeds, return; if parse fails, continue
3. **Attempt 3**: Send the original prompt with the format reminder prepended to the **original backend** (same as before) → parse fails → `ParseRetriesExhausted`

The only change is in attempt 2: use the opposite backend instead of the same backend. The reformat prompt content stays the same. Attempt 3 stays the same (original backend, reminded prompt).

## Implementation Details

### Signature change for `execute_with_parse_retries`

Add a `registry: &BackendRegistry` parameter so the function can look up the opposite backend. The function already receives `backend: Arc<dyn Backend>` for the primary backend — it needs the registry to resolve the opposite.

All call sites (planner, implementer initial, implementer feedback, reviewer, completer) already have `&registry` in scope — just pass it through.

### Resolving the opposite backend

```rust
let opposite_name = registry.opposite(backend.name())?;
let reformatter = registry.get(opposite_name).ok_or_else(|| {
    RalphError::BackendUnavailable { backend: opposite_name.to_owned() }
})?;
```

If the opposite backend is unavailable (e.g. single-backend config), fall back to using the original backend for the reformat attempt (current behavior). This should be a graceful fallback, not an error.

### Logging

Update log messages in the retry flow:
- Attempt 2: `"parse failed, requesting reformat via {reformatter_name} (attempt 2/3)"`
- Attempt 3: stays the same

### What NOT to change

- Do not change `execute_with_timeout_retries` — it stays as-is
- Do not change any prompt content or templates
- Do not change the parser functions
- Do not add new config options — the reformatter is always the opposite backend
- Do not change `BackendRegistry` or `Backend` trait — they already have everything needed
- Do not change the number of retry attempts (still 3 total)

### Tests

- Add a unit test (or extend existing orchestrator tests) that verifies the reformatter uses the opposite backend
- The existing integration tests should continue to pass since the mock backends all use the same script (both backends produce the same parseable output)
