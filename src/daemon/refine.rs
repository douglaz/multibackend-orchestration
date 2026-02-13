use crate::backend::{claude, codex, parse_backend_spec, Backend, CliBackend};
use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::Result;

const REFINEMENT_SYSTEM_PROMPT: &str = r#"You are a prompt refinement assistant. Rewrite the following GitHub issue into a
clear, structured task description suitable for an autonomous coding agent.

Include:
- A concise summary of what needs to be done
- Specific requirements and constraints
- Acceptance criteria as a checklist

Do NOT include meta-commentary. Output ONLY the refined task description.

--- ISSUE ---
"#;

/// Minimum length for refinement output to be considered valid.
const MIN_OUTPUT_LENGTH: usize = 20;

/// Build the refinement prompt by wrapping the raw idea with the system prompt.
pub fn build_refinement_prompt(raw_idea: &str) -> String {
    format!("{REFINEMENT_SYSTEM_PROMPT}{raw_idea}")
}

/// Create a CLI backend from a backend spec string and global config.
fn create_backend(backend_spec: &str, global_config: &GlobalConfig) -> Result<CliBackend> {
    let spec = parse_backend_spec(backend_spec)?;
    let model = spec.model.as_deref();
    match spec.name.as_str() {
        "claude" => Ok(claude::backend_from_config(global_config, model)),
        "codex" => Ok(codex::backend_from_config(global_config, model)),
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

/// Refine raw issue text into a structured ralph auto prompt.
///
/// The daemon's synchronous dispatch code runs inside the tokio runtime
/// established by `#[tokio::main]`. We use `block_in_place` to tell the
/// scheduler this thread will block, then `Handle::current().block_on()`
/// to drive the async backend execution. This avoids creating a nested
/// runtime (which would panic) while safely blocking within the existing
/// multi-threaded runtime.
pub fn refine_prompt(
    raw_idea: &str,
    backend_spec: &str,
    global_config: &GlobalConfig,
) -> Result<String> {
    let backend = create_backend(backend_spec, global_config)?;
    let prompt = build_refinement_prompt(raw_idea);

    let raw_output = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(backend.execute(&prompt))
    })?;

    validate_output(&raw_output)
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
