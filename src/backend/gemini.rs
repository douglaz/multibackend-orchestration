use std::time::Duration;

use crate::backend::CliBackend;
use crate::config::GlobalConfig;

fn ensure_gemini_prompt_value(args: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::with_capacity(args.len() + 1);
    let mut idx = 0;

    while idx < args.len() {
        let arg = &args[idx];
        normalized.push(arg.clone());

        if arg == "-p" || arg == "--prompt" {
            let next = args.get(idx + 1);
            if let Some(value) = next {
                if !value.starts_with('-') {
                    normalized.push(value.clone());
                    idx += 2;
                    continue;
                }
            } else {
                normalized.push(String::new());
                idx += 1;
                continue;
            }

            // Some Gemini CLIs require an explicit prompt value after -p/--prompt.
            // Provide an empty value so stdin-only prompt streaming remains valid.
            normalized.push(String::new());
        }

        idx += 1;
    }

    normalized
}

pub fn ensure_gemini_stream_json_args(args: Vec<String>) -> Vec<String> {
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
    ensure_gemini_prompt_value(sanitized)
}

pub fn backend_from_config(
    config: &GlobalConfig,
    model: Option<&str>,
    role: Option<&str>,
) -> CliBackend {
    let backend = &config.backends.gemini;
    let mut args = backend.args.clone();
    let name = if let Some(model_name) = model {
        args.splice(0..0, ["--model".to_owned(), model_name.to_owned()]);
        format!("gemini({model_name})")
    } else {
        "gemini".to_owned()
    };
    let args = ensure_gemini_stream_json_args(args);

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
    use super::ensure_gemini_stream_json_args;

    fn stream_json_pair_count(args: &[String]) -> usize {
        args.windows(2)
            .filter(|pair| pair[0] == "--output-format" && pair[1] == "stream-json")
            .count()
    }

    #[test]
    fn ensure_gemini_stream_json_args_strips_flag_and_equals_variants() {
        let args = vec![
            "-p".to_owned(),
            "--output-format".to_owned(),
            "json".to_owned(),
            "--output-format=text".to_owned(),
            "--output-format=stream-json".to_owned(),
            "--other".to_owned(),
        ];

        let normalized = ensure_gemini_stream_json_args(args);
        assert_eq!(stream_json_pair_count(&normalized), 1);
        assert!(!normalized.iter().any(|arg| arg == "--output-format=text"));
        assert!(!normalized
            .iter()
            .any(|arg| arg == "--output-format=stream-json"));
        assert!(
            normalized.contains(&"stream-json".to_owned()),
            "stream-json value missing"
        );
    }

    #[test]
    fn ensure_gemini_stream_json_args_is_idempotent() {
        let args = vec![
            "--output-format".to_owned(),
            "stream-json".to_owned(),
            "--foo".to_owned(),
        ];

        let first = ensure_gemini_stream_json_args(args);
        let second = ensure_gemini_stream_json_args(first.clone());
        assert_eq!(first, second);
        assert_eq!(stream_json_pair_count(&second), 1);
    }

    #[test]
    fn ensure_gemini_stream_json_args_inserts_empty_prompt_for_bare_p() {
        let args = vec!["-p".to_owned(), "--yolo".to_owned()];

        let normalized = ensure_gemini_stream_json_args(args);
        assert_eq!(
            normalized,
            vec![
                "-p".to_owned(),
                "".to_owned(),
                "--yolo".to_owned(),
                "--output-format".to_owned(),
                "stream-json".to_owned()
            ]
        );
    }

    #[test]
    fn ensure_gemini_stream_json_args_keeps_existing_prompt_value() {
        let args = vec![
            "--prompt".to_owned(),
            "hello".to_owned(),
            "--yolo".to_owned(),
        ];

        let normalized = ensure_gemini_stream_json_args(args);
        assert_eq!(
            normalized,
            vec![
                "--prompt".to_owned(),
                "hello".to_owned(),
                "--yolo".to_owned(),
                "--output-format".to_owned(),
                "stream-json".to_owned()
            ]
        );
    }
}
