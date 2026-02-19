use std::time::Duration;

use crate::backend::CliBackend;
use crate::config::GlobalConfig;

pub fn ensure_stream_json_args(args: Vec<String>) -> Vec<String> {
    let mut sanitized = Vec::with_capacity(args.len() + 2);
    let mut idx = 0;

    while idx < args.len() {
        let arg = &args[idx];
        if arg == "--output-format" {
            idx += 1;
            if idx < args.len() {
                idx += 1;
            }
            continue;
        }
        if arg.starts_with("--output-format=") {
            idx += 1;
            continue;
        }
        sanitized.push(arg.clone());
        idx += 1;
    }

    sanitized.push("--output-format".to_owned());
    sanitized.push("stream-json".to_owned());
    sanitized
}

pub fn effective_args_claude(base_args: &[String], model: Option<&str>) -> Vec<String> {
    let mut args = base_args.to_vec();
    if let Some(model_name) = model {
        args.splice(0..0, ["--model".to_owned(), model_name.to_owned()]);
    }
    ensure_stream_json_args(args)
}

pub fn backend_from_config(
    config: &GlobalConfig,
    model: Option<&str>,
    role: Option<&str>,
) -> CliBackend {
    let backend = &config.backends.claude;
    let args = effective_args_claude(&backend.args, model);
    let name = if let Some(model_name) = model {
        format!("claude({model_name})")
    } else {
        "claude".to_owned()
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
}

#[cfg(test)]
mod tests {
    use super::{effective_args_claude, ensure_stream_json_args};

    fn stream_json_pair_count(args: &[String]) -> usize {
        args.windows(2)
            .filter(|pair| pair[0] == "--output-format" && pair[1] == "stream-json")
            .count()
    }

    #[test]
    fn ensure_stream_json_args_strips_flag_and_equals_variants() {
        let args = vec![
            "-p".to_owned(),
            "--output-format".to_owned(),
            "json".to_owned(),
            "--output-format=text".to_owned(),
            "--output-format=stream-json".to_owned(),
            "--other".to_owned(),
        ];

        let normalized = ensure_stream_json_args(args);
        assert_eq!(stream_json_pair_count(&normalized), 1);
        assert!(!normalized.iter().any(|arg| arg == "--output-format=text"));
        assert!(!normalized.iter().any(|arg| arg == "--output-format=stream-json"));
        assert_eq!(
            normalized.last().expect("stream-json value missing"),
            "stream-json"
        );
    }

    #[test]
    fn ensure_stream_json_args_is_idempotent() {
        let args = vec![
            "--output-format".to_owned(),
            "stream-json".to_owned(),
            "--foo".to_owned(),
        ];

        let first = ensure_stream_json_args(args);
        let second = ensure_stream_json_args(first.clone());
        assert_eq!(first, second);
        assert_eq!(stream_json_pair_count(&second), 1);
    }

    #[test]
    fn effective_args_claude_with_model_still_has_single_stream_json_pair() {
        let base_args = vec![
            "--output-format".to_owned(),
            "text".to_owned(),
            "-p".to_owned(),
        ];

        let args = effective_args_claude(&base_args, Some("opus"));
        assert_eq!(stream_json_pair_count(&args), 1);
        assert_eq!(args[0], "--model");
        assert_eq!(args[1], "opus");
    }

    #[test]
    fn effective_args_claude_without_model_has_single_stream_json_pair() {
        let base_args = vec![
            "--allowedTools".to_owned(),
            "Read,Write".to_owned(),
            "--output-format=text".to_owned(),
        ];
        let args = effective_args_claude(&base_args, None);
        assert_eq!(stream_json_pair_count(&args), 1);
        assert!(args.contains(&"--allowedTools".to_owned()));
    }
}
