use serde_json::Value;

use crate::error::RalphError;
use crate::Result;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizedOutput {
    pub text: String,
    pub session_id: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub cached_in: Option<u64>,
}

/// Known NDJSON stream event types used to distinguish multi-line streams
/// from single-object JSON responses that happen to contain a `type` field.
///
/// Includes:
/// - Claude API stream events (`message_start`, `content_block_*`, etc.)
/// - Claude CLI verbose events (`system` init is the first event)
/// - Codex CLI events (`thread.started` is the first event)
const STREAM_EVENT_TYPES: &[&str] = &[
    // Claude API stream-json
    "message_start",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
    "message_delta",
    "message_stop",
    "ping",
    "summary",
    // Claude CLI --verbose (first event is "system")
    "system",
    // Codex CLI (first event is "thread.started")
    "thread.started",
    // Additional stream-json event names emitted by CLI backends
    "init",
    "message",
    "tool_use",
    "tool_result",
    // Goose CLI
    "complete",
    "notification",
    // OpenCode CLI
    "step_start",
];

pub fn normalize_output(raw: &str) -> Result<NormalizedOutput> {
    let raw_len = raw.len();
    let first_content_line = raw.lines().map(str::trim).find(|l| !l.is_empty());
    let first_preview: String = first_content_line.unwrap_or("").chars().take(80).collect();

    if !first_content_line.is_some_and(|l| l.starts_with('{')) {
        // Before returning raw text, check for multi-line JSON after preamble
        // (some CLIs print status lines before the JSON response).
        if let Some(output) = try_extract_multiline_json_after_preamble(raw) {
            tracing::debug!(
                path = "preamble_multiline",
                raw_len,
                first_line = %first_preview,
                extracted_len = output.text.len(),
                "normalize_output: extracted multi-line JSON after preamble"
            );
            return Ok(output);
        }
        tracing::debug!(
            path = "raw_text",
            raw_len,
            first_line = %first_preview,
            "normalize_output: returning raw text (first line not JSON)"
        );
        return Ok(NormalizedOutput {
            text: raw.to_owned(),
            ..NormalizedOutput::default()
        });
    }

    let Some(first_json) = first_valid_json_object(raw) else {
        // First line starts with '{' but no single line parses as valid JSON —
        // the output is likely a multi-line pretty-printed JSON object.
        if let Some(output) = try_extract_multiline_json_after_preamble(raw) {
            tracing::debug!(
                path = "multiline_json",
                raw_len,
                first_line = %first_preview,
                extracted_len = output.text.len(),
                "normalize_output: extracted multi-line JSON (no single-line match)"
            );
            return Ok(output);
        }
        tracing::debug!(
            path = "raw_text_json_start",
            raw_len,
            first_line = %first_preview,
            "normalize_output: returning raw text (JSON start but no parse)"
        );
        return Ok(NormalizedOutput {
            text: raw.to_owned(),
            ..NormalizedOutput::default()
        });
    };

    // Route to stream normalization only if the first JSON object's `type`
    // matches a known Claude stream event type, not just any `type` field.
    let is_stream = first_json
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|t| STREAM_EVENT_TYPES.contains(&t));

    if is_stream {
        normalize_claude_stream_json(raw)
    } else {
        normalize_claude_single_json(raw)
    }
}

pub fn normalize_claude_stream_json(raw: &str) -> Result<NormalizedOutput> {
    let mut output = NormalizedOutput::default();
    let mut json_event_count = 0_usize;
    let mut last_goose_msg_id: Option<String> = None;
    let mut result_text: Option<String> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if !event.is_object() {
            continue;
        }
        json_event_count += 1;

        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            // --- Claude API stream-json events ---
            "message_start" => {
                if output.session_id.is_none() {
                    output.session_id = event
                        .pointer("/message/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                merge_usage_from_event(&event, &mut output);
            }
            "content_block_delta" => {
                if let Some(text) = event.pointer("/delta/text").and_then(Value::as_str) {
                    output.text.push_str(text);
                }
                merge_usage_from_event(&event, &mut output);
            }
            "message_delta" | "message_stop" | "summary" => {
                merge_usage_from_event(&event, &mut output);
            }
            "content_block_start" | "content_block_stop" | "ping" => {}

            // --- Claude CLI --verbose events ---
            "system" if output.session_id.is_none() => {
                // Init event; extract session_id
                output.session_id = event
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "assistant" => {
                // Response event; text is in message.content[].text
                if let Some(content) = event.pointer("/message/content") {
                    if let Some(text) = extract_text_from_content(content) {
                        if !output.text.is_empty() && !output.text.ends_with('\n') {
                            output.text.push('\n');
                        }
                        output.text.push_str(&text);
                    }
                }
                if output.session_id.is_none() {
                    output.session_id = event
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                merge_usage_from_event(&event, &mut output);
            }
            "init" if output.session_id.is_none() => {
                output.session_id = event
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "message" => {
                // Goose CLI wraps messages in {"type":"message","message":{...}}.
                // Some CLIs emit {"type":"message","role":"assistant","content":"..."}.
                if let Some(inner) = event.get("message") {
                    // Goose format: extract text from inner message.content[]
                    // Only extract "text" type content, skip "reasoning" and "toolRequest".
                    // Goose streams token-by-token: each token arrives as a separate
                    // "message" event sharing the same message ID. We concatenate
                    // tokens from the same message directly (no separator), but insert
                    // a newline when a new message ID appears (different assistant turn).
                    let role = inner.get("role").and_then(Value::as_str);
                    if role == Some("assistant") {
                        let msg_id = inner.get("id").and_then(Value::as_str);
                        let is_new_message = msg_id.is_some_and(|id| {
                            last_goose_msg_id.as_deref().is_some_and(|prev| prev != id)
                        });
                        if let Some(id) = msg_id {
                            last_goose_msg_id = Some(id.to_owned());
                        }
                        if let Some(items) = inner.pointer("/content").and_then(Value::as_array) {
                            for item in items {
                                let ct = item.get("type").and_then(Value::as_str);
                                if ct == Some("text") {
                                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                                        if is_new_message
                                            && !output.text.is_empty()
                                            && !output.text.ends_with('\n')
                                        {
                                            output.text.push('\n');
                                        }
                                        output.text.push_str(text);
                                    }
                                }
                            }
                        }
                    }
                    // Extract session from inner message.id
                    if output.session_id.is_none() {
                        output.session_id =
                            inner.get("id").and_then(Value::as_str).map(str::to_owned);
                    }
                } else if event.get("role").and_then(Value::as_str) == Some("assistant") {
                    // Alternate stream format: role is at top level
                    if let Some(content) = event.get("content") {
                        if let Some(text) = extract_text_from_content(content) {
                            if !output.text.is_empty() && !output.text.ends_with('\n') {
                                output.text.push('\n');
                            }
                            output.text.push_str(&text);
                        }
                    }
                }
                if output.session_id.is_none() {
                    output.session_id = event
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
            }
            "tool_use" | "tool_result" => {}
            "result" => {
                // Summary event; capture result text — it contains the
                // clean final response without narration from assistant events.
                // When multiple result events exist (e.g. claude uses tools
                // between responses), keep the longest one since shorter
                // follow-ups are typically summaries, not the full content.
                if let Some(text) = extract_result_event_text(&event) {
                    if !text.is_empty() {
                        let dominated = result_text
                            .as_ref()
                            .is_some_and(|prev| prev.len() >= text.len());
                        if !dominated {
                            result_text = Some(text);
                        }
                    }
                }
                if output.session_id.is_none() {
                    output.session_id = event
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                merge_usage_from_event(&event, &mut output);
            }

            // --- Codex CLI events ---
            "thread.started" if output.session_id.is_none() => {
                output.session_id = event
                    .get("thread_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "item.completed" => {
                // Only extract text from agent_message items, not reasoning items.
                let item_type = event.pointer("/item/type").and_then(Value::as_str);
                if item_type == Some("agent_message") {
                    if let Some(text) = event.pointer("/item/text").and_then(Value::as_str) {
                        // Separate consecutive agent_message items with a newline
                        // so that markdown headings in later messages remain on
                        // their own line after concatenation.
                        if !output.text.is_empty() && !output.text.ends_with('\n') {
                            output.text.push('\n');
                        }
                        output.text.push_str(text);
                    }
                }
            }
            "turn.completed" => {
                merge_usage_from_event(&event, &mut output);
            }
            "turn.started" => {}

            // --- Goose CLI events ---
            "complete" => {
                // Final event: {"type":"complete","total_tokens":N}
                if let Some(total) = event.get("total_tokens").and_then(Value::as_u64) {
                    // total_tokens is the sum; we don't get a breakdown, but
                    // store in tokens_in if no prior value (best-effort).
                    if output.tokens_in.is_none() && output.tokens_out.is_none() {
                        output.tokens_in = Some(total);
                    }
                }
            }
            "notification" => {
                // Log/notification events from goose extensions; skip.
            }

            // --- OpenCode CLI events ---
            "step_start" if output.session_id.is_none() => {
                output.session_id = event
                    .get("sessionID")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "text" => {
                if let Some(text) = event.pointer("/part/text").and_then(Value::as_str) {
                    if !output.text.is_empty() && !output.text.ends_with('\n') {
                        output.text.push('\n');
                    }
                    output.text.push_str(text);
                }
                if output.session_id.is_none() {
                    output.session_id = event
                        .get("sessionID")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
            }
            "step_finish" => {
                // Extract tokens from part.tokens: {input, output, reasoning, cache: {read, write}}
                if let Some(tokens) = event.pointer("/part/tokens") {
                    output.tokens_in = extract_u64(tokens, &["input"]).or(output.tokens_in);
                    output.tokens_out = extract_u64(tokens, &["output"]).or(output.tokens_out);
                    if let Some(cache) = tokens.get("cache") {
                        output.cached_in = extract_u64(cache, &["read"]).or(output.cached_in);
                    }
                }
                if output.session_id.is_none() {
                    output.session_id = event
                        .get("sessionID")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
            }

            _ => {}
        }
    }

    // For Claude CLI verbose streams, prefer the clean result text over
    // the concatenation of assistant narration events.
    if let Some(rt) = result_text {
        output.text = rt;
    }

    if json_event_count == 0 {
        return Ok(NormalizedOutput {
            text: raw.to_owned(),
            ..output
        });
    }

    if output.text.is_empty() {
        return Err(RalphError::ParseError(
            "claude stream-json output contained JSON events but no text deltas".to_owned(),
        ));
    }

    Ok(output)
}

pub fn normalize_claude_single_json(raw: &str) -> Result<NormalizedOutput> {
    let value = first_valid_json_object(raw).ok_or_else(|| {
        RalphError::ParseError("claude json output did not contain a valid JSON object".to_owned())
    })?;

    let text = extract_single_json_text(&value).ok_or_else(|| {
        RalphError::ParseError("claude json output contained no text content".to_owned())
    })?;

    let mut output = NormalizedOutput {
        text,
        session_id: value
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                value
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
        ..NormalizedOutput::default()
    };
    merge_usage_from_event(&value, &mut output);
    Ok(output)
}

fn first_valid_json_object(raw: &str) -> Option<Value> {
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                serde_json::from_str::<Value>(trimmed).ok()
            }
        })
        .find(|value| value.is_object())
}

/// Attempt to extract a multi-line JSON object from output that may have
/// non-JSON preamble lines (for example, status messages printed before a
/// pretty-printed JSON response body).
///
/// When the output contains multiple JSON-like blocks (e.g. a 429 error JSON
/// followed by the actual response JSON), we try each `{`-starting line from
/// **last to first** so we pick up the final (actual) response rather than
/// an intermediate error object.
fn try_extract_multiline_json_after_preamble(raw: &str) -> Option<NormalizedOutput> {
    let lines: Vec<&str> = raw.lines().collect();

    // Collect all line indices where trimmed content starts with '{'.
    let brace_positions: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim().starts_with('{'))
        .map(|(i, _)| i)
        .collect();

    if brace_positions.is_empty() {
        return None;
    }

    // Check the very first brace position's preamble for markdown indicators.
    // If the content before the first '{' line has markdown, this is a markdown
    // document with embedded JSON — bail out.
    let first_brace = brace_positions[0];
    for line in &lines[..first_brace] {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('#') || t.starts_with("```") {
            tracing::debug!(
                preamble_line = %t,
                "try_extract_multiline_json: bailing out — preamble has markdown"
            );
            return None;
        }
    }

    // Try each '{' position from last to first — the final JSON block in the
    // output is most likely the actual response (earlier ones may be error
    // JSON from 429 retries, etc.).
    for &start in brace_positions.iter().rev() {
        let json_text: String = lines[start..].join("\n");
        let value: Value = match serde_json::from_str(&json_text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !value.is_object() {
            continue;
        }

        let text = match extract_single_json_text(&value) {
            Some(t) => t,
            None => continue,
        };

        let mut output = NormalizedOutput {
            text,
            session_id: value
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    value
                        .pointer("/message/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }),
            ..NormalizedOutput::default()
        };
        merge_usage_from_event(&value, &mut output);

        tracing::debug!(
            json_start = start,
            total_lines = lines.len(),
            extracted_len = output.text.len(),
            "try_extract_multiline_json: found valid JSON block"
        );
        return Some(output);
    }

    tracing::debug!(
        candidates = brace_positions.len(),
        total_lines = lines.len(),
        "try_extract_multiline_json: no candidate parsed as valid JSON with text key"
    );
    None
}

fn extract_single_json_text(value: &Value) -> Option<String> {
    for key in ["result", "response", "output", "text", "completion"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return Some(text.to_owned());
        }
    }

    if let Some(content) = value.get("content") {
        if let Some(text) = extract_text_from_content(content) {
            return Some(text);
        }
    }

    if let Some(message) = value.get("message") {
        if let Some(text) = message.get("text").and_then(Value::as_str) {
            return Some(text.to_owned());
        }
        if let Some(content) = message.get("content") {
            if let Some(text) = extract_text_from_content(content) {
                return Some(text);
            }
        }
    }

    None
}

fn extract_result_event_text(event: &Value) -> Option<String> {
    if let Some(text) = event.get("result").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(text) = event.get("text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(content) = event.get("content") {
        if let Some(text) = extract_text_from_content(content) {
            return Some(text);
        }
    }
    None
}

fn extract_text_from_content(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }

    let mut parts = Vec::new();
    let items = content.as_array()?;

    for item in items {
        if let Some(text) = item.as_str() {
            parts.push(text);
            continue;
        }
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            parts.push(text);
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.concat())
    }
}

fn merge_usage_from_event(event: &Value, output: &mut NormalizedOutput) {
    for pointer in ["/usage", "/message/usage", "/delta/usage", "/summary/usage"] {
        if let Some(usage) = event.pointer(pointer) {
            merge_usage_fields(usage, output);
        }
    }
    merge_usage_fields(event, output);
}

fn merge_usage_fields(usage: &Value, output: &mut NormalizedOutput) {
    output.tokens_in = extract_u64(usage, &["tokens_in", "input_tokens"]).or(output.tokens_in);
    output.tokens_out = extract_u64(usage, &["tokens_out", "output_tokens"]).or(output.tokens_out);
    output.cached_in = extract_u64(
        usage,
        &[
            "cached_in",
            "cache_read_input_tokens",
            "cached_input_tokens",
            "cached_tokens",
            "cache_creation_input_tokens",
        ],
    )
    .or(output.cached_in);
}

fn extract_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        let Some(field) = value.get(*key) else {
            continue;
        };
        if let Some(n) = field.as_u64() {
            return Some(n);
        }
        if let Some(n) = field.as_i64().and_then(|n| u64::try_from(n).ok()) {
            return Some(n);
        }
        if let Some(parsed) = field.as_str().and_then(|s| s.parse::<u64>().ok()) {
            return Some(parsed);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{normalize_claude_stream_json, normalize_output};
    use crate::error::RalphError;

    #[test]
    fn stream_json_concatenates_deltas_in_order_and_extracts_metadata() {
        let raw = r#"
{"type":"message_start","message":{"id":"msg_123"}}
{"type":"content_block_delta","delta":{"text":"Hello "}}
{"type":"content_block_delta","delta":{"text":"world"}}
{"type":"message_delta","usage":{"tokens_in":12,"tokens_out":34,"cached_in":5}}
"#;

        let normalized = normalize_claude_stream_json(raw).expect("stream normalization");
        assert_eq!(normalized.text, "Hello world");
        assert_eq!(normalized.session_id.as_deref(), Some("msg_123"));
        assert_eq!(normalized.tokens_in, Some(12));
        assert_eq!(normalized.tokens_out, Some(34));
        assert_eq!(normalized.cached_in, Some(5));
    }

    #[test]
    fn stream_json_extracts_usage_from_summary_event() {
        let raw = r#"
{"type":"message_start","message":{"id":"msg_1"}}
{"type":"content_block_delta","delta":{"text":"ok"}}
{"type":"summary","usage":{"tokens_in":7,"tokens_out":9,"cached_in":2}}
"#;

        let normalized = normalize_claude_stream_json(raw).expect("stream normalization");
        assert_eq!(normalized.text, "ok");
        assert_eq!(normalized.tokens_in, Some(7));
        assert_eq!(normalized.tokens_out, Some(9));
        assert_eq!(normalized.cached_in, Some(2));
    }

    #[test]
    fn stream_json_errors_when_events_exist_without_text_deltas() {
        let raw = r#"
{"type":"message_start","message":{"id":"msg_1"}}
{"type":"ping"}
{"type":"message_stop"}
"#;

        match normalize_claude_stream_json(raw) {
            Err(RalphError::ParseError(message)) => {
                assert!(message.contains("no text deltas"), "message={message}");
            }
            other => panic!("expected ParseError, got: {other:?}"),
        }
    }

    #[test]
    fn stream_json_skips_unknown_events_and_malformed_lines() {
        let raw = r#"
not-json
{"type":"unknown_event","foo":"bar"}
{"type":"content_block_delta","delta":{"text":"A"}}
{"type":"content_block_delta","delta":{"text":"B"}}
"#;

        let normalized = normalize_claude_stream_json(raw).expect("stream normalization");
        assert_eq!(normalized.text, "AB");
    }

    #[test]
    fn normalize_output_routes_stream_json_when_first_line_is_stream_event() {
        let raw = r#"{"type":"message_start","message":{"id":"msg_22"}}
{"type":"content_block_delta","delta":{"text":"NDJSON"}}
"#;

        let normalized = normalize_output(raw).expect("normalize_output");
        assert_eq!(normalized.text, "NDJSON");
        assert_eq!(normalized.session_id.as_deref(), Some("msg_22"));
    }

    #[test]
    fn normalize_output_routes_single_json_when_first_json_has_no_type() {
        let raw = r#"{"result":"single-json","session_id":"sess_1","usage":{"tokens_in":1,"tokens_out":2,"cached_in":3}}"#;

        let normalized = normalize_output(raw).expect("normalize_output");
        assert_eq!(normalized.text, "single-json");
        assert_eq!(normalized.session_id.as_deref(), Some("sess_1"));
        assert_eq!(normalized.tokens_in, Some(1));
        assert_eq!(normalized.tokens_out, Some(2));
        assert_eq!(normalized.cached_in, Some(3));
    }

    #[test]
    fn normalize_output_extracts_json_after_non_json_preamble() {
        // Non-JSON preamble lines followed by a single-line JSON object.
        // The multi-line JSON fallback should extract the JSON text.
        let raw = "nope\nstill nope\n{\"result\":\"from-json\"}";
        let normalized = normalize_output(raw).expect("normalize_output");
        assert_eq!(normalized.text, "from-json");
    }

    #[test]
    fn normalize_output_returns_raw_text_when_no_json_events_exist() {
        let raw = "# Implementation Notes\n\n## Decisions Made\n- text";
        let normalized = normalize_output(raw).expect("raw text fallback");
        assert_eq!(normalized.text, raw);
        assert_eq!(normalized.session_id, None);
    }

    // --- P1 regression: markdown with embedded JSON must not be misrouted ---

    #[test]
    fn normalize_output_markdown_with_embedded_json_returns_raw() {
        // Markdown response that includes a JSON example in a code block.
        // The first non-empty line is markdown, not JSON, so the whole
        // response must be returned as raw text.
        let raw = r#"# Implementation Notes

Here's the JSON format:

```json
{"type":"message_start","message":{"id":"msg_1"}}
{"type":"content_block_delta","delta":{"text":"example"}}
```

Done."#;
        let normalized = normalize_output(raw).expect("should return raw");
        assert_eq!(normalized.text, raw);
        assert_eq!(normalized.session_id, None);
    }

    // --- P2 regression: unknown `type` field must route to single-json ---

    #[test]
    fn normalize_output_single_json_with_type_field_routes_to_single() {
        // A single-object JSON response with a `type` field that is NOT
        // a known Claude stream event type. Must be handled by the
        // single-object path, not the stream normalizer.
        let raw = r#"{"type":"result","text":"ok","session_id":"s1"}"#;
        let normalized = normalize_output(raw).expect("should route to single-json");
        assert_eq!(normalized.text, "ok");
        assert_eq!(normalized.session_id.as_deref(), Some("s1"));
    }

    // --- Claude CLI --verbose output ---

    #[test]
    fn normalize_output_claude_cli_verbose_extracts_text_and_metadata() {
        let raw = concat!(
            r#"{"type":"system","subtype":"init","session_id":"sess-abc","model":"claude-opus-4-6"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello from verbose"}],"usage":{"input_tokens":10,"output_tokens":5}},"session_id":"sess-abc"}"#,
            "\n",
            r#"{"type":"result","result":"Hello from verbose","session_id":"sess-abc","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100}}"#,
        );
        let normalized = normalize_output(raw).expect("claude cli verbose");
        assert_eq!(normalized.text, "Hello from verbose");
        assert_eq!(normalized.session_id.as_deref(), Some("sess-abc"));
        assert_eq!(normalized.tokens_in, Some(10));
        assert_eq!(normalized.tokens_out, Some(5));
        assert_eq!(normalized.cached_in, Some(100));
    }

    #[test]
    fn normalize_output_claude_cli_verbose_prefers_result_over_assistant() {
        // When both assistant and result events have text, prefer the result
        // event's clean text over the assistant narration.
        let raw = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"primary text"}]},"session_id":"s1"}"#,
            "\n",
            r#"{"type":"result","result":"clean result","session_id":"s1"}"#,
        );
        let normalized = normalize_output(raw).expect("prefers result text");
        assert_eq!(normalized.text, "clean result");
    }

    #[test]
    fn normalize_output_claude_cli_verbose_falls_back_to_result_text() {
        // If assistant event has no content, fall back to result event text.
        let raw = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s2"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[]},"session_id":"s2"}"#,
            "\n",
            r#"{"type":"result","result":"fallback text","session_id":"s2"}"#,
        );
        let normalized = normalize_output(raw).expect("falls back to result text");
        assert_eq!(normalized.text, "fallback text");
        assert_eq!(normalized.session_id.as_deref(), Some("s2"));
    }

    // --- Codex CLI output ---

    #[test]
    fn normalize_output_codex_cli_extracts_text_and_metadata() {
        let raw = concat!(
            r#"{"type":"thread.started","thread_id":"thread-xyz"}"#,
            "\n",
            r#"{"type":"turn.started"}"#,
            "\n",
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"Codex response"}}"#,
            "\n",
            r#"{"type":"turn.completed","usage":{"input_tokens":50,"cached_input_tokens":20,"output_tokens":30}}"#,
        );
        let normalized = normalize_output(raw).expect("codex cli");
        assert_eq!(normalized.text, "Codex response");
        assert_eq!(normalized.session_id.as_deref(), Some("thread-xyz"));
        assert_eq!(normalized.tokens_in, Some(50));
        assert_eq!(normalized.tokens_out, Some(30));
        assert_eq!(normalized.cached_in, Some(20));
    }

    #[test]
    fn normalize_output_claude_cli_verbose_result_overrides_concatenated_narration() {
        // Multiple assistant events produce concatenated narration that hides the H1.
        // The result event has the clean final response and should win.
        let raw = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            "\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Let me work on this...\"}]},\"session_id\":\"s1\"}",
            "\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"# Implementation Notes\\n\\n## Decisions Made\\nDone.\"}]},\"session_id\":\"s1\"}",
            "\n",
            "{\"type\":\"result\",\"result\":\"# Implementation Notes\\n\\n## Decisions Made\\nDone.\",\"session_id\":\"s1\"}",
        );
        let normalized = normalize_output(raw).expect("result overrides narration");
        assert_eq!(
            normalized.text,
            "# Implementation Notes\n\n## Decisions Made\nDone."
        );
        assert!(
            normalized.text.starts_with("# "),
            "H1 must be at the start of the text"
        );
    }

    #[test]
    fn normalize_output_claude_cli_verbose_no_result_uses_assistant_text() {
        // When the result event is missing (e.g. process killed), fall back to
        // assistant text with newline separators between events.
        let raw = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"first block"}]},"session_id":"s1"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"second block"}]},"session_id":"s1"}"#,
        );
        let normalized = normalize_output(raw).expect("fallback to assistant");
        assert_eq!(normalized.text, "first block\nsecond block");
    }

    #[test]
    fn normalize_output_codex_cli_filters_reasoning_items() {
        // Codex reasoning items should be excluded; only agent_message items count.
        let raw = concat!(
            r#"{"type":"thread.started","thread_id":"t-1"}"#,
            "\n",
            r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"internal reasoning"}}"#,
            "\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_1\",\"type\":\"agent_message\",\"text\":\"# Implementation Notes\\n\\nDone.\"}}",
            "\n",
            r#"{"type":"turn.completed","usage":{"input_tokens":50,"output_tokens":30}}"#,
        );
        let normalized = normalize_output(raw).expect("codex filters reasoning");
        assert_eq!(normalized.text, "# Implementation Notes\n\nDone.");
        assert!(
            !normalized.text.contains("internal reasoning"),
            "reasoning text should be filtered out"
        );
    }

    #[test]
    fn normalize_output_codex_cli_separates_multiple_agent_messages() {
        // When codex returns multiple agent_message items, a newline must be
        // inserted between them so that an H1 heading in a later message
        // stays on its own line and is found by first_h1_line().
        let raw = concat!(
            r#"{"type":"thread.started","thread_id":"t-multi"}"#,
            "\n",
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"All tests pass and clippy is clean."}}"#,
            "\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_1\",\"type\":\"agent_message\",\"text\":\"# Verdict: COMPLETE\\n\\nEverything looks good.\"}}",
            "\n",
            r#"{"type":"turn.completed","usage":{"input_tokens":100,"output_tokens":60}}"#,
        );
        let normalized = normalize_output(raw).expect("codex multi agent_message");
        // The H1 must start on its own line, not glued to the previous message.
        assert!(
            normalized.text.contains("\n# Verdict: COMPLETE"),
            "H1 heading must be on its own line; got: {:?}",
            normalized.text,
        );
        assert_eq!(
            normalized.text,
            "All tests pass and clippy is clean.\n# Verdict: COMPLETE\n\nEverything looks good."
        );
    }

    #[test]
    fn normalize_output_stream_events_extracts_session_and_text() {
        let raw = concat!(
            r#"{"type":"init","session_id":"gem-sess-1"}"#,
            "\n",
            r#"{"type":"message","role":"assistant","content":"first"}"#,
            "\n",
            r#"{"type":"tool_use","name":"search"}"#,
            "\n",
            r#"{"type":"tool_result","name":"search","content":"ignored"}"#,
            "\n",
            r#"{"type":"message","role":"assistant","content":[{"text":"second"}]}"#,
            "\n",
            r#"{"type":"result","text":"final response"}"#,
        );
        let normalized = normalize_output(raw).expect("stream parse");
        assert_eq!(normalized.session_id.as_deref(), Some("gem-sess-1"));
        assert_eq!(normalized.text, "final response");
    }

    #[test]
    fn normalize_output_message_event_requires_assistant_role_for_text() {
        let raw = concat!(
            r#"{"type":"init","session_id":"gem-sess-2"}"#,
            "\n",
            r#"{"type":"message","role":"user","content":"ignore this"}"#,
            "\n",
            r#"{"type":"result","content":[{"text":"assistant final"}]}"#,
        );
        let normalized = normalize_output(raw).expect("stream parse");
        assert_eq!(normalized.session_id.as_deref(), Some("gem-sess-2"));
        assert_eq!(normalized.text, "assistant final");
    }

    // --- CLI pipe-mode: multi-line JSON with preamble ---

    #[test]
    fn normalize_output_pipe_multiline_json_with_preamble() {
        // Pipe mode may output status lines then a pretty-printed JSON summary.
        let raw = "YOLO mode is enabled. All tool calls will be automatically approved.\n\
                    Loaded cached credentials.\n\
                    YOLO mode is enabled. All tool calls will be automatically approved.\n\
                    {\n\
                    \x20 \"session_id\": \"gem-pipe-1\",\n\
                    \x20 \"response\": \"# Verdict: COMPLETE\\n\\nAll requirements satisfied.\",\n\
                    \x20 \"stats\": { \"models\": {} }\n\
                    }";
        let normalized = normalize_output(raw).expect("pipe mode");
        assert_eq!(normalized.session_id.as_deref(), Some("gem-pipe-1"));
        assert_eq!(
            normalized.text,
            "# Verdict: COMPLETE\n\nAll requirements satisfied."
        );
    }

    #[test]
    fn normalize_output_multiline_json_without_preamble() {
        // Multi-line pretty-printed JSON with no preamble lines.
        let raw = "{\n\
                    \x20 \"session_id\": \"s1\",\n\
                    \x20 \"response\": \"# Verdict: CONTINUE\\n\\n## Issues\\n- bug found\"\n\
                    }";
        let normalized = normalize_output(raw).expect("multiline json no preamble");
        assert_eq!(normalized.session_id.as_deref(), Some("s1"));
        assert_eq!(
            normalized.text,
            "# Verdict: CONTINUE\n\n## Issues\n- bug found"
        );
    }

    #[test]
    fn normalize_output_markdown_with_json_block_still_returns_raw() {
        // Markdown starting with H1 that contains JSON — must NOT be parsed as JSON.
        let raw =
            "# Review\n\nHere is the config:\n\n```json\n{\"response\": \"fake\"}\n```\n\nDone.";
        let normalized = normalize_output(raw).expect("markdown with json block");
        assert_eq!(normalized.text, raw);
    }

    #[test]
    fn normalize_output_429_error_before_response_json() {
        // Some CLIs output 429 retry error messages (including error JSON +
        // stack traces) on stdout before the actual response JSON.
        let raw = "Attempt 1 failed with status 429. Retrying with backoff... GaxiosError: [{\n\
                    \x20 \"error\": {\n\
                    \x20   \"code\": 429,\n\
                    \x20   \"message\": \"No capacity available for model\",\n\
                    \x20   \"status\": \"RESOURCE_EXHAUSTED\"\n\
                    \x20 }\n\
                    }]\n\
                    \x20   at Gaxios._request (/usr/lib/node_modules/gaxios/src/gaxios.ts:200:15)\n\
                    \x20   at process.processTicksAndRejections (node:internal/process/task_queues:105:5)\n\
                    {\n\
                    \x20 \"session_id\": \"gem-429-test\",\n\
                    \x20 \"response\": \"# Verdict: COMPLETE\\n\\nAll requirements met.\",\n\
                    \x20 \"stats\": { \"models\": {} }\n\
                    }";
        let normalized = normalize_output(raw).expect("429 then response");
        assert_eq!(normalized.session_id.as_deref(), Some("gem-429-test"));
        assert_eq!(
            normalized.text,
            "# Verdict: COMPLETE\n\nAll requirements met."
        );
    }

    #[test]
    fn normalize_output_claude_cli_verbose_multiple_results_keeps_longest() {
        // When claude uses tools between responses, multiple result events
        // may appear. The first contains the full spec; the last is a short
        // summary after tool use. We must keep the longest result.
        let full_spec = "## Summary\nFull spec.\n\n## Acceptance Criteria\n- AC1";
        let summary = "The spec has been written to file.";
        let escaped_spec = full_spec.replace('\n', "\\n");
        let line_init = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
        let line_asst1 = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{escaped_spec}"}}]}},"session_id":"s1"}}"#,
        );
        let line_res1 =
            format!(r#"{{"type":"result","result":"{escaped_spec}","session_id":"s1"}}"#,);
        let line_asst2 = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{summary}"}}]}},"session_id":"s1"}}"#,
        );
        let line_res2 = format!(r#"{{"type":"result","result":"{summary}","session_id":"s1"}}"#,);
        let raw = [line_init, &line_asst1, &line_res1, &line_asst2, &line_res2].join("\n");
        let normalized = normalize_output(&raw).expect("keeps longest result");
        assert!(
            normalized.text.contains("## Summary"),
            "should keep the longer result with section headers, got: {:?}",
            normalized.text,
        );
        assert!(
            normalized.text.contains("## Acceptance Criteria"),
            "should preserve all sections"
        );
        assert!(
            !normalized.text.contains("written to file"),
            "should not use the short summary result"
        );
    }

    // --- Goose CLI output ---

    #[test]
    fn normalize_output_goose_cli_extracts_text_and_tokens() {
        let raw = concat!(
            r#"{"type":"message","message":{"id":"gen-abc123","role":"assistant","created":1772238503,"content":[{"type":"text","text":"Hello from goose!"}],"metadata":{"userVisible":true}}}"#,
            "\n",
            r#"{"type":"complete","total_tokens":1073}"#,
        );
        let normalized = normalize_output(raw).expect("goose cli");
        assert_eq!(normalized.text, "Hello from goose!");
        assert_eq!(normalized.session_id.as_deref(), Some("gen-abc123"));
        assert_eq!(normalized.tokens_in, Some(1073));
    }

    #[test]
    fn normalize_output_goose_cli_filters_reasoning_and_notifications() {
        let raw = concat!(
            r#"{"type":"message","message":{"id":"gen-xyz","role":"assistant","created":1,"content":[{"type":"reasoning","text":"thinking..."}],"metadata":{}}}"#,
            "\n",
            r#"{"type":"notification","extension_id":"call_1","log":{"message":"running shell"}}"#,
            "\n",
            "{\"type\":\"message\",\"message\":{\"id\":\"gen-xyz\",\"role\":\"assistant\",\"created\":2,\"content\":[{\"type\":\"text\",\"text\":\"# Implementation Notes\\n\\nDone.\"}],\"metadata\":{}}}",
            "\n",
            r#"{"type":"complete","total_tokens":500}"#,
        );
        let normalized = normalize_output(raw).expect("goose filters reasoning");
        assert_eq!(normalized.text, "# Implementation Notes\n\nDone.");
        assert!(
            !normalized.text.contains("thinking"),
            "reasoning text should be filtered out"
        );
    }

    #[test]
    fn normalize_output_goose_cli_multiple_text_messages() {
        let raw = concat!(
            r#"{"type":"message","message":{"id":"gen-1","role":"assistant","created":1,"content":[{"type":"text","text":"First part."}],"metadata":{}}}"#,
            "\n",
            r#"{"type":"message","message":{"id":"gen-1","role":"user","created":2,"content":[{"type":"toolResponse","id":"call_1","toolResult":{}}],"metadata":{}}}"#,
            "\n",
            "{\"type\":\"message\",\"message\":{\"id\":\"gen-2\",\"role\":\"assistant\",\"created\":3,\"content\":[{\"type\":\"text\",\"text\":\"# Final Answer\\n\\nAll done.\"}],\"metadata\":{}}}",
            "\n",
            r#"{"type":"complete","total_tokens":2000}"#,
        );
        let normalized = normalize_output(raw).expect("goose multi text");
        assert!(
            normalized.text.contains("\n# Final Answer"),
            "H1 heading must be on its own line; got: {:?}",
            normalized.text,
        );
        assert_eq!(normalized.text, "First part.\n# Final Answer\n\nAll done.");
    }

    #[test]
    fn normalize_output_goose_cli_token_streaming_preserves_fenced_json() {
        // Goose streams token-by-token with the same message ID.
        // Tokens must be concatenated directly without inserting newlines,
        // otherwise "```json" gets split into "```\njson" and fenced JSON parsing breaks.
        let raw = concat!(
            r#"{"type":"message","message":{"id":"gen-abc","role":"assistant","created":1,"content":[{"type":"text","text":"```"}],"metadata":{}}}"#,
            "\n",
            r#"{"type":"message","message":{"id":"gen-abc","role":"assistant","created":1,"content":[{"type":"text","text":"json"}],"metadata":{}}}"#,
            "\n",
            "{\"type\":\"message\",\"message\":{\"id\":\"gen-abc\",\"role\":\"assistant\",\"created\":1,\"content\":[{\"type\":\"text\",\"text\":\"\\n\"}],\"metadata\":{}}}",
            "\n",
            "{\"type\":\"message\",\"message\":{\"id\":\"gen-abc\",\"role\":\"assistant\",\"created\":1,\"content\":[{\"type\":\"text\",\"text\":\"{\\\"approved\\\": true}\\n\"}],\"metadata\":{}}}",
            "\n",
            r#"{"type":"message","message":{"id":"gen-abc","role":"assistant","created":1,"content":[{"type":"text","text":"```"}],"metadata":{}}}"#,
            "\n",
            r#"{"type":"complete","total_tokens":100}"#,
        );
        let normalized = normalize_output(raw).expect("goose token streaming");
        assert!(
            normalized.text.contains("```json"),
            "fenced JSON opener must not be split; got: {:?}",
            normalized.text,
        );
        assert_eq!(normalized.text, "```json\n{\"approved\": true}\n```");
    }

    // --- OpenCode CLI output ---

    #[test]
    fn normalize_output_opencode_cli_extracts_text_and_metadata() {
        let raw = concat!(
            r#"{"type":"step_start","timestamp":1772226736532,"sessionID":"ses_abc123","part":{"id":"prt_1","sessionID":"ses_abc123","messageID":"msg_1","type":"step-start","snapshot":"abc"}}"#,
            "\n",
            r#"{"type":"text","timestamp":1772226737081,"sessionID":"ses_abc123","part":{"id":"prt_2","sessionID":"ses_abc123","messageID":"msg_1","type":"text","text":"Hello from opencode!","time":{"start":1772226737080,"end":1772226737080}}}"#,
            "\n",
            r#"{"type":"step_finish","timestamp":1772226737089,"sessionID":"ses_abc123","part":{"id":"prt_3","sessionID":"ses_abc123","messageID":"msg_1","type":"step-finish","reason":"stop","snapshot":"abc","cost":0.001888,"tokens":{"input":248,"output":65,"reasoning":51,"cache":{"read":12220,"write":0}}}}"#,
        );
        let normalized = normalize_output(raw).expect("opencode cli");
        assert_eq!(normalized.text, "Hello from opencode!");
        assert_eq!(normalized.session_id.as_deref(), Some("ses_abc123"));
        assert_eq!(normalized.tokens_in, Some(248));
        assert_eq!(normalized.tokens_out, Some(65));
        assert_eq!(normalized.cached_in, Some(12220));
    }

    #[test]
    fn normalize_output_opencode_cli_multiple_text_events() {
        let raw = concat!(
            r#"{"type":"step_start","timestamp":1,"sessionID":"ses_multi","part":{"type":"step-start"}}"#,
            "\n",
            r#"{"type":"text","timestamp":2,"sessionID":"ses_multi","part":{"type":"text","text":"First part."}}"#,
            "\n",
            "{\"type\":\"text\",\"timestamp\":3,\"sessionID\":\"ses_multi\",\"part\":{\"type\":\"text\",\"text\":\"# Implementation Notes\\n\\nDone.\"}}",
            "\n",
            r#"{"type":"step_finish","timestamp":4,"sessionID":"ses_multi","part":{"type":"step-finish","tokens":{"input":100,"output":50}}}"#,
        );
        let normalized = normalize_output(raw).expect("opencode multi text");
        assert!(
            normalized.text.contains("\n# Implementation Notes"),
            "H1 heading must be on its own line; got: {:?}",
            normalized.text,
        );
        assert_eq!(normalized.session_id.as_deref(), Some("ses_multi"));
        assert_eq!(normalized.tokens_in, Some(100));
        assert_eq!(normalized.tokens_out, Some(50));
    }
}
