use crate::backend::parse_backend_spec;
use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::Result;

/// Validate a backend spec string for CLI input surfaces.
///
/// Parses the spec via `parse_backend_spec`, then checks that the base backend
/// name is a known backend in the global config. Returns the original spec
/// string unchanged on success so it can be stored as-is.
pub fn validate_backend_spec(spec: &str, config: &GlobalConfig) -> Result<()> {
    let parsed = parse_backend_spec(spec)?;
    if config.backend_config(&parsed.name).is_none() {
        return Err(RalphError::Validation(format!(
            "unknown backend: {}",
            parsed.name
        )));
    }
    Ok(())
}

/// Validate a backend spec string using only the hardcoded known backend names
/// (claude, codex, gemini). Use this when a `GlobalConfig` is not available.
pub fn validate_backend_spec_name(spec: &str) -> Result<()> {
    let parsed = parse_backend_spec(spec)?;
    match parsed.name.as_str() {
        "claude" | "codex" | "gemini" => Ok(()),
        _ => Err(RalphError::Validation(format!(
            "unknown backend: {}",
            parsed.name
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GlobalConfig;

    #[test]
    fn validate_bare_claude() {
        let config = GlobalConfig::default();
        validate_backend_spec("claude", &config).expect("bare claude should be valid");
    }

    #[test]
    fn validate_bare_codex() {
        let config = GlobalConfig::default();
        validate_backend_spec("codex", &config).expect("bare codex should be valid");
    }

    #[test]
    fn validate_claude_with_model() {
        let config = GlobalConfig::default();
        validate_backend_spec("claude(opus)", &config).expect("claude(opus) should be valid");
    }

    #[test]
    fn validate_codex_with_model() {
        let config = GlobalConfig::default();
        validate_backend_spec("codex(gpt-5.3-codex-xhigh)", &config)
            .expect("codex with model should be valid");
    }

    #[test]
    fn reject_unknown_base_backend() {
        let config = GlobalConfig::default();
        let err = validate_backend_spec("unknown(opus)", &config)
            .expect_err("unknown backend should fail");
        assert!(err.to_string().contains("unknown backend: unknown"));
    }

    #[test]
    fn reject_unknown_bare_backend() {
        let config = GlobalConfig::default();
        let err =
            validate_backend_spec("foobar", &config).expect_err("unknown bare backend should fail");
        assert!(err.to_string().contains("unknown backend: foobar"));
    }

    #[test]
    fn reject_empty_model_in_parens() {
        let config = GlobalConfig::default();
        let err = validate_backend_spec("claude()", &config).expect_err("empty model should fail");
        assert!(err.to_string().contains("invalid") || err.to_string().contains("empty"));
    }

    #[test]
    fn reject_missing_closing_paren() {
        let config = GlobalConfig::default();
        let err = validate_backend_spec("claude(opus", &config)
            .expect_err("missing closing paren should fail");
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn reject_empty_name_with_model() {
        let config = GlobalConfig::default();
        let err = validate_backend_spec("(opus)", &config).expect_err("empty name should fail");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn reject_empty_spec() {
        let config = GlobalConfig::default();
        let err = validate_backend_spec("", &config).expect_err("empty spec should fail");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn validate_name_only_bare_claude() {
        validate_backend_spec_name("claude").expect("bare claude should be valid");
    }

    #[test]
    fn validate_name_only_with_model() {
        validate_backend_spec_name("claude(opus)").expect("claude(opus) should be valid");
    }

    #[test]
    fn validate_name_only_rejects_unknown() {
        let err =
            validate_backend_spec_name("unknown(opus)").expect_err("unknown backend should fail");
        assert!(err.to_string().contains("unknown backend: unknown"));
    }

    #[test]
    fn validate_name_only_accepts_optional_gemini_with_model() {
        validate_backend_spec_name("?gemini(gemini-3-pro-preview)")
            .expect("optional gemini with model should be valid");
    }
}
