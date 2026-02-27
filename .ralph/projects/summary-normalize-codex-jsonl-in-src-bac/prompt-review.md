---
artifact: prompt-review
project: summary-normalize-codex-jsonl-in-src-bac
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-18T03:59:31Z
---

# Prompt Review

## Issues Found
- The spec references exact line numbers in `src/backend/output_normalizer.rs`; line-anchored instructions are brittle and can mislead implementation after unrelated edits.
- `cached_input_tokens` vs `cache_read_input_tokens` conflict resolution is not explicit when both keys are present; this can produce nondeterministic `cached_in`.
- Event-type scoping is described but not fully test-enforced; missing negative tests can allow regressions where `thread_id`/`usage` are read from wrong event types.
- Text precedence is explicit, but session/usage precedence in mixed streams is only partially defined; implementers may choose inconsistent behavior.
- Malformed JSONL line handling is not explicitly constrained; parser robustness could regress (panic, early return, or dropped valid later lines).
- The "no other files require changes" statement is over-constraining; if test organization requires minor updates elsewhere, this can block correct implementation.
- Validation commands include a target pattern that may not match all cargo setups; verification instructions should be resilient.

## Refined Prompt

### Title
Fix Codex JSONL event parsing in `normalize_codex_jsonl` with explicit event scoping and precedence

### Objective
Update Codex output normalization so real `codex exec --json` event streams are parsed correctly, preventing false `Err` results and unnecessary Claude reformat fallback.

### File Scope
- Primary file: `src/backend/output_normalizer.rs`
- Modify only what is required for this fix and unit tests in the same module/file (or existing test location for this module).

### Required Behavior
1. Parse Codex JSONL line-by-line, ignoring malformed lines without panicking.
2. Extract `session_id` only from:
   - `{"type":"thread.started","thread_id":"..."}`
3. Extract assistant text from event stream only from:
   - `{"type":"item.completed","item":{"type":"agent_message","text":"..."}}`
4. If multiple `agent_message` events exist, the last one wins.
5. Keep flat-format fallback text parsing for:
   - `{"role":"assistant","content":"..."}`
   - `{"message":{"role":"assistant","content":"..."}}`
6. Text precedence rule:
   - Event-based text (`agent_message`) must override flat-format assistant text when both exist.
7. Extract usage only from:
   - `{"type":"turn.completed","usage":{...}}`
   - Map:
     - `input_tokens -> tokens_in`
     - `output_tokens -> tokens_out`
     - `cached_input_tokens -> cached_in` (preferred key)
     - fallback to `cache_read_input_tokens -> cached_in` when `cached_input_tokens` is absent
8. Flat fallback for `session_id` and `usage` is allowed only when the JSON object has no `type` field (legacy format compatibility).
9. Preserve existing behavior for:
   - non-JSON raw fallback
   - empty output handling
   - unknown backend handling
   - Claude normalization path

### Implementation Constraints
- Introduce separate accumulators for text:
  - `event_text: Option<String>`
  - `flat_text: Option<String>`
- Resolve final text with explicit precedence: `event_text` first, then `flat_text`.
- Do not change `NormalizedOutput` shape or `normalize_output()` backend dispatch contract.
- Keep changes minimal and additive.

### Test Requirements
Add/ensure unit tests that cover:

1. `codex_real_jsonl_extracts_text_and_session`
   - Event stream includes `thread.started`, non-message item, `agent_message`, `turn.completed`.
   - Asserts: text from `agent_message`, session id from `thread.started`, tokens in/out, `cached_in` from `cached_input_tokens`.

2. `codex_last_agent_message_wins`
   - Two `agent_message` events.
   - Asserts final text is from the second event.

3. `codex_event_text_precedes_flat_text`
   - Stream contains both flat assistant text and event `agent_message`.
   - Asserts event text wins regardless of order.

4. `codex_thread_and_usage_are_type_scoped`
   - Include objects with `thread_id`/`usage` under incorrect `type`.
   - Asserts they are ignored unless `type` is `thread.started` / `turn.completed`.

5. Existing tests remain passing, especially:
   - `codex_jsonl_extracts_text_and_session`
   - `codex_jsonl_missing_assistant_returns_err`
   - `codex_non_json_returns_raw`
   - `malformed_json_does_not_panic`

### Definition of Done
- All required tests pass.
- No regressions in existing test suite.
- Codex JSONL event streams no longer fail text extraction when valid `agent_message` events are present.

### Verification Commands
```bash
cargo test output_normalizer
cargo test
```

### Out of Scope
- Streaming incremental parse behavior during process runtime.
- New event types beyond `thread.started`, `item.completed(agent_message)`, `turn.completed`.
- Refactors outside output normalization and its direct tests.
