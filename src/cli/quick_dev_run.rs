use std::path::PathBuf;

use clap::Args;

use tokio_util::sync::CancellationToken;
use tracing::instrument::WithSubscriber;

use super::init;
use super::parse_positive_u32;
use crate::daemon::tasks::{self, QuickDevRunTaskParams};
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
    /// Maximum number of backend timeout retries per invocation.
    /// Defaults to 3 when omitted.
    #[arg(long = "max-backend-retries")]
    pub max_backend_retries: Option<u8>,
}

pub async fn execute(args: QuickDevRunArgs) -> Result<()> {
    let workspace_root = if let Some(root) = args.workspace_root {
        let ralph_dir = root.join(".ralph");
        if !ralph_dir.join("ralph.toml").is_file() {
            let _ = init::create_workspace(&ralph_dir)?;
            eprintln!("initialized workspace at {}", ralph_dir.display());
        }
        root
    } else {
        let ws = Workspace::discover()?;
        ws.root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| ws.root.clone())
    };

    let dispatch = tasks::cli_stderr_dispatch();
    let result = tasks::run_quick_dev_run_task(QuickDevRunTaskParams {
        workspace_root,
        project: args.project,
        pr_url: args.pr_url,
        cancel: CancellationToken::new(),
        max_backend_retries: args.max_backend_retries,
        implementer_backend: args.implementer_backend,
        reviewer_backend: args.reviewer_backend,
        skip_commit: args.skip_commit,
        max_review_iterations: args.max_review_iterations,
        max_final_review_retries: args.max_final_review_retries,
    })
    .with_subscriber(dispatch)
    .await?;
    println!("{}", result.summary);

    Ok(())
}
