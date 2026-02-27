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
        // Inject -m <model> before the "run" subcommand.
        args.splice(0..0, ["-m".to_owned(), model_name.to_owned()]);
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
