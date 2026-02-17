pub mod global;
pub mod project;

use std::path::{Path, PathBuf};

use crate::backend::parse_backend_spec;
use crate::error::RalphError;
use crate::Result;

pub use global::{
    CommitMessageStyle, GlobalConfig, PlannerStateInPrompt, PreviousSpecsInPrompt,
    PromptChangeAction,
};
pub use project::{ProjectConfig, ProjectDaemonOverrides};

#[derive(Debug, Clone)]
pub struct EffectiveWorkflowConfig {
    pub starting_backend: String,
    pub prompt_review_enabled: bool,
    pub prompt_review_backend: String,
    pub planner_backend: Option<String>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub qa_backend: Option<String>,
    pub completer_backend: Option<String>,
    pub qa_enabled: bool,
    pub max_qa_iterations: u32,
    pub max_review_iterations: u32,
    pub auto_commit: bool,
    pub commit_message_style: CommitMessageStyle,
    pub commit_tag_format: String,
    pub prompt_change_action: PromptChangeAction,
    pub planner_state_in_prompt: PlannerStateInPrompt,
    pub planner_previous_specs_in_prompt: PreviousSpecsInPrompt,
    pub planner_max_prior_loops: Option<usize>,
    pub max_review_history_entries_in_prompt: usize,
    pub max_qa_history_entries_in_prompt: usize,
    pub include_history_when_session_reuse_enabled: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RunWorkflowOverrides<'a> {
    pub starting_backend: Option<&'a str>,
    pub planner_backend: Option<&'a str>,
    pub implementer_backend: Option<&'a str>,
    pub reviewer_backend: Option<&'a str>,
    pub qa_backend: Option<&'a str>,
    pub completer_backend: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct EffectiveTemplateConfig {
    pub planner: PathBuf,
    pub implementer: PathBuf,
    pub reviewer: PathBuf,
    pub prompt_reviewer: PathBuf,
    pub completer: PathBuf,
    pub qa: PathBuf,
}

#[derive(Debug, Clone)]
pub struct EffectiveDaemonConfig {
    pub poll_seconds: u64,
    pub max_concurrent: u32,
    pub labels: Vec<String>,
    pub repo: Option<String>,
    pub refinement_enabled: bool,
    pub refinement_backend: String,
    pub auto_rebase_enabled: bool,
    pub rebase_interval_seconds: u64,
    pub max_rebases_per_cycle: u32,
    pub rebase_timeout_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub workflow: EffectiveWorkflowConfig,
    pub templates: EffectiveTemplateConfig,
    pub daemon: EffectiveDaemonConfig,
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
    let daemon = resolve_daemon_config(&global, project_ref);

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
    let qa_backend = resolve_optional_backend_override(
        run_overrides.qa_backend,
        project_ref.and_then(|p| p.workflow.qa_backend.as_deref()),
        global.workflow.qa_backend.as_deref(),
    );
    let prompt_review_backend = project_ref
        .and_then(|p| p.workflow.prompt_review_backend.clone())
        .unwrap_or_else(|| global.workflow.prompt_review_backend.clone());

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
    if let Some(spec) = qa_backend.as_deref() {
        validate_backend_spec(&global, spec, "qa backend override")?;
    }
    validate_backend_spec(
        &global,
        &prompt_review_backend,
        "prompt review backend override",
    )?;

    let workflow = EffectiveWorkflowConfig {
        starting_backend,
        prompt_review_enabled: project_ref
            .and_then(|p| p.workflow.prompt_review_enabled)
            .unwrap_or(global.workflow.prompt_review_enabled),
        prompt_review_backend,
        planner_backend,
        implementer_backend,
        reviewer_backend,
        qa_backend,
        completer_backend,
        qa_enabled: project_ref
            .and_then(|p| p.workflow.qa_enabled)
            .unwrap_or(global.workflow.qa_enabled),
        max_qa_iterations: project_ref
            .and_then(|p| p.workflow.max_qa_iterations)
            .unwrap_or(global.workflow.max_qa_iterations),
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
        planner_state_in_prompt: project_ref
            .and_then(|p| p.workflow.planner_state_in_prompt)
            .unwrap_or(global.workflow.planner_state_in_prompt),
        planner_previous_specs_in_prompt: project_ref
            .and_then(|p| p.workflow.planner_previous_specs_in_prompt)
            .unwrap_or(global.workflow.planner_previous_specs_in_prompt),
        planner_max_prior_loops: project_ref
            .and_then(|p| p.workflow.planner_max_prior_loops)
            .unwrap_or(global.workflow.planner_max_prior_loops),
        max_review_history_entries_in_prompt: project_ref
            .and_then(|p| p.workflow.max_review_history_entries_in_prompt)
            .unwrap_or(global.workflow.max_review_history_entries_in_prompt),
        max_qa_history_entries_in_prompt: project_ref
            .and_then(|p| p.workflow.max_qa_history_entries_in_prompt)
            .unwrap_or(global.workflow.max_qa_history_entries_in_prompt),
        include_history_when_session_reuse_enabled: project_ref
            .and_then(|p| p.workflow.include_history_when_session_reuse_enabled)
            .unwrap_or(global.workflow.include_history_when_session_reuse_enabled),
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
        prompt_reviewer: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.prompt_reviewer.as_deref()),
            &global.templates.prompt_reviewer,
        ),
        completer: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.completer.as_deref()),
            &global.templates.completer,
        ),
        qa: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.qa.as_deref()),
            &global.templates.qa,
        ),
    };

    Ok(EffectiveConfig {
        workflow,
        templates,
        daemon,
        global,
        project,
    })
}

pub fn resolve_daemon_config(
    global: &GlobalConfig,
    project: Option<&ProjectConfig>,
) -> EffectiveDaemonConfig {
    let daemon_overrides = project.map(|cfg| &cfg.daemon);
    EffectiveDaemonConfig {
        poll_seconds: daemon_overrides
            .and_then(|cfg| cfg.poll_seconds)
            .unwrap_or(global.workspace.daemon_poll_seconds),
        max_concurrent: daemon_overrides
            .and_then(|cfg| cfg.max_concurrent)
            .unwrap_or(global.workspace.daemon_max_concurrent),
        labels: daemon_overrides
            .and_then(|cfg| cfg.labels.clone())
            .unwrap_or_else(|| global.workspace.daemon_labels.clone()),
        repo: daemon_overrides
            .and_then(|cfg| cfg.repo.clone())
            .or_else(|| global.workspace.daemon_repo.clone()),
        refinement_enabled: daemon_overrides
            .and_then(|cfg| cfg.refinement_enabled)
            .unwrap_or(global.workspace.daemon_refinement_enabled),
        refinement_backend: daemon_overrides
            .and_then(|cfg| cfg.refinement_backend.clone())
            .unwrap_or_else(|| global.workspace.daemon_refinement_backend.clone()),
        auto_rebase_enabled: daemon_overrides
            .and_then(|cfg| cfg.auto_rebase_enabled)
            .unwrap_or(global.workspace.daemon_auto_rebase_enabled),
        rebase_interval_seconds: daemon_overrides
            .and_then(|cfg| cfg.rebase_interval_seconds)
            .unwrap_or(global.workspace.daemon_rebase_interval_seconds),
        max_rebases_per_cycle: daemon_overrides
            .and_then(|cfg| cfg.max_rebases_per_cycle)
            .unwrap_or(global.workspace.daemon_max_rebases_per_cycle),
        rebase_timeout_seconds: daemon_overrides
            .and_then(|cfg| cfg.rebase_timeout_seconds)
            .unwrap_or(global.workspace.daemon_rebase_timeout_seconds),
    }
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
        global::{PlannerStateInPrompt, PreviousSpecsInPrompt},
        project::{ProjectDaemonOverrides, ProjectWorkflowOverrides},
        resolve_daemon_config, resolve_effective_config, GlobalConfig, ProjectConfig,
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

    #[test]
    fn resolve_effective_config_resolves_qa_precedence_and_template_paths() {
        let mut global = GlobalConfig::default();
        global.workflow.qa_backend = Some("claude(opus)".to_owned());
        global.workflow.qa_enabled = false;
        global.workflow.max_qa_iterations = 3;
        global.workflow.prompt_review_enabled = true;
        global.workflow.prompt_review_backend = "codex(gpt-5.3-codex-xhigh)".to_owned();
        global.templates.qa = "templates/qa-global.md".to_owned();
        global.templates.prompt_reviewer = "templates/prompt-reviewer-global.md".to_owned();

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                qa_backend: Some("codex".to_owned()),
                qa_enabled: Some(true),
                max_qa_iterations: Some(9),
                prompt_review_enabled: Some(false),
                prompt_review_backend: Some("claude(opus)".to_owned()),
                ..ProjectWorkflowOverrides::default()
            },
            templates: crate::config::project::ProjectTemplateOverrides {
                qa: Some("templates/qa-project.md".to_owned()),
                prompt_reviewer: Some("templates/prompt-reviewer-project.md".to_owned()),
                ..crate::config::project::ProjectTemplateOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project-a"),
            global,
            Some(project),
            RunWorkflowOverrides {
                qa_backend: Some("claude(sonnet)"),
                ..RunWorkflowOverrides::default()
            },
        )
        .expect("qa settings should resolve");

        assert_eq!(
            effective.workflow.qa_backend.as_deref(),
            Some("claude(sonnet)")
        );
        assert!(effective.workflow.qa_enabled);
        assert_eq!(effective.workflow.max_qa_iterations, 9);
        assert!(!effective.workflow.prompt_review_enabled);
        assert_eq!(effective.workflow.prompt_review_backend, "claude(opus)");
        assert_eq!(
            effective.templates.qa,
            Path::new("/workspace/project-a/templates/qa-project.md")
        );
        assert_eq!(
            effective.templates.prompt_reviewer,
            Path::new("/workspace/project-a/templates/prompt-reviewer-project.md")
        );
    }

    #[test]
    fn resolve_effective_config_rejects_unknown_prompt_review_backend() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backend = "unknown(model)".to_owned();

        let error = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("unknown prompt review backend should fail validation");

        assert!(error
            .to_string()
            .contains("unknown backend configured as prompt review backend override"));
    }

    #[test]
    fn resolve_daemon_config_applies_project_overrides_over_workspace_defaults() {
        let mut global = GlobalConfig::default();
        global.workspace.daemon_poll_seconds = 60;
        global.workspace.daemon_max_concurrent = 1;
        global.workspace.daemon_labels = vec!["ralph:ready".to_owned()];
        global.workspace.daemon_repo = Some("acme/global".to_owned());
        global.workspace.daemon_refinement_enabled = true;
        global.workspace.daemon_refinement_backend = "claude(sonnet)".to_owned();
        global.workspace.daemon_auto_rebase_enabled = true;
        global.workspace.daemon_rebase_interval_seconds = 1800;
        global.workspace.daemon_max_rebases_per_cycle = 3;
        global.workspace.daemon_rebase_timeout_seconds = 120;

        let project = ProjectConfig {
            daemon: ProjectDaemonOverrides {
                poll_seconds: Some(15),
                max_concurrent: Some(3),
                labels: Some(vec!["l1".to_owned(), "l2".to_owned()]),
                repo: Some("acme/project".to_owned()),
                refinement_enabled: Some(false),
                refinement_backend: Some("codex(gpt-5.3-codex-medium)".to_owned()),
                auto_rebase_enabled: Some(false),
                rebase_interval_seconds: Some(900),
                max_rebases_per_cycle: Some(5),
                rebase_timeout_seconds: Some(240),
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_daemon_config(&global, Some(&project));
        assert_eq!(effective.poll_seconds, 15);
        assert_eq!(effective.max_concurrent, 3);
        assert_eq!(effective.labels, vec!["l1".to_owned(), "l2".to_owned()]);
        assert_eq!(effective.repo.as_deref(), Some("acme/project"));
        assert!(!effective.refinement_enabled);
        assert_eq!(effective.refinement_backend, "codex(gpt-5.3-codex-medium)");
        assert!(!effective.auto_rebase_enabled);
        assert_eq!(effective.rebase_interval_seconds, 900);
        assert_eq!(effective.max_rebases_per_cycle, 5);
        assert_eq!(effective.rebase_timeout_seconds, 240);

        let no_project = resolve_daemon_config(&global, None);
        assert_eq!(no_project.poll_seconds, 60);
        assert_eq!(no_project.max_concurrent, 1);
        assert_eq!(no_project.labels, vec!["ralph:ready".to_owned()]);
        assert_eq!(no_project.repo.as_deref(), Some("acme/global"));
        assert!(no_project.refinement_enabled);
        assert_eq!(no_project.refinement_backend, "claude(sonnet)");
        assert!(no_project.auto_rebase_enabled);
        assert_eq!(no_project.rebase_interval_seconds, 1800);
        assert_eq!(no_project.max_rebases_per_cycle, 3);
        assert_eq!(no_project.rebase_timeout_seconds, 120);
    }

    #[test]
    fn resolve_effective_config_defaults_planner_compression_fields() {
        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            GlobalConfig::default(),
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve defaults");

        assert_eq!(
            effective.workflow.planner_state_in_prompt,
            PlannerStateInPrompt::Summary
        );
        assert_eq!(
            effective.workflow.planner_previous_specs_in_prompt,
            PreviousSpecsInPrompt::Titles
        );
        assert_eq!(effective.workflow.planner_max_prior_loops, Some(10));
        assert_eq!(effective.workflow.max_review_history_entries_in_prompt, 3);
        assert_eq!(effective.workflow.max_qa_history_entries_in_prompt, 2);
        assert!(
            !effective
                .workflow
                .include_history_when_session_reuse_enabled
        );
    }

    #[test]
    fn resolve_effective_config_global_overrides_planner_compression() {
        let mut global = GlobalConfig::default();
        global.workflow.planner_state_in_prompt = PlannerStateInPrompt::FullJson;
        global.workflow.planner_previous_specs_in_prompt = PreviousSpecsInPrompt::FullText;
        global.workflow.planner_max_prior_loops = None;

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve global overrides");

        assert_eq!(
            effective.workflow.planner_state_in_prompt,
            PlannerStateInPrompt::FullJson
        );
        assert_eq!(
            effective.workflow.planner_previous_specs_in_prompt,
            PreviousSpecsInPrompt::FullText
        );
        assert_eq!(effective.workflow.planner_max_prior_loops, None);
    }

    #[test]
    fn resolve_effective_config_project_overrides_planner_compression() {
        let mut global = GlobalConfig::default();
        global.workflow.planner_state_in_prompt = PlannerStateInPrompt::FullJson;
        global.workflow.planner_previous_specs_in_prompt = PreviousSpecsInPrompt::FullText;
        global.workflow.planner_max_prior_loops = Some(10);

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                planner_state_in_prompt: Some(PlannerStateInPrompt::Summary),
                planner_previous_specs_in_prompt: Some(PreviousSpecsInPrompt::None),
                planner_max_prior_loops: Some(Some(5)),
                ..ProjectWorkflowOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            Some(project),
            RunWorkflowOverrides::default(),
        )
        .expect("resolve project overrides");

        assert_eq!(
            effective.workflow.planner_state_in_prompt,
            PlannerStateInPrompt::Summary
        );
        assert_eq!(
            effective.workflow.planner_previous_specs_in_prompt,
            PreviousSpecsInPrompt::None
        );
        assert_eq!(effective.workflow.planner_max_prior_loops, Some(5));
    }

    #[test]
    fn resolve_effective_config_project_override_unlimited_loops() {
        let mut global = GlobalConfig::default();
        global.workflow.planner_max_prior_loops = Some(10);

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                planner_max_prior_loops: Some(None), // override to unlimited
                ..ProjectWorkflowOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            Some(project),
            RunWorkflowOverrides::default(),
        )
        .expect("resolve unlimited override");

        assert_eq!(effective.workflow.planner_max_prior_loops, None);
    }

    #[test]
    fn resolve_effective_config_project_absent_inherits_global() {
        let mut global = GlobalConfig::default();
        global.workflow.planner_state_in_prompt = PlannerStateInPrompt::FullJson;
        global.workflow.planner_previous_specs_in_prompt = PreviousSpecsInPrompt::FullText;
        global.workflow.planner_max_prior_loops = Some(3);

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                // All None => inherit from global
                ..ProjectWorkflowOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            Some(project),
            RunWorkflowOverrides::default(),
        )
        .expect("resolve inherit");

        assert_eq!(
            effective.workflow.planner_state_in_prompt,
            PlannerStateInPrompt::FullJson
        );
        assert_eq!(
            effective.workflow.planner_previous_specs_in_prompt,
            PreviousSpecsInPrompt::FullText
        );
        assert_eq!(effective.workflow.planner_max_prior_loops, Some(3));
    }

    #[test]
    fn resolve_effective_config_history_capping_fields_follow_precedence() {
        let mut global = GlobalConfig::default();
        global.workflow.max_review_history_entries_in_prompt = 9;
        global.workflow.max_qa_history_entries_in_prompt = 8;
        global.workflow.include_history_when_session_reuse_enabled = true;

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                max_review_history_entries_in_prompt: Some(4),
                max_qa_history_entries_in_prompt: None,
                include_history_when_session_reuse_enabled: Some(false),
                ..ProjectWorkflowOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            Some(project),
            RunWorkflowOverrides::default(),
        )
        .expect("resolve history capping config");

        assert_eq!(effective.workflow.max_review_history_entries_in_prompt, 4);
        assert_eq!(effective.workflow.max_qa_history_entries_in_prompt, 8);
        assert!(
            !effective
                .workflow
                .include_history_when_session_reuse_enabled
        );
    }
}
