use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ProjectConfig;
use crate::git::branch::{branch_exists, create_branch, resolve_branch_name};
use crate::git::is_git_repo;
use crate::project::state::ProjectState;
use crate::util::hash::sha256_hex;
use crate::util::lock::ProjectLock;
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

pub fn create_project(workspace: &Workspace, options: CreateProjectOptions) -> Result<()> {
    let id = options.id;
    let name = options.name;

    if workspace.project_exists(&id) {
        return Err(RalphError::Validation(format!(
            "project '{id}' already exists"
        )));
    }

    validate_project_id(&id)?;

    let (prompt_content, parent_project) = match options.source {
        PromptSource::File(path) => {
            if !path.exists() {
                return Err(RalphError::Validation(format!(
                    "prompt file not found: '{}'\n\
                     hint: --prompt expects a file path (e.g., --prompt PLAN.md)",
                    path.display()
                )));
            }
            (fs::read_to_string(&path)?, None)
        }
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

    // Auto-activate if no local active project is set.
    if workspace.active_project_id().is_none() {
        workspace.set_active_project_id(&id)?;
    }

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
    let state_path = project_dir.join("state.json");
    let raw = fs::read_to_string(&state_path)?;

    match parse_and_validate_state(&raw) {
        Ok(state) => Ok(state),
        Err(reason) => recover_state_from_git(project_dir, &state_path, &reason),
    }
}

pub fn save_project_state(project_dir: &Path, state: &ProjectState) -> Result<()> {
    if let Err(reason) = state.validate_invariants() {
        return Err(RalphError::Orchestration(format!(
            "refusing to save invalid state: {reason}"
        )));
    }
    state.save(&project_dir.join("state.json"))
}

fn parse_and_validate_state(raw: &str) -> std::result::Result<ProjectState, String> {
    let state: ProjectState =
        serde_json::from_str(raw).map_err(|err| format!("invalid JSON: {err}"))?;
    state
        .validate_invariants()
        .map_err(|reason| format!("invalid invariants: {reason}"))?;
    Ok(state)
}

fn recover_state_from_git(
    project_dir: &Path,
    state_path: &Path,
    corruption_reason: &str,
) -> Result<ProjectState> {
    let repo_root = find_repo_root(project_dir).ok_or_else(|| RalphError::CorruptedState {
        path: state_path.to_path_buf(),
        reason: format!(
            "{corruption_reason}; recovery failed because project is not inside a git repository"
        ),
    })?;

    let rel = state_path
        .strip_prefix(&repo_root)
        .map_err(|_| RalphError::CorruptedState {
            path: state_path.to_path_buf(),
            reason: format!(
            "{corruption_reason}; recovery failed because state path is outside repository root"
        ),
        })?;
    let rel = rel.to_string_lossy().replace('\\', "/");

    let git_ref = format!("HEAD:{rel}");
    let output = Command::new("git")
        .args(["show", &git_ref])
        .current_dir(&repo_root)
        .output()
        .map_err(|err| RalphError::CorruptedState {
            path: state_path.to_path_buf(),
            reason: format!("{corruption_reason}; failed to run git show for recovery: {err}"),
        })?;

    if !output.status.success() {
        return Err(RalphError::CorruptedState {
            path: state_path.to_path_buf(),
            reason: format!(
                "{corruption_reason}; git recovery failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }

    let recovered_raw = String::from_utf8_lossy(&output.stdout).to_string();
    let recovered_state = parse_and_validate_state(&recovered_raw).map_err(|recovery_reason| {
        RalphError::CorruptedState {
            path: state_path.to_path_buf(),
            reason: format!(
                "{corruption_reason}; git-provided state is still invalid: {recovery_reason}"
            ),
        }
    })?;

    fs::write(state_path, recovered_raw)?;
    eprintln!(
        "warning: recovered corrupted state from git HEAD at {}",
        state_path.display()
    );

    Ok(recovered_state)
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}
