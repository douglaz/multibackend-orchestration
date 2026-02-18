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

/// Known Claude stream-json event types used to distinguish NDJSON streams
/// from single-object JSON responses that happen to contain a `type` field.
const CLAUDE_STREAM_EVENT_TYPES: &[&str] = &[
    "message_start",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
    "message_delta",
    "message_stop",
    "ping",
    "summary",
];

pub fn normalize_output(raw: &str) -> Result<NormalizedOutput> {
    // Only attempt JSON routing if the first non-empty line looks like JSON.
    // This prevents markdown responses containing embedded JSON examples
    // (e.g. in code blocks) from being misrouted into JSON normalization.
    let first_content_line = raw.lines().map(str::trim).find(|l| !l.is_empty());
    if !first_content_line.is_some_and(|l| l.starts_with('{')) {
        return Ok(NormalizedOutput {
            text: raw.to_owned(),
            ..NormalizedOutput::default()
        });
    }

    let Some(first_json) = first_valid_json_object(raw) else {
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
        .is_some_and(|t| CLAUDE_STREAM_EVENT_TYPES.contains(&t));

    if is_stream {
        normalize_claude_stream_json(raw)
    } else {
        normalize_claude_single_json(raw)
    }
}

pub fn normalize_claude_stream_json(raw: &str) -> Result<NormalizedOutput> {
    let mut output = NormalizedOutput::default();
    let mut json_event_count = 0_usize;

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
            _ => {}
        }
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

fn extract_single_json_text(value: &Value) -> Option<String> {
    for key in ["result", "output", "text", "completion"] {
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

fn extract_text_from_content(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }

    let mut parts = Vec::new();
    let Some(items) = content.as_array() else {
        return None;
    };

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
    output.tokens_out =
        extract_u64(usage, &["tokens_out", "output_tokens"]).or(output.tokens_out);
    output.cached_in = extract_u64(
        usage,
        &[
            "cached_in",
            "cache_read_input_tokens",
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
        let raw =
            r#"{"result":"single-json","session_id":"sess_1","usage":{"tokens_in":1,"tokens_out":2,"cached_in":3}}"#;

        let normalized = normalize_output(raw).expect("normalize_output");
        assert_eq!(normalized.text, "single-json");
        assert_eq!(normalized.session_id.as_deref(), Some("sess_1"));
        assert_eq!(normalized.tokens_in, Some(1));
        assert_eq!(normalized.tokens_out, Some(2));
        assert_eq!(normalized.cached_in, Some(3));
    }

    #[test]
    fn normalize_output_returns_raw_when_first_line_is_not_json() {
        // First non-empty line is plain text, so even though later lines
        // contain valid JSON, the response is treated as raw text.
        let raw = "nope\nstill nope\n{\"result\":\"from-json\"}";
        let normalized = normalize_output(raw).expect("normalize_output");
        assert_eq!(normalized.text, raw);
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
}
