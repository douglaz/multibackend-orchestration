use tokio_util::sync::CancellationToken;

use crate::cli::init;
use crate::cli::RunArgs;
use crate::daemon::tasks::{self, RunTaskParams};
use crate::workspace::Workspace;
use crate::Result;

/// Read `RALPH_MAX_BACKEND_RETRIES` from the environment for backward
/// compatibility.  The orchestrator no longer reads this env var directly;
/// the CLI bridges it into `RunOptions.max_backend_retries`.
fn parse_max_backend_retries_env() -> Option<u8> {
    std::env::var("RALPH_MAX_BACKEND_RETRIES")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|&v| v > 0)
}

pub async fn execute(args: RunArgs) -> Result<()> {
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

    let result = tasks::run_run_task(RunTaskParams {
        workspace_root,
        project: args.project,
        pr_url: args.pr_url,
        cancel: CancellationToken::new(),
        max_backend_retries: parse_max_backend_retries_env(),
        loops: args.loops,
        until_review: args.until_review,
        until_complete: args.until_complete,
        dry_run: args.dry_run,
        backend: args.backend,
        planner_backend: args.planner_backend,
        implementer_backend: args.implementer_backend,
        reviewer_backend: args.reviewer_backend,
        qa_backend: args.qa_backend,
        completer_backend: args.completer_backend,
        tmux: args.tmux.or(args.no_tmux),
        on_prompt_change: args.on_prompt_change,
        skip_commit: args.skip_commit,
        skip_prompt_review: args.skip_prompt_review,
    })
    .await?;

    println!("{}", result.summary);
    Ok(())
}
