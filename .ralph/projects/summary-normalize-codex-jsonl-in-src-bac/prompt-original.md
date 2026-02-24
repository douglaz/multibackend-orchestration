Now I have everything I need. Let me write the revised spec addressing all three review issues:

1. **Testing coverage**: Add a test with two `agent_message` events asserting the second wins.
2. **Parser precedence / fallback semantics**: Track event-based text separately from flat-role text and prefer event-based text at resolution time.
3. **Event-type scoping**: Gate `thread_id` extraction on `type == "thread.started"` and `usage` extraction on `type == "turn.completed"`.

Here is the revised specification:

---

## Summary

Fix `normalize_codex_jsonl()` in `src/backend/output_normalizer.rs` to correctly parse the agentic event JSONL format emitted by `codex exec --json`. The current implementation only looks for `{"role":"assistant","content":"..."}` flat objects, but real Codex output is a typed event stream using `type` discriminators (`thread.started`, `item.completed`, `turn.completed`). This causes every Codex backend call to fail text extraction (returning `Err`), triggering a fallback to the Claude Sonnet reformatter — adding latency, cost, and a point of failure.

## Acceptance Criteria

- `normalize_codex_jsonl` extracts `thread_id` from `{"type":"thread.started","thread_id":"..."}` events into `session_id`, gated on `type == "thread.started"` (not bare `thread_id` key presence)
- `normalize_codex_jsonl` extracts text from `{"type":"item.completed","item":{"type":"agent_message","text":"..."}}`, using the **last** such event when multiple exist
- `normalize_codex_jsonl` extracts usage from `{"type":"turn.completed","usage":{...}}`, gated on `type == "turn.completed"`, mapping `cached_input_tokens` → `cached_in` (note: differs from Claude's `cache_read_input_tokens`)
- Event-format text (`agent_message`) takes explicit precedence over flat-format text (`role:assistant`); if both appear in the same stream, the event-format text wins
- Existing flat-format fallback (`{"role":"assistant","content":"..."}`) still works when no event-format text is present — the old test `codex_jsonl_extracts_text_and_session` continues to pass
- New unit test `codex_real_jsonl_extracts_text_and_session` passes with real Codex JSONL event format
- New unit test `codex_last_agent_message_wins` passes, verifying that when multiple `agent_message` events exist the last one is used
- All existing unit tests continue to pass (`cargo test`)

## Technical Approach

Modify the `normalize_codex_jsonl` function (lines 113–201) to add event-type-scoped extraction branches and explicit precedence between event-format and flat-format text.

### 0. New local variable for event-based text

Add a separate accumulator `event_text: Option<String>` alongside the existing `last_text` (which is renamed to `flat_text` for clarity). At resolution time, `event_text` is preferred over `flat_text`:

```rust
fn normalize_codex_jsonl(raw: &str, _raw_fallback: &str) -> Result<NormalizedOutput> {
    let mut session_id: Option<String> = None;
    let mut event_text: Option<String> = None;   // from item.completed / agent_message
    let mut flat_text: Option<String> = None;     // from role:assistant fallback
    let mut tokens_in: Option<u64> = None;
    let mut tokens_out: Option<u64> = None;
    let mut cached_in: Option<u64> = None;
    let mut found_json = false;
    // ...
```

This ensures that even if a mixed stream contains both flat-format and event-format lines, the event-format text always wins — addressing review issue #2.

### 1. `thread.started` → `session_id` (scoped by event type)

**Replace** the existing bare `thread_id` / `session_id` key check (lines 133–139) with a type-scoped check. This addresses review issue #3: extraction is gated on the `type` field, not just key presence.

```rust
// Extract session/thread id — only from thread.started events
if value.get("type").and_then(|v| v.as_str()) == Some("thread.started") {
    if let Some(id) = value.get("thread_id").and_then(|v| v.as_str()) {
        session_id = Some(id.to_owned());
    }
}

// Flat-format fallback: bare session_id key (no "type" field present)
if session_id.is_none() && value.get("type").is_none() {
    if let Some(id) = value
        .get("thread_id")
        .or_else(|| value.get("session_id"))
        .and_then(|v| v.as_str())
    {
        session_id = Some(id.to_owned());
    }
}
```

The flat-format fallback (guarded by `value.get("type").is_none()`) preserves the existing behavior for old-style `{"thread_id":"..."}` lines that lack a `type` field, keeping the `codex_jsonl_extracts_text_and_session` test passing.

### 2. `item.completed` with `item.type == "agent_message"` → `event_text`

Add a new branch that writes to `event_text` (not `flat_text`), scoped to the `item.completed` event type:

```rust
// Extract text from Codex agentic event format:
// {"type":"item.completed","item":{"type":"agent_message","text":"..."}}
if value.get("type").and_then(|v| v.as_str()) == Some("item.completed") {
    if let Some(item) = value.get("item") {
        if item.get("type").and_then(|v| v.as_str()) == Some("agent_message") {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                event_text = Some(text.to_owned());
            }
        }
    }
}
```

Because later lines overwrite `event_text`, the last `agent_message` event naturally wins. Events with `item.type == "reasoning"` or other non-`agent_message` types are ignored.

### 3. Flat-format text extraction → `flat_text` (existing, renamed)

The existing `role`-based (lines 142–158) and `message`-based text extraction blocks remain but write to `flat_text` instead of `last_text`:

```rust
// Flat-format fallback: extract message text from assistant role
if let Some(role) = value.get("role").and_then(|v| v.as_str()) {
    if role == "assistant" {
        if let Some(text) = value.get("content").and_then(|v| v.as_str()) {
            flat_text = Some(text.to_owned());
        }
    }
}

// Flat-format fallback: extract text from message field
if let Some(msg) = value.get("message") {
    if let Some(role) = msg.get("role").and_then(|v| v.as_str()) {
        if role == "assistant" {
            if let Some(text) = msg.get("content").and_then(|v| v.as_str()) {
                flat_text = Some(text.to_owned());
            }
        }
    }
}
```

### 4. `turn.completed` → usage with `cached_input_tokens` (scoped by event type)

**Replace** the existing bare `usage` key check (lines 162–175) with a two-branch approach: type-scoped for events, unscoped fallback for flat format. Add `cached_input_tokens` as an additional key:

```rust
// Extract usage — from turn.completed events (type-scoped)
if value.get("type").and_then(|v| v.as_str()) == Some("turn.completed") {
    if let Some(usage) = value.get("usage") {
        if let Some(v) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
            tokens_in = Some(v);
        }
        if let Some(v) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
            tokens_out = Some(v);
        }
        if let Some(v) = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()) {
            cached_in = Some(v);
        }
        if let Some(v) = usage.get("cached_input_tokens").and_then(|v| v.as_u64()) {
            cached_in = Some(v);
        }
    }
}

// Flat-format fallback: bare usage object (no "type" field present)
if value.get("type").is_none() {
    if let Some(usage) = value.get("usage") {
        if let Some(v) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
            tokens_in = Some(v);
        }
        if let Some(v) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
            tokens_out = Some(v);
        }
        if let Some(v) = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()) {
            cached_in = Some(v);
        }
    }
}
```

This addresses review issue #3: event-type extraction is scoped to `turn.completed`, while the flat fallback only activates for lines without a `type` field.

### 5. Resolution: event-format text takes precedence

After the loop, resolve the final text with explicit precedence:

```rust
let last_text = event_text.or(flat_text);
```

The rest of the post-loop logic (`found_json` check, `last_text` unwrap-or-error) remains unchanged. This directly addresses review issue #2: in a mixed stream, event-format text always wins regardless of line ordering.

### Change summary

The modification is **additive only** with one rename (`last_text` → split into `event_text` + `flat_text`). The existing extraction branches are preserved and redirected to `flat_text`. No existing branches are removed. Two new conditional blocks are inserted into the per-line loop, the session_id and usage blocks are tightened with type-scoping, and two new unit tests are added.

## Files & Modules

| File | Change |
|------|--------|
| `src/backend/output_normalizer.rs` | Modify `normalize_codex_jsonl()` (lines 113–201): split `last_text` into `event_text` + `flat_text` with explicit precedence; add `item.completed`/`agent_message` → `event_text` extraction; scope `thread_id` extraction to `type == "thread.started"` with flat fallback; scope `usage` extraction to `type == "turn.completed"` with flat fallback; add `cached_input_tokens` usage key. Add tests `codex_real_jsonl_extracts_text_and_session` and `codex_last_agent_message_wins` to `mod tests`. |

No other files require changes. The `NormalizedOutput` struct and `normalize_output()` dispatch function are unchanged. The caller in `src/workflow/orchestrator.rs` already handles all `NormalizedOutput` fields correctly.

## Testing Strategy

### New tests

**1. `codex_real_jsonl_extracts_text_and_session`** — validates the primary event-format parsing path with a realistic multi-event JSONL payload:

```rust
#[test]
fn codex_real_jsonl_extracts_text_and_session() {
    let jsonl = r#"{"type":"thread.started","thread_id":"019c6e78-abc"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"thinking..."}}
{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"# Implementation Notes\n\nDone."}}
{"type":"turn.completed","usage":{"input_tokens":8790,"cached_input_tokens":6784,"output_tokens":24}}"#;
    let result = normalize_output("codex", jsonl).unwrap();
    assert_eq!(result.text, "# Implementation Notes\n\nDone.");
    assert_eq!(result.session_id.as_deref(), Some("019c6e78-abc"));
    assert_eq!(result.tokens_in, Some(8790));
    assert_eq!(result.tokens_out, Some(24));
    assert_eq!(result.cached_in, Some(6784));
}
```

Asserts: text from `agent_message` (not `reasoning`), `session_id` from `thread.started`, usage with `cached_input_tokens` mapping.

**2. `codex_last_agent_message_wins`** — validates that when multiple `agent_message` events exist, the last one is used (review issue #1):

```rust
#[test]
fn codex_last_agent_message_wins() {
    let jsonl = r#"{"type":"thread.started","thread_id":"t-1"}
{"type":"item.completed","item":{"type":"agent_message","text":"First draft"}}
{"type":"item.completed","item":{"type":"agent_message","text":"Revised answer"}}
{"type":"turn.completed","usage":{"input_tokens":100,"output_tokens":10}}"#;
    let result = normalize_output("codex", jsonl).unwrap();
    assert_eq!(result.text, "Revised answer");
}
```

Asserts: the second `agent_message` text overwrites the first.

### Existing tests to verify

| Test | Expectation | Why it still passes |
|------|-------------|---------------------|
| `codex_jsonl_extracts_text_and_session` | Still passes | Flat-format lines have no `type` field, so they hit the `flat_text` path; `event_text` is `None`, so `flat_text` is used |
| `codex_non_json_returns_raw` | Still passes | Non-JSON fallback unchanged |
| `codex_jsonl_missing_assistant_returns_err` | Still passes | System-role-only JSONL has neither `agent_message` events nor `role:assistant` lines, so both `event_text` and `flat_text` are `None` |
| All `claude_*` tests | Unaffected | Different code path (`normalize_claude_json`) |
| `malformed_json_does_not_panic` | Unaffected | Malformed JSON falls through to Claude's raw fallback |
| `empty_output_returns_raw` | Unaffected | Early return before backend dispatch |
| `unknown_backend_returns_raw` | Unaffected | Non-matching backend name, raw fallback |

### Validation commands

```bash
cargo test --lib output_normalizer
cargo test
```

## Out of Scope

- **Streaming / partial event handling**: This fix handles complete JSONL output after process exit; incremental streaming parse is not addressed.
- **Other Codex event types**: Events like `tool_call`, `tool_result`, `error`, etc. are not extracted. Only the three event types needed for `NormalizedOutput` fields are handled.
- **Codex content array format**: If Codex ever emits `agent_message` with a `content` array instead of a `text` string, that would require a separate change.
- **Integration tests against a real Codex binary**: The fix is validated via unit tests with representative JSONL; end-to-end integration testing is out of scope for this change.
- **Refactoring the exhaustive match arms in `orchestrator.rs`**: The metrics logging function has a verbose pattern match over token field combinations; cleaning that up is unrelated.
- **Mixed-stream event+flat precedence for session_id/usage**: Event-format text has explicit precedence over flat-format text. For `session_id` and usage, the simpler approach of "flat fallback only when no `type` field" is used rather than a full dual-accumulator pattern, since these fields are less likely to conflict in practice.