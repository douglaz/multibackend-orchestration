use std::io::IsTerminal;
use std::path::PathBuf;

use clap::Args;

use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use crate::cli::backend_spec;
use crate::workspace::Workspace;
use crate::Result;

#[derive(Debug, Args)]
pub struct PrdArgs {
    #[arg(long)]
    pub idea: String,
    #[arg(long, conflicts_with = "interactive")]
    pub non_interactive: bool,
    #[arg(long, conflicts_with = "non_interactive")]
    pub interactive: bool,
    #[arg(long, default_value_t = 3)]
    pub ask_max: u32,
    #[arg(long)]
    pub answers: Option<PathBuf>,
    #[arg(long)]
    pub resume: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub backend: Option<String>,
}

pub async fn execute(args: PrdArgs) -> Result<()> {
    let workspace = Workspace::discover()?;

    let mut registry = BackendRegistry::new(
        &workspace.config,
        BackendRegistryTmuxConfig {
            enabled: false,
            session_name: workspace.config.workspace.tmux_session.clone(),
            window_keep_seconds: workspace.config.workspace.tmux_window_keep_seconds,
        },
    );

    let backend_spec = args
        .backend
        .clone()
        .unwrap_or_else(|| workspace.config.workspace.default_backend.clone());
    backend_spec::validate_backend_spec(&backend_spec, &workspace.config)?;

    let backend = registry.get_or_create_for_spec(&backend_spec)?;
    backend.health_check().await?;

    let non_interactive =
        args.non_interactive || (!std::io::stdin().is_terminal() && !args.interactive);
    let answers_path = args
        .answers
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_owned());

    println!("PRD pipeline is not yet implemented.");
    println!("idea: {}", args.idea);
    println!("backend: {}", backend.name());
    println!(
        "mode: {}",
        if non_interactive {
            "non-interactive"
        } else {
            "interactive"
        }
    );
    println!("ask max rounds: {}", args.ask_max);
    println!("answers file: {answers_path}");
    println!("resume: {}", args.resume);
    println!("dry run: {}", args.dry_run);

    Ok(())
}
