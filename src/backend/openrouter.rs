use std::path::PathBuf;
use std::time::Duration;

use crate::backend::claude::effective_args_claude;
use crate::backend::CliBackend;
use crate::config::GlobalConfig;

pub fn backend_from_config(
    config: &GlobalConfig,
    model: Option<&str>,
    role: Option<&str>,
    cwd: Option<PathBuf>,
) -> CliBackend {
    let backend = &config.backends.openrouter;
    let args = effective_args_claude(&backend.args, model);
    let name = if let Some(model_name) = model {
        format!("openrouter({model_name})")
    } else {
        "openrouter".to_owned()
    };

    let timeout = match role {
        Some(r) => backend.timeout_for_role(r),
        None => Duration::from_secs(backend.timeout_seconds),
    };

    let mut env = backend.env.clone();
    // Always ensure OpenRouter API routing.
    env.entry("ANTHROPIC_BASE_URL".to_owned())
        .or_insert_with(|| "https://openrouter.ai/api".to_owned());
    env.entry("ANTHROPIC_API_KEY".to_owned())
        .or_default();
    // If no explicit auth token in config, try OPENROUTER_API_KEY from env.
    if !env.contains_key("ANTHROPIC_AUTH_TOKEN") {
        if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
            env.insert("ANTHROPIC_AUTH_TOKEN".to_owned(), key);
        }
    }

    CliBackend::new(&name, backend.command.clone(), args, timeout, env).with_cwd(cwd)
}
