use crate::cli::StatusArgs;
use crate::project::lifecycle::load_project_state;
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub fn execute(args: StatusArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let project_id = if let Some(project) = args.project {
        project
    } else {
        workspace
            .index
            .active_project
            .clone()
            .ok_or(RalphError::ActiveProjectNotSet)?
    };

    let project_ref = workspace
        .index
        .get_project(&project_id)
        .ok_or_else(|| RalphError::ProjectNotFound(project_id.clone()))?;
    let state = load_project_state(&workspace.project_dir(&project_id))?;

    println!("WORKSPACE: {}", workspace.root.display());
    println!("ACTIVE PROJECT: {} ({})", project_ref.id, project_ref.name);
    println!();
    println!("Project Status: {}", project_status_label(&state.status));
    println!("Current Loop: {}", state.current_loop);
    println!(
        "Current Phase: {} (iteration {})",
        phase_label(&state.current_phase),
        state.phase_iteration
    );
    println!();
    println!("Previous Feature Loops:");
    for loop_state in &state.loops {
        let marker = if loop_state.status == crate::project::state::LoopStatus::Completed {
            "[✓]"
        } else {
            "[ ]"
        };
        println!(
            "  {} Loop {}: {}",
            marker, loop_state.loop_number, loop_state.feature_name
        );
    }

    if state.loops.is_empty() {
        println!("  (none)");
    }

    Ok(())
}

fn project_status_label(status: &crate::project::state::ProjectStatus) -> &'static str {
    match status {
        crate::project::state::ProjectStatus::Pending => "pending",
        crate::project::state::ProjectStatus::InProgress => "in_progress",
        crate::project::state::ProjectStatus::Completed => "completed",
    }
}

fn phase_label(phase: &crate::project::state::Phase) -> &'static str {
    match phase {
        crate::project::state::Phase::Planning => "planning",
        crate::project::state::Phase::Implementing => "implementing",
        crate::project::state::Phase::Reviewing => "reviewing",
        crate::project::state::Phase::Committing => "committing",
        crate::project::state::Phase::Completing => "completing",
    }
}
