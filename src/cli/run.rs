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
            planner_backend: args.planner_backend,
            implementer_backend: args.implementer_backend,
            reviewer_backend: args.reviewer_backend,
            qa_backend: args.qa_backend,
            completer_backend: args.completer_backend,
            on_prompt_change: args.on_prompt_change,
            skip_commit: args.skip_commit,
            skip_prompt_review: args.skip_prompt_review,
            tmux: args.tmux.or(args.no_tmux),
        })
        .await?;

    println!("{}", result.summary);
    Ok(())
}
