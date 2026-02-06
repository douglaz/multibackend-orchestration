use crate::cli::{ProjectArgs, ProjectCommand};
use crate::git::branch::{branch_exists, checkout_branch, resolve_branch_name};
use crate::git::is_git_repo;
use crate::project::lifecycle::{
    create_project, load_project_state, CreateProjectOptions, PromptSource,
};
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub fn execute(args: ProjectArgs) -> Result<()> {
    match args.command {
        ProjectCommand::New(new_args) => {
            let mut workspace = Workspace::discover()?;

            let source = match (new_args.prompt, new_args.from) {
                (Some(prompt), None) => PromptSource::File(prompt),
                (None, Some(parent)) => PromptSource::ParentProject(parent),
                _ => {
                    return Err(RalphError::Validation(
                        "provide exactly one of --prompt or --from".to_owned(),
                    ))
                }
            };

            if let Some(backend) = new_args.backend.as_deref() {
                if backend != "claude" && backend != "codex" {
                    return Err(RalphError::Validation(
                        "--backend must be one of: claude, codex".to_owned(),
                    ));
                }
            }

            create_project(
                &mut workspace,
                CreateProjectOptions {
                    id: new_args.id,
                    name: new_args.name,
                    source,
                    starting_backend: new_args.backend,
                },
            )?;
            println!("project created");
            Ok(())
        }
        ProjectCommand::List => {
            let workspace = Workspace::discover()?;
            println!("PROJECTS IN WORKSPACE");
            println!();
            println!(
                "{:<14} {:<24} {:<12} {:<10} {:<10} ACTIVE",
                "ID", "NAME", "STATUS", "FEATURES", "LAST_LOOP"
            );
            for project in &workspace.index.projects {
                let active = workspace
                    .index
                    .active_project
                    .as_deref()
                    .is_some_and(|id| id == project.id);
                println!(
                    "{:<14} {:<24} {:<12} {:<10} {:<10} {}",
                    project.id,
                    project.name,
                    project_status_label(&project.status),
                    project.total_feature_loops,
                    project.last_loop_number,
                    if active { "*" } else { "" }
                );
            }
            Ok(())
        }
        ProjectCommand::Use(use_args) => {
            let mut workspace = Workspace::discover()?;
            workspace.index.set_active_project(&use_args.project_id)?;
            workspace.save_index()?;

            if workspace.config.git.auto_branch {
                if let Some(repo_root) = workspace.root.parent() {
                    if is_git_repo(repo_root) {
                        let branch = resolve_branch_name(
                            &workspace.config.git.branch_format,
                            &use_args.project_id,
                        );
                        if branch_exists(repo_root, &branch)? {
                            checkout_branch(repo_root, &branch)?;
                        }
                    }
                }
            }
            println!("active project set to {}", use_args.project_id);
            Ok(())
        }
        ProjectCommand::Show(show_args) => {
            let workspace = Workspace::discover()?;
            let project_id = if let Some(id) = show_args.project_id {
                id
            } else {
                workspace
                    .index
                    .active_project
                    .clone()
                    .ok_or(RalphError::ActiveProjectNotSet)?
            };
            let project_meta = workspace
                .index
                .get_project(&project_id)
                .ok_or_else(|| RalphError::ProjectNotFound(project_id.clone()))?;
            let project_dir = workspace.project_dir(&project_id);
            let state = load_project_state(&project_dir)?;

            if show_args.json {
                let value = serde_json::json!({
                    "project": project_meta,
                    "state": state,
                });
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Ok(());
            }

            println!("ID: {}", project_meta.id);
            println!("Name: {}", project_meta.name);
            println!("Status: {}", project_status_label(&project_meta.status));
            println!(
                "Parent: {}",
                project_meta.parent_project.as_deref().unwrap_or("none")
            );
            println!("Prompt Hash: {}", state.prompt_hash);
            println!("Current Loop: {}", state.current_loop);
            println!("Current Phase: {}", phase_label(&state.current_phase));
            println!("Feature Loops: {}", state.loops.len());
            println!("Completion Attempts: {}", state.completion_attempts.len());
            Ok(())
        }
    }
}

fn project_status_label(status: &crate::workspace::index::ProjectLifecycleStatus) -> &'static str {
    match status {
        crate::workspace::index::ProjectLifecycleStatus::Pending => "pending",
        crate::workspace::index::ProjectLifecycleStatus::InProgress => "in_progress",
        crate::workspace::index::ProjectLifecycleStatus::Completed => "completed",
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
