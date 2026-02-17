use crate::backend::{claude, codex, parse_backend_spec, Backend, CliBackend};
use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::Result;

const REFINEMENT_SYSTEM_PROMPT: &str = r#"You are a prompt refinement assistant. Rewrite the following GitHub issue into a
clear, structured task description suitable for an autonomous coding agent.

Output format (required):
TITLE: <concise title, max 80 chars>
---
<refined task description>
=== CLEANED BODY ===
<cleaned issue body only>

Include:
- A concise summary of what needs to be done
- Specific requirements and constraints
- Acceptance criteria as a checklist
- The cleaned-body section must be a light editorial pass of the original issue body
- Preserve intent, scope, and structure (headers, bullets, code blocks)
- Fix typos/grammar/readability only; do not add scope or merge with task description
- The cleaned-body section must contain only issue body text, never the title

Do NOT include meta-commentary. Output ONLY the required format.
Delimiters must be exact and in the order shown above.

--- ISSUE ---
"#;

/// Minimum length for refinement output to be considered valid.
const MIN_OUTPUT_LENGTH: usize = 20;
const MAX_TITLE_LENGTH: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefinedPrompt {
    pub title: Option<String>,
    pub body: String,
    pub cleaned_body: Option<String>,
}

/// Build the refinement prompt by wrapping the raw idea with the system prompt.
pub fn build_refinement_prompt(raw_idea: &str) -> String {
    format!("{REFINEMENT_SYSTEM_PROMPT}{raw_idea}")
}

/// Create a CLI backend from a backend spec string and global config.
fn create_backend(backend_spec: &str, global_config: &GlobalConfig) -> Result<CliBackend> {
    let spec = parse_backend_spec(backend_spec)?;
    let model = spec.model.as_deref();
    match spec.name.as_str() {
        "claude" => Ok(claude::backend_from_config(global_config, model, None)),
        "codex" => Ok(codex::backend_from_config(global_config, model, None)),
        _ => Err(RalphError::Validation(format!(
            "unknown refinement backend: {backend_spec}"
        ))),
    }
}

/// Validate that the refinement output is non-empty and of meaningful length.
fn validate_output(output: &str) -> Result<String> {
    let trimmed = output.trim().to_owned();
    if trimmed.is_empty() {
        return Err(RalphError::Orchestration(
            "refinement produced empty output".into(),
        ));
    }
    if trimmed.len() < MIN_OUTPUT_LENGTH {
        return Err(RalphError::Orchestration(format!(
            "refinement output too short ({} chars, minimum {})",
            trimmed.len(),
            MIN_OUTPUT_LENGTH
        )));
    }
    Ok(trimmed)
}

fn validate_cleaned_body(output: &str) -> Option<String> {
    let trimmed = output.trim().to_owned();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

/// Parse backend output into optional structured title + body.
fn parse_refined_output(output: &str) -> Result<RefinedPrompt> {
    let lines: Vec<&str> = output.lines().collect();
    let first_non_empty = lines.iter().position(|line| !line.trim().is_empty());

    if let Some(first_idx) = first_non_empty {
        let first_line = lines[first_idx];
        if let Some(raw_title) = first_line.strip_prefix("TITLE:") {
            if let Some(delim_rel_idx) = lines[first_idx + 1..]
                .iter()
                .position(|line| line.trim() == "---")
            {
                let title = raw_title.trim().to_owned();
                if title.is_empty() {
                    return Err(RalphError::Orchestration(
                        "refinement title cannot be empty".into(),
                    ));
                }
                if title.chars().count() > MAX_TITLE_LENGTH {
                    return Err(RalphError::Orchestration(format!(
                        "refinement title too long ({} chars, maximum {})",
                        title.chars().count(),
                        MAX_TITLE_LENGTH
                    )));
                }

                let delim_idx = first_idx + 1 + delim_rel_idx;
                let output_lines = &lines[delim_idx + 1..];
                let cleaned_delim_idx = output_lines
                    .iter()
                    .position(|line| line.trim() == "=== CLEANED BODY ===");
                let (body_raw, cleaned_body_raw) = if let Some(idx) = cleaned_delim_idx {
                    (
                        output_lines[..idx].join("\n"),
                        Some(output_lines[idx + 1..].join("\n")),
                    )
                } else {
                    (output_lines.join("\n"), None)
                };
                let body = validate_output(&body_raw)?;
                let cleaned_body = cleaned_body_raw.as_deref().and_then(validate_cleaned_body);
                return Ok(RefinedPrompt {
                    title: Some(title),
                    body,
                    cleaned_body,
                });
            }
        }
    }

    let body = validate_output(output)?;
    Ok(RefinedPrompt {
        title: None,
        body,
        cleaned_body: None,
    })
}

/// Refine raw issue text into a structured ralph auto prompt.
///
/// Awaits backend execution directly on the async runtime.
pub async fn refine_prompt(
    raw_idea: &str,
    backend_spec: &str,
    global_config: &GlobalConfig,
) -> Result<RefinedPrompt> {
    let backend = create_backend(backend_spec, global_config)?;
    let prompt = build_refinement_prompt(raw_idea);

    let raw_output = backend.execute(&prompt).await?;

    parse_refined_output(&raw_output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_refinement_prompt_wraps_raw_idea() {
        let raw = "Fix the login bug";
        let prompt = build_refinement_prompt(raw);
        assert!(prompt.contains(REFINEMENT_SYSTEM_PROMPT));
        assert!(prompt.ends_with("Fix the login bug"));
    }

    #[test]
    fn build_refinement_prompt_includes_system_instructions() {
        let prompt = build_refinement_prompt("some issue");
        assert!(prompt.contains("prompt refinement assistant"));
        assert!(prompt.contains("TITLE: <concise title, max 80 chars>"));
        assert!(prompt.contains("\n---\n"));
        assert!(prompt.contains("=== CLEANED BODY ==="));
        assert!(prompt.contains("never the title"));
        assert!(prompt.contains("Acceptance criteria"));
        assert!(prompt.contains("--- ISSUE ---"));
    }

    #[test]
    fn validate_output_rejects_empty() {
        let result = validate_output("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn validate_output_rejects_whitespace_only() {
        let result = validate_output("   \n\t  ");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn validate_output_rejects_too_short() {
        let result = validate_output("short");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn validate_output_accepts_valid_length() {
        let input = "This is a sufficiently long refined prompt for testing purposes.";
        let result = validate_output(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), input);
    }

    #[test]
    fn validate_output_trims_whitespace() {
        let input = "  This is a sufficiently long refined prompt for testing.  \n";
        let result = validate_output(input).unwrap();
        assert_eq!(
            result,
            "This is a sufficiently long refined prompt for testing."
        );
    }

    #[test]
    fn validate_output_boundary_at_min_length() {
        // Exactly MIN_OUTPUT_LENGTH chars should pass
        let input = "a".repeat(MIN_OUTPUT_LENGTH);
        assert!(validate_output(&input).is_ok());

        // One less should fail
        let short = "a".repeat(MIN_OUTPUT_LENGTH - 1);
        assert!(validate_output(&short).is_err());
    }

    #[test]
    fn parse_refined_output_three_section_success() {
        let input = "TITLE: Fix SSO login handling\n---\nImplement robust SSO login error handling and add regression coverage.\n=== CLEANED BODY ===\nFix SSO login handling for users with clearer steps and corrected grammar.";
        let parsed = parse_refined_output(input).unwrap();
        assert_eq!(parsed.title, Some("Fix SSO login handling".to_owned()));
        assert_eq!(
            parsed.body,
            "Implement robust SSO login error handling and add regression coverage."
        );
        assert_eq!(
            parsed.cleaned_body,
            Some(
                "Fix SSO login handling for users with clearer steps and corrected grammar."
                    .to_owned()
            )
        );
    }

    #[test]
    fn parse_refined_output_no_cleaned_body_fallback() {
        let input = "TITLE: This looks structured and has no cleaned body section\n---\nThis remains structured output content and should be preserved fully.";
        let parsed = parse_refined_output(input).unwrap();
        assert_eq!(
            parsed.title,
            Some("This looks structured and has no cleaned body section".to_owned())
        );
        assert_eq!(
            parsed.body,
            "This remains structured output content and should be preserved fully."
        );
        assert_eq!(parsed.cleaned_body, None);
    }

    #[test]
    fn parse_refined_output_empty_cleaned_body_degraded() {
        let input = "TITLE: Keep validation strict\n---\nStructured body remains valid and long enough for strict checks.\n=== CLEANED BODY ===\n   ";
        let parsed = parse_refined_output(input).unwrap();
        assert_eq!(
            parsed.body,
            "Structured body remains valid and long enough for strict checks."
        );
        assert_eq!(parsed.cleaned_body, None);
    }

    #[test]
    fn parse_refined_output_cleaned_body_preserves_structure() {
        let input = "TITLE: Improve docs clarity\n---\nStructured implementation brief for coding agent with clear acceptance criteria.\n=== CLEANED BODY ===\n## Summary\n- Keep bullets\n- Keep headings\n\n```text\nexample block\n```";
        let parsed = parse_refined_output(input).unwrap();
        assert_eq!(
            parsed.cleaned_body,
            Some(
                "## Summary\n- Keep bullets\n- Keep headings\n\n```text\nexample block\n```"
                    .to_owned()
            )
        );
    }

    #[test]
    fn parse_refined_output_delimiter_in_content_not_split() {
        let input = "TITLE: Keep literal delimiter text\n---\nStructured body includes literal text like keep === CLEANED BODY === inline and should remain intact.";
        let parsed = parse_refined_output(input).unwrap();
        assert_eq!(
            parsed.body,
            "Structured body includes literal text like keep === CLEANED BODY === inline and should remain intact."
        );
        assert_eq!(parsed.cleaned_body, None);
    }

    #[test]
    fn parse_refined_output_multi_delimiter_first_split_point() {
        let input = "TITLE: Multiple cleaned delimiters\n---\nStructured body stays before first split marker and remains valid.\n=== CLEANED BODY ===\nFirst cleaned section.\n=== CLEANED BODY ===\nSecond marker remains content.";
        let parsed = parse_refined_output(input).unwrap();
        assert_eq!(
            parsed.body,
            "Structured body stays before first split marker and remains valid."
        );
        assert_eq!(
            parsed.cleaned_body,
            Some(
                "First cleaned section.\n=== CLEANED BODY ===\nSecond marker remains content."
                    .to_owned()
            )
        );
    }

    #[test]
    fn parse_refined_output_short_cleaned_body_accepted() {
        let input = "TITLE: Fix typo in readme\n---\nCorrect the misspelling in the README file and verify formatting.\n=== CLEANED BODY ===\nFix typo";
        let parsed = parse_refined_output(input).unwrap();
        assert_eq!(
            parsed.cleaned_body,
            Some("Fix typo".to_owned())
        );
    }

    #[test]
    fn parse_refined_output_rejects_empty_structured_title() {
        let input =
            "TITLE:   \n---\nThis is a valid length body that should not mask title validation.";
        let err = parse_refined_output(input).unwrap_err();
        assert!(err.to_string().contains("title cannot be empty"));
    }

    #[test]
    fn parse_refined_output_rejects_overlong_title() {
        let title = "a".repeat(MAX_TITLE_LENGTH + 1);
        let input =
            format!("TITLE: {title}\n---\nThis is a sufficiently long body for refinement output.");
        let err = parse_refined_output(&input).unwrap_err();
        assert!(err.to_string().contains("title too long"));
    }

    #[test]
    fn parse_refined_output_enforces_body_validation_for_structured_output() {
        let input = "TITLE: Valid title\n---\nshort";
        let err = parse_refined_output(input).unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    #[test]
    fn create_backend_rejects_unknown() {
        let config = GlobalConfig::default();
        let result = create_backend("gemini(pro)", &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown"));
    }

    #[test]
    fn create_backend_accepts_claude() {
        let config = GlobalConfig::default();
        let result = create_backend("claude(sonnet)", &config);
        assert!(result.is_ok());
    }

    #[test]
    fn create_backend_accepts_codex() {
        let config = GlobalConfig::default();
        let result = create_backend("codex(gpt-5.3-codex-medium)", &config);
        assert!(result.is_ok());
    }

    #[test]
    fn create_backend_accepts_bare_name() {
        let config = GlobalConfig::default();
        assert!(create_backend("claude", &config).is_ok());
        assert!(create_backend("codex", &config).is_ok());
    }
}
