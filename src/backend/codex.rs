use std::time::Duration;

use crate::backend::CliBackend;
use crate::config::GlobalConfig;

pub fn backend_from_config(config: &GlobalConfig, model: Option<&str>) -> CliBackend {
    let backend = &config.backends.codex;
    let mut args = backend.args.clone();
    let name = if let Some(model_name) = model {
        args.splice(0..0, ["--model".to_owned(), model_name.to_owned()]);
        format!("codex({model_name})")
    } else {
        "codex".to_owned()
    };

    CliBackend::new(
        &name,
        backend.command.clone(),
        args,
        Duration::from_secs(backend.timeout_seconds),
        backend.env.clone(),
    )
}
