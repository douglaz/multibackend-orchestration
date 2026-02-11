use crate::cli::backend_spec::validate_backend_spec_name;
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
                validate_backend_spec_name(backend)?;
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
            println!(
                "Prompt Hash At Loop Start: {}",
                state.prompt_hash_at_loop_start
            );
            println!("Current Loop: {}", state.current_loop);
            println!(
                "Current Phase: {} (iteration {})",
                phase_label(&state.current_phase),
                state.phase_iteration
            );
            println!("Feature Loops: {}", state.loops.len());
            println!("Completion Attempts: {}", state.completion_attempts.len());

            if let Some(loop_state) = state.current_feature_loop() {
                println!("Current Feature: {}", loop_state.feature_name);
                println!(
                    "Current Backends: planner={}, implementer={}, reviewer={}, qa={}",
                    loop_state.backends.planner,
                    loop_state.backends.implementer,
                    loop_state.backends.reviewer,
                    loop_state.backends.qa
                );
            } else if let Some(attempt) = state.current_completion_attempt() {
                println!("Current Completion Attempt: loop {}", attempt.loop_number);
                println!(
                    "Current Backends: planner={}, completer={}",
                    attempt.backends.planner, attempt.backends.completer
                );
            }
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
        crate::project::state::Phase::QA => "qa",
        crate::project::state::Phase::Reviewing => "reviewing",
        crate::project::state::Phase::Committing => "committing",
        crate::project::state::Phase::Completing => "completing",
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::backend_spec::validate_backend_spec_name;

    #[test]
    fn project_backend_accepts_bare_claude() {
        validate_backend_spec_name("claude").expect("bare claude should pass");
    }

    #[test]
    fn project_backend_accepts_bare_codex() {
        validate_backend_spec_name("codex").expect("bare codex should pass");
    }

    #[test]
    fn project_backend_accepts_claude_with_model() {
        validate_backend_spec_name("claude(opus)").expect("claude(opus) should pass");
    }

    #[test]
    fn project_backend_accepts_codex_with_model() {
        validate_backend_spec_name("codex(gpt-5.3-codex-xhigh)")
            .expect("codex with model should pass");
    }

    #[test]
    fn project_backend_rejects_unknown_base() {
        let err =
            validate_backend_spec_name("unknown(opus)").expect_err("unknown backend should fail");
        assert!(err.to_string().contains("unknown backend"));
    }

    #[test]
    fn project_backend_rejects_malformed_empty_model() {
        validate_backend_spec_name("claude()").expect_err("empty model should fail");
    }

    #[test]
    fn project_backend_rejects_malformed_missing_close_paren() {
        validate_backend_spec_name("claude(opus").expect_err("missing close paren should fail");
    }

    #[test]
    fn project_backend_rejects_malformed_empty_name() {
        validate_backend_spec_name("(opus)").expect_err("empty name should fail");
    }
}
