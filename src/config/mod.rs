pub mod global;
pub mod project;

use std::path::{Path, PathBuf};

use crate::backend::parse_backend_spec;
use crate::error::RalphError;
use crate::Result;

pub use global::{CommitMessageStyle, GlobalConfig, PromptChangeAction};
pub use project::ProjectConfig;

#[derive(Debug, Clone)]
pub struct EffectiveWorkflowConfig {
    pub starting_backend: String,
    pub planner_backend: Option<String>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub completer_backend: Option<String>,
    pub max_review_iterations: u32,
    pub auto_commit: bool,
    pub commit_message_style: CommitMessageStyle,
    pub commit_tag_format: String,
    pub prompt_change_action: PromptChangeAction,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RunWorkflowOverrides<'a> {
    pub starting_backend: Option<&'a str>,
    pub planner_backend: Option<&'a str>,
    pub implementer_backend: Option<&'a str>,
    pub reviewer_backend: Option<&'a str>,
    pub completer_backend: Option<&'a str>,
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
    run_overrides: RunWorkflowOverrides<'_>,
) -> Result<EffectiveConfig> {
    let project_ref = project.as_ref();

    let starting_backend = if let Some(override_backend) = run_overrides.starting_backend {
        override_backend.to_owned()
    } else if let Some(value) = project_ref.and_then(|p| p.workflow.starting_backend.clone()) {
        value
    } else {
        global.workspace.default_backend.clone()
    };
    validate_backend_spec(&global, &starting_backend, "starting backend")?;

    let planner_backend = resolve_optional_backend_override(
        run_overrides.planner_backend,
        project_ref.and_then(|p| p.workflow.planner_backend.as_deref()),
        global.workflow.planner_backend.as_deref(),
    );
    let implementer_backend = resolve_optional_backend_override(
        run_overrides.implementer_backend,
        project_ref.and_then(|p| p.workflow.implementer_backend.as_deref()),
        global.workflow.implementer_backend.as_deref(),
    );
    let reviewer_backend = resolve_optional_backend_override(
        run_overrides.reviewer_backend,
        project_ref.and_then(|p| p.workflow.reviewer_backend.as_deref()),
        global.workflow.reviewer_backend.as_deref(),
    );
    let completer_backend = resolve_optional_backend_override(
        run_overrides.completer_backend,
        project_ref.and_then(|p| p.workflow.completer_backend.as_deref()),
        global.workflow.completer_backend.as_deref(),
    );

    if let Some(spec) = planner_backend.as_deref() {
        validate_backend_spec(&global, spec, "planner backend override")?;
    }
    if let Some(spec) = implementer_backend.as_deref() {
        validate_backend_spec(&global, spec, "implementer backend override")?;
    }
    if let Some(spec) = reviewer_backend.as_deref() {
        validate_backend_spec(&global, spec, "reviewer backend override")?;
    }
    if let Some(spec) = completer_backend.as_deref() {
        validate_backend_spec(&global, spec, "completer backend override")?;
    }

    let workflow = EffectiveWorkflowConfig {
        starting_backend,
        planner_backend,
        implementer_backend,
        reviewer_backend,
        completer_backend,
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

fn resolve_optional_backend_override(
    cli_value: Option<&str>,
    project_value: Option<&str>,
    global_value: Option<&str>,
) -> Option<String> {
    cli_value
        .or(project_value)
        .or(global_value)
        .map(ToOwned::to_owned)
}

fn validate_backend_spec(global: &GlobalConfig, backend_spec: &str, label: &str) -> Result<()> {
    let parsed = parse_backend_spec(backend_spec)?;
    if global.backend_config(&parsed.name).is_none() {
        return Err(RalphError::Validation(format!(
            "unknown backend configured as {label}: {backend_spec}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::config::{
        project::ProjectWorkflowOverrides, resolve_effective_config, GlobalConfig, ProjectConfig,
        RunWorkflowOverrides,
    };

    #[test]
    fn resolve_effective_config_accepts_starting_backend_with_model_spec() {
        let mut global = GlobalConfig::default();
        global.workspace.default_backend = "claude(opus)".to_owned();

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("model backend spec should resolve");

        assert_eq!(effective.workflow.starting_backend, "claude(opus)");
    }

    #[test]
    fn resolve_effective_config_rejects_unknown_base_backend_in_spec() {
        let mut global = GlobalConfig::default();
        global.workspace.default_backend = "unknown(opus)".to_owned();

        let error = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("unknown backend should fail validation");

        assert!(error
            .to_string()
            .contains("unknown backend configured as starting backend: unknown(opus)"));
    }

    #[test]
    fn resolve_effective_config_applies_role_override_precedence() {
        let mut global = GlobalConfig::default();
        global.workflow.planner_backend = Some("codex(gpt-5)".to_owned());
        global.workflow.implementer_backend = Some("claude(sonnet)".to_owned());
        global.workflow.reviewer_backend = Some("codex".to_owned());
        global.workflow.completer_backend = Some("claude(opus)".to_owned());

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                planner_backend: Some("claude".to_owned()),
                implementer_backend: None,
                reviewer_backend: Some("claude(haiku)".to_owned()),
                completer_backend: Some("codex".to_owned()),
                ..ProjectWorkflowOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            Some(project),
            RunWorkflowOverrides {
                planner_backend: Some("codex(gpt-5.3-codex)"),
                implementer_backend: Some("claude(opus)"),
                ..RunWorkflowOverrides::default()
            },
        )
        .expect("overrides should resolve");

        assert_eq!(
            effective.workflow.planner_backend.as_deref(),
            Some("codex(gpt-5.3-codex)")
        );
        assert_eq!(
            effective.workflow.implementer_backend.as_deref(),
            Some("claude(opus)")
        );
        assert_eq!(
            effective.workflow.reviewer_backend.as_deref(),
            Some("claude(haiku)")
        );
        assert_eq!(
            effective.workflow.completer_backend.as_deref(),
            Some("codex")
        );
    }

    #[test]
    fn resolve_effective_config_rejects_unknown_role_override_backend() {
        let mut global = GlobalConfig::default();
        global.workflow.implementer_backend = Some("unknown(model)".to_owned());

        let error = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("unknown role override should fail validation");

        assert!(error
            .to_string()
            .contains("unknown backend configured as implementer backend override"));
    }
}
