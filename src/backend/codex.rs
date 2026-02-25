use std::path::PathBuf;
use std::time::Duration;

use crate::backend::CliBackend;
use crate::config::GlobalConfig;

/// Recognized codex reasoning-effort suffixes, longest-first.
const CODEX_EFFORT_SUFFIXES: &[(&str, &str)] = &[
    ("-xhigh", "xhigh"),
    ("-medium", "medium"),
    ("-high", "high"),
    ("-low", "low"),
];

/// If `model_name` ends with a known effort suffix, return `(base_model, Some(effort))`.
/// Otherwise return `(model_name, None)`.
pub fn parse_codex_model_effort(model_name: &str) -> (&str, Option<&str>) {
    for &(suffix, effort) in CODEX_EFFORT_SUFFIXES {
        if let Some(base_model) = model_name.strip_suffix(suffix) {
            return (base_model, Some(effort));
        }
    }

    (model_name, None)
}

pub fn backend_from_config(
    config: &GlobalConfig,
    model: Option<&str>,
    role: Option<&str>,
    cwd: Option<PathBuf>,
) -> CliBackend {
    let backend = &config.backends.codex;
    let mut args = backend.args.clone();
    let name = if let Some(model_name) = model {
        let (base_model, effort) = parse_codex_model_effort(model_name);
        args.splice(0..0, ["--model".to_owned(), base_model.to_owned()]);
        if let Some(effort_level) = effort {
            args.splice(
                0..0,
                [
                    "-c".to_owned(),
                    format!("model_reasoning_effort=\"{effort_level}\""),
                ],
            );
        }
        format!("codex({model_name})")
    } else {
        "codex".to_owned()
    };

    let timeout = match role {
        Some(r) => backend.timeout_for_role(r),
        None => Duration::from_secs(backend.timeout_seconds),
    };

    CliBackend::new(
        &name,
        backend.command.clone(),
        args,
        timeout,
        backend.env.clone(),
    )
    .with_cwd(cwd)
}

#[cfg(test)]
mod tests {
    use super::parse_codex_model_effort;

    #[test]
    fn parse_codex_model_effort_strips_xhigh() {
        let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-xhigh");
        assert_eq!(base, "gpt-5.3-codex");
        assert_eq!(effort, Some("xhigh"));
    }

    #[test]
    fn parse_codex_model_effort_strips_high() {
        let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-high");
        assert_eq!(base, "gpt-5.3-codex");
        assert_eq!(effort, Some("high"));
    }

    #[test]
    fn parse_codex_model_effort_strips_medium() {
        let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-medium");
        assert_eq!(base, "gpt-5.3-codex");
        assert_eq!(effort, Some("medium"));
    }

    #[test]
    fn parse_codex_model_effort_strips_low() {
        let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-low");
        assert_eq!(base, "gpt-5.3-codex");
        assert_eq!(effort, Some("low"));
    }

    #[test]
    fn parse_codex_model_effort_no_suffix() {
        let (base, effort) = parse_codex_model_effort("gpt-5.3-codex");
        assert_eq!(base, "gpt-5.3-codex");
        assert_eq!(effort, None);
    }

    #[test]
    fn parse_codex_model_effort_unknown_suffix() {
        let (base, effort) = parse_codex_model_effort("gpt-5.3-codex-turbo");
        assert_eq!(base, "gpt-5.3-codex-turbo");
        assert_eq!(effort, None);
    }
}
