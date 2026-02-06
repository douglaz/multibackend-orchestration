pub mod global;
pub mod project;

use std::path::{Path, PathBuf};

use crate::error::RalphError;
use crate::Result;

pub use global::{CommitMessageStyle, GlobalConfig, PromptChangeAction};
pub use project::ProjectConfig;

#[derive(Debug, Clone)]
pub struct EffectiveWorkflowConfig {
    pub starting_backend: String,
    pub max_review_iterations: u32,
    pub auto_commit: bool,
    pub commit_message_style: CommitMessageStyle,
    pub commit_tag_format: String,
    pub prompt_change_action: PromptChangeAction,
}

#[derive(Debug, Clone)]
pub struct EffectiveTemplateConfig {
    pub planner: PathBuf,
    pub implementer: PathBuf,
    pub reviewer: PathBuf,
    pub completer: PathBuf,
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub workflow: EffectiveWorkflowConfig,
    pub templates: EffectiveTemplateConfig,
    pub global: GlobalConfig,
    pub project: Option<ProjectConfig>,
}

pub fn resolve_effective_config(
    workspace_root: &Path,
    project_dir: &Path,
    global: GlobalConfig,
    project: Option<ProjectConfig>,
    run_backend_override: Option<&str>,
) -> Result<EffectiveConfig> {
    let project_ref = project.as_ref();

    let starting_backend = if let Some(override_backend) = run_backend_override {
        override_backend.to_owned()
    } else if let Some(value) = project_ref.and_then(|p| p.workflow.starting_backend.clone()) {
        value
    } else {
        global.workspace.default_backend.clone()
    };

    if global.backend_config(&starting_backend).is_none() {
        return Err(RalphError::Validation(format!(
            "unknown backend configured as starting backend: {starting_backend}"
        )));
    }

    let workflow = EffectiveWorkflowConfig {
        starting_backend,
        max_review_iterations: project_ref
            .and_then(|p| p.workflow.max_review_iterations)
            .unwrap_or(global.workflow.max_review_iterations),
        auto_commit: project_ref
            .and_then(|p| p.workflow.auto_commit)
            .unwrap_or(global.workflow.auto_commit),
        commit_message_style: project_ref
            .and_then(|p| p.workflow.commit_message_style.clone())
            .unwrap_or(global.workflow.commit_message_style.clone()),
        commit_tag_format: global.workflow.commit_tag_format.clone(),
        prompt_change_action: project_ref
            .and_then(|p| p.workflow.prompt_change_action)
            .unwrap_or(global.workflow.prompt_change_action),
    };

    let templates = EffectiveTemplateConfig {
        planner: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.planner.as_deref()),
            &global.templates.planner,
        ),
        implementer: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.implementer.as_deref()),
            &global.templates.implementer,
        ),
        reviewer: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.reviewer.as_deref()),
            &global.templates.reviewer,
        ),
        completer: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.completer.as_deref()),
            &global.templates.completer,
        ),
    };

    Ok(EffectiveConfig {
        workflow,
        templates,
        global,
        project,
    })
}

fn resolve_template_path(
    workspace_root: &Path,
    project_dir: &Path,
    project_override: Option<&str>,
    global_value: &str,
) -> PathBuf {
    if let Some(path) = project_override {
        return project_dir.join(path);
    }

    workspace_root.join(global_value)
}
