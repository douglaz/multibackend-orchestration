use std::path::PathBuf;

use clap::Args;

use tokio_util::sync::CancellationToken;

use super::init;
use super::parse_positive_u32;
use crate::workflow::quick_dev_orchestrator::{QuickDevOrchestrator, QuickDevRunOptions};
use crate::workspace::Workspace;
use crate::Result;

#[derive(Debug, Args)]
pub struct QuickDevRunArgs {
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long = "implementer-backend")]
    pub implementer_backend: Option<String>,
    #[arg(long = "reviewer-backend")]
    pub reviewer_backend: Option<String>,
    #[arg(long = "pr-url")]
    pub pr_url: Option<String>,
    /// Workspace root directory. When set, config is loaded from this
    /// directory instead of walking up the directory tree. Used by the
    /// daemon to isolate each worktree's configuration.
    #[arg(long = "workspace-root")]
    pub workspace_root: Option<PathBuf>,
    #[arg(long)]
    pub skip_commit: bool,
    #[arg(long, value_parser = parse_positive_u32)]
    pub max_review_iterations: Option<u32>,
    #[arg(long, value_parser = parse_positive_u32)]
    pub max_final_review_retries: Option<u32>,
}

pub async fn execute(args: QuickDevRunArgs) -> Result<()> {
    let workspace = if let Some(ref root) = args.workspace_root {
        let ralph_dir = root.join(".ralph");
        if ralph_dir.join("ralph.toml").is_file() {
            Workspace::load(ralph_dir)?
        } else {
            let ws = init::create_workspace(&ralph_dir)?;
            eprintln!("initialized workspace at {}", ralph_dir.display());
            ws
        }
    } else {
        Workspace::discover()?
    };

    let mut orchestrator = QuickDevOrchestrator::new(workspace);
    let result = orchestrator
        .run(QuickDevRunOptions {
            project: args.project,
            implementer_backend: args.implementer_backend,
            reviewer_backend: args.reviewer_backend,
            pr_url: args.pr_url,
            skip_commit: args.skip_commit,
            max_review_iterations: args.max_review_iterations,
            max_final_review_retries: args.max_final_review_retries,
            cancel: CancellationToken::new(),
            max_backend_retries: None,
        })
        .await?;
    println!("{}", result.summary);

    Ok(())
}
