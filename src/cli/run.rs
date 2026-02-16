use crate::cli::RunArgs;
use crate::error::RalphError;
use crate::project::lifecycle::{load_project_state, save_project_state};
use crate::project::state::ProjectStatus;
use crate::workflow::orchestrator::{Orchestrator, RunOptions};
use crate::workspace::Workspace;
use crate::Result;

pub async fn execute(args: RunArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let project_id_for_failure = workspace.resolve_project_id(args.project.as_deref()).ok();
    let mut orchestrator = Orchestrator::new(workspace.clone());

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
        .await;

    match result {
        Ok(result) => {
            println!("{}", result.summary);
            Ok(())
        }
        Err(err) => {
            if matches!(err, RalphError::BackendTimeoutExhausted { .. }) {
                mark_project_failed(&workspace, project_id_for_failure.as_deref());
            }
            Err(err)
        }
    }
}

fn mark_project_failed(workspace: &Workspace, project_id: Option<&str>) {
    let Some(project_id) = project_id else {
        return;
    };
    let project_dir = workspace.project_dir(project_id);
    let Ok(mut state) = load_project_state(&project_dir) else {
        return;
    };
    state.status = ProjectStatus::Failed;
    let _ = save_project_state(&project_dir, &state);
}
