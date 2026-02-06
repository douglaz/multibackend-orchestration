use std::time::Duration;

use crate::backend::CliBackend;
use crate::config::GlobalConfig;

pub fn backend_from_config(config: &GlobalConfig) -> CliBackend {
    let backend = &config.backends.codex;
    CliBackend::new(
        "codex",
        backend.command.clone(),
        backend.args.clone(),
        Duration::from_secs(backend.timeout_seconds),
        backend.env.clone(),
    )
}
