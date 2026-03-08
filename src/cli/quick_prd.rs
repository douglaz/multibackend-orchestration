use std::io::IsTerminal;

use clap::Args;

use super::parse_positive_u32;
use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use crate::cli::backend_spec;
use crate::error::RalphError;
use crate::prd::quick::{render_prompt, QuickPrdOptions, QuickPrdPipeline, DRAFT_PROMPT};
use crate::workspace::Workspace;
use crate::Result;

#[derive(Debug, Args)]
pub struct QuickPrdArgs {
    #[arg(long)]
    pub idea: String,
    #[arg(long, default_value = "")]
    pub writer_backend: String,
    #[arg(long, default_value = "")]
    pub reviewer_backend: String,
    #[arg(long, default_value_t = 1, value_parser = parse_positive_u32)]
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

    if args.dry_run {
        let prompt = render_prompt(DRAFT_PROMPT, &[("{{idea}}", &idea)]);
        println!("{prompt}");
        return Ok(());
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
        workspace.config.workspace.daemon_prd_writer_backend.clone()
    } else {
        args.writer_backend.clone()
    };
    let reviewer_spec = if args.reviewer_backend.trim().is_empty() {
        workspace
            .config
            .workspace
            .daemon_prd_reviewer_backend
            .clone()
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

    if !non_interactive {
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
    }

    let options = QuickPrdOptions {
        idea,
        writer_spec,
        reviewer_spec,
        max_revisions: args.max_revisions,
        dry_run: args.dry_run,
    };

    let repo_root = workspace
        .root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| workspace.root.clone());

    let pipeline = QuickPrdPipeline::new(writer, reviewer, options);
    let result = pipeline.run(repo_root).await?;

    if !non_interactive {
        println!("Quick PRD pipeline completed!");
        println!("  Spec written to: {}", result.spec_path.display());
        println!("  Cache directory: {}", result.cache_dir.display());
        println!("  Revisions used: {}", result.revision_count);
        println!("  {}", result.summary);
        println!();
    } else {
        println!("{}", result.spec_path.display());
    }

    if !result.approved {
        eprintln!("warning: reviewer did not approve before max revisions were exhausted");
    }

    Ok(())
}
