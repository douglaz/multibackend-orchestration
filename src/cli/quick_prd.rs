use std::io::IsTerminal;

use clap::Args;

use super::parse_positive_u32;
use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use crate::cli::backend_spec;
use crate::error::RalphError;
use crate::prd::quick::{QuickPrdOptions, QuickPrdPipeline};
use crate::workspace::Workspace;
use crate::Result;

#[derive(Debug, Args)]
pub struct QuickPrdArgs {
    #[arg(long)]
    pub idea: String,
    #[arg(long, default_value = "claude")]
    pub writer_backend: String,
    #[arg(long, default_value = "codex")]
    pub reviewer_backend: String,
    #[arg(long, default_value_t = 2, value_parser = parse_positive_u32)]
    pub max_revisions: u32,
    #[arg(long, conflicts_with = "interactive")]
    pub non_interactive: bool,
    #[arg(long, conflicts_with = "non_interactive")]
    pub interactive: bool,
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn execute(args: QuickPrdArgs) -> Result<()> {
    let idea = args.idea.trim().to_owned();
    if idea.is_empty() {
        return Err(RalphError::Validation(
            "--idea must not be empty".to_owned(),
        ));
    }

    let workspace = Workspace::discover()?;

    let mut registry = BackendRegistry::new(
        &workspace.config,
        BackendRegistryTmuxConfig {
            enabled: false,
            session_name: workspace.config.workspace.tmux_session.clone(),
            window_keep_seconds: workspace.config.workspace.tmux_window_keep_seconds,
        },
    );

    let writer_spec = if args.writer_backend.trim().is_empty() {
        workspace.config.workspace.default_backend.clone()
    } else {
        args.writer_backend.clone()
    };
    let reviewer_spec = if args.reviewer_backend.trim().is_empty() {
        workspace.config.workspace.default_backend.clone()
    } else {
        args.reviewer_backend.clone()
    };

    backend_spec::validate_backend_spec(&writer_spec, &workspace.config)?;
    backend_spec::validate_backend_spec(&reviewer_spec, &workspace.config)?;

    let writer = registry.get_or_create_for_spec(&writer_spec)?;
    let reviewer = registry.get_or_create_for_spec(&reviewer_spec)?;
    writer.health_check().await?;
    reviewer.health_check().await?;

    // Auto-detect TTY mode: non-interactive if --non-interactive or stdin is not a terminal
    // (unless --interactive is explicitly passed).
    let non_interactive =
        args.non_interactive || (!std::io::stdin().is_terminal() && !args.interactive);

    println!("Starting quick PRD pipeline...");
    println!("  idea: {idea}");
    println!("  writer backend: {writer_spec}");
    println!("  reviewer backend: {reviewer_spec}");
    println!(
        "  mode: {}",
        if non_interactive {
            "non-interactive"
        } else {
            "interactive"
        }
    );
    println!("  max revisions: {}", args.max_revisions);
    println!();

    let options = QuickPrdOptions {
        idea,
        writer_spec,
        reviewer_spec,
        max_revisions: args.max_revisions,
        dry_run: args.dry_run,
    };

    let pipeline = QuickPrdPipeline::new(writer, reviewer, options);
    let result = pipeline.run().await?;

    println!("Quick PRD pipeline completed!");
    println!("  Spec written to: {}", result.spec_path.display());
    println!("  Cache directory: {}", result.cache_dir.display());
    println!("  Revisions used: {}", result.revision_count);
    if !result.approved {
        println!("  warning: reviewer did not approve before max revisions were exhausted");
    }
    println!("  {}", result.summary);
    println!();

    Ok(())
}
