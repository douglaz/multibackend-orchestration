use std::path::PathBuf;
use std::time::Duration;

use crate::backend::CliBackend;
use crate::config::GlobalConfig;

pub fn backend_from_config(
    config: &GlobalConfig,
    model: Option<&str>,
    role: Option<&str>,
    cwd: Option<PathBuf>,
) -> CliBackend {
    let backend = &config.backends.openrouter;
    let mut args = backend.args.clone();
    let name = if let Some(model_name) = model {
        // Inject --model <model> after the "run" subcommand (or at end if no "run").
        if let Some(pos) = args.iter().position(|a| a == "run") {
            args.insert(pos + 1, model_name.to_owned());
            args.insert(pos + 1, "--model".to_owned());
        } else {
            args.push("--model".to_owned());
            args.push(model_name.to_owned());
        }
        format!("openrouter({model_name})")
    } else {
        "openrouter".to_owned()
    };

    let timeout = match role {
        Some(r) => backend.timeout_for_role(r),
        None => Duration::from_secs(backend.timeout_seconds),
    };

    CliBackend::new(&name, backend.command.clone(), args, timeout, backend.env.clone())
        .with_cwd(cwd)
}
