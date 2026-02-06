use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::config::ProjectConfig;
use crate::git::branch::{branch_exists, create_branch, resolve_branch_name};
use crate::git::is_git_repo;
use crate::project::state::ProjectState;
use crate::util::hash::sha256_hex;
use crate::util::lock::ProjectLock;
use crate::workspace::index::{ProjectLifecycleStatus, ProjectRef};
use crate::workspace::Workspace;
use crate::{error::RalphError, Result};

pub enum PromptSource {
    File(PathBuf),
    ParentProject(String),
}

pub struct CreateProjectOptions {
    pub id: String,
    pub name: String,
    pub source: PromptSource,
    pub starting_backend: Option<String>,
}

pub fn create_project(workspace: &mut Workspace, options: CreateProjectOptions) -> Result<()> {
    let id = options.id;
    let name = options.name;

    if workspace.index.get_project(&id).is_some() {
        return Err(RalphError::Validation(format!(
            "project '{id}' already exists"
        )));
    }

    validate_project_id(&id)?;

    let (prompt_content, parent_project) = match options.source {
        PromptSource::File(path) => (fs::read_to_string(path)?, None),
        PromptSource::ParentProject(parent_id) => {
            let parent_dir = workspace.project_dir(&parent_id);
            if !parent_dir.exists() {
                return Err(RalphError::ProjectNotFound(parent_id));
            }
            let content = fs::read_to_string(parent_dir.join("prompt.md"))?;
            (content, Some(parent_id))
        }
    };

    let project_dir = workspace.project_dir(&id);
    fs::create_dir_all(&project_dir)?;
    let _lock = ProjectLock::acquire(&project_dir, &id)?;
    fs::create_dir_all(project_dir.join("loops"))?;
    fs::write(project_dir.join("prompt.md"), &prompt_content)?;

    let prompt_hash = sha256_hex(&prompt_content);
    let state = ProjectState::new(&id, &name, &prompt_hash, parent_project.clone());
    state.save(&project_dir.join("state.json"))?;

    if let Some(starting_backend) = options.starting_backend {
        let mut project_config = ProjectConfig::default();
        project_config.workflow.starting_backend = Some(starting_backend);
        project_config.save(&project_dir.join("config.toml"))?;
    }

    maybe_create_project_branch(workspace, &id, parent_project.as_deref())?;

    let now = Utc::now();
    workspace.index.add_project(ProjectRef {
        id: id.clone(),
        name,
        status: ProjectLifecycleStatus::Pending,
        created_at: now,
        completed_at: None,
        total_feature_loops: 0,
        total_completion_attempts: 0,
        last_loop_number: 0,
        parent_project,
    })?;

    if workspace.index.active_project.is_none() {
        workspace.index.active_project = Some(id);
    }

    workspace.save_index()?;
    Ok(())
}

fn maybe_create_project_branch(
    workspace: &Workspace,
    project_id: &str,
    parent_project: Option<&str>,
) -> Result<()> {
    if !workspace.config.git.auto_branch {
        return Ok(());
    }

    let Some(repo_root) = workspace.root.parent() else {
        return Ok(());
    };

    if !is_git_repo(repo_root) {
        return Ok(());
    }

    let branch_name = resolve_branch_name(&workspace.config.git.branch_format, project_id);
    if branch_exists(repo_root, &branch_name)? {
        return Err(RalphError::Validation(format!(
            "git branch '{}' already exists",
            branch_name
        )));
    }

    let from_ref = if let Some(parent_id) = parent_project {
        resolve_branch_name(&workspace.config.git.branch_format, parent_id)
    } else {
        workspace.config.git.base_branch.clone()
    };

    create_branch(repo_root, &branch_name, &from_ref)?;
    Ok(())
}

fn validate_project_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(RalphError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }

    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(RalphError::Validation(
            "project id may only contain [a-zA-Z0-9_-]".to_owned(),
        ));
    }

    Ok(())
}

pub fn load_project_state(project_dir: &Path) -> Result<ProjectState> {
    ProjectState::load(&project_dir.join("state.json"))
}

pub fn save_project_state(project_dir: &Path, state: &ProjectState) -> Result<()> {
    state.save(&project_dir.join("state.json"))
}
