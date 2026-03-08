use std::path::{Path, PathBuf};

use clap::Args;
use tokio_util::sync::CancellationToken;
use tracing::instrument::WithSubscriber;

use super::init;
use super::parse_positive_u32;
use crate::daemon::tasks::{self, QuickDevAutoTaskParams};
use crate::error::RalphError;
use crate::workspace::Workspace;
use crate::Result;

#[derive(Debug, Args)]
pub struct QuickDevAutoArgs {
    #[arg(long, value_parser = parse_non_empty_idea)]
    pub idea: String,
    #[arg(long = "implementer-backend")]
    pub implementer_backend: Option<String>,
    #[arg(long = "reviewer-backend")]
    pub reviewer_backend: Option<String>,
    #[arg(long)]
    pub project_id: Option<String>,
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

fn parse_non_empty_idea(value: &str) -> std::result::Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("--idea must not be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn ensure_workspace(workspace_root: Option<&PathBuf>, fallback_cwd: &Path) -> Result<Workspace> {
    if let Some(root) = workspace_root {
        let ralph_dir = root.join(".ralph");
        if ralph_dir.join("ralph.toml").is_file() {
            return Workspace::load(ralph_dir);
        }
        let workspace = init::create_workspace(&ralph_dir)?;
        eprintln!("initialized workspace at {}", ralph_dir.display());
        return Ok(workspace);
    }

    match Workspace::discover() {
        Ok(workspace) => Ok(workspace),
        Err(RalphError::WorkspaceNotFound) => {
            let workspace = init::create_workspace(&fallback_cwd.join(".ralph"))?;
            eprintln!("initialized workspace at .ralph");
            Ok(workspace)
        }
        Err(err) => Err(err),
    }
}

pub async fn execute(args: QuickDevAutoArgs) -> Result<()> {
    let idea = args.idea.trim().to_owned();
    if idea.is_empty() {
        return Err(RalphError::Validation(
            "--idea must not be empty".to_owned(),
        ));
    }

    // Resolve CWD once at the CLI boundary for workspace fallback.
    let cwd = std::env::current_dir()?;

    // Ensure workspace exists and resolve workspace_root for the task.
    let workspace = ensure_workspace(args.workspace_root.as_ref(), &cwd)?;
    let workspace_root = args.workspace_root.unwrap_or_else(|| {
        workspace
            .root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| workspace.root.clone())
    });

    let dispatch = tasks::cli_stderr_dispatch();
    let result = tasks::run_quick_dev_auto_task(QuickDevAutoTaskParams {
        workspace_root,
        idea,
        project_id: args.project_id,
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
