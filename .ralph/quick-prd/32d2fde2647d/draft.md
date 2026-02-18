Now I have a complete picture of the code and its usage. Let me write the spec.

## Summary

Fix `normalize_codex_jsonl()` in `src/backend/output_normalizer.rs` to correctly parse the agentic event JSONL format emitted by `codex exec --json`. The current implementation only looks for `{"role":"assistant","content":"..."}` flat objects, but real Codex output is a typed event stream using `type` discriminators (`thread.started`, `item.completed`, `turn.completed`). This causes every Codex backend call to fail text extraction (returning `Err`), triggering a fallback to the Claude Sonnet reformatter — adding latency, cost, and a point of failure.

## Acceptance Criteria

- `normalize_codex_jsonl` extracts `thread_id` from `{"type":"thread.started","thread_id":"..."}` events into `session_id`
- `normalize_codex_jsonl` extracts text from `{"type":"item.completed","item":{"type":"agent_message","text":"..."}}`, using the **last** such event when multiple exist
- `normalize_codex_jsonl` extracts usage from `{"type":"turn.completed","usage":{...}}`, mapping `cached_input_tokens` → `cached_in` (note: differs from Claude's `cache_read_input_tokens`)
- Existing flat-format fallback (`{"role":"assistant","content":"..."}`) still works — the old test `codex_jsonl_extracts_text_and_session` continues to pass
- New unit test `codex_real_jsonl_extracts_text_and_session` passes with real Codex JSONL event format
- All existing unit tests continue to pass (`cargo test`)

## Technical Approach

Modify the `normalize_codex_jsonl` function (lines 113–201) to add three new extraction branches **before** the existing fallback branches, checked via the `"type"` field on each parsed JSON line:

### 1. `thread.started` → `session_id` (lines ~133–139)

The existing code already extracts `thread_id` from flat top-level keys, which accidentally works for `{"type":"thread.started","thread_id":"..."}` since `thread_id` is a top-level field in that event. **No change needed here** — the current `value.get("thread_id")` check at line 133 already handles this.

### 2. `item.completed` with `item.type == "agent_message"` → `last_text`

Add a new branch that checks for the typed event format. Insert **before** the existing `role`-based extraction (line 142) so the event-based format takes priority when both patterns could match:

```rust
// Extract text from Codex agentic event format:
// {"type":"item.completed","item":{"type":"agent_message","text":"..."}}
if let Some(event_type) = value.get("type").and_then(|v| v.as_str()) {
    if event_type == "item.completed" {
        if let Some(item) = value.get("item") {
            if item.get("type").and_then(|v| v.as_str()) == Some("agent_message") {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    last_text = Some(text.to_owned());
                }
            }
        }
    }
}
```

This uses the last occurrence naturally since later lines overwrite `last_text`, matching the existing behavior for the `role`-based path. Events with `item.type == "reasoning"` or other non-`agent_message` types are correctly ignored.

### 3. `turn.completed` → usage with `cached_input_tokens`

Add `cached_input_tokens` as an alternative key in the existing usage extraction block (line 162). The current code only checks `cache_read_input_tokens` (Claude's key), but Codex uses `cached_input_tokens`. Add it as a fallback:

```rust
// In the existing usage block, after the cache_read_input_tokens check:
if cached_in.is_none() {
    if let Some(v) = usage.get("cached_input_tokens").and_then(|v| v.as_u64()) {
        cached_in = Some(v);
    }
}
```

The existing `input_tokens` and `output_tokens` extraction already works for both backends since the field names match. The usage block is also already reached for `turn.completed` events because the check is `value.get("usage")` — the `usage` object is a top-level field in `{"type":"turn.completed","usage":{...}}`.

### 4. Retain existing fallback paths

The existing `role`-based text extraction (lines 142–158) and `cache_read_input_tokens` key remain untouched, preserving backward compatibility with the old flat format used in existing tests.

### Change summary

The modification is **additive only** — two new conditional blocks inserted into the existing per-line loop, plus one unit test. No existing branches are removed or reordered.

## Files & Modules

| File | Change |
|------|--------|
| `src/backend/output_normalizer.rs` | Modify `normalize_codex_jsonl()` (lines 113–201): add `item.completed`/`agent_message` text extraction, add `cached_input_tokens` usage key. Add test `codex_real_jsonl_extracts_text_and_session` to `mod tests`. |

No other files require changes. The `NormalizedOutput` struct and `normalize_output()` dispatch function are unchanged. The caller in `src/workflow/orchestrator.rs` already handles all `NormalizedOutput` fields correctly.

## Testing Strategy

### New test

Add `codex_real_jsonl_extracts_text_and_session` to the existing `mod tests` block. This test uses a realistic multi-event JSONL payload including `thread.started`, `turn.started`, two `item.completed` events (one `reasoning`, one `agent_message`), and `turn.completed` with usage. It asserts:

- `text` equals the `agent_message` text (not the `reasoning` text)
- `session_id` equals the `thread_id` from `thread.started`
- `tokens_in`, `tokens_out`, `cached_in` are correctly extracted with `cached_input_tokens` mapping

### Existing tests to verify

| Test | Expectation |
|------|-------------|
| `codex_jsonl_extracts_text_and_session` | Still passes — the flat `{"role":"assistant"}` path is untouched |
| `codex_non_json_returns_raw` | Still passes — non-JSON fallback unchanged |
| `codex_jsonl_missing_assistant_returns_err` | Still passes — system-role-only JSONL still has no `agent_message` event |
| All `claude_*` tests | Unaffected — different code path |
| `malformed_json_does_not_panic` | Unaffected |

### Validation command

```bash
cargo test --lib output_normalizer
```

Then full suite:

```bash
cargo test
```

## Out of Scope

- **Streaming / partial event handling**: This fix handles complete JSONL output after process exit; incremental streaming parse is not addressed.
- **Other Codex event types**: Events like `tool_call`, `tool_result`, `error`, etc. are not extracted. Only the three event types needed for `NormalizedOutput` fields are handled.
- **Codex content array format**: If Codex ever emits `agent_message` with a `content` array instead of a `text` string, that would require a separate change.
- **Integration tests against a real Codex binary**: The fix is validated via unit tests with representative JSONL; end-to-end integration testing is out of scope for this change.
- **Refactoring the exhaustive match arms in `orchestrator.rs`**: The metrics logging function has a verbose pattern match over token field combinations; cleaning that up is unrelated.