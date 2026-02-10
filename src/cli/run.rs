use crate::cli::RunArgs;
use crate::workflow::orchestrator::{Orchestrator, RunOptions};
use crate::workspace::Workspace;
use crate::Result;

pub async fn execute(args: RunArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let mut orchestrator = Orchestrator::new(workspace);

    let result = orchestrator
        .run(RunOptions {
            project: args.project,
            loops: args.loops,
            until_review: args.until_review,
            until_complete: args.until_complete,
            dry_run: args.dry_run,
            backend: args.backend,
            on_prompt_change: args.on_prompt_change,
            skip_commit: args.skip_commit,
            tmux: args.tmux.or(args.no_tmux),
        })
        .await?;

    println!("{}", result.summary);
    Ok(())
}
