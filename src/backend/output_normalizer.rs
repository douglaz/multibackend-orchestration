use serde_json::Value;

use crate::error::RalphError;
use crate::Result;

/// Normalized output from a backend execution, extracting structured metadata
/// when available (Claude JSON, Codex JSONL) and falling back to raw text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizedOutput {
    pub text: String,
    pub session_id: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub cached_in: Option<u64>,
}

/// Normalize raw stdout from a backend execution.
///
/// - Claude JSON: extract response text, session_id, usage.
/// - Codex JSONL: extract thread/session id, last assistant message text, usage.
/// - Non-JSON fallback: text=raw, metadata None.
/// - Malformed JSON/JSONL must not panic; fall back to raw text.
/// - If structured JSON exists but assistant message text is missing, return Err.
pub fn normalize_output(backend_name: &str, raw_stdout: &str) -> Result<NormalizedOutput> {
    let trimmed = raw_stdout.trim();

    if trimmed.is_empty() {
        return Ok(NormalizedOutput {
            text: raw_stdout.to_owned(),
            ..Default::default()
        });
    }

    // Strip parenthesized model qualifier: "codex(gpt-5.3-codex-high)" -> "codex"
    let base_name = backend_name
        .split_once('(')
        .map(|(name, _)| name)
        .unwrap_or(backend_name);

    if base_name.starts_with("claude") {
        if trimmed.starts_with('{') {
            return normalize_claude_json(trimmed, raw_stdout);
        }
    } else if base_name.starts_with("codex") {
        return normalize_codex_jsonl(raw_stdout);
    }

    // Non-JSON fallback
    Ok(NormalizedOutput {
        text: raw_stdout.to_owned(),
        ..Default::default()
    })
}

fn normalize_claude_json(raw: &str, raw_fallback: &str) -> Result<NormalizedOutput> {
    let value: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => {
            // Malformed JSON: fall back to raw text without panic
            return Ok(NormalizedOutput {
                text: raw_fallback.to_owned(),
                ..Default::default()
            });
        }
    };

    let session_id = value
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());

    // Extract usage
    let usage = value.get("usage");
    let tokens_in = usage.and_then(|u| u.get("input_tokens")).and_then(|v| v.as_u64());
    let tokens_out = usage.and_then(|u| u.get("output_tokens")).and_then(|v| v.as_u64());
    let cached_in = usage
        .and_then(|u| u.get("cache_read_input_tokens"))
        .and_then(|v| v.as_u64());

    // Extract text from content array or result field.
    let text = extract_claude_text(&value)?;

    Ok(NormalizedOutput {
        text,
        session_id,
        tokens_in,
        tokens_out,
        cached_in,
    })
}

fn extract_claude_text(value: &Value) -> Result<String> {
    // Try content array first
    if let Some(content) = value.get("content").and_then(|v| v.as_array()) {
        let mut texts = Vec::new();
        for block in content {
            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                texts.push(text);
            }
        }
        if !texts.is_empty() {
            return Ok(texts.join(""));
        }
    }

    // Try result field
    if let Some(result) = value.get("result").and_then(|v| v.as_str()) {
        return Ok(result.to_owned());
    }

    // Structured JSON exists but no text found
    Err(RalphError::ParseError(
        "claude JSON response has no assistant message text".to_owned(),
    ))
}

pub fn normalize_codex_jsonl(raw: &str) -> Result<NormalizedOutput> {
    let mut event_text: Option<String> = None;
    let mut flat_text: Option<String> = None;
    let mut event_session_id: Option<String> = None;
    let mut flat_session_id: Option<String> = None;
    let mut event_tokens_in: Option<u64> = None;
    let mut event_tokens_out: Option<u64> = None;
    let mut event_cached_in: Option<u64> = None;
    let mut flat_tokens_in: Option<u64> = None;
    let mut flat_tokens_out: Option<u64> = None;
    let mut flat_cached_in: Option<u64> = None;
    let mut saw_json_object = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value = match serde_json::from_str::<Value>(trimmed) {
            Ok(value) => value,
            Err(_) => continue,
        };

        saw_json_object = true;
        let has_type_field = value.get("type").is_some();

        if let Some(event_type) = value.get("type").and_then(Value::as_str) {
            match event_type {
                "thread.started" => {
                    if let Some(thread_id) = value.get("thread_id").and_then(Value::as_str) {
                        event_session_id = Some(thread_id.to_owned());
                    }
                }
                "item.completed" => {
                    if let Some(item) = value.get("item").and_then(Value::as_object) {
                        let item_type = item.get("type").and_then(Value::as_str);
                        if item_type == Some("agent_message") {
                            if let Some(text) = item.get("text").and_then(Value::as_str) {
                                event_text = Some(text.to_owned());
                            }
                        }
                    }
                }
                "turn.completed" => {
                    if let Some(usage) = value.get("usage").and_then(Value::as_object) {
                        let (tokens_in, tokens_out, cached_in) = extract_usage_fields(usage);
                        if let Some(value) = tokens_in {
                            event_tokens_in = Some(value);
                        }
                        if let Some(value) = tokens_out {
                            event_tokens_out = Some(value);
                        }
                        if let Some(value) = cached_in {
                            event_cached_in = Some(value);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        if has_type_field {
            continue;
        }

        flat_text = extract_flat_text(&value).or(flat_text);
        flat_session_id = value
            .get("thread_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or(flat_session_id);

        if let Some(usage) = value.get("usage").and_then(Value::as_object) {
            let (tokens_in, tokens_out, cached_in) = extract_usage_fields(usage);
            if let Some(value) = tokens_in {
                flat_tokens_in = Some(value);
            }
            if let Some(value) = tokens_out {
                flat_tokens_out = Some(value);
            }
            if let Some(value) = cached_in {
                flat_cached_in = Some(value);
            }
        }
    }

    if !saw_json_object {
        return Ok(NormalizedOutput {
            text: raw.to_owned(),
            ..Default::default()
        });
    }

    let text = match event_text.or(flat_text) {
        Some(t) => t,
        None => {
            return Err(RalphError::ParseError(
                "codex JSONL response has no assistant message text".to_owned(),
            ));
        }
    };

    Ok(NormalizedOutput {
        text,
        session_id: event_session_id.or(flat_session_id),
        tokens_in: event_tokens_in.or(flat_tokens_in),
        tokens_out: event_tokens_out.or(flat_tokens_out),
        cached_in: event_cached_in.or(flat_cached_in),
    })
}

fn extract_flat_text(value: &Value) -> Option<String> {
    if value.get("role").and_then(Value::as_str) == Some("assistant") {
        return value
            .get("content")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }

    value
        .get("message")
        .and_then(Value::as_object)
        .and_then(|message| {
            let role = message.get("role").and_then(Value::as_str);
            if role == Some("assistant") {
                return message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            None
        })
}

fn extract_usage_fields(
    usage: &serde_json::Map<String, Value>,
) -> (Option<u64>, Option<u64>, Option<u64>) {
    let tokens_in = usage.get("input_tokens").and_then(Value::as_u64);
    let tokens_out = usage.get("output_tokens").and_then(Value::as_u64);
    let cached_in = usage
        .get("cached_input_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_u64));

    (tokens_in, tokens_out, cached_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Claude tests ---

    #[test]
    fn claude_json_extracts_text_and_session() {
        let json = r#"{
            "session_id": "sess-123",
            "content": [{"type": "text", "text": "Hello world"}],
            "usage": {"input_tokens": 100, "output_tokens": 50, "cache_read_input_tokens": 20}
        }"#;
        let result = normalize_output("claude", json).unwrap();
        assert_eq!(result.text, "Hello world");
        assert_eq!(result.session_id.as_deref(), Some("sess-123"));
        assert_eq!(result.tokens_in, Some(100));
        assert_eq!(result.tokens_out, Some(50));
        assert_eq!(result.cached_in, Some(20));
    }

    #[test]
    fn claude_json_missing_text_returns_err() {
        let json = r#"{"session_id": "sess-123", "content": []}"#;
        let result = normalize_output("claude", json);
        assert!(result.is_err(), "expected Err for missing assistant text in structured JSON");
    }

    #[test]
    fn claude_non_json_returns_raw() {
        let raw = "# Implementation Notes\n\nSome markdown output";
        let result = normalize_output("claude", raw).unwrap();
        assert_eq!(result.text, raw);
        assert!(result.session_id.is_none());
    }

    #[test]
    fn claude_result_field_extracts_text() {
        let json = r#"{"session_id": "s1", "result": "extracted text"}"#;
        let result = normalize_output("claude", json).unwrap();
        assert_eq!(result.text, "extracted text");
        assert_eq!(result.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn malformed_json_does_not_panic() {
        let malformed = r#"{"invalid json..."#;
        let result = normalize_output("claude", malformed).unwrap();
        assert_eq!(result.text, malformed);
    }

    #[test]
    fn empty_output_returns_raw() {
        let result = normalize_output("claude", "").unwrap();
        assert_eq!(result.text, "");
        assert!(result.session_id.is_none());
    }

    #[test]
    fn unknown_backend_returns_raw() {
        let raw = "some output";
        let result = normalize_output("unknown-backend", raw).unwrap();
        assert_eq!(result.text, raw);
    }

    #[test]
    fn normalize_output_idempotent_for_plain_text() {
        let raw = "# Review: APPROVED\n\n## Checklist\n- [x] done";
        let r1 = normalize_output("claude", raw).unwrap();
        let r2 = normalize_output("claude", &r1.text).unwrap();
        assert_eq!(r1.text, r2.text);
    }

    // --- Codex tests ---

    #[test]
    fn codex_real_jsonl_extracts_text_and_session() {
        let raw = r#"{"type":"thread.started","thread_id":"thread_abc123"}
{"type":"item.completed","item":{"type":"reasoning","text":"thinking"}}
{"type":"item.completed","item":{"type":"agent_message","text":"Event stream answer"}}
{"type":"turn.completed","usage":{"input_tokens":42,"output_tokens":9,"cached_input_tokens":7}}"#;

        let output = normalize_output("codex", raw).unwrap();

        assert_eq!(output.text, "Event stream answer");
        assert_eq!(output.session_id.as_deref(), Some("thread_abc123"));
        assert_eq!(output.tokens_in, Some(42));
        assert_eq!(output.tokens_out, Some(9));
        assert_eq!(output.cached_in, Some(7));
    }

    #[test]
    fn codex_last_agent_message_wins() {
        let raw = r#"{"type":"item.completed","item":{"type":"agent_message","text":"First answer"}}
{"type":"item.completed","item":{"type":"agent_message","text":"Second answer"}}"#;

        let output = normalize_output("codex", raw).unwrap();

        assert_eq!(output.text, "Second answer");
    }

    #[test]
    fn codex_event_text_precedes_flat_text() {
        let raw = r#"{"role":"assistant","content":"Flat assistant content"}
{"type":"item.completed","item":{"type":"agent_message","text":"Event assistant content"}}
{"message":{"role":"assistant","content":"Flat message assistant content"}}"#;

        let output = normalize_output("codex", raw).unwrap();

        assert_eq!(output.text, "Event assistant content");
    }

    #[test]
    fn codex_thread_and_usage_are_type_scoped() {
        let raw = r#"{"type":"thread.updated","thread_id":"wrong-thread"}
{"type":"item.completed","usage":{"input_tokens":999,"output_tokens":999,"cached_input_tokens":999}}
{"type":"thread.started","thread_id":"right-thread"}
{"type":"item.completed","item":{"type":"agent_message","text":"Scoped answer"}}
{"type":"turn.completed","usage":{"input_tokens":8,"output_tokens":5,"cache_read_input_tokens":3}}"#;

        let output = normalize_output("codex", raw).unwrap();

        assert_eq!(output.text, "Scoped answer");
        assert_eq!(output.session_id.as_deref(), Some("right-thread"));
        assert_eq!(output.tokens_in, Some(8));
        assert_eq!(output.tokens_out, Some(5));
        assert_eq!(output.cached_in, Some(3));
    }

    #[test]
    fn codex_jsonl_extracts_text_and_session() {
        let raw = r#"{"thread_id":"legacy-thread"}
{"role":"assistant","content":"Legacy assistant response"}
{"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":2}}"#;

        let output = normalize_output("codex", raw).unwrap();

        assert_eq!(output.text, "Legacy assistant response");
        assert_eq!(output.session_id.as_deref(), Some("legacy-thread"));
        assert_eq!(output.tokens_in, Some(11));
        assert_eq!(output.tokens_out, Some(4));
        assert_eq!(output.cached_in, Some(2));
    }

    #[test]
    fn codex_jsonl_missing_assistant_returns_err() {
        let raw = r#"{"type":"thread.started","thread_id":"thread_abc123"}
{"type":"turn.completed","usage":{"input_tokens":42,"output_tokens":9,"cached_input_tokens":7}}"#;

        let result = normalize_output("codex", raw);
        assert!(result.is_err(), "expected Err for missing assistant text in structured JSONL");
    }

    #[test]
    fn codex_non_json_returns_raw() {
        let raw = "plain text output without json";
        let result = normalize_output("codex", raw).unwrap();
        assert_eq!(result.text, raw);
    }

    #[test]
    fn codex_malformed_json_skips_bad_lines() {
        let raw = "{this is malformed json\n{\"role\":\"assistant\",\"content\":\"Recovered\"}";
        let output = normalize_output("codex", raw).unwrap();
        assert_eq!(output.text, "Recovered");
    }

    #[test]
    fn normalize_output_dispatches_backends() {
        let codex = normalize_output(
            "codex",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"codex\"}}",
        )
        .unwrap();
        assert_eq!(codex.text, "codex");

        let claude = normalize_output("claude", "claude output").unwrap();
        assert_eq!(claude.text, "claude output");
    }
}
