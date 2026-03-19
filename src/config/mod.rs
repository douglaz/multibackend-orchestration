pub mod global;
pub mod project;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::backend::parse_backend_spec;
use crate::error::RalphError;
use crate::project::artifacts::slugify_backend;
use crate::Result;
use tracing::warn;

pub(crate) use global::save_sparse;
pub(crate) use global::set_global_config_value;
pub use global::{
    AmendmentsConfig, CommitMessageStyle, GlobalConfig, PlannerStateInPrompt,
    PreviousSpecsInPrompt, PromptChangeAction,
};
pub use project::{ProjectConfig, ProjectDaemonOverrides};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationSurface {
    Required,
    RequiredPanel,
    PanelList,
}

impl ValidationSurface {
    fn allows_optional(self) -> bool {
        matches!(self, ValidationSurface::PanelList)
    }
}

#[derive(Debug, Clone)]
pub struct EffectiveWorkflowConfig {
    pub starting_backend: String,
    pub prompt_review_enabled: bool,
    pub prompt_review_backends: Vec<String>,
    pub prompt_review_min_reviewers: u32,
    pub planner_backend: Option<String>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub qa_backend: Option<String>,
    pub completer_backend: Option<String>,
    pub final_review_enabled: bool,
    pub final_review_backends: Vec<String>,
    pub final_review_arbiter_backend: String,
    pub final_review_min_reviewers: u32,
    pub final_review_consensus_threshold: f64,
    pub max_final_review_restarts: u32,
    pub completion_backends: Vec<String>,
    pub completion_min_completers: u32,
    pub completion_consensus_threshold: f64,
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
    pub session_reuse_enabled: bool,
    pub session_reuse_roles: Vec<String>,
    pub session_reuse_reset_on_prompt_change: bool,
    pub session_reuse_reset_on_rollback: bool,
    pub pre_commit_fmt: bool,
    pub pre_commit_clippy: bool,
    pub pre_commit_nix_build: bool,
    pub pre_commit_fmt_auto_fix: bool,
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
    pub prompt_review_validator: PathBuf,
    pub completer: PathBuf,
    pub qa: PathBuf,
    pub final_reviewer: PathBuf,
    pub quick_dev_plan_implement: PathBuf,
    pub quick_dev_codex_review: PathBuf,
    pub quick_dev_apply_fixes: PathBuf,
    pub planner_position: PathBuf,
    pub vote: PathBuf,
    pub arbiter: PathBuf,
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
    pub rebase_agent_backend: String,
    pub prd_enabled: bool,
    pub prd_question_backends: Vec<String>,
    pub prd_writer_backend: String,
    pub prd_reviewer_backend: String,
    pub prd_max_revisions: u32,
    pub prd_backend_timeout_secs: u64,
    pub prd_shutdown_timeout_secs: u64,
    pub oracle_review_enabled: bool,
    pub oracle_review_timeout_secs: u64,
    pub oracle_review_authors: Vec<String>,
    pub oracle_review_max_per_cycle: u32,
    /// Maximum number of backend timeout retries per invocation.
    pub max_backend_retries: Option<u8>,
    pub pr_review_whitelist: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EffectiveAmendmentsConfig {
    pub unify_final_review: bool,
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub workflow: EffectiveWorkflowConfig,
    pub templates: EffectiveTemplateConfig,
    pub daemon: EffectiveDaemonConfig,
    pub amendments: EffectiveAmendmentsConfig,
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
    validate_backend_spec(
        &global,
        &starting_backend,
        "starting backend",
        ValidationSurface::Required,
    )?;

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
    let prompt_review_backends = if let Some(backends) =
        project_ref.and_then(|p| p.workflow.prompt_review_backends.clone())
    {
        backends
    } else if let Some(backend) = project_ref.and_then(|p| p.workflow.prompt_review_backend.clone())
    {
        validate_backend_spec(
            &global,
            &backend,
            "workflow.prompt_review_backend",
            ValidationSurface::Required,
        )?;
        vec![backend]
    } else if let Some(backends) = global.workflow.prompt_review_backends.clone() {
        backends
    } else {
        validate_backend_spec(
            &global,
            &global.workflow.prompt_review_backend,
            "workflow.prompt_review_backend",
            ValidationSurface::Required,
        )?;
        vec![global.workflow.prompt_review_backend.clone()]
    };

    if let Some(spec) = planner_backend.as_deref() {
        validate_backend_spec(
            &global,
            spec,
            "planner backend override",
            ValidationSurface::Required,
        )?;
    }
    if let Some(spec) = implementer_backend.as_deref() {
        validate_backend_spec(
            &global,
            spec,
            "implementer backend override",
            ValidationSurface::Required,
        )?;
    }
    if let Some(spec) = reviewer_backend.as_deref() {
        validate_backend_spec(
            &global,
            spec,
            "reviewer backend override",
            ValidationSurface::Required,
        )?;
    }
    if let Some(spec) = completer_backend.as_deref() {
        validate_backend_spec(
            &global,
            spec,
            "completer backend override",
            ValidationSurface::Required,
        )?;
    }
    if let Some(spec) = qa_backend.as_deref() {
        validate_backend_spec(
            &global,
            spec,
            "qa backend override",
            ValidationSurface::Required,
        )?;
    }
    let workflow = EffectiveWorkflowConfig {
        starting_backend,
        prompt_review_enabled: project_ref
            .and_then(|p| p.workflow.prompt_review_enabled)
            .unwrap_or(global.workflow.prompt_review_enabled),
        prompt_review_backends,
        prompt_review_min_reviewers: project_ref
            .and_then(|p| p.workflow.prompt_review_min_reviewers)
            .unwrap_or(global.workflow.prompt_review_min_reviewers),
        planner_backend,
        implementer_backend,
        reviewer_backend,
        qa_backend,
        completer_backend,
        final_review_enabled: project_ref
            .and_then(|p| p.workflow.final_review_enabled)
            .unwrap_or(global.workflow.final_review_enabled),
        final_review_backends: project_ref
            .and_then(|p| p.workflow.final_review_backends.clone())
            .unwrap_or_else(|| global.workflow.final_review_backends.clone()),
        final_review_arbiter_backend: project_ref
            .and_then(|p| p.workflow.final_review_arbiter_backend.clone())
            .unwrap_or_else(|| global.workflow.final_review_arbiter_backend.clone()),
        final_review_min_reviewers: project_ref
            .and_then(|p| p.workflow.final_review_min_reviewers)
            .unwrap_or(global.workflow.final_review_min_reviewers),
        final_review_consensus_threshold: project_ref
            .and_then(|p| p.workflow.final_review_consensus_threshold)
            .unwrap_or(global.workflow.final_review_consensus_threshold),
        max_final_review_restarts: project_ref
            .and_then(|p| p.workflow.max_final_review_restarts)
            .unwrap_or(global.workflow.max_final_review_restarts),
        completion_backends: project_ref
            .and_then(|p| p.workflow.completion_backends.clone())
            .unwrap_or_else(|| global.workflow.completion_backends.clone()),
        completion_min_completers: project_ref
            .and_then(|p| p.workflow.completion_min_completers)
            .unwrap_or(global.workflow.completion_min_completers),
        completion_consensus_threshold: project_ref
            .and_then(|p| p.workflow.completion_consensus_threshold)
            .unwrap_or(global.workflow.completion_consensus_threshold),
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
        session_reuse_enabled: project_ref
            .and_then(|p| p.workflow.session_reuse_enabled)
            .unwrap_or(global.workflow.session_reuse_enabled),
        session_reuse_roles: project_ref
            .and_then(|p| p.workflow.session_reuse_roles.clone())
            .unwrap_or_else(|| global.workflow.session_reuse_roles.clone()),
        session_reuse_reset_on_prompt_change: project_ref
            .and_then(|p| p.workflow.session_reuse_reset_on_prompt_change)
            .unwrap_or(global.workflow.session_reuse_reset_on_prompt_change),
        session_reuse_reset_on_rollback: project_ref
            .and_then(|p| p.workflow.session_reuse_reset_on_rollback)
            .unwrap_or(global.workflow.session_reuse_reset_on_rollback),
        pre_commit_fmt: project_ref
            .and_then(|p| p.workflow.pre_commit_fmt)
            .unwrap_or(global.workflow.pre_commit_fmt),
        pre_commit_clippy: project_ref
            .and_then(|p| p.workflow.pre_commit_clippy)
            .unwrap_or(global.workflow.pre_commit_clippy),
        pre_commit_nix_build: project_ref
            .and_then(|p| p.workflow.pre_commit_nix_build)
            .unwrap_or(global.workflow.pre_commit_nix_build),
        pre_commit_fmt_auto_fix: project_ref
            .and_then(|p| p.workflow.pre_commit_fmt_auto_fix)
            .unwrap_or(global.workflow.pre_commit_fmt_auto_fix),
    };
    let final_review_validation = validate_final_review_config(&global, &workflow)?;
    if workflow.final_review_enabled && final_review_validation.arbiter_overlaps_reviewer_family {
        warn!(
            arbiter_backend = %workflow.final_review_arbiter_backend,
            arbiter_family = %final_review_validation.arbiter_family,
            reviewers = ?final_review_validation.normalized_reviewers,
            reviewer_families = ?final_review_validation.reviewer_families,
            "final review arbiter backend family overlaps configured reviewer families"
        );
    }

    validate_completion_panel_config(&global, &workflow)?;
    validate_prompt_review_panel_config(&global, &workflow)?;

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
        prompt_review_validator: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.prompt_review_validator.as_deref()),
            &global.templates.prompt_review_validator,
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
        final_reviewer: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.final_reviewer.as_deref()),
            &global.templates.final_reviewer,
        ),
        quick_dev_plan_implement: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.quick_dev_plan_implement.as_deref()),
            &global.templates.quick_dev_plan_implement,
        ),
        quick_dev_codex_review: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.quick_dev_codex_review.as_deref()),
            &global.templates.quick_dev_codex_review,
        ),
        quick_dev_apply_fixes: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.quick_dev_apply_fixes.as_deref()),
            &global.templates.quick_dev_apply_fixes,
        ),
        planner_position: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.planner_position.as_deref()),
            &global.templates.planner_position,
        ),
        vote: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.vote.as_deref()),
            &global.templates.vote,
        ),
        arbiter: resolve_template_path(
            workspace_root,
            project_dir,
            project_ref.and_then(|p| p.templates.arbiter.as_deref()),
            &global.templates.arbiter,
        ),
    };

    let amendments = EffectiveAmendmentsConfig {
        unify_final_review: project_ref
            .and_then(|p| p.amendments.unify_final_review)
            .unwrap_or(global.amendments.unify_final_review),
    };

    Ok(EffectiveConfig {
        workflow,
        templates,
        daemon,
        amendments,
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
        rebase_agent_backend: daemon_overrides
            .and_then(|cfg| cfg.rebase_agent_backend.clone())
            .unwrap_or_else(|| global.workspace.daemon_rebase_agent_backend.clone()),
        prd_enabled: global.workspace.daemon_prd_enabled,
        prd_question_backends: global.workspace.daemon_prd_question_backends.clone(),
        prd_writer_backend: global.workspace.daemon_prd_writer_backend.clone(),
        prd_reviewer_backend: global.workspace.daemon_prd_reviewer_backend.clone(),
        prd_max_revisions: global.workspace.daemon_prd_max_revisions,
        prd_backend_timeout_secs: global.workspace.daemon_prd_backend_timeout_secs,
        prd_shutdown_timeout_secs: global.workspace.daemon_prd_shutdown_timeout_secs,
        oracle_review_enabled: global.workspace.daemon_oracle_review_enabled,
        oracle_review_timeout_secs: global.workspace.daemon_oracle_review_timeout_secs,
        oracle_review_authors: global.workspace.daemon_oracle_review_authors.clone(),
        oracle_review_max_per_cycle: global.workspace.daemon_oracle_review_max_per_cycle,
        max_backend_retries: global.workspace.daemon_max_backend_retries,
        pr_review_whitelist: global.workspace.daemon_pr_review_whitelist.clone(),
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

fn validate_backend_spec(
    global: &GlobalConfig,
    backend_spec: &str,
    label: &str,
    surface: ValidationSurface,
) -> Result<crate::backend::BackendSpec> {
    let parsed = parse_backend_spec(backend_spec)?;
    if parsed.optional && !surface.allows_optional() {
        return Err(RalphError::Validation(format!(
            "optional backend specs (?backend) are not supported for {label}; optional syntax is allowed only in panel backend lists"
        )));
    }

    if global.backend_config(&parsed.name).is_none() {
        return Err(RalphError::Validation(format!(
            "unknown backend configured as {label}: {backend_spec}"
        )));
    }

    Ok(parsed)
}

/// Public wrapper for validating a backend spec with `Required` surface
/// semantics (rejects optional `?backend` prefixes).
/// Use this when pre-validating backends outside `resolve_effective_config`,
/// e.g. in `quick-dev-auto` preflight.
pub fn validate_required_backend_spec(
    global: &GlobalConfig,
    spec: &str,
    label: &str,
) -> Result<()> {
    validate_backend_spec(global, spec, label, ValidationSurface::Required)?;
    Ok(())
}

pub fn validate_interactive_prd_workspace_config(global: &GlobalConfig) -> Result<()> {
    let workspace = &global.workspace;
    if workspace.daemon_prd_question_backends.len() != 2 {
        return Err(RalphError::Validation(format!(
            "workspace.daemon_prd_question_backends must contain exactly 2 backend specs, got {}",
            workspace.daemon_prd_question_backends.len()
        )));
    }

    for (index, backend_spec) in workspace.daemon_prd_question_backends.iter().enumerate() {
        validate_backend_spec(
            global,
            backend_spec,
            &format!("workspace.daemon_prd_question_backends[{index}]"),
            ValidationSurface::Required,
        )?;
    }

    validate_backend_spec(
        global,
        &workspace.daemon_prd_writer_backend,
        "workspace.daemon_prd_writer_backend",
        ValidationSurface::Required,
    )?;
    validate_backend_spec(
        global,
        &workspace.daemon_prd_reviewer_backend,
        "workspace.daemon_prd_reviewer_backend",
        ValidationSurface::Required,
    )?;

    Ok(())
}

pub fn validate_daemon_workspace_config(global: &GlobalConfig) -> Result<()> {
    validate_daemon_refinement_backend(
        global,
        &global.workspace.daemon_refinement_backend,
        "workspace.daemon_refinement_backend",
    )?;
    Ok(())
}

pub fn validate_effective_daemon_config(
    global: &GlobalConfig,
    daemon: &EffectiveDaemonConfig,
) -> Result<()> {
    validate_daemon_refinement_backend(
        global,
        &daemon.refinement_backend,
        "daemon.refinement_backend",
    )?;

    if daemon.prd_shutdown_timeout_secs < 1 {
        return Err(RalphError::Validation(format!(
            "workspace.daemon_prd_shutdown_timeout_secs must be >= 1, got {}",
            daemon.prd_shutdown_timeout_secs
        )));
    }

    if daemon.oracle_review_timeout_secs < 1 {
        return Err(RalphError::Validation(format!(
            "workspace.daemon_oracle_review_timeout_secs must be >= 1, got {}",
            daemon.oracle_review_timeout_secs
        )));
    }

    if daemon.oracle_review_max_per_cycle < 1 {
        return Err(RalphError::Validation(format!(
            "workspace.daemon_oracle_review_max_per_cycle must be >= 1, got {}",
            daemon.oracle_review_max_per_cycle
        )));
    }

    Ok(())
}

fn validate_daemon_refinement_backend(
    global: &GlobalConfig,
    backend: &str,
    label: &str,
) -> Result<()> {
    validate_backend_spec(global, backend, label, ValidationSurface::Required)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FinalReviewValidation {
    normalized_reviewers: Vec<String>,
    reviewer_families: Vec<String>,
    arbiter_family: String,
    arbiter_overlaps_reviewer_family: bool,
}

fn validate_final_review_config(
    global: &GlobalConfig,
    workflow: &EffectiveWorkflowConfig,
) -> Result<FinalReviewValidation> {
    if !workflow.final_review_consensus_threshold.is_finite()
        || workflow.final_review_consensus_threshold <= 0.0
        || workflow.final_review_consensus_threshold > 1.0
    {
        return Err(RalphError::Validation(format!(
            "final_review_consensus_threshold must be > 0.0 and <= 1.0, got {}",
            workflow.final_review_consensus_threshold
        )));
    }

    let normalized_reviewers = normalize_backend_specs(global, &workflow.final_review_backends)?;
    if workflow.final_review_enabled && normalized_reviewers.is_empty() {
        return Err(RalphError::Validation(
            "final_review_enabled=true requires at least one backend in final_review_backends"
                .to_owned(),
        ));
    }
    if workflow.final_review_enabled
        && normalized_reviewers.len() < workflow.final_review_min_reviewers as usize
    {
        return Err(RalphError::Validation(format!(
            "final_review_backends has {} unique backend specs after deduplication, but final_review_min_reviewers is {}",
            normalized_reviewers.len(),
            workflow.final_review_min_reviewers
        )));
    }

    let reviewer_families = unique_backend_families(&normalized_reviewers)?;

    let arbiter_parsed = validate_backend_spec(
        global,
        &workflow.final_review_arbiter_backend,
        "final review arbiter backend",
        ValidationSurface::RequiredPanel,
    )?;
    let arbiter_backend = match arbiter_parsed.model.as_deref() {
        Some(model) => format!("{}({model})", arbiter_parsed.name),
        None => arbiter_parsed.name.clone(),
    };
    let arbiter_family = parse_backend_spec(&arbiter_backend)?.name;

    Ok(FinalReviewValidation {
        normalized_reviewers,
        arbiter_overlaps_reviewer_family: reviewer_families.contains(&arbiter_family),
        reviewer_families,
        arbiter_family,
    })
}

fn normalize_backend_specs(global: &GlobalConfig, specs: &[String]) -> Result<Vec<String>> {
    normalize_backend_specs_labeled(global, specs, "final review backend", false)
}

fn normalize_backend_specs_labeled(
    global: &GlobalConfig,
    specs: &[String],
    label: &str,
    reject_duplicates: bool,
) -> Result<Vec<String>> {
    normalize_backend_specs_labeled_role(global, specs, label, reject_duplicates, None)
}

fn normalize_backend_specs_labeled_role(
    global: &GlobalConfig,
    specs: &[String],
    label: &str,
    reject_duplicates: bool,
    resolve_role: Option<&str>,
) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();

    for spec in specs {
        // Validate the raw entry first so optional marker semantics are preserved.
        validate_backend_spec(global, spec, label, ValidationSurface::PanelList)?;
        let canonical = canonicalize_backend_spec(spec)?;
        // Derive a resolution key that collapses optional/required variants
        // (e.g. `claude` and `?claude`) to the same target.
        // When a resolve_role is provided, also apply role-model injection so
        // that e.g. `claude` and `claude(opus)` collapse when the completer
        // role model for claude is `opus`.
        let resolved = match resolve_role {
            Some(role) => resolve_spec_for_role(global, &canonical, role),
            None => canonical.clone(),
        };
        let parsed = parse_backend_spec(&resolved)?;
        let resolution_key = match &parsed.model {
            Some(model) => format!("{}({model})", parsed.name),
            None => parsed.name.clone(),
        };
        if !seen.insert(resolution_key.clone()) {
            if reject_duplicates {
                return Err(RalphError::Validation(format!(
                    "duplicate resolved {label} spec after canonicalization: {resolution_key}"
                )));
            }
            // Silently deduplicate for surfaces that allow it
            continue;
        }
        normalized.push(canonical);
    }

    Ok(normalized)
}

/// Apply the same role-model resolution that `BackendRegistry::resolve_backend_for_role`
/// performs, but using `GlobalConfig` directly.  When the spec already has an explicit
/// model, it is returned unchanged; otherwise the configured role model is injected.
fn resolve_spec_for_role(global: &GlobalConfig, canonical_spec: &str, role: &str) -> String {
    let parsed = match parse_backend_spec(canonical_spec) {
        Ok(p) => p,
        Err(_) => return canonical_spec.to_owned(),
    };
    if parsed.model.is_some() {
        return canonical_spec.to_owned();
    }
    let model = global
        .backend_config(&parsed.name)
        .and_then(|bc| bc.models.for_role(role));
    match model {
        Some(m) => format!("{}({m})", parsed.name),
        None => canonical_spec.to_owned(),
    }
}

fn unique_backend_families(specs: &[String]) -> Result<Vec<String>> {
    let mut families = Vec::new();
    let mut seen = HashSet::new();
    for spec in specs {
        let family = parse_backend_spec(spec)?.name;
        if seen.insert(family.clone()) {
            families.push(family);
        }
    }
    Ok(families)
}

fn validate_completion_panel_config(
    global: &GlobalConfig,
    workflow: &EffectiveWorkflowConfig,
) -> Result<()> {
    if workflow.completion_backends.is_empty() {
        return Err(RalphError::Validation(
            "completion_backends must not be empty".to_owned(),
        ));
    }

    if workflow.completion_min_completers < 1 {
        return Err(RalphError::Validation(format!(
            "completion_min_completers must be >= 1, got {}",
            workflow.completion_min_completers
        )));
    }

    if !workflow.completion_consensus_threshold.is_finite()
        || workflow.completion_consensus_threshold <= 0.0
        || workflow.completion_consensus_threshold > 1.0
    {
        return Err(RalphError::Validation(format!(
            "completion_consensus_threshold must be > 0.0 and <= 1.0, got {}",
            workflow.completion_consensus_threshold
        )));
    }

    // normalize_backend_specs_labeled_role with reject_duplicates=true rejects
    // duplicate resolved specs (including optional/required variants and
    // role-model injection collapsing to the same target, e.g. `claude` and
    // `claude(opus)` when the completer role model for claude is `opus`).
    let normalized = normalize_backend_specs_labeled_role(
        global,
        &workflow.completion_backends,
        "completion backend",
        true,
        Some("completer"),
    )?;

    // Check for per-backend verdict filename collisions using the same
    // slugification logic as ArtifactKind::CompleterVerdictBackend, applied
    // to role-resolved specs (matching what the runtime orchestrator writes).
    let mut seen_filenames = HashSet::new();
    for spec in &normalized {
        let resolved = resolve_spec_for_role(global, spec, "completer");
        let filename = completion_verdict_filename(&resolved);
        if !seen_filenames.insert(filename.clone()) {
            return Err(RalphError::Validation(format!(
                "completion verdict filename collision for spec '{spec}': {filename}"
            )));
        }
    }

    Ok(())
}

fn validate_prompt_review_panel_config(
    global: &GlobalConfig,
    workflow: &EffectiveWorkflowConfig,
) -> Result<()> {
    if workflow.prompt_review_backends.is_empty() {
        return Err(RalphError::Validation(
            "prompt_review_backends must not be empty".to_owned(),
        ));
    }

    if workflow.prompt_review_min_reviewers < 1 {
        return Err(RalphError::Validation(format!(
            "prompt_review_min_reviewers must be >= 1, got {}",
            workflow.prompt_review_min_reviewers
        )));
    }

    normalize_backend_specs_labeled_role(
        global,
        &workflow.prompt_review_backends,
        "prompt review backend",
        true,
        Some("prompt_reviewer"),
    )?;

    Ok(())
}

/// Generate the deterministic verdict filename for a completion backend spec.
/// Uses the same `slugify_backend` logic as `ArtifactKind::CompleterVerdictBackend`
/// to ensure collision detection matches actual artifact naming.
/// Return the effective completion consensus settings by merging
/// global workflow defaults with optional project overrides.  Used by
/// reconstruction to match the runtime consensus behaviour.
pub fn effective_completion_consensus(
    global: &GlobalConfig,
    project: Option<&project::ProjectConfig>,
) -> (u32, f64) {
    let min = project
        .and_then(|p| p.workflow.completion_min_completers)
        .unwrap_or(global.workflow.completion_min_completers);
    let threshold = project
        .and_then(|p| p.workflow.completion_consensus_threshold)
        .unwrap_or(global.workflow.completion_consensus_threshold);
    (min, threshold)
}

fn completion_verdict_filename(spec: &str) -> String {
    format!("completer-verdict-{}.md", slugify_backend(spec))
}

fn canonicalize_backend_spec(spec: &str) -> Result<String> {
    let parsed = parse_backend_spec(spec)?;
    Ok(match parsed.model {
        Some(model) => format!("{}({model})", parsed.name),
        None => parsed.name,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::config::{
        global::{PlannerStateInPrompt, PreviousSpecsInPrompt},
        project::{ProjectDaemonOverrides, ProjectWorkflowOverrides},
        resolve_daemon_config, resolve_effective_config, validate_daemon_workspace_config,
        validate_effective_daemon_config, validate_interactive_prd_workspace_config, GlobalConfig,
        ProjectConfig, RunWorkflowOverrides,
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
    fn resolve_effective_config_rejects_optional_backend_on_required_surface() {
        let mut global = GlobalConfig::default();
        global.workspace.default_backend = "?claude".to_owned();

        let error = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("optional syntax on required surface should fail");

        assert!(error
            .to_string()
            .contains("optional backend specs (?backend) are not supported for starting backend"));
    }

    #[test]
    fn resolve_effective_config_rejects_unknown_backend_on_required_surfaces() {
        let mut global = GlobalConfig::default();
        global.workspace.default_backend = "badbackend".to_owned();
        let error = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("unknown backend should be rejected for starting backend");
        assert!(error
            .to_string()
            .contains("unknown backend configured as starting backend"));

        let mut global = GlobalConfig::default();
        global.workflow.planner_backend = Some("badbackend(pro)".to_owned());
        let error = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("unknown backend should be rejected for planner backend");
        assert!(error.to_string().contains("planner backend override"));
        assert!(error
            .to_string()
            .contains("unknown backend configured as planner backend override"));
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
                planner_backend: Some("codex(gpt-5.4)"),
                implementer_backend: Some("claude(opus)"),
                ..RunWorkflowOverrides::default()
            },
        )
        .expect("overrides should resolve");

        assert_eq!(
            effective.workflow.planner_backend.as_deref(),
            Some("codex(gpt-5.4)")
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
        global.workflow.prompt_review_backend = "codex(gpt-5.4-xhigh)".to_owned();
        global.workflow.prompt_review_backends = Some(vec!["codex(gpt-5.4-xhigh)".to_owned()]);
        global.templates.qa = "templates/qa-global.md".to_owned();
        global.templates.prompt_reviewer = "templates/prompt-reviewer-global.md".to_owned();
        global.templates.prompt_review_validator =
            "templates/prompt-review-validator-global.md".to_owned();

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
                prompt_review_validator: Some(
                    "templates/prompt-review-validator-project.md".to_owned(),
                ),
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
        assert_eq!(
            effective.workflow.prompt_review_backends,
            vec!["claude(opus)".to_owned()]
        );
        assert_eq!(
            effective.templates.qa,
            Path::new("/workspace/project-a/templates/qa-project.md")
        );
        assert_eq!(
            effective.templates.prompt_reviewer,
            Path::new("/workspace/project-a/templates/prompt-reviewer-project.md")
        );
        assert_eq!(
            effective.templates.prompt_review_validator,
            Path::new("/workspace/project-a/templates/prompt-review-validator-project.md")
        );
    }

    #[test]
    fn resolve_effective_config_resolves_quick_dev_template_paths() {
        let mut global = GlobalConfig::default();
        global.templates.quick_dev_plan_implement = "templates/quick-plan-global.md".to_owned();
        global.templates.quick_dev_codex_review = "templates/quick-codex-global.md".to_owned();
        global.templates.quick_dev_apply_fixes = "templates/quick-apply-global.md".to_owned();

        let project = ProjectConfig {
            templates: crate::config::project::ProjectTemplateOverrides {
                quick_dev_codex_review: Some("templates/quick-codex-project.md".to_owned()),
                ..crate::config::project::ProjectTemplateOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project-a"),
            global,
            Some(project),
            RunWorkflowOverrides::default(),
        )
        .expect("quick-dev template settings should resolve");

        assert_eq!(
            effective.templates.quick_dev_plan_implement,
            Path::new("/workspace/templates/quick-plan-global.md")
        );
        assert_eq!(
            effective.templates.quick_dev_codex_review,
            Path::new("/workspace/project-a/templates/quick-codex-project.md")
        );
        assert_eq!(
            effective.templates.quick_dev_apply_fixes,
            Path::new("/workspace/templates/quick-apply-global.md")
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
            .contains("unknown backend configured as workflow.prompt_review_backend"));
    }

    #[test]
    fn validate_prd_config_rejects_invalid_question_backend_count() {
        let mut global = GlobalConfig::default();
        global.workspace.daemon_prd_question_backends = vec!["claude".to_owned()];

        let error = validate_interactive_prd_workspace_config(&global)
            .expect_err("invalid PRD backend count should fail");

        assert!(error.to_string().contains(
            "workspace.daemon_prd_question_backends must contain exactly 2 backend specs"
        ));
    }

    #[test]
    fn validate_prd_config_rejects_invalid_backend_specs() {
        let mut global = GlobalConfig::default();
        global.workspace.daemon_prd_question_backends =
            vec!["claude(opus)".to_owned(), "codex(gpt-5.4-high)".to_owned()];
        global.workspace.daemon_prd_writer_backend = "unknown(model)".to_owned();

        let error = validate_interactive_prd_workspace_config(&global)
            .expect_err("invalid writer backend should fail");

        assert!(error
            .to_string()
            .contains("unknown backend configured as workspace.daemon_prd_writer_backend"));
    }

    #[test]
    fn validate_prd_config_rejects_unknown_backend_specs() {
        let mut global = GlobalConfig::default();
        global.workspace.daemon_prd_question_backends =
            vec!["claude(opus)".to_owned(), "badbackend(pro)".to_owned()];

        let error = validate_interactive_prd_workspace_config(&global)
            .expect_err("unknown backend should be rejected on daemon PRD surfaces");
        assert!(error
            .to_string()
            .contains("unknown backend configured as workspace.daemon_prd_question_backends[1]"));
        assert!(error
            .to_string()
            .contains("workspace.daemon_prd_question_backends[1]"));
    }

    #[test]
    fn validate_daemon_workspace_config_rejects_unknown_refinement_backend() {
        let mut global = GlobalConfig::default();
        global.workspace.daemon_refinement_backend = "badbackend(pro)".to_owned();

        let error = validate_daemon_workspace_config(&global)
            .expect_err("unknown backend should be rejected on daemon refinement backend");
        assert!(error
            .to_string()
            .contains("unknown backend configured as workspace.daemon_refinement_backend"));
        assert!(error
            .to_string()
            .contains("workspace.daemon_refinement_backend"));
    }

    #[test]
    fn validate_effective_daemon_config_rejects_project_unknown_refinement_backend() {
        let global = GlobalConfig::default();
        let project = ProjectConfig {
            daemon: ProjectDaemonOverrides {
                refinement_backend: Some("badbackend(pro)".to_owned()),
                ..ProjectDaemonOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let daemon = resolve_daemon_config(&global, Some(&project));
        let error = validate_effective_daemon_config(&global, &daemon).expect_err(
            "unknown backend should be rejected on effective daemon refinement backend",
        );
        assert!(error
            .to_string()
            .contains("unknown backend configured as daemon.refinement_backend"));
        assert!(error.to_string().contains("daemon.refinement_backend"));
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
        global.workspace.daemon_rebase_agent_backend = "claude(opus)".to_owned();
        global.workspace.daemon_prd_enabled = true;
        global.workspace.daemon_prd_question_backends =
            vec!["claude(opus)".to_owned(), "codex(gpt-5.4-high)".to_owned()];
        global.workspace.daemon_prd_writer_backend = "claude".to_owned();
        global.workspace.daemon_prd_reviewer_backend = "codex".to_owned();
        global.workspace.daemon_prd_max_revisions = 3;
        global.workspace.daemon_prd_backend_timeout_secs = 120;

        let project = ProjectConfig {
            daemon: ProjectDaemonOverrides {
                poll_seconds: Some(15),
                max_concurrent: Some(3),
                labels: Some(vec!["l1".to_owned(), "l2".to_owned()]),
                repo: Some("acme/project".to_owned()),
                refinement_enabled: Some(false),
                refinement_backend: Some("codex(gpt-5.4-medium)".to_owned()),
                auto_rebase_enabled: Some(false),
                rebase_interval_seconds: Some(900),
                max_rebases_per_cycle: Some(5),
                rebase_timeout_seconds: Some(240),
                rebase_agent_backend: Some("none".to_owned()),
            },
            ..ProjectConfig::default()
        };

        let effective = resolve_daemon_config(&global, Some(&project));
        assert_eq!(effective.poll_seconds, 15);
        assert_eq!(effective.max_concurrent, 3);
        assert_eq!(effective.labels, vec!["l1".to_owned(), "l2".to_owned()]);
        assert_eq!(effective.repo.as_deref(), Some("acme/project"));
        assert!(!effective.refinement_enabled);
        assert_eq!(effective.refinement_backend, "codex(gpt-5.4-medium)");
        assert!(!effective.auto_rebase_enabled);
        assert_eq!(effective.rebase_interval_seconds, 900);
        assert_eq!(effective.max_rebases_per_cycle, 5);
        assert_eq!(effective.rebase_timeout_seconds, 240);
        assert_eq!(effective.rebase_agent_backend, "none");
        assert!(effective.prd_enabled);
        assert_eq!(
            effective.prd_question_backends,
            vec!["claude(opus)".to_owned(), "codex(gpt-5.4-high)".to_owned()]
        );
        assert_eq!(effective.prd_writer_backend, "claude");
        assert_eq!(effective.prd_reviewer_backend, "codex");
        assert_eq!(effective.prd_max_revisions, 3);
        assert_eq!(effective.prd_backend_timeout_secs, 120);
        assert_eq!(effective.prd_shutdown_timeout_secs, 60);

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
        assert_eq!(no_project.rebase_agent_backend, "claude(opus)");
        assert!(no_project.prd_enabled);
        assert_eq!(
            no_project.prd_question_backends,
            vec!["claude(opus)".to_owned(), "codex(gpt-5.4-high)".to_owned()]
        );
        assert_eq!(no_project.prd_writer_backend, "claude");
        assert_eq!(no_project.prd_reviewer_backend, "codex");
        assert_eq!(no_project.prd_max_revisions, 3);
        assert_eq!(no_project.prd_backend_timeout_secs, 120);
        assert_eq!(no_project.prd_shutdown_timeout_secs, 60);
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

    #[test]
    fn resolve_effective_config_final_review_fields_follow_precedence() {
        let mut global = GlobalConfig::default();
        global.workflow.final_review_enabled = false;
        global.workflow.final_review_backends = vec!["claude".to_owned()];
        global.workflow.final_review_arbiter_backend = "codex".to_owned();
        global.workflow.final_review_min_reviewers = 1;
        global.workflow.final_review_consensus_threshold = 0.75;
        global.workflow.max_final_review_restarts = 4;

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                final_review_enabled: Some(true),
                final_review_backends: Some(vec![
                    "codex".to_owned(),
                    "claude(opus)".to_owned(),
                    "codex".to_owned(),
                ]),
                final_review_arbiter_backend: Some("claude".to_owned()),
                final_review_min_reviewers: Some(2),
                final_review_consensus_threshold: Some(1.0),
                max_final_review_restarts: Some(3),
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
        .expect("final review config should resolve");

        assert!(effective.workflow.final_review_enabled);
        assert_eq!(
            effective.workflow.final_review_backends,
            vec![
                "codex".to_owned(),
                "claude(opus)".to_owned(),
                "codex".to_owned()
            ]
        );
        assert_eq!(effective.workflow.final_review_arbiter_backend, "claude");
        assert_eq!(effective.workflow.final_review_min_reviewers, 2);
        assert_eq!(effective.workflow.final_review_consensus_threshold, 1.0);
        assert_eq!(effective.workflow.max_final_review_restarts, 3);
    }

    #[test]
    fn resolve_effective_config_rejects_invalid_final_review_threshold_bounds() {
        for invalid in [0.0, -0.1, 1.1] {
            let mut global = GlobalConfig::default();
            global.workflow.final_review_consensus_threshold = invalid;

            let err = resolve_effective_config(
                Path::new("/workspace"),
                Path::new("/workspace/project"),
                global,
                None,
                RunWorkflowOverrides::default(),
            )
            .expect_err("invalid threshold should fail");
            assert!(
                err.to_string()
                    .contains("final_review_consensus_threshold must be > 0.0 and <= 1.0"),
                "unexpected error for {invalid}: {err}"
            );
        }

        let mut global = GlobalConfig::default();
        global.workflow.final_review_consensus_threshold = 1.0;
        resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("1.0 threshold should be allowed");
    }

    #[test]
    fn resolve_effective_config_rejects_empty_final_review_backends_when_enabled() {
        let mut global = GlobalConfig::default();
        global.workflow.final_review_enabled = true;
        global.workflow.final_review_backends = Vec::new();

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("empty reviewer list should fail");
        assert!(err
            .to_string()
            .contains("final_review_enabled=true requires at least one backend"));
    }

    #[test]
    fn resolve_effective_config_rejects_when_unique_reviewer_count_below_minimum() {
        let mut global = GlobalConfig::default();
        global.workflow.final_review_enabled = true;
        global.workflow.final_review_backends = vec![
            "claude".to_owned(),
            "claude".to_owned(),
            "claude".to_owned(),
        ];
        global.workflow.final_review_min_reviewers = 2;

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("deduplicated reviewer set should fail minimum");
        assert!(err
            .to_string()
            .contains("unique backend specs after deduplication"));
    }

    #[test]
    fn resolve_effective_config_deduplicates_final_review_backends_before_minimum_check() {
        let mut global = GlobalConfig::default();
        global.workflow.final_review_enabled = true;
        global.workflow.final_review_backends = vec![
            "claude".to_owned(),
            "claude".to_owned(),
            "codex".to_owned(),
            "codex".to_owned(),
        ];
        global.workflow.final_review_min_reviewers = 2;

        resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("deduplicated unique reviewer set should pass minimum");
    }

    #[test]
    fn resolve_effective_config_accepts_optional_syntax_in_final_review_list() {
        let mut global = GlobalConfig::default();
        global.workflow.final_review_enabled = true;
        global.workflow.final_review_backends = vec![
            "claude".to_owned(),
            "codex".to_owned(),
            "?openrouter".to_owned(),
        ];
        global.workflow.final_review_min_reviewers = 2;

        resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("optional syntax should be accepted on final_review_backends");
    }

    #[test]
    fn resolve_effective_config_rejects_optional_syntax_on_final_review_arbiter() {
        let mut global = GlobalConfig::default();
        global.workflow.final_review_arbiter_backend = "?claude".to_owned();

        let error = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("optional syntax should be rejected on required arbiter surface");
        assert!(error.to_string().contains(
            "optional backend specs (?backend) are not supported for final review arbiter backend"
        ));
    }

    #[test]
    fn resolve_effective_config_rejects_unknown_final_review_arbiter_family() {
        let mut global = GlobalConfig::default();
        global.workflow.final_review_arbiter_backend = "unknown(model)".to_owned();

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("unknown arbiter backend should fail");
        assert!(err
            .to_string()
            .contains("unknown backend configured as final review arbiter backend"));
    }

    #[test]
    fn final_review_overlap_warning_detection_triggers_for_matching_backend_family() {
        let mut global = GlobalConfig::default();
        global.workflow.final_review_backends =
            vec!["claude(opus)".to_owned(), "codex(gpt-5)".to_owned()];
        global.workflow.final_review_arbiter_backend = "claude(sonnet)".to_owned();

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("overlap should only warn, not fail");

        let validation =
            super::validate_final_review_config(&effective.global, &effective.workflow)
                .expect("validation should succeed");
        assert!(validation.arbiter_overlaps_reviewer_family);
    }

    // --- Prompt review panel config validation tests ---

    #[test]
    fn prompt_review_alias_synthesizes_plural_when_plural_unset() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backend = "claude(opus)".to_owned();

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("alias synthesis should resolve");

        assert_eq!(
            effective.workflow.prompt_review_backends,
            vec!["claude(opus)".to_owned()]
        );
    }

    #[test]
    fn prompt_review_alias_rejects_optional_global_singular_backend() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backend = "?openrouter".to_owned();
        global.workflow.prompt_review_backends = None;

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("optional syntax on singular alias should fail");
        assert!(err.to_string().contains(
            "optional backend specs (?backend) are not supported for workflow.prompt_review_backend"
        ));
    }

    #[test]
    fn prompt_review_alias_rejects_optional_project_singular_backend() {
        let global = GlobalConfig::default();
        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                prompt_review_backend: Some("?claude".to_owned()),
                prompt_review_backends: None,
                ..ProjectWorkflowOverrides::default()
            },
            ..ProjectConfig::default()
        };

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            Some(project),
            RunWorkflowOverrides::default(),
        )
        .expect_err("optional syntax on project singular alias should fail");
        assert!(err.to_string().contains(
            "optional backend specs (?backend) are not supported for workflow.prompt_review_backend"
        ));
    }

    #[test]
    fn prompt_review_alias_explicit_global_plural_wins_even_when_equal_to_default() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backend = "claude(opus)".to_owned();
        global.workflow.prompt_review_backends = Some(vec!["codex(gpt-5.4-xhigh)".to_owned()]);

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("explicit global plural should win over singular alias");

        assert_eq!(
            effective.workflow.prompt_review_backends,
            vec!["codex(gpt-5.4-xhigh)".to_owned()]
        );
    }

    #[test]
    fn prompt_review_project_singular_override_wins_over_global_plural_when_project_plural_absent()
    {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backends = Some(vec![
            "codex(gpt-5.4-xhigh)".to_owned(),
            "claude(opus)".to_owned(),
        ]);

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                prompt_review_backend: Some("claude(sonnet)".to_owned()),
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
        .expect("project singular override should win when project plural is absent");

        assert_eq!(
            effective.workflow.prompt_review_backends,
            vec!["claude(sonnet)".to_owned()]
        );
    }

    #[test]
    fn prompt_review_plural_project_override_takes_precedence_over_singular() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backend = "claude(opus)".to_owned();

        let project = ProjectConfig {
            workflow: ProjectWorkflowOverrides {
                prompt_review_backend: Some("codex(gpt-5.4-xhigh)".to_owned()),
                prompt_review_backends: Some(vec!["claude".to_owned(), "codex".to_owned()]),
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
        .expect("project plural should win");

        assert_eq!(
            effective.workflow.prompt_review_backends,
            vec!["claude".to_owned(), "codex".to_owned()]
        );
    }

    #[test]
    fn prompt_review_panel_rejects_empty_backends() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backends = Some(vec![]);

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("empty prompt_review_backends should fail");
        assert!(err
            .to_string()
            .contains("prompt_review_backends must not be empty"));
    }

    #[test]
    fn prompt_review_panel_rejects_min_reviewers_zero() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_min_reviewers = 0;

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("prompt_review_min_reviewers=0 should fail");
        assert!(err
            .to_string()
            .contains("prompt_review_min_reviewers must be >= 1"));
    }

    #[test]
    fn prompt_review_panel_rejects_duplicate_specs_after_canonicalization() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backends =
            Some(vec!["claude".to_owned(), "?claude".to_owned()]);

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("duplicate resolved prompt review specs should fail");
        assert!(
            err.to_string()
                .contains("duplicate resolved prompt review backend spec"),
            "error should mention duplicate: {err}"
        );
    }

    #[test]
    fn prompt_review_panel_accepts_optional_openrouter_backend() {
        let mut global = GlobalConfig::default();
        global.workflow.prompt_review_backends =
            Some(vec!["claude".to_owned(), "?openrouter".to_owned()]);

        resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("optional openrouter should be accepted on prompt review panel");
    }

    // --- Completion panel config validation tests ---

    #[test]
    fn completion_panel_rejects_empty_backends() {
        let mut global = GlobalConfig::default();
        global.workflow.completion_backends = vec![];

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("empty completion_backends should fail");
        assert!(err
            .to_string()
            .contains("completion_backends must not be empty"));
    }

    #[test]
    fn completion_panel_rejects_min_completers_zero() {
        let mut global = GlobalConfig::default();
        global.workflow.completion_min_completers = 0;

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("min_completers=0 should fail");
        assert!(err
            .to_string()
            .contains("completion_min_completers must be >= 1"));
    }

    #[test]
    fn completion_panel_rejects_threshold_out_of_range() {
        let mut global = GlobalConfig::default();
        global.workflow.completion_consensus_threshold = 0.0;

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("threshold=0.0 should fail");
        assert!(err
            .to_string()
            .contains("completion_consensus_threshold must be > 0.0"));

        let mut global2 = GlobalConfig::default();
        global2.workflow.completion_consensus_threshold = 1.5;

        let err2 = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global2,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("threshold=1.5 should fail");
        assert!(err2
            .to_string()
            .contains("completion_consensus_threshold must be > 0.0"));
    }

    #[test]
    fn completion_panel_rejects_duplicate_specs_after_canonicalization() {
        let mut global = GlobalConfig::default();
        // `claude` and `?claude` collapse to the same resolved target
        global.workflow.completion_backends = vec!["claude".to_owned(), "?claude".to_owned()];

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("duplicate resolved specs should fail");
        assert!(
            err.to_string()
                .contains("duplicate resolved completion backend spec"),
            "error should mention duplicate: {err}"
        );
    }

    #[test]
    fn completion_panel_rejects_role_resolution_collapse() {
        // `claude` and `claude(opus)` collapse to the same resolved target
        // when the completer role model for claude is `opus` (the default).
        // Validation must detect this via role-model resolution.
        let mut global = GlobalConfig::default();
        // Default claude completer model is "opus", so bare `claude` resolves
        // to `claude(opus)` at runtime.
        global.workflow.completion_backends = vec!["claude".to_owned(), "claude(opus)".to_owned()];

        let err = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect_err("role-resolved duplicate specs should fail");
        assert!(
            err.to_string()
                .contains("duplicate resolved completion backend spec"),
            "error should mention duplicate: {err}"
        );
    }

    #[test]
    fn completion_panel_accepts_valid_partial_threshold() {
        let mut global = GlobalConfig::default();
        global.workflow.completion_backends = vec!["claude".to_owned(), "codex".to_owned()];
        global.workflow.completion_min_completers = 1;
        global.workflow.completion_consensus_threshold = 0.5;

        let effective = resolve_effective_config(
            Path::new("/workspace"),
            Path::new("/workspace/project"),
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("valid partial threshold should resolve");
        assert_eq!(effective.workflow.completion_consensus_threshold, 0.5);
        assert_eq!(effective.workflow.completion_min_completers, 1);
    }

    #[test]
    fn completion_verdict_filename_matches_artifact_slug() {
        // Verify that the config validation slug matches the artifact writer slug.
        // Both use slugify_backend which trims leading/trailing dashes.
        use crate::project::artifacts::slugify_backend;

        let filename = super::completion_verdict_filename("claude(opus-v2)");
        let artifact_slug = slugify_backend("claude(opus-v2)");
        assert_eq!(artifact_slug, "claude-opus-v2");
        assert_eq!(
            filename,
            format!("completer-verdict-{artifact_slug}.md"),
            "config collision check must produce same filename as artifact writer"
        );
        assert_eq!(filename, "completer-verdict-claude-opus-v2.md");

        // Also check a simple backend name
        let simple = super::completion_verdict_filename("claude");
        assert_eq!(simple, "completer-verdict-claude.md");
    }
}
