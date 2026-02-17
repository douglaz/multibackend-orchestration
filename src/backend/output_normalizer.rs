use crate::error::RalphError;
use crate::Result;

/// Normalized output from a backend execution, extracting structured metadata
/// when available (Claude JSON, Codex JSONL) and falling back to raw text.
#[derive(Debug, Clone, Default)]
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

    if backend_name.starts_with("claude") {
        if trimmed.starts_with('{') {
            return normalize_claude_json(trimmed, raw_stdout);
        }
    } else if backend_name.starts_with("codex") {
        if trimmed.contains('{') {
            return normalize_codex_jsonl(trimmed, raw_stdout);
        }
    }

    // Non-JSON fallback
    Ok(NormalizedOutput {
        text: raw_stdout.to_owned(),
        ..Default::default()
    })
}

fn normalize_claude_json(raw: &str, raw_fallback: &str) -> Result<NormalizedOutput> {
    let value: serde_json::Value = match serde_json::from_str(raw) {
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
    // If structured JSON exists but required message text is missing, return Err
    // so caller can degrade gracefully (not fall back to raw).
    let text = extract_claude_text(&value)?;

    Ok(NormalizedOutput {
        text,
        session_id,
        tokens_in,
        tokens_out,
        cached_in,
    })
}

fn extract_claude_text(value: &serde_json::Value) -> Result<String> {
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

fn normalize_codex_jsonl(raw: &str, _raw_fallback: &str) -> Result<NormalizedOutput> {
    let mut session_id: Option<String> = None;
    let mut last_text: Option<String> = None;
    let mut tokens_in: Option<u64> = None;
    let mut tokens_out: Option<u64> = None;
    let mut cached_in: Option<u64> = None;
    let mut found_json = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        found_json = true;

        // Extract session/thread id
        if let Some(id) = value
            .get("thread_id")
            .or_else(|| value.get("session_id"))
            .and_then(|v| v.as_str())
        {
            session_id = Some(id.to_owned());
        }

        // Extract message text from assistant role
        if let Some(role) = value.get("role").and_then(|v| v.as_str()) {
            if role == "assistant" {
                if let Some(text) = value.get("content").and_then(|v| v.as_str()) {
                    last_text = Some(text.to_owned());
                }
            }
        }

        // Extract text from message field
        if let Some(msg) = value.get("message") {
            if let Some(role) = msg.get("role").and_then(|v| v.as_str()) {
                if role == "assistant" {
                    if let Some(text) = msg.get("content").and_then(|v| v.as_str()) {
                        last_text = Some(text.to_owned());
                    }
                }
            }
        }

        // Extract usage
        if let Some(usage) = value.get("usage") {
            if let Some(v) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                tokens_in = Some(v);
            }
            if let Some(v) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                tokens_out = Some(v);
            }
            if let Some(v) = usage
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
            {
                cached_in = Some(v);
            }
        }
    }

    if !found_json {
        // Not JSONL at all, fallback
        return Ok(NormalizedOutput {
            text: raw.to_owned(),
            ..Default::default()
        });
    }

    let text = match last_text {
        Some(t) => t,
        None => {
            return Err(RalphError::ParseError(
                "codex JSONL response has no assistant message text".to_owned(),
            ));
        }
    };

    Ok(NormalizedOutput {
        text,
        session_id,
        tokens_in,
        tokens_out,
        cached_in,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Structured JSON exists but assistant text is missing => Err
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
    fn codex_jsonl_extracts_text_and_session() {
        let jsonl = r#"{"thread_id": "thread-abc"}
{"role": "assistant", "content": "The answer is 42"}
{"usage": {"input_tokens": 200, "output_tokens": 75}}"#;
        let result = normalize_output("codex", jsonl).unwrap();
        assert_eq!(result.text, "The answer is 42");
        assert_eq!(result.session_id.as_deref(), Some("thread-abc"));
        assert_eq!(result.tokens_in, Some(200));
        assert_eq!(result.tokens_out, Some(75));
    }

    #[test]
    fn codex_non_json_returns_raw() {
        let raw = "plain text output without json";
        let result = normalize_output("codex", raw).unwrap();
        assert_eq!(result.text, raw);
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
    fn codex_jsonl_missing_assistant_returns_err() {
        let jsonl = r#"{"thread_id": "thread-abc"}
{"role": "system", "content": "system message"}"#;
        // Structured JSONL exists but assistant text is missing => Err
        let result = normalize_output("codex", jsonl);
        assert!(result.is_err(), "expected Err for missing assistant text in structured JSONL");
    }

    #[test]
    fn claude_result_field_extracts_text() {
        let json = r#"{"session_id": "s1", "result": "extracted text"}"#;
        let result = normalize_output("claude", json).unwrap();
        assert_eq!(result.text, "extracted text");
        assert_eq!(result.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn normalize_output_idempotent_for_plain_text() {
        let raw = "# Review: APPROVED\n\n## Checklist\n- [x] done";
        let r1 = normalize_output("claude", raw).unwrap();
        let r2 = normalize_output("claude", &r1.text).unwrap();
        assert_eq!(r1.text, r2.text);
    }
}
