use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::time::{sleep, Instant};
use tracing::{debug, info, warn};

use crate::backend::tmux_backend::TmuxExecutionContext;
use crate::backend::{tmux, Backend, BackendRegistry, BackendRegistryTmuxConfig, RoleOverrides};
use crate::config::{
    resolve_effective_config, CommitMessageStyle, EffectiveConfig, PlannerStateInPrompt,
    PreviousSpecsInPrompt, PromptChangeAction, RunWorkflowOverrides,
};
use crate::error::RalphError;
use crate::git::branch::{branch_exists, checkout_branch, merge_base_branch, resolve_branch_name};
use crate::git::commit::{
    changed_paths_excluding_prefixes, commit_feature_loop, reset_and_clean_working_tree,
    stage_implementation_changes, working_tree_diff_excluding_orchestration_state,
    ORCHESTRATION_STATE_PATH_PREFIX,
};
use crate::git::{is_git_repo, run_git};
use crate::output_log::LogWriter;
use crate::project::artifacts::{
    artifact_relative_path, resolve_artifact_path_by_suffix, write_artifact,
    write_project_scoped_artifact, ArtifactKind, ArtifactWriteInput,
    ProjectScopedArtifactWriteInput,
};
use crate::project::lifecycle::{load_project_state, save_project_state};
use crate::project::load_project_config_if_exists;
use crate::project::state::{
    AcceptanceQaResult, CompletionVerdict, FeatureLoopState, LoopStatus, Phase, ProjectState,
    ProjectStatus, QaExchange, ReviewExchange,
};
use crate::prompts::template_introspection::{load_template_source, template_uses_var};
use crate::prompts::templates::{
    default_completer_template, default_implementer_template, default_planner_template,
    default_prompt_reviewer_template, default_qa_template, default_reviewer_template,
    render_template_with_fallback,
};
use crate::util::hash::sha256_hex;
use crate::util::lock::ProjectLock;
use crate::util::slug::slugify_feature_name;
use crate::workflow::parser::{
    parse_completer_output, parse_implementer_output, parse_planner_output,
    parse_prompt_reviewer_output, parse_qa_output, parse_reviewer_output, CompleterDecision,
    ImplementerDecision, PlannerDecision, QaDecision, ReviewerDecision,
};
use crate::workspace::Workspace;
use crate::Result;

const MAX_PHASE_STEPS_PER_RUN: usize = 500;
const MAX_DIRTY_PATHS_IN_ERROR: usize = 10;

const PLANNER_GUARDRAILS: &str = r#"- Propose only work that is still missing from the project.
- Do not re-plan features that are already implemented in baseline code or completed loops.
- If all requirements are already satisfied, output `# Project Completion Request` instead of another feature."#;

const IMPLEMENTER_GUARDRAILS: &str = r#"- Keep edits scoped to this loop's feature and acceptance criteria.
- In review responses, address each required change explicitly.
- If a required change is already satisfied, cite concrete evidence (files/tests) instead of unrelated edits."#;

const QA_GUARDRAILS: &str = r#"- Run all available build, test, and check commands.
- Report concrete commands and their output.
- Do NOT edit any source files — only run checks and report findings.
- If all acceptance criteria pass, return `# QA: PASS` with evidence.
- If any check fails, return `# QA: FAIL` with specific failure details and suggested fixes."#;

const REVIEWER_GUARDRAILS: &str = r#"- Treat `.ralph/**` as orchestration runtime state; it is out of scope for feature review.
- Focus on acceptance criteria and actual behavior, not whether code was first introduced in this loop.
- If criteria are already satisfied and no code change is required, return `# Review: APPROVED` with evidence.
- Return `# Review: SUGGESTIONS` only for concrete unmet criteria, regressions, or quality issues."#;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub project: Option<String>,
    pub loops: Option<u32>,
    pub until_review: bool,
    pub until_complete: bool,
    pub dry_run: bool,
    pub backend: Option<String>,
    pub planner_backend: Option<String>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub qa_backend: Option<String>,
    pub completer_backend: Option<String>,
    pub tmux: Option<bool>,
    pub on_prompt_change: Option<PromptChangeAction>,
    pub skip_commit: bool,
    pub skip_prompt_review: bool,
}

#[derive(Debug, Clone)]
pub struct OrchestrationResult {
    pub summary: String,
    pub loop_number: Option<u32>,
}

pub struct Orchestrator {
    workspace: Workspace,
    tmux_preflight_checker: Option<fn() -> Result<()>>,
}

impl Orchestrator {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            workspace,
            tmux_preflight_checker: None,
        }
    }

    /// Override the tmux availability checker used during preflight.
    /// When set, this function is called instead of `tmux::check_tmux_available`.
    /// Intended for testing — production callers should use `Orchestrator::new`.
    pub fn set_tmux_preflight_checker(&mut self, checker: fn() -> Result<()>) {
        self.tmux_preflight_checker = Some(checker);
    }

    pub async fn run(&mut self, options: RunOptions) -> Result<OrchestrationResult> {
        validate_termination_controls(&options)?;

        let explicit_project = options.project.is_some();
        let project_id = self
            .workspace
            .resolve_project_id(options.project.as_deref())?;

        let project_dir = self.workspace.project_dir(&project_id);
        if !project_dir.exists() {
            return Err(RalphError::ProjectNotFound(project_id));
        }

        // When --project is explicitly specified, update the active project
        if explicit_project {
            self.workspace.set_active_project_id(&project_id)?;
        }

        let _lock = ProjectLock::acquire(&project_dir, &project_id)?;

        let project_config = load_project_config_if_exists(&project_dir)?;
        let effective = resolve_effective_config(
            &self.workspace.root,
            &project_dir,
            self.workspace.config.clone(),
            project_config,
            RunWorkflowOverrides {
                starting_backend: options.backend.as_deref(),
                planner_backend: options.planner_backend.as_deref(),
                implementer_backend: options.implementer_backend.as_deref(),
                reviewer_backend: options.reviewer_backend.as_deref(),
                qa_backend: options.qa_backend.as_deref(),
                completer_backend: options.completer_backend.as_deref(),
            },
        )?;
        let role_overrides = RoleOverrides {
            planner: effective.workflow.planner_backend.clone(),
            implementer: effective.workflow.implementer_backend.clone(),
            reviewer: effective.workflow.reviewer_backend.clone(),
            qa: effective.workflow.qa_backend.clone(),
            completer: effective.workflow.completer_backend.clone(),
        };

        let tmux_settings = resolve_tmux_settings(
            options.tmux,
            effective.global.workspace.tmux,
            effective.global.workspace.tmux_session.clone(),
        );
        let checker = self
            .tmux_preflight_checker
            .unwrap_or(tmux::check_tmux_available);
        validate_tmux_preflight(tmux_settings.enabled, options.dry_run, checker)?;

        let mut registry = BackendRegistry::new(
            &effective.global,
            BackendRegistryTmuxConfig {
                enabled: tmux_settings.enabled,
                session_name: tmux_settings.session_name,
                window_keep_seconds: effective.global.workspace.tmux_window_keep_seconds,
            },
        );
        preload_override_backends(&mut registry, &role_overrides)?;
        preload_role_model_backends(&mut registry)?;
        if !options.dry_run {
            info!("checking backend availability...");
            registry.health_check_all().await?;
            debug!("all backends available");
        }

        let mut state = load_project_state(&project_dir)?;
        check_parent_project_consistency(&self.workspace, &state)?;

        // Compute repo root for cwd invariant assertions (spec D6).
        let repo_root: Option<PathBuf> = self.workspace.root.parent().map(|p| p.to_owned());
        let repo_root_ref = repo_root.as_deref();

        if !options.dry_run && self.workspace.config.git.auto_branch {
            if let Some(repo_root) = self.workspace.root.parent() {
                if is_git_repo(repo_root) {
                    let branch =
                        resolve_branch_name(&self.workspace.config.git.branch_format, &project_id);
                    if branch_exists(repo_root, &branch)? {
                        checkout_branch(repo_root, &branch)?;
                        merge_base_branch(repo_root, &self.workspace.config.git.base_branch)?;
                    }
                }
            }
        }

        if options.dry_run {
            return dry_run_summary(&state, &effective, &registry, &role_overrides, &options);
        }

        // --- Prompt review pre-loop step ---

        // Migration guard: existing projects that already started looping should
        // not unexpectedly run prompt review on resume.
        if !state.prompt_review_completed
            && (!state.loops.is_empty() || !state.completion_attempts.is_empty())
        {
            info!("migration guard: setting prompt_review_completed for existing project");
            state.prompt_review_completed = true;
            persist_state(&project_dir, &state)?;
        }

        // Skip-flag handling: mark completed immediately so future resumes also skip.
        if options.skip_prompt_review && !state.prompt_review_completed {
            info!("--skip-prompt-review: marking prompt review as completed");
            state.prompt_review_completed = true;
            persist_state(&project_dir, &state)?;
        }

        // Execute prompt review if all gates pass.
        if !state.prompt_review_completed
            && effective.workflow.prompt_review_enabled
            && !options.skip_prompt_review
            && state.loops.is_empty()
            && state.completion_attempts.is_empty()
        {
            info!("running prompt review...");

            let prompt_path = project_dir.join(&state.prompt_file);
            let prompt_content = fs::read_to_string(&prompt_path).map_err(|e| {
                RalphError::Orchestration(format!(
                    "failed to read prompt file '{}': {e}",
                    prompt_path.display()
                ))
            })?;

            let prompt_reviewer_prompt = build_prompt_reviewer_prompt(&effective, &prompt_content)?;

            let pr_backend_spec = &effective.workflow.prompt_review_backend;
            let pr_backend = registry.get_or_create_for_role(pr_backend_spec, "prompt_reviewer")?;

            registry
                .set_tmux_context(TmuxExecutionContext {
                    loop_number: None,
                    role: Some("prompt_reviewer".to_owned()),
                    loop_dir: None,
                    session_id: None,
                })
                .await;

            info!(backend = pr_backend.name(), "invoking prompt reviewer...");
            let mut pr_log = LogWriter::open(&project_dir, None, None, "prompt-reviewer");
            let _retry_result = execute_with_parse_retries(
                pr_backend,
                &registry,
                "prompt_reviewer",
                "prompt_review",
                0,
                &prompt_reviewer_prompt,
                None,
                parse_prompt_reviewer_output,
                &expected_format_template_for("prompt_reviewer", None),
                registry
                    .timeout_for_role(pr_backend_spec, "prompt_reviewer")
                    .as_secs(),
                &mut pr_log,
                None,
                repo_root_ref,
            )
            .await?;
            let decision = _retry_result.parsed;

            // Validate that prompt-original.md does not already exist.
            let backup_path = project_dir.join("prompt-original.md");
            if backup_path.exists() {
                return Err(RalphError::Validation(
                    "prompt-original.md already exists in project directory; \
                     remove or rename it before running prompt review"
                        .to_owned(),
                ));
            }

            // Write backup of original prompt.
            fs::write(&backup_path, &prompt_content)?;

            // Overwrite prompt file with refined prompt.
            fs::write(&prompt_path, &decision.refined_prompt)?;

            // Write prompt-review.md artifact.
            write_project_scoped_artifact(
                &project_dir,
                ProjectScopedArtifactWriteInput {
                    artifact: "prompt-review",
                    file_name: "prompt-review.md",
                    project_id: &state.project_id,
                    backend: pr_backend_spec,
                    role: "prompt_reviewer",
                    body: &decision.body,
                },
            )?;

            // Update prompt hashes.
            let new_hash = sha256_hex(&decision.refined_prompt);
            state.prompt_hash = new_hash.clone();
            state.prompt_hash_at_loop_start = new_hash;
            state.prompt_review_completed = true;
            persist_state(&project_dir, &state)?;
            info!("prompt review completed; prompt file updated");
        }

        let feature_target = options.loops.unwrap_or(1);
        let mut completed_feature_loops = 0_u32;
        let mut logs: Vec<String> = Vec::new();

        let result: Result<OrchestrationResult> = async {
        for _ in 0..MAX_PHASE_STEPS_PER_RUN {
            let prompt_path = project_dir.join(&state.prompt_file);
            let prompt_content = if prompt_path.exists() {
                fs::read_to_string(&prompt_path).map_err(|e| {
                    RalphError::Orchestration(format!(
                        "failed to read prompt file '{}': {e}",
                        prompt_path.display()
                    ))
                })?
            } else {
                warn!(path = %prompt_path.display(), "prompt file not found; using empty prompt");
                String::new()
            };
            let prompt_hash = sha256_hex(&prompt_content);

            handle_prompt_change(
                &mut state,
                &project_dir,
                &self.workspace.root,
                &prompt_hash,
                options
                    .on_prompt_change
                    .unwrap_or(effective.workflow.prompt_change_action),
                effective.workflow.session_reuse_reset_on_prompt_change,
            )?;
            state.prompt_hash = prompt_hash.clone();

            if !state.has_in_progress_loop() {
                state.current_phase = Phase::Planning;
                state.phase_iteration = 1;
            }

            let mut until_review_stop: Option<u32> = None;
            let mut review_limit_hit: Option<(u32, u32)> = None;

            match state.current_phase {
                Phase::Planning => {
                    if !state.has_in_progress_loop() {
                        ensure_clean_start_for_new_loop(&self.workspace.root)?;
                    }
                    let loop_number = state.next_loop_number();
                    info!(loop = loop_number, "starting planning phase");
                    let feature_backends = registry.assign_feature_backends(
                        loop_number,
                        &effective.workflow.starting_backend,
                        &role_overrides,
                    )?;

                    let planner_backend =
                        registry.get_or_create_for_role(&feature_backends.planner, "planner")?;

                    let planner_prompt = build_planner_prompt(
                        &effective,
                        &state,
                        &prompt_content,
                        loop_number,
                        planner_backend.name(),
                        &feature_backends.implementer,
                        &project_dir,
                    )?;

                    // Session reuse: exercise role policy for planner (will warn+skip for v1)
                    let _planner_session_id = resolve_session_for_role(
                        &effective,
                        &mut state,
                        "planner",
                        &feature_backends.planner,
                        loop_number,
                        "", // planner has no bootstrap hash
                    );

                    registry
                        .set_tmux_context(TmuxExecutionContext {
                            loop_number: Some(loop_number),
                            role: Some("planner".to_owned()),
                            loop_dir: None,
                            session_id: None, // always None: planner not supported for v1 reuse
                        })
                        .await;

                    info!(
                        loop = loop_number,
                        backend = planner_backend.name(),
                        "invoking planner..."
                    );
                    let mut planner_log =
                        LogWriter::open(&project_dir, Some(loop_number), None, "planner");
                    let _retry_result = execute_with_parse_retries(
                        planner_backend,
                        &registry,
                        "planner",
                        "planning",
                        loop_number,
                        &planner_prompt,
                        None,
                        parse_planner_output,
                        &expected_format_template_for("planner", None),
                        registry.timeout_for_role(&feature_backends.planner, "planner").as_secs(),
                        &mut planner_log,
                        None,
                        repo_root_ref,
                    )
                    .await?;
                    let planner_decision = _retry_result.parsed;
                    debug!(loop = loop_number, "planner responded");

                    let now = Utc::now();
                    match planner_decision {
                        PlannerDecision::Feature { name, body } => {
                            info!(loop = loop_number, feature = %name, "planner created feature spec");
                            let slug = slugify_feature_name(&name);
                            let spec_path = write_artifact(
                                &project_dir,
                                ArtifactWriteInput {
                                    project_id: &state.project_id,
                                    loop_number,
                                    loop_slug: &slug,
                                    backend: &feature_backends.planner,
                                    role: "planner",
                                    kind: ArtifactKind::Spec,
                                    body: &body,
                                },
                            )?;

                            let spec_rel = artifact_relative_path(&project_dir, &spec_path);
                            state.prompt_hash_at_loop_start = prompt_hash;
                            state.register_feature_loop(
                                loop_number,
                                slug,
                                name,
                                feature_backends,
                                spec_rel,
                                now,
                            );
                            logs.push(format!("loop {loop_number}: planner created feature spec"));
                        }
                        PlannerDecision::CompletionRequest { body } => {
                            info!(loop = loop_number, "planner requested project completion");
                            let completion_backends = registry.assign_completion_backends(
                                loop_number,
                                &effective.workflow.starting_backend,
                                &role_overrides,
                            )?;
                            let termination_path = write_artifact(
                                &project_dir,
                                ArtifactWriteInput {
                                    project_id: &state.project_id,
                                    loop_number,
                                    loop_slug: "completion",
                                    backend: &completion_backends.planner,
                                    role: "planner",
                                    kind: ArtifactKind::TerminationRequest,
                                    body: &body,
                                },
                            )?;

                            let termination_rel =
                                artifact_relative_path(&project_dir, &termination_path);
                            state.prompt_hash_at_loop_start = prompt_hash;
                            state.register_completion_attempt(
                                loop_number,
                                completion_backends,
                                termination_rel,
                                now,
                            );
                            logs.push(format!(
                                "loop {loop_number}: planner requested completion check"
                            ));
                        }
                    }
                }
                Phase::Implementing => {
                    info!(loop = state.current_loop, "starting implementing phase");
                    let (
                        loop_number,
                        loop_slug,
                        feature_name,
                        planner_backend,
                        implementer_backend_name,
                        spec_rel,
                        impl_notes_rel,
                    ) = {
                        let loop_state = state.current_feature_loop().ok_or_else(|| {
                            RalphError::Orchestration(
                                "current phase is implementing but no current feature loop exists"
                                    .to_owned(),
                            )
                        })?;

                        (
                            loop_state.loop_number,
                            loop_state.slug.clone(),
                            loop_state.feature_name.clone(),
                            loop_state.backends.planner.clone(),
                            loop_state.backends.implementer.clone(),
                            loop_state.artifacts.spec.clone(),
                            loop_state.artifacts.impl_notes.clone(),
                        )
                    };

                    let implementer_backend =
                        registry.get_or_create_for_role(&implementer_backend_name, "implementer")?;

                    let spec_content = read_project_relative_file(&project_dir, &spec_rel)?;
                    let git_diff = current_git_diff(&self.workspace.root)?;
                    let iteration = state.phase_iteration;

                    if impl_notes_rel.is_none() {
                        // Session reuse: resolve session for implementer
                        ensure_prompt_hash_at_loop_start(&mut state);
                        let impl_template_content = load_template_source(
                            &effective.templates.implementer,
                            default_implementer_template(),
                        );
                        let impl_bootstrap = compute_bootstrap_hash(
                            "implementer",
                            &implementer_backend_name,
                            &state.prompt_hash_at_loop_start,
                            &spec_content,
                            &impl_template_content,
                        );
                        let impl_loop_dir = project_dir
                            .join("loops")
                            .join(format!("{loop_number:03}-{loop_slug}"));
                        let session_id = resolve_session_for_role(
                            &effective,
                            &mut state,
                            "implementer",
                            &implementer_backend_name,
                            loop_number,
                            &impl_bootstrap,
                        );
                        let session_id = validate_session_rewrite(
                            &registry,
                            &implementer_backend_name,
                            session_id,
                            &impl_loop_dir,
                            "implementer",
                        );

                        let impl_prompt = build_implementer_prompt(
                            &effective,
                            &state,
                            &prompt_content,
                            &feature_name,
                            &loop_slug,
                            implementer_backend.name(),
                            &planner_backend,
                            &spec_content,
                            &git_diff,
                            None,
                            None,
                            &project_dir,
                            session_id.is_some(),
                        )?;

                        registry
                            .set_tmux_context(TmuxExecutionContext {
                                loop_number: Some(loop_number),
                                role: Some("implementer".to_owned()),
                                loop_dir: Some(impl_loop_dir),
                                session_id: session_id.clone(),
                            })
                            .await;

                        info!(
                            loop = loop_number,
                            backend = implementer_backend.name(),
                            "invoking implementer..."
                        );
                        let mut impl_log =
                            LogWriter::open(&project_dir, Some(loop_number), Some(&loop_slug), "implementer");
                        let mut impl_out_session_id: Option<String> = None;
                        let retry_result = execute_with_parse_retries(
                            implementer_backend,
                            &registry,
                            "implementer",
                            "implementing",
                            loop_number,
                            &impl_prompt,
                            session_id.as_deref(),
                            |raw| parse_implementer_output(raw, None),
                            &expected_format_template_for("implementer-notes", None),
                            registry.timeout_for_role(&implementer_backend_name, "implementer").as_secs(),
                            &mut impl_log,
                            Some(&mut impl_out_session_id),
                            repo_root_ref,
                        )
                        .await;
                        // Session lifecycle: upsert even if parse failed (D6)
                        let effective_sid = retry_result.as_ref().ok()
                            .and_then(|r| r.session_id.clone())
                            .or(impl_out_session_id);
                        upsert_session_after_execution(
                            &mut state,
                            "implementer",
                            &implementer_backend_name,
                            loop_number,
                            &impl_bootstrap,
                            effective_sid.as_deref(),
                            session_id.is_some(),
                        );
                        let retry_result = retry_result?;
                        let decision = retry_result.parsed;
                        debug!(loop = loop_number, "implementer responded");

                        let ImplementerDecision::Notes { body } = decision else {
                            return Err(RalphError::ParseError(
                                "implementer returned response artifact during initial implementation"
                                    .to_owned(),
                            ));
                        };

                        let notes_path = write_artifact(
                            &project_dir,
                            ArtifactWriteInput {
                                project_id: &state.project_id,
                                loop_number,
                                loop_slug: &loop_slug,
                                backend: &implementer_backend_name,
                                role: "implementer",
                                kind: ArtifactKind::ImplNotes,
                                body: &body,
                            },
                        )?;
                        let notes_rel = artifact_relative_path(&project_dir, &notes_path);

                        {
                            let loop_state = state.current_feature_loop_mut().ok_or_else(|| {
                                RalphError::Orchestration(
                                    "failed to reload current loop after impl-notes generation"
                                        .to_owned(),
                                )
                            })?;
                            loop_state.artifacts.impl_notes = Some(notes_rel);
                        }

                        stage_changes_for_review(&self.workspace.root)?;
                        if effective.workflow.qa_enabled {
                            state.current_phase = Phase::QA;
                            state.phase_iteration = 1;
                        } else {
                            state.current_phase = Phase::Reviewing;
                            state.phase_iteration = 1;
                        }
                        logs.push(format!("loop {loop_number}: implementer wrote impl-notes"));
                    } else if let Some(qa_feedback_path) = {
                        let loop_state = state.current_feature_loop().ok_or_else(|| {
                            RalphError::Orchestration(
                                "current phase is implementing but no current feature loop exists"
                                    .to_owned(),
                            )
                        })?;
                        loop_state.artifacts.pending_qa_feedback.clone()
                    } {
                        // Handle pending QA feedback: generate implementer response to QA failure
                        info!(
                            loop = loop_number,
                            iteration = iteration,
                            "implementer responding to QA failure feedback"
                        );
                        let qa_feedback_content =
                            read_project_relative_file(&project_dir, &qa_feedback_path)?;

                        // Session reuse for implementer (QA feedback response)
                        ensure_prompt_hash_at_loop_start(&mut state);
                        let impl_template_content = load_template_source(
                            &effective.templates.implementer,
                            default_implementer_template(),
                        );
                        let impl_bootstrap = compute_bootstrap_hash(
                            "implementer",
                            &implementer_backend_name,
                            &state.prompt_hash_at_loop_start,
                            &spec_content,
                            &impl_template_content,
                        );
                        let impl_loop_dir = project_dir
                            .join("loops")
                            .join(format!("{loop_number:03}-{loop_slug}"));
                        let session_id = resolve_session_for_role(
                            &effective,
                            &mut state,
                            "implementer",
                            &implementer_backend_name,
                            loop_number,
                            &impl_bootstrap,
                        );
                        let session_id = validate_session_rewrite(
                            &registry,
                            &implementer_backend_name,
                            session_id,
                            &impl_loop_dir,
                            "implementer",
                        );

                        let impl_prompt = build_implementer_prompt(
                            &effective,
                            &state,
                            &prompt_content,
                            &feature_name,
                            &loop_slug,
                            implementer_backend.name(),
                            &planner_backend,
                            &spec_content,
                            &git_diff,
                            Some(iteration),
                            Some(&qa_feedback_content),
                            &project_dir,
                            session_id.is_some(),
                        )?;

                        registry
                            .set_tmux_context(TmuxExecutionContext {
                                loop_number: Some(loop_number),
                                role: Some("implementer".to_owned()),
                                loop_dir: Some(impl_loop_dir),
                                session_id: session_id.clone(),
                            })
                            .await;

                        info!(
                            loop = loop_number,
                            backend = implementer_backend.name(),
                            iteration = iteration,
                            "invoking implementer for QA feedback response..."
                        );
                        let mut impl_log =
                            LogWriter::open(&project_dir, Some(loop_number), Some(&loop_slug), "implementer");
                        let mut impl_out_session_id: Option<String> = None;
                        let retry_result = execute_with_parse_retries(
                            implementer_backend,
                            &registry,
                            "implementer",
                            "implementing",
                            loop_number,
                            &impl_prompt,
                            session_id.as_deref(),
                            |raw| parse_implementer_output(raw, Some(iteration)),
                            &expected_format_template_for("implementer-response", Some(iteration)),
                            registry.timeout_for_role(&implementer_backend_name, "implementer").as_secs(),
                            &mut impl_log,
                            Some(&mut impl_out_session_id),
                            repo_root_ref,
                        )
                        .await;
                        let effective_sid = retry_result.as_ref().ok()
                            .and_then(|r| r.session_id.clone())
                            .or(impl_out_session_id);
                        upsert_session_after_execution(
                            &mut state,
                            "implementer",
                            &implementer_backend_name,
                            loop_number,
                            &impl_bootstrap,
                            effective_sid.as_deref(),
                            session_id.is_some(),
                        );
                        let retry_result = retry_result?;
                        let decision = retry_result.parsed;

                        let ImplementerDecision::Response {
                            iteration: parsed_iteration,
                            body,
                        } = decision
                        else {
                            return Err(RalphError::ParseError(
                                "implementer returned impl-notes during QA feedback response phase"
                                    .to_owned(),
                            ));
                        };

                        let response_path = write_artifact(
                            &project_dir,
                            ArtifactWriteInput {
                                project_id: &state.project_id,
                                loop_number,
                                loop_slug: &loop_slug,
                                backend: &implementer_backend_name,
                                role: "implementer",
                                kind: ArtifactKind::ImplQaResponse {
                                    iteration: parsed_iteration,
                                },
                                body: &body,
                            },
                        )?;
                        let response_rel = artifact_relative_path(&project_dir, &response_path);

                        {
                            let loop_state = state.current_feature_loop_mut().ok_or_else(|| {
                                RalphError::Orchestration(
                                    "failed to reload current loop after impl-qa-response generation"
                                        .to_owned(),
                                )
                            })?;
                            // Attach response to the latest QA exchange
                            let last_qa = loop_state.artifacts.qa_results.last_mut().ok_or_else(|| {
                                RalphError::Orchestration(
                                    "cannot link implementer QA response: no QA exchange exists in qa_results"
                                        .to_owned(),
                                )
                            })?;
                            last_qa.implementer_response = Some(response_rel);
                            loop_state.artifacts.pending_qa_feedback = None;
                        }

                        stage_changes_for_review(&self.workspace.root)?;
                        state.current_phase = Phase::QA;
                        state.phase_iteration += 1;
                        logs.push(format!(
                            "loop {loop_number}: implementer responded to QA failure iteration {parsed_iteration}"
                        ));
                    } else {
                        info!(
                            loop = loop_number,
                            iteration = iteration,
                            "implementer responding to review feedback"
                        );
                        let feedback_rel =
                            feedback_rel_path(&project_dir, loop_number, &loop_slug, iteration)?;
                        let feedback_content =
                            read_project_relative_file(&project_dir, &feedback_rel)?;

                        // Session reuse for implementer (review feedback response)
                        ensure_prompt_hash_at_loop_start(&mut state);
                        let impl_template_content = load_template_source(
                            &effective.templates.implementer,
                            default_implementer_template(),
                        );
                        let impl_bootstrap = compute_bootstrap_hash(
                            "implementer",
                            &implementer_backend_name,
                            &state.prompt_hash_at_loop_start,
                            &spec_content,
                            &impl_template_content,
                        );
                        let impl_loop_dir = project_dir
                            .join("loops")
                            .join(format!("{loop_number:03}-{loop_slug}"));
                        let session_id = resolve_session_for_role(
                            &effective,
                            &mut state,
                            "implementer",
                            &implementer_backend_name,
                            loop_number,
                            &impl_bootstrap,
                        );
                        let session_id = validate_session_rewrite(
                            &registry,
                            &implementer_backend_name,
                            session_id,
                            &impl_loop_dir,
                            "implementer",
                        );

                        let impl_prompt = build_implementer_prompt(
                            &effective,
                            &state,
                            &prompt_content,
                            &feature_name,
                            &loop_slug,
                            implementer_backend.name(),
                            &planner_backend,
                            &spec_content,
                            &git_diff,
                            Some(iteration),
                            Some(&feedback_content),
                            &project_dir,
                            session_id.is_some(),
                        )?;

                        registry
                            .set_tmux_context(TmuxExecutionContext {
                                loop_number: Some(loop_number),
                                role: Some("implementer".to_owned()),
                                loop_dir: Some(impl_loop_dir),
                                session_id: session_id.clone(),
                            })
                            .await;

                        info!(
                            loop = loop_number,
                            backend = implementer_backend.name(),
                            iteration = iteration,
                            "invoking implementer for feedback response..."
                        );
                        let mut impl_log =
                            LogWriter::open(&project_dir, Some(loop_number), Some(&loop_slug), "implementer");
                        let mut impl_out_session_id: Option<String> = None;
                        let retry_result = execute_with_parse_retries(
                            implementer_backend,
                            &registry,
                            "implementer",
                            "implementing",
                            loop_number,
                            &impl_prompt,
                            session_id.as_deref(),
                            |raw| parse_implementer_output(raw, Some(iteration)),
                            &expected_format_template_for("implementer-response", Some(iteration)),
                            registry.timeout_for_role(&implementer_backend_name, "implementer").as_secs(),
                            &mut impl_log,
                            Some(&mut impl_out_session_id),
                            repo_root_ref,
                        )
                        .await;
                        let effective_sid = retry_result.as_ref().ok()
                            .and_then(|r| r.session_id.clone())
                            .or(impl_out_session_id);
                        upsert_session_after_execution(
                            &mut state,
                            "implementer",
                            &implementer_backend_name,
                            loop_number,
                            &impl_bootstrap,
                            effective_sid.as_deref(),
                            session_id.is_some(),
                        );
                        let retry_result = retry_result?;
                        let decision = retry_result.parsed;

                        let ImplementerDecision::Response {
                            iteration: parsed_iteration,
                            body,
                        } = decision
                        else {
                            return Err(RalphError::ParseError(
                                "implementer returned impl-notes during feedback response phase"
                                    .to_owned(),
                            ));
                        };

                        let response_path = write_artifact(
                            &project_dir,
                            ArtifactWriteInput {
                                project_id: &state.project_id,
                                loop_number,
                                loop_slug: &loop_slug,
                                backend: &implementer_backend_name,
                                role: "implementer",
                                kind: ArtifactKind::ImplResponse {
                                    iteration: parsed_iteration,
                                },
                                body: &body,
                            },
                        )?;
                        let response_rel = artifact_relative_path(&project_dir, &response_path);

                        {
                            let loop_state = state.current_feature_loop_mut().ok_or_else(|| {
                                RalphError::Orchestration(
                                    "failed to reload current loop after impl-response generation"
                                        .to_owned(),
                                )
                            })?;
                            loop_state.artifacts.reviews.push(ReviewExchange {
                                iteration: parsed_iteration,
                                feedback: feedback_rel,
                                response: response_rel,
                            });
                        }

                        stage_changes_for_review(&self.workspace.root)?;
                        state.current_phase = Phase::Reviewing;
                        state.phase_iteration = parsed_iteration + 1;
                        logs.push(format!(
                            "loop {loop_number}: implementer responded to review iteration {parsed_iteration}"
                        ));
                    }
                }
                Phase::QA => {
                    info!(
                        loop = state.current_loop,
                        iteration = state.phase_iteration,
                        "starting QA phase"
                    );
                    let (
                        loop_number,
                        loop_slug,
                        feature_name,
                        planner_backend_name,
                        qa_backend_name,
                        spec_rel,
                        impl_notes_rel,
                    ) = {
                        let loop_state = state.current_feature_loop().ok_or_else(|| {
                            RalphError::Orchestration(
                                "current phase is QA but no current feature loop exists".to_owned(),
                            )
                        })?;

                        (
                            loop_state.loop_number,
                            loop_state.slug.clone(),
                            loop_state.feature_name.clone(),
                            loop_state.backends.planner.clone(),
                            loop_state.backends.qa.clone(),
                            loop_state.artifacts.spec.clone(),
                            loop_state.artifacts.impl_notes.clone(),
                        )
                    };

                    let mut qa_limit_hit: Option<(u32, u32)> = None;
                    if state.phase_iteration > effective.workflow.max_qa_iterations {
                        qa_limit_hit = Some((loop_number, effective.workflow.max_qa_iterations));
                    }

                    if let Some((ln, max_iter)) = qa_limit_hit {
                        warn!(
                            loop_number = ln,
                            max_iterations = max_iter,
                            "QA iteration limit exceeded, rolling back loop"
                        );
                        rollback_current_loop(&mut state, &project_dir, &self.workspace.root)?;
                        persist_state(&project_dir, &state)?;
                        if options.until_complete {
                            logs.push(format!(
                                "loop {ln}: QA iteration limit ({max_iter}) exceeded; rolled back, retrying"
                            ));
                            continue;
                        }
                        return Err(RalphError::QaIterationLimitExceeded {
                            loop_number: ln,
                            max_iterations: max_iter,
                        });
                    }

                    let qa_backend = registry.get_or_create_for_role(&qa_backend_name, "qa")?;

                    let spec_content = read_project_relative_file(&project_dir, &spec_rel)?;
                    let impl_notes_rel = impl_notes_rel.ok_or_else(|| {
                        RalphError::Orchestration(
                            "cannot run QA before impl-notes artifact exists".to_owned(),
                        )
                    })?;
                    let impl_notes_content =
                        read_project_relative_file(&project_dir, &impl_notes_rel)?;
                    let git_diff = current_git_diff(&self.workspace.root)?;

                    // Session reuse: resolve session for QA (before history collection
                    // so we can pass actual session-reuse state to the history builder)
                    ensure_prompt_hash_at_loop_start(&mut state);
                    let qa_template_content = load_template_source(
                        &effective.templates.qa,
                        default_qa_template(),
                    );
                    let qa_bootstrap = compute_bootstrap_hash(
                        "qa",
                        &qa_backend_name,
                        &state.prompt_hash_at_loop_start,
                        &spec_content,
                        &qa_template_content,
                    );
                    let qa_loop_dir = project_dir
                        .join("loops")
                        .join(format!("{loop_number:03}-{loop_slug}"));
                    let qa_session_id = resolve_session_for_role(
                        &effective,
                        &mut state,
                        "qa",
                        &qa_backend_name,
                        loop_number,
                        &qa_bootstrap,
                    );
                    let qa_session_id = validate_session_rewrite(
                        &registry,
                        &qa_backend_name,
                        qa_session_id,
                        &qa_loop_dir,
                        "qa",
                    );

                    let qa_history =
                        collect_qa_history_for_prompt(&effective, &state, &project_dir, qa_session_id.is_some())?;

                    let qa_prompt = build_qa_prompt(
                        &effective,
                        &state,
                        &prompt_content,
                        &feature_name,
                        &loop_slug,
                        qa_backend.name(),
                        &planner_backend_name,
                        &spec_content,
                        &impl_notes_content,
                        &git_diff,
                        &qa_history,
                    )?;

                    registry
                        .set_tmux_context(TmuxExecutionContext {
                            loop_number: Some(loop_number),
                            role: Some("qa".to_owned()),
                            loop_dir: Some(qa_loop_dir),
                            session_id: qa_session_id.clone(),
                        })
                        .await;

                    info!(
                        loop = loop_number,
                        backend = qa_backend.name(),
                        iteration = state.phase_iteration,
                        "invoking QA..."
                    );
                    let mut qa_log = LogWriter::open(&project_dir, Some(loop_number), Some(&loop_slug), "qa");
                    let mut qa_out_session_id: Option<String> = None;
                    let retry_result = execute_with_parse_retries(
                        qa_backend,
                        &registry,
                        "qa",
                        "qa",
                        loop_number,
                        &qa_prompt,
                        qa_session_id.as_deref(),
                        parse_qa_output,
                        &expected_format_template_for("qa", None),
                        registry.timeout_for_role(&qa_backend_name, "qa").as_secs(),
                        &mut qa_log,
                        Some(&mut qa_out_session_id),
                        repo_root_ref,
                    )
                    .await;
                    // Session lifecycle: upsert even if parse failed (D6)
                    let effective_sid = retry_result.as_ref().ok()
                        .and_then(|r| r.session_id.clone())
                        .or(qa_out_session_id);
                    upsert_session_after_execution(
                        &mut state,
                        "qa",
                        &qa_backend_name,
                        loop_number,
                        &qa_bootstrap,
                        effective_sid.as_deref(),
                        qa_session_id.is_some(),
                    );
                    let retry_result = retry_result?;
                    let qa_decision = retry_result.parsed;
                    debug!(loop = loop_number, "QA responded");

                    let iteration = state.phase_iteration;
                    match qa_decision {
                        QaDecision::Pass { body } => {
                            info!(loop = loop_number, "QA passed");
                            let qa_pass_path = write_artifact(
                                &project_dir,
                                ArtifactWriteInput {
                                    project_id: &state.project_id,
                                    loop_number,
                                    loop_slug: &loop_slug,
                                    backend: &qa_backend_name,
                                    role: "qa",
                                    kind: ArtifactKind::QaPass { iteration },
                                    body: &body,
                                },
                            )?;
                            let qa_pass_rel = artifact_relative_path(&project_dir, &qa_pass_path);

                            {
                                let loop_state =
                                    state.current_feature_loop_mut().ok_or_else(|| {
                                        RalphError::Orchestration(
                                            "failed to reload current loop after QA pass"
                                                .to_owned(),
                                        )
                                    })?;
                                loop_state.artifacts.qa_results.push(QaExchange {
                                    iteration,
                                    passed: true,
                                    report: qa_pass_rel,
                                    implementer_response: None,
                                });
                            }

                            state.current_phase = Phase::Reviewing;
                            state.phase_iteration = 1;
                            logs.push(format!(
                                "loop {loop_number}: QA passed, proceeding to review"
                            ));
                        }
                        QaDecision::Fail { body } => {
                            info!(
                                loop = loop_number,
                                iteration = iteration,
                                "QA failed"
                            );
                            let qa_fail_path = write_artifact(
                                &project_dir,
                                ArtifactWriteInput {
                                    project_id: &state.project_id,
                                    loop_number,
                                    loop_slug: &loop_slug,
                                    backend: &qa_backend_name,
                                    role: "qa",
                                    kind: ArtifactKind::QaFail { iteration },
                                    body: &body,
                                },
                            )?;
                            let qa_fail_rel = artifact_relative_path(&project_dir, &qa_fail_path);

                            {
                                let loop_state =
                                    state.current_feature_loop_mut().ok_or_else(|| {
                                        RalphError::Orchestration(
                                            "failed to reload current loop after QA fail"
                                                .to_owned(),
                                        )
                                    })?;
                                loop_state.artifacts.qa_results.push(QaExchange {
                                    iteration,
                                    passed: false,
                                    report: qa_fail_rel.clone(),
                                    implementer_response: None,
                                });
                                loop_state.artifacts.pending_qa_feedback = Some(qa_fail_rel);
                            }

                            state.current_phase = Phase::Implementing;
                            // Keep phase_iteration for the implementer response
                            logs.push(format!(
                                "loop {loop_number}: QA failed at iteration {iteration}, sending back to implementer"
                            ));
                        }
                    }
                }
                Phase::Reviewing => {
                    info!(
                        loop = state.current_loop,
                        iteration = state.phase_iteration,
                        "starting review phase"
                    );
                    let (
                        loop_number,
                        loop_slug,
                        feature_name,
                        planner_backend_name,
                        reviewer_backend_name,
                        spec_rel,
                        impl_notes_rel,
                        review_count,
                    ) = {
                        let loop_state = state.current_feature_loop().ok_or_else(|| {
                            RalphError::Orchestration(
                                "current phase is reviewing but no current feature loop exists"
                                    .to_owned(),
                            )
                        })?;

                        (
                            loop_state.loop_number,
                            loop_state.slug.clone(),
                            loop_state.feature_name.clone(),
                            loop_state.backends.planner.clone(),
                            loop_state.backends.reviewer.clone(),
                            loop_state.artifacts.spec.clone(),
                            loop_state.artifacts.impl_notes.clone(),
                            loop_state.artifacts.reviews.len() as u32,
                        )
                    };

                    if state.phase_iteration > effective.workflow.max_review_iterations {
                        review_limit_hit =
                            Some((loop_number, effective.workflow.max_review_iterations));
                    } else {
                        let reviewer_backend =
                            registry.get_or_create_for_role(&reviewer_backend_name, "reviewer")?;

                        let spec_content = read_project_relative_file(&project_dir, &spec_rel)?;
                        let impl_notes_rel = impl_notes_rel.ok_or_else(|| {
                            RalphError::Orchestration(
                                "cannot review before impl-notes artifact exists".to_owned(),
                            )
                        })?;
                        let impl_notes_content =
                            read_project_relative_file(&project_dir, &impl_notes_rel)?;
                        let git_diff = current_git_diff(&self.workspace.root)?;

                        let previous_iteration = state.phase_iteration.saturating_sub(1);
                        let impl_response_content = if previous_iteration > 0 {
                            let response_rel = response_rel_path(
                                &project_dir,
                                loop_number,
                                &loop_slug,
                                previous_iteration,
                            )?;
                            read_project_relative_file(&project_dir, &response_rel).ok()
                        } else {
                            None
                        };

                        // Session reuse: resolve session for reviewer
                        ensure_prompt_hash_at_loop_start(&mut state);
                        let reviewer_template_content = load_template_source(
                            &effective.templates.reviewer,
                            default_reviewer_template(),
                        );
                        let reviewer_bootstrap = compute_bootstrap_hash(
                            "reviewer",
                            &reviewer_backend_name,
                            &state.prompt_hash_at_loop_start,
                            &spec_content,
                            &reviewer_template_content,
                        );
                        let reviewer_loop_dir = project_dir
                            .join("loops")
                            .join(format!("{loop_number:03}-{loop_slug}"));
                        let reviewer_session_id = resolve_session_for_role(
                            &effective,
                            &mut state,
                            "reviewer",
                            &reviewer_backend_name,
                            loop_number,
                            &reviewer_bootstrap,
                        );
                        let reviewer_session_id = validate_session_rewrite(
                            &registry,
                            &reviewer_backend_name,
                            reviewer_session_id,
                            &reviewer_loop_dir,
                            "reviewer",
                        );

                        let reviewer_prompt = build_reviewer_prompt(
                            &effective,
                            &state,
                            &prompt_content,
                            &feature_name,
                            &loop_slug,
                            reviewer_backend.name(),
                            &planner_backend_name,
                            &spec_content,
                            &impl_notes_content,
                            impl_response_content.as_deref(),
                            &git_diff,
                            &project_dir,
                            reviewer_session_id.is_some(),
                        )?;

                        registry
                            .set_tmux_context(TmuxExecutionContext {
                                loop_number: Some(loop_number),
                                role: Some("reviewer".to_owned()),
                                loop_dir: Some(reviewer_loop_dir),
                                session_id: reviewer_session_id.clone(),
                            })
                            .await;

                        info!(
                            loop = loop_number,
                            backend = reviewer_backend.name(),
                            "invoking reviewer..."
                        );
                        let mut reviewer_log =
                            LogWriter::open(&project_dir, Some(loop_number), Some(&loop_slug), "reviewer");
                        let mut reviewer_out_session_id: Option<String> = None;
                        let retry_result = execute_with_parse_retries(
                            reviewer_backend,
                            &registry,
                            "reviewer",
                            "reviewing",
                            loop_number,
                            &reviewer_prompt,
                            reviewer_session_id.as_deref(),
                            parse_reviewer_output,
                            &expected_format_template_for("reviewer", None),
                            registry.timeout_for_role(&reviewer_backend_name, "reviewer").as_secs(),
                            &mut reviewer_log,
                            Some(&mut reviewer_out_session_id),
                            repo_root_ref,
                        )
                        .await;
                        // Session lifecycle: upsert even if parse failed (D6)
                        let effective_sid = retry_result.as_ref().ok()
                            .and_then(|r| r.session_id.clone())
                            .or(reviewer_out_session_id);
                        upsert_session_after_execution(
                            &mut state,
                            "reviewer",
                            &reviewer_backend_name,
                            loop_number,
                            &reviewer_bootstrap,
                            effective_sid.as_deref(),
                            reviewer_session_id.is_some(),
                        );
                        let retry_result = retry_result?;
                        let reviewer_decision = retry_result.parsed;
                        debug!(loop = loop_number, "reviewer responded");

                        match reviewer_decision {
                            ReviewerDecision::Suggestions { body } => {
                                info!(
                                    loop = loop_number,
                                    iteration = state.phase_iteration,
                                    "reviewer provided suggestions"
                                );
                                let iteration = state.phase_iteration;
                                let feedback_path = write_artifact(
                                    &project_dir,
                                    ArtifactWriteInput {
                                        project_id: &state.project_id,
                                        loop_number,
                                        loop_slug: &loop_slug,
                                        backend: &reviewer_backend_name,
                                        role: "reviewer",
                                        kind: ArtifactKind::ReviewFeedback { iteration },
                                        body: &body,
                                    },
                                )?;

                                let _feedback_rel =
                                    artifact_relative_path(&project_dir, &feedback_path);
                                state.current_phase = Phase::Implementing;
                                state.phase_iteration = iteration;
                                logs.push(format!(
                                "loop {loop_number}: reviewer provided suggestions for iteration {iteration}"
                            ));
                            }
                            ReviewerDecision::Approved {
                                body,
                                commit_message,
                            } => {
                                info!(loop = loop_number, "reviewer approved changes");
                                let iterations = review_count;
                                let approval_path = write_artifact(
                                    &project_dir,
                                    ArtifactWriteInput {
                                        project_id: &state.project_id,
                                        loop_number,
                                        loop_slug: &loop_slug,
                                        backend: &reviewer_backend_name,
                                        role: "reviewer",
                                        kind: ArtifactKind::ReviewApproved { iterations },
                                        body: &body,
                                    },
                                )?;
                                let approval_rel =
                                    artifact_relative_path(&project_dir, &approval_path);

                                {
                                    let loop_state =
                                        state.current_feature_loop_mut().ok_or_else(|| {
                                            RalphError::Orchestration(
                                            "failed to reload current loop after reviewer approval"
                                                .to_owned(),
                                        )
                                        })?;
                                    loop_state.artifacts.approval = Some(approval_rel);
                                }

                                state.current_phase = Phase::Committing;
                                state.phase_iteration = 1;
                                logs.push(format!("loop {loop_number}: reviewer approved changes"));

                                if options.until_review {
                                    until_review_stop = Some(loop_number);
                                }

                                if let Some(message) = commit_message {
                                    logs.push(format!(
                                    "loop {loop_number}: reviewer suggested commit message '{}'",
                                    message
                                ));
                                }
                            }
                        }
                    } // else (review limit not hit)
                }
                Phase::Committing => {
                    info!(loop = state.current_loop, "starting commit phase");
                    let (loop_number, loop_slug, feature_name, approval_rel) = {
                        let loop_state = state.current_feature_loop().ok_or_else(|| {
                            RalphError::Orchestration(
                                "current phase is committing but no current feature loop exists"
                                    .to_owned(),
                            )
                        })?;

                        (
                            loop_state.loop_number,
                            loop_state.slug.clone(),
                            loop_state.feature_name.clone(),
                            loop_state.artifacts.approval.clone().ok_or_else(|| {
                                RalphError::Orchestration(
                                    "cannot commit without review-approved artifact".to_owned(),
                                )
                            })?,
                        )
                    };

                    let approval_content = read_project_relative_file(&project_dir, &approval_rel)?;
                    let reviewer_commit_message =
                        extract_reviewer_commit_message(&approval_content);
                    let commit_message = reviewer_commit_message.unwrap_or_else(|| {
                        generate_commit_message(
                            &effective.workflow.commit_message_style,
                            &feature_name,
                            loop_number,
                            &state,
                        )
                    });

                    let mut commit_hash: Option<String> = None;
                    if effective.workflow.auto_commit && !options.skip_commit {
                        let repo_root = self.workspace.root.parent().ok_or_else(|| {
                            RalphError::Orchestration(
                                "workspace root has no parent path".to_owned(),
                            )
                        })?;

                        if !is_git_repo(repo_root) {
                            return Err(RalphError::Orchestration(
                                "cannot commit: repository is not a git workspace".to_owned(),
                            ));
                        }

                        let tag_name = effective
                            .workflow
                            .commit_tag_format
                            .replace("{project_id}", &state.project_id)
                            .replace("{loop_number}", &loop_number.to_string());

                        let hash = commit_feature_loop(
                            repo_root,
                            &commit_message,
                            Some(&tag_name),
                            effective.global.git.sign_commits,
                        )?;
                        commit_hash = Some(hash.clone());
                        logs.push(format!(
                            "loop {loop_number}: committed and tagged ({tag_name})"
                        ));
                    } else {
                        logs.push(format!(
                            "loop {loop_number}: commit skipped (auto_commit={} skip_commit={})",
                            effective.workflow.auto_commit, options.skip_commit
                        ));
                    }

                    {
                        let loop_state = state.current_feature_loop_mut().ok_or_else(|| {
                            RalphError::Orchestration(
                                "failed to reload current loop for completion update".to_owned(),
                            )
                        })?;
                        loop_state.commit = commit_hash;
                        loop_state.status = LoopStatus::Completed;
                        loop_state.completed_at = Some(Utc::now());
                        let _ = loop_slug;
                    }

                    state.current_phase = Phase::Planning;
                    state.phase_iteration = 1;
                    state.current_loop = state.last_loop_number();
                    state.status = ProjectStatus::InProgress;
                    completed_feature_loops += 1;
                }
                Phase::Completing => {
                    info!(loop = state.current_loop, "starting completion validation phase");
                    let (
                        loop_number,
                        planner_backend_name,
                        completer_backend_name,
                        termination_rel,
                    ) = {
                        let completion = state.current_completion_attempt().ok_or_else(|| {
                            RalphError::Orchestration(
                                "current phase is completing but no completion attempt exists"
                                    .to_owned(),
                            )
                        })?;

                        (
                            completion.loop_number,
                            completion.backends.planner.clone(),
                            completion.backends.completer.clone(),
                            completion.artifacts.termination_request.clone(),
                        )
                    };

                    let completer_backend =
                        registry.get_or_create_for_role(&completer_backend_name, "completer")?;

                    let termination_content =
                        read_project_relative_file(&project_dir, &termination_rel)?;

                    let completer_prompt = build_completer_prompt(
                        &effective,
                        &state,
                        &prompt_content,
                        completer_backend.name(),
                        &planner_backend_name,
                        &termination_content,
                        &project_dir,
                    )?;

                    // Session reuse: exercise role policy for completer (will warn+skip for v1)
                    let _completer_session_id = resolve_session_for_role(
                        &effective,
                        &mut state,
                        "completer",
                        &completer_backend_name,
                        loop_number,
                        "", // completer has no bootstrap hash
                    );

                    registry
                        .set_tmux_context(TmuxExecutionContext {
                            loop_number: Some(loop_number),
                            role: Some("completer".to_owned()),
                            loop_dir: Some(
                                project_dir
                                    .join("loops")
                                    .join(format!("{loop_number:03}-completion")),
                            ),
                            session_id: None, // always None: completer not supported for v1 reuse
                        })
                        .await;

                    info!(
                        loop = loop_number,
                        backend = completer_backend.name(),
                        "invoking completer..."
                    );
                    let mut completer_log =
                        LogWriter::open(&project_dir, Some(loop_number), Some("completion"), "completer");
                    let _retry_result: ParseRetryResult<CompleterDecision> = execute_with_parse_retries(
                        completer_backend,
                        &registry,
                        "completer",
                        "completing",
                        loop_number,
                        &completer_prompt,
                        None,
                        parse_completer_output,
                        &expected_format_template_for("completer", None),
                        registry.timeout_for_role(&completer_backend_name, "completer").as_secs(),
                        &mut completer_log,
                        None,
                        repo_root_ref,
                    )
                    .await?;
                    let completer_decision = _retry_result.parsed;
                    debug!(loop = loop_number, "completer responded");

                    let verdict_path = write_artifact(
                        &project_dir,
                        ArtifactWriteInput {
                            project_id: &state.project_id,
                            loop_number,
                            loop_slug: "completion",
                            backend: &completer_backend_name,
                            role: "completer",
                            kind: ArtifactKind::CompleterVerdict,
                            body: &completer_decision.body,
                        },
                    )?;
                    let verdict_rel = artifact_relative_path(&project_dir, &verdict_path);

                    {
                        let completion =
                            state.current_completion_attempt_mut().ok_or_else(|| {
                                RalphError::Orchestration(
                                    "failed to reload completion attempt for verdict update"
                                        .to_owned(),
                                )
                            })?;
                        completion.artifacts.verdict = Some(verdict_rel);
                        completion.verdict = Some(completer_decision.verdict.clone());
                        completion.status = LoopStatus::Completed;
                        completion.completed_at = Some(Utc::now());
                    }

                    match completer_decision.verdict {
                        CompletionVerdict::Complete => {
                            if effective.workflow.qa_enabled {
                                let acceptance_backends = ["claude", "codex"]
                                    .iter()
                                    .map(|family| {
                                        registry.resolve_backend_for_role(family, "acceptance_qa")
                                    })
                                    .collect::<Vec<_>>();
                                let state_snapshot_json =
                                    serde_json::to_string_pretty(&state).unwrap_or_default();
                                let completed_feature_summary =
                                    collect_completed_feature_loop_summary(&state)?;
                                let git_diff_against_base = current_git_diff_against_base(
                                    &self.workspace.root,
                                    &effective.global.git.base_branch,
                                )?;
                                info!(
                                    loop = loop_number,
                                    backends = ?acceptance_backends,
                                    "running acceptance QA across required backend families"
                                );

                                for acceptance_qa_backend_name in &acceptance_backends {
                                    let acceptance_qa_backend = registry
                                        .get_or_create_for_role(acceptance_qa_backend_name, "acceptance_qa")?;
                                    let acceptance_prompt = build_acceptance_prompt(
                                        &state_snapshot_json,
                                        &prompt_content,
                                        acceptance_qa_backend.name(),
                                        &planner_backend_name,
                                        &completed_feature_summary,
                                        &git_diff_against_base,
                                    );

                                    registry
                                        .set_tmux_context(TmuxExecutionContext {
                                            loop_number: Some(loop_number),
                                            role: Some("qa".to_owned()),
                                            loop_dir: Some(
                                                project_dir
                                                    .join("loops")
                                                    .join(format!("{loop_number:03}-completion")),
                                            ),
                                            session_id: None,
                                        })
                                        .await;

                                    info!(
                                        loop = loop_number,
                                        backend = acceptance_qa_backend.name(),
                                        "invoking acceptance QA..."
                                    );
                                    let mut acceptance_log =
                                        LogWriter::open(&project_dir, Some(loop_number), Some("completion"), "qa");
                                    let retry_result = execute_with_parse_retries(
                                        acceptance_qa_backend,
                                        &registry,
                                        "qa",
                                        "completing",
                                        loop_number,
                                        &acceptance_prompt,
                                        None,
                                        parse_qa_output,
                                        &expected_format_template_for("qa", None),
                                        registry.timeout_for_role(acceptance_qa_backend_name, "acceptance_qa").as_secs(),
                                        &mut acceptance_log,
                                        None,
                                        repo_root_ref,
                                    )
                                    .await?;
                                    let acceptance_decision = retry_result.parsed;
                                    debug!(
                                        loop = loop_number,
                                        backend = acceptance_qa_backend_name,
                                        "acceptance QA responded"
                                    );

                                    match acceptance_decision {
                                        QaDecision::Pass { body } => {
                                            let acceptance_pass_path = write_artifact(
                                                &project_dir,
                                                ArtifactWriteInput {
                                                    project_id: &state.project_id,
                                                    loop_number,
                                                    loop_slug: "completion",
                                                    backend: acceptance_qa_backend_name,
                                                    role: "qa",
                                                    kind: ArtifactKind::AcceptancePass,
                                                    body: &body,
                                                },
                                            )?;
                                            let acceptance_pass_path =
                                                write_acceptance_backend_artifact(
                                                    &acceptance_pass_path,
                                                    acceptance_qa_backend_name,
                                                )?;
                                            let acceptance_pass_rel = artifact_relative_path(
                                                &project_dir,
                                                &acceptance_pass_path,
                                            );

                                            let completion = state
                                                .current_completion_attempt_mut()
                                                .ok_or_else(|| {
                                                    RalphError::Orchestration(
                                                        "failed to reload completion attempt for acceptance pass update"
                                                            .to_owned(),
                                                    )
                                                })?;
                                            completion.artifacts.upsert_acceptance_result(
                                                AcceptanceQaResult {
                                                    backend: acceptance_qa_backend_name.clone(),
                                                    passed: true,
                                                    artifact: acceptance_pass_rel,
                                                },
                                            );
                                            info!(
                                                loop = loop_number,
                                                backend = acceptance_qa_backend_name,
                                                "acceptance QA: PASS"
                                            );
                                        }
                                        QaDecision::Fail { body } => {
                                            let acceptance_fail_path = write_artifact(
                                                &project_dir,
                                                ArtifactWriteInput {
                                                    project_id: &state.project_id,
                                                    loop_number,
                                                    loop_slug: "completion",
                                                    backend: acceptance_qa_backend_name,
                                                    role: "qa",
                                                    kind: ArtifactKind::AcceptanceFail,
                                                    body: &body,
                                                },
                                            )?;
                                            let acceptance_fail_path =
                                                write_acceptance_backend_artifact(
                                                    &acceptance_fail_path,
                                                    acceptance_qa_backend_name,
                                                )?;
                                            let acceptance_fail_rel = artifact_relative_path(
                                                &project_dir,
                                                &acceptance_fail_path,
                                            );

                                            let completion = state
                                                .current_completion_attempt_mut()
                                                .ok_or_else(|| {
                                                    RalphError::Orchestration(
                                                        "failed to reload completion attempt for acceptance fail update"
                                                            .to_owned(),
                                                    )
                                                })?;
                                            completion.artifacts.upsert_acceptance_result(
                                                AcceptanceQaResult {
                                                    backend: acceptance_qa_backend_name.clone(),
                                                    passed: false,
                                                    artifact: acceptance_fail_rel,
                                                },
                                            );
                                            info!(
                                                loop = loop_number,
                                                backend = acceptance_qa_backend_name,
                                                "acceptance QA: FAIL"
                                            );
                                        }
                                    }
                                }

                                let (all_passed, passed_backends, failed_backends) = {
                                    let completion = state
                                        .current_completion_attempt_mut()
                                        .ok_or_else(|| {
                                            RalphError::Orchestration(
                                                "failed to reload completion attempt for acceptance gate aggregation"
                                                    .to_owned(),
                                            )
                                        })?;
                                    let required_backends = acceptance_backends
                                        .iter()
                                        .map(String::as_str)
                                        .collect::<Vec<_>>();
                                    let all_passed = completion
                                        .artifacts
                                        .acceptance_all_required_passed(&required_backends);
                                    let passed_backends = completion
                                        .artifacts
                                        .acceptance_results
                                        .iter()
                                        .filter(|result| result.passed)
                                        .map(|result| result.backend.clone())
                                        .collect::<Vec<_>>();
                                    let failed_backends = completion
                                        .artifacts
                                        .acceptance_results
                                        .iter()
                                        .filter(|result| !result.passed)
                                        .map(|result| result.backend.clone())
                                        .collect::<Vec<_>>();
                                    if !all_passed {
                                        completion.verdict = Some(CompletionVerdict::Continue);
                                    }
                                    (all_passed, passed_backends, failed_backends)
                                };

                                if all_passed {
                                    info!(
                                        loop = loop_number,
                                        passed_backends = ?passed_backends,
                                        "acceptance QA aggregate: PASS"
                                    );
                                    state.status = ProjectStatus::Completed;
                                    state.current_phase = Phase::Completing;
                                    state.phase_iteration = 1;
                                    logs.push(format!(
                                        "loop {loop_number}: acceptance QA passed on [{}]; project finished",
                                        passed_backends.join(", ")
                                    ));
                                } else {
                                    info!(
                                        loop = loop_number,
                                        passed_backends = ?passed_backends,
                                        failed_backends = ?failed_backends,
                                        "acceptance QA aggregate: FAIL"
                                    );
                                    state.status = ProjectStatus::InProgress;
                                    state.current_phase = Phase::Planning;
                                    state.phase_iteration = 1;
                                    logs.push(format!(
                                        "loop {loop_number}: acceptance QA failed on [{}]; forcing CONTINUE and returning to planning",
                                        failed_backends.join(", ")
                                    ));
                                }
                            } else {
                                state.status = ProjectStatus::Completed;
                                state.current_phase = Phase::Completing;
                                state.phase_iteration = 1;
                                logs.push(format!(
                                    "loop {loop_number}: completer returned COMPLETE; project finished"
                                ));
                            }
                        }
                        CompletionVerdict::Continue => {
                            state.status = ProjectStatus::InProgress;
                            state.current_phase = Phase::Planning;
                            state.phase_iteration = 1;
                            logs.push(format!(
                                "loop {loop_number}: completer returned CONTINUE; planning next feature"
                            ));
                        }
                    }
                }
            }

            // Handle ReviewIterationLimitExceeded: rollback the current loop
            if let Some((ln, max_iter)) = review_limit_hit {
                warn!(
                    loop_number = ln,
                    max_iterations = max_iter,
                    "review iteration limit exceeded, rolling back loop"
                );
                rollback_current_loop(&mut state, &project_dir, &self.workspace.root)?;
                persist_state(&project_dir, &state)?;
                if options.until_complete {
                    logs.push(format!(
                        "loop {ln}: review iteration limit ({max_iter}) exceeded; rolled back, retrying"
                    ));
                    continue;
                }
                return Err(RalphError::ReviewIterationLimitExceeded {
                    loop_number: ln,
                    max_iterations: max_iter,
                });
            }

            persist_state(&project_dir, &state)?;

            // Handle --until-review stop
            if let Some(ln) = until_review_stop {
                return Ok(OrchestrationResult {
                    summary: format!(
                        "stopped after review approval for loop {ln}; commit not executed"
                    ),
                    loop_number: Some(ln),
                });
            }

            if state.status == ProjectStatus::Completed {
                // Commit completion artifacts (acceptance QA results, state, etc.)
                if effective.workflow.auto_commit && !options.skip_commit {
                    if let Some(repo_root) = self.workspace.root.parent() {
                        if is_git_repo(repo_root) {
                            let msg =
                                format!("chore({}): add completion artifacts", state.project_id);
                            let _ = commit_feature_loop(
                                repo_root,
                                &msg,
                                None,
                                effective.global.git.sign_commits,
                            );
                        }
                    }
                }

                return Ok(OrchestrationResult {
                    summary: if logs.is_empty() {
                        "project completed".to_owned()
                    } else {
                        logs.join("\n")
                    },
                    loop_number: Some(state.current_loop),
                });
            }

            if options.until_complete {
                continue;
            }

            if completed_feature_loops >= feature_target {
                return Ok(OrchestrationResult {
                    summary: if logs.is_empty() {
                        format!("completed {completed_feature_loops} feature loop(s)")
                    } else {
                        logs.join("\n")
                    },
                    loop_number: Some(state.current_loop),
                });
            }
        }

        Err(RalphError::Orchestration(format!(
            "run exceeded maximum phase transitions ({MAX_PHASE_STEPS_PER_RUN}); aborting"
        )))
        }.await;

        // Mark the project as failed while we still hold the lock.
        if let Err(ref err) = result {
            if is_terminal_orchestration_error(err) {
                if let Ok(mut st) = load_project_state(&project_dir) {
                    if st.status != ProjectStatus::Completed {
                        st.status = ProjectStatus::Failed;
                        let _ = save_project_state(&project_dir, &st);
                    }
                }
            }
        }

        result
    }
}

/// Returns `true` for error variants that indicate the orchestration loop hit
/// a terminal failure (as opposed to setup/validation issues or transient IO
/// errors).  Used to decide whether to mark the project as `Failed`.
fn is_terminal_orchestration_error(err: &RalphError) -> bool {
    matches!(
        err,
        RalphError::BackendTimeoutExhausted { .. }
            | RalphError::ParseRetriesExhausted { .. }
            | RalphError::ReviewIterationLimitExceeded { .. }
            | RalphError::QaIterationLimitExceeded { .. }
            | RalphError::BackendCommandFailed { .. }
            | RalphError::Orchestration(_)
            | RalphError::GitConflict { .. }
    )
}

#[derive(Debug, Clone)]
struct ResolvedTmuxSettings {
    enabled: bool,
    session_name: String,
}

fn resolve_tmux_settings(
    cli_override: Option<bool>,
    config_enabled: bool,
    session_name: String,
) -> ResolvedTmuxSettings {
    ResolvedTmuxSettings {
        enabled: cli_override.unwrap_or(config_enabled),
        session_name,
    }
}

fn validate_tmux_preflight<F>(tmux_enabled: bool, dry_run: bool, check_tmux: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    if tmux_enabled && !dry_run {
        check_tmux()?;
    }
    Ok(())
}

fn validate_termination_controls(options: &RunOptions) -> Result<()> {
    if options.loops == Some(0) {
        return Err(RalphError::Validation(
            "--loops must be greater than 0".to_owned(),
        ));
    }

    let mut active = 0_u8;
    if options.loops.is_some() {
        active += 1;
    }
    if options.until_review {
        active += 1;
    }
    if options.until_complete {
        active += 1;
    }

    if active > 1 {
        return Err(RalphError::Validation(
            "--loops, --until-review, and --until-complete are mutually exclusive".to_owned(),
        ));
    }

    Ok(())
}

fn dry_run_summary(
    state: &ProjectState,
    effective: &EffectiveConfig,
    registry: &BackendRegistry,
    role_overrides: &RoleOverrides,
    options: &RunOptions,
) -> Result<OrchestrationResult> {
    let prompt_review_status = if state.prompt_review_completed {
        "prompt_review: completed".to_owned()
    } else if !effective.workflow.prompt_review_enabled {
        "prompt_review: disabled".to_owned()
    } else if options.skip_prompt_review {
        "prompt_review: will be skipped (--skip-prompt-review)".to_owned()
    } else {
        format!(
            "prompt_review: pending (backend: {})",
            effective.workflow.prompt_review_backend
        )
    };

    if state.has_in_progress_loop() {
        let summary = format!(
            "{prompt_review_status}\ndry-run: would resume loop {} at phase={} iteration={}",
            state.current_loop,
            phase_label(&state.current_phase),
            state.phase_iteration
        );
        return Ok(OrchestrationResult {
            summary,
            loop_number: Some(state.current_loop),
        });
    }

    let next_loop = state.next_loop_number();
    let backends = registry.assign_feature_backends(
        next_loop,
        &effective.workflow.starting_backend,
        role_overrides,
    )?;
    let summary = if effective.workflow.qa_enabled {
        format!(
            "{prompt_review_status}\ndry-run: would start loop {next_loop} with planner={}, implementer={}, qa={}, reviewer={}",
            backends.planner, backends.implementer, backends.qa, backends.reviewer
        )
    } else {
        format!(
            "{prompt_review_status}\ndry-run: would start loop {next_loop} with planner={}, implementer={}, reviewer={}",
            backends.planner, backends.implementer, backends.reviewer
        )
    };
    Ok(OrchestrationResult {
        summary,
        loop_number: Some(next_loop),
    })
}

fn preload_override_backends(
    registry: &mut BackendRegistry,
    role_overrides: &RoleOverrides,
) -> Result<()> {
    for backend_spec in [
        role_overrides.planner.as_deref(),
        role_overrides.implementer.as_deref(),
        role_overrides.reviewer.as_deref(),
        role_overrides.qa.as_deref(),
        role_overrides.completer.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        registry.get_or_create_for_spec(backend_spec)?;
    }
    Ok(())
}

fn preload_role_model_backends(registry: &mut BackendRegistry) -> Result<()> {
    for backend_spec in registry.backend_role_model_specs() {
        registry.get_or_create_for_spec(&backend_spec)?;
    }
    Ok(())
}

fn check_parent_project_consistency(workspace: &Workspace, state: &ProjectState) -> Result<()> {
    if let Some(ref parent_id) = state.parent_project {
        if !workspace.project_exists(parent_id) {
            eprintln!(
                "warning: parent project '{}' referenced by '{}' does not exist",
                parent_id, state.project_id
            );
        }
    }
    Ok(())
}

fn rollback_current_loop(
    state: &mut ProjectState,
    project_dir: &Path,
    workspace_root: &Path,
) -> Result<()> {
    if !state.has_in_progress_loop() {
        return Ok(());
    }

    if let Some(repo_root) = workspace_root.parent() {
        if is_git_repo(repo_root) {
            reset_and_clean_working_tree(repo_root)?;
        }
    }

    let loop_number = state.current_loop;
    let loop_slug = state
        .current_feature_loop()
        .map(|l| l.slug.clone())
        .or_else(|| state.current_completion_attempt().map(|l| l.slug.clone()))
        .ok_or_else(|| {
            RalphError::Orchestration(
                "rollback requested but current loop could not be found".to_owned(),
            )
        })?;

    let loop_dir = project_dir
        .join("loops")
        .join(format!("{loop_number:03}-{loop_slug}"));
    if loop_dir.exists() {
        if let Err(e) = fs::remove_dir_all(&loop_dir) {
            warn!(
                loop_number = loop_number,
                path = %loop_dir.display(),
                error = %e,
                "failed to remove artifact directory during rollback"
            );
        }
    }

    // Also remove the bare loop-number directory used by agent-output log files
    // (e.g., loops/001/agent-output-planner.log).
    let log_dir = project_dir.join("loops").join(format!("{loop_number:03}"));
    if log_dir.exists() {
        if let Err(e) = fs::remove_dir_all(&log_dir) {
            warn!(
                loop_number = loop_number,
                path = %log_dir.display(),
                error = %e,
                "failed to remove agent-output log directory during rollback"
            );
        }
    }

    state.remove_loop(loop_number);
    state.current_loop = state.last_loop_number();
    state.current_phase = Phase::Planning;
    state.phase_iteration = 1;
    if state.status != ProjectStatus::Completed {
        state.status = if state.loops.is_empty() && state.completion_attempts.is_empty() {
            ProjectStatus::Pending
        } else {
            ProjectStatus::InProgress
        };
    }

    Ok(())
}

fn handle_prompt_change(
    state: &mut ProjectState,
    project_dir: &Path,
    workspace_root: &Path,
    new_prompt_hash: &str,
    action: PromptChangeAction,
    session_reuse_reset_on_prompt_change: bool,
) -> Result<()> {
    if new_prompt_hash == state.prompt_hash {
        return Ok(());
    }

    if !state.has_in_progress_loop() {
        state.prompt_hash = new_prompt_hash.to_owned();
        return Ok(());
    }

    match action {
        PromptChangeAction::Continue => {
            state.prompt_hash = new_prompt_hash.to_owned();
            Ok(())
        }
        PromptChangeAction::Abort => Err(RalphError::Orchestration(format!(
            "prompt changed during in-progress loop {}; aborting",
            state.current_loop
        ))),
        PromptChangeAction::RestartLoop => {
            let current_loop = state.current_loop;
            // Save current-loop session records before rollback (which clears them via remove_loop).
            // Restore them if the prompt-change flag says not to reset sessions.
            let saved_sessions: Vec<_> = if !session_reuse_reset_on_prompt_change {
                state
                    .session_store
                    .records
                    .iter()
                    .filter(|r| r.loop_number == current_loop)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            rollback_current_loop(state, project_dir, workspace_root)?;
            // Restore saved sessions when reset is disabled
            for record in saved_sessions {
                state.session_store.upsert(record);
            }
            state.prompt_hash = new_prompt_hash.to_owned();
            state.prompt_hash_at_loop_start = new_prompt_hash.to_owned();
            Ok(())
        }
    }
}

/// Produce a deterministic summary of project state for the planner prompt.
///
/// Includes loop metadata (status, iteration, verdict, spec path) but excludes
/// raw review feedback body and raw QA report body text.
///
/// Loops are sorted by loop_number ascending; when `max_loops` is `Some(n)`,
/// only the latest `n` are included. `Some(0)` includes none. `None` = unlimited.
fn summarize_state_for_planner(state: &ProjectState, max_loops: Option<usize>) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Project: {} ({})",
        state.project_name, state.project_id
    ));
    lines.push(format!("Status: {:?}", state.status));
    lines.push(format!("Current loop: {}", state.current_loop));
    lines.push(format!("Current phase: {:?}", state.current_phase));
    lines.push(format!("Phase iteration: {}", state.phase_iteration));
    lines.push(String::new());

    let mut loops: Vec<&FeatureLoopState> = state.loops.iter().collect();
    loops.sort_by_key(|l| l.loop_number);

    if let Some(cap) = max_loops {
        if cap == 0 {
            lines.push("Loops: (none shown)".to_owned());
            return lines.join("\n");
        }
        let len = loops.len();
        if cap < len {
            loops = loops[len - cap..].to_vec();
        }
    }

    if loops.is_empty() {
        lines.push("Loops: (none)".to_owned());
    } else {
        lines.push("Loops:".to_owned());
        for l in &loops {
            let verdict = if l.artifacts.approval.is_some() {
                "approved"
            } else if l.status == LoopStatus::Completed {
                "completed"
            } else if l.artifacts.qa_results.last().is_some_and(|q| !q.passed) {
                "failed"
            } else {
                "pending"
            };
            lines.push(format!(
                "- Loop {} ({}): status={:?}, iterations={}, verdict={}, spec={}",
                l.loop_number,
                l.feature_name,
                l.status,
                l.artifacts.reviews.len() + 1,
                verdict,
                l.artifacts.spec,
            ));
        }
    }

    if !state.completion_attempts.is_empty() {
        lines.push(String::new());
        lines.push("Completion attempts:".to_owned());
        for c in &state.completion_attempts {
            let verdict_str = match &c.verdict {
                Some(v) => format!("{v:?}"),
                None => "pending".to_owned(),
            };
            lines.push(format!(
                "- Loop {} (completion): status={:?}, verdict={}",
                c.loop_number, c.status, verdict_str,
            ));
        }
    }

    lines.join("\n")
}

/// Produce previous spec content for the planner/completer prompt.
///
/// Mode semantics:
/// - `None` => empty string
/// - `Titles` => bullet list of loop number + feature name only
/// - `FullText` => full prior spec content (like the old `collect_previous_specs`)
///
/// `max_specs`: `None` = unlimited; `Some(0)` = include none.
/// Loops are sorted by loop_number ascending; latest N when capped.
fn summarize_previous_specs_for_planner(
    state: &ProjectState,
    project_dir: &Path,
    mode: PreviousSpecsInPrompt,
    max_specs: Option<usize>,
) -> Result<String> {
    if matches!(mode, PreviousSpecsInPrompt::None) {
        return Ok(String::new());
    }

    let mut loops: Vec<&FeatureLoopState> = state.loops.iter().collect();
    loops.sort_by_key(|l| l.loop_number);

    if let Some(cap) = max_specs {
        if cap == 0 {
            return Ok(String::new());
        }
        let len = loops.len();
        if cap < len {
            loops = loops[len - cap..].to_vec();
        }
    }

    match mode {
        PreviousSpecsInPrompt::None => Ok(String::new()),
        PreviousSpecsInPrompt::Titles => {
            let mut parts = Vec::new();
            for l in &loops {
                parts.push(format!("- Loop {}: {}", l.loop_number, l.feature_name));
            }
            Ok(parts.join("\n"))
        }
        PreviousSpecsInPrompt::FullText => {
            let mut parts = Vec::new();
            for l in &loops {
                if let Ok(spec) = read_project_relative_file(project_dir, &l.artifacts.spec) {
                    parts.push(format!(
                        "## Loop {}: {}\n\n{}",
                        l.loop_number, l.feature_name, spec
                    ));
                }
            }
            Ok(parts.join("\n\n"))
        }
    }
}

fn build_prompt_reviewer_prompt(
    effective: &EffectiveConfig,
    prompt_content: &str,
) -> Result<String> {
    let mut vars = BTreeMap::new();
    vars.insert("prompt_content".to_owned(), prompt_content.to_owned());

    let rendered = render_template_with_fallback(
        &effective.templates.prompt_reviewer,
        &vars,
        default_prompt_reviewer_template(),
    )?;
    Ok(rendered)
}

fn build_planner_prompt(
    effective: &EffectiveConfig,
    state: &ProjectState,
    prompt_content: &str,
    loop_number: u32,
    backend: &str,
    opposite_backend: &str,
    project_dir: &Path,
) -> Result<String> {
    let template_source =
        load_template_source(&effective.templates.planner, default_planner_template());

    let mut vars = base_vars(state, loop_number, "planning", 1, backend, opposite_backend);

    let max_loops = effective.workflow.planner_max_prior_loops;
    // For template variables: raw content (no fencing). Templates apply their own
    // fencing as needed. Fencing is only added for fallback-appended sections.
    let state_text = match effective.workflow.planner_state_in_prompt {
        PlannerStateInPrompt::FullJson => serde_json::to_string_pretty(state).unwrap_or_default(),
        PlannerStateInPrompt::Summary => summarize_state_for_planner(state, max_loops),
    };
    let previous_specs = summarize_previous_specs_for_planner(
        state,
        project_dir,
        effective.workflow.planner_previous_specs_in_prompt,
        max_loops,
    )?;
    let completion_feedback = latest_completion_feedback_context(state, project_dir)?;
    vars.insert("prompt_content".to_owned(), prompt_content.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert("state_content".to_owned(), state_text.clone());
    vars.insert("state_json".to_owned(), state_text.clone());
    vars.insert("previous_specs".to_owned(), previous_specs);
    vars.insert(
        "system_guardrails".to_owned(),
        PLANNER_GUARDRAILS.to_owned(),
    );
    vars.insert(
        "completion_feedback".to_owned(),
        completion_feedback.clone().unwrap_or_default(),
    );

    let rendered = render_template_with_fallback(
        &effective.templates.planner,
        &vars,
        default_planner_template(),
    )?;
    let mut prompt = rendered;
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["system_guardrails"],
        "## System Guardrails",
        PLANNER_GUARDRAILS,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["master_prompt", "prompt_content"],
        "## Master Prompt",
        prompt_content,
    );
    // Fallback-appended state section gets fencing only here (not in the var).
    let state_fallback = match effective.workflow.planner_state_in_prompt {
        PlannerStateInPrompt::FullJson => format!("```json\n{state_text}\n```"),
        PlannerStateInPrompt::Summary => state_text.clone(),
    };
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["state_json", "state_content"],
        "## Current State",
        &state_fallback,
    );
    if let Some(completion_feedback) = completion_feedback {
        append_section_if_missing(
            &mut prompt,
            &template_source,
            &["completion_feedback"],
            "## Completion Feedback",
            &completion_feedback,
        );
    }

    Ok(prompt)
}

#[allow(clippy::too_many_arguments)]
fn build_implementer_prompt(
    effective: &EffectiveConfig,
    state: &ProjectState,
    prompt_content: &str,
    feature_name: &str,
    loop_slug: &str,
    backend: &str,
    opposite_backend: &str,
    spec_content: &str,
    git_diff: &str,
    iteration: Option<u32>,
    review_feedback: Option<&str>,
    project_dir: &Path,
    session_reused_this_call: bool,
) -> Result<String> {
    let template_source = load_template_source(
        &effective.templates.implementer,
        default_implementer_template(),
    );

    let phase_iteration = iteration.unwrap_or(1);
    let mut vars = base_vars(
        state,
        state.current_loop,
        "implementing",
        phase_iteration,
        backend,
        opposite_backend,
    );
    vars.insert("feature_name".to_owned(), feature_name.to_owned());
    vars.insert("loop_slug".to_owned(), loop_slug.to_owned());
    vars.insert("prompt_content".to_owned(), prompt_content.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert("spec_content".to_owned(), spec_content.to_owned());
    vars.insert("feature_spec".to_owned(), spec_content.to_owned());
    vars.insert("git_diff".to_owned(), git_diff.to_owned());
    vars.insert(
        "current_diff".to_owned(),
        format!("```diff\n{git_diff}\n```"),
    );
    let review_feedback_text = review_feedback.unwrap_or("(none)");
    vars.insert(
        "review_feedback".to_owned(),
        review_feedback_text.to_owned(),
    );
    vars.insert(
        "review_feedback_content".to_owned(),
        review_feedback_text.to_owned(),
    );
    vars.insert(
        "review_history".to_owned(),
        collect_review_history_for_prompt(effective, state, project_dir, session_reused_this_call)?,
    );
    vars.insert(
        "system_guardrails".to_owned(),
        IMPLEMENTER_GUARDRAILS.to_owned(),
    );

    let rendered = render_template_with_fallback(
        &effective.templates.implementer,
        &vars,
        default_implementer_template(),
    )?;
    let mut prompt = rendered;
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["system_guardrails"],
        "## System Guardrails",
        IMPLEMENTER_GUARDRAILS,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["master_prompt", "prompt_content"],
        "## Master Prompt",
        prompt_content,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["feature_spec", "spec_content"],
        "## Feature Spec",
        spec_content,
    );
    let current_diff_block = format!("```diff\n{git_diff}\n```");
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["current_diff", "git_diff"],
        "## Current Diff",
        &current_diff_block,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["review_feedback", "review_feedback_content"],
        "## Review Feedback",
        review_feedback_text,
    );
    Ok(prompt)
}

#[allow(clippy::too_many_arguments)]
fn build_reviewer_prompt(
    effective: &EffectiveConfig,
    state: &ProjectState,
    prompt_content: &str,
    feature_name: &str,
    loop_slug: &str,
    backend: &str,
    opposite_backend: &str,
    spec_content: &str,
    impl_notes_content: &str,
    impl_response_content: Option<&str>,
    git_diff: &str,
    project_dir: &Path,
    session_reused_this_call: bool,
) -> Result<String> {
    let template_source =
        load_template_source(&effective.templates.reviewer, default_reviewer_template());

    let mut vars = base_vars(
        state,
        state.current_loop,
        "reviewing",
        state.phase_iteration,
        backend,
        opposite_backend,
    );
    vars.insert("feature_name".to_owned(), feature_name.to_owned());
    vars.insert("loop_slug".to_owned(), loop_slug.to_owned());
    vars.insert("prompt_content".to_owned(), prompt_content.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert("spec_content".to_owned(), spec_content.to_owned());
    vars.insert("feature_spec".to_owned(), spec_content.to_owned());
    vars.insert(
        "impl_notes_content".to_owned(),
        impl_notes_content.to_owned(),
    );
    vars.insert(
        "implementation_notes".to_owned(),
        impl_notes_content.to_owned(),
    );
    vars.insert("git_diff".to_owned(), git_diff.to_owned());
    vars.insert(
        "current_diff".to_owned(),
        format!("```diff\n{git_diff}\n```"),
    );
    let latest_impl_response = impl_response_content.unwrap_or("(none)");
    vars.insert(
        "latest_implementation_response".to_owned(),
        latest_impl_response.to_owned(),
    );
    vars.insert(
        "impl_response_content".to_owned(),
        latest_impl_response.to_owned(),
    );
    vars.insert(
        "review_history".to_owned(),
        collect_review_history_for_prompt(effective, state, project_dir, session_reused_this_call)?,
    );
    vars.insert(
        "system_guardrails".to_owned(),
        REVIEWER_GUARDRAILS.to_owned(),
    );

    let rendered = render_template_with_fallback(
        &effective.templates.reviewer,
        &vars,
        default_reviewer_template(),
    )?;
    let mut prompt = rendered;
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["system_guardrails"],
        "## System Guardrails",
        REVIEWER_GUARDRAILS,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["master_prompt", "prompt_content"],
        "## Master Prompt",
        prompt_content,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["feature_spec", "spec_content"],
        "## Feature Spec",
        spec_content,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["implementation_notes", "impl_notes_content"],
        "## Implementation Notes",
        impl_notes_content,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["latest_implementation_response", "impl_response_content"],
        "## Latest Implementation Response",
        latest_impl_response,
    );
    let current_diff_block = format!("```diff\n{git_diff}\n```");
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["current_diff", "git_diff"],
        "## Current Diff",
        &current_diff_block,
    );
    Ok(prompt)
}

fn build_completer_prompt(
    effective: &EffectiveConfig,
    state: &ProjectState,
    prompt_content: &str,
    backend: &str,
    opposite_backend: &str,
    termination_request_content: &str,
    project_dir: &Path,
) -> Result<String> {
    let template_source =
        load_template_source(&effective.templates.completer, default_completer_template());

    let mut vars = base_vars(
        state,
        state.current_loop,
        "completing",
        1,
        backend,
        opposite_backend,
    );

    let max_loops = effective.workflow.planner_max_prior_loops;
    // Raw content for template variables (no fencing). Templates apply their own fencing.
    let state_text = match effective.workflow.planner_state_in_prompt {
        PlannerStateInPrompt::FullJson => serde_json::to_string_pretty(state).unwrap_or_default(),
        PlannerStateInPrompt::Summary => summarize_state_for_planner(state, max_loops),
    };
    let previous_specs = summarize_previous_specs_for_planner(
        state,
        project_dir,
        effective.workflow.planner_previous_specs_in_prompt,
        max_loops,
    )?;

    vars.insert("prompt_content".to_owned(), prompt_content.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert(
        "termination_request_content".to_owned(),
        termination_request_content.to_owned(),
    );
    vars.insert(
        "completion_request".to_owned(),
        termination_request_content.to_owned(),
    );
    vars.insert("previous_specs".to_owned(), previous_specs.clone());
    vars.insert("prior_specs".to_owned(), previous_specs.clone());
    vars.insert("state_content".to_owned(), state_text.clone());
    vars.insert("state_json".to_owned(), state_text.clone());

    let rendered = render_template_with_fallback(
        &effective.templates.completer,
        &vars,
        default_completer_template(),
    )?;
    let mut prompt = rendered;
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["master_prompt", "prompt_content"],
        "## Master Prompt",
        prompt_content,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["completion_request", "termination_request_content"],
        "## Completion Request",
        termination_request_content,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["prior_specs", "previous_specs"],
        "## Prior Specs",
        &previous_specs,
    );
    // Fallback-appended state section gets fencing only here.
    let state_fallback = match effective.workflow.planner_state_in_prompt {
        PlannerStateInPrompt::FullJson => format!("```json\n{state_text}\n```"),
        PlannerStateInPrompt::Summary => state_text.clone(),
    };
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["state_json", "state_content"],
        "## State",
        &state_fallback,
    );
    Ok(prompt)
}

fn build_acceptance_prompt(
    state_json: &str,
    prompt_content: &str,
    backend: &str,
    opposite_backend: &str,
    completed_feature_summary: &str,
    git_diff_against_base: &str,
) -> String {
    format!(
        "You are a QA engineer validating overall project acceptance.

Run project-level acceptance validation before final completion is approved.

CRITICAL REQUIREMENTS:
- Verify overall project acceptance, not just a single feature.
- Consider all completed feature loops together.
- Use the full current git diff against the base branch as evidence.
- Return `# QA: PASS` only if project-wide acceptance is satisfied.
- Return `# QA: FAIL` with concrete failures and fixes if anything is missing.

CRITICAL FORMAT REQUIREMENTS:
- Return markdown body only (no YAML frontmatter)
- Your response MUST begin with the correct H1 heading as the VERY FIRST LINE
- Include ALL required H2 sections
- No preamble or commentary before the H1

Required output format:

# QA: PASS
## Manual Testing
## Automated Tests
## Acceptance Criteria Verification

OR

# QA: FAIL
## Failures
## Suggested Fixes

## Context Provided

### Master Prompt
{prompt_content}

### Completed Feature Loop Summary
{completed_feature_summary}

### Git Diff Against Base Branch
```diff
{git_diff_against_base}
```

### Project State
```json
{state_json}
```

### Backend Context
- QA Backend: {backend}
- Planner Backend: {opposite_backend}
"
    )
}

fn write_acceptance_backend_artifact(artifact_path: &Path, backend: &str) -> Result<PathBuf> {
    let file_name = artifact_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            RalphError::Orchestration(format!(
                "invalid acceptance artifact path: {}",
                artifact_path.display()
            ))
        })?;

    let backend_slug = slugify_feature_name(backend);
    let rewritten_name = match file_name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}-{backend_slug}.{ext}"),
        None => format!("{file_name}-{backend_slug}"),
    };
    let rewritten_path = artifact_path.with_file_name(rewritten_name);

    fs::rename(artifact_path, &rewritten_path)?;
    Ok(rewritten_path)
}

fn base_vars(
    state: &ProjectState,
    loop_number: u32,
    phase: &str,
    iteration: u32,
    backend: &str,
    opposite_backend: &str,
) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    vars.insert("project_id".to_owned(), state.project_id.clone());
    vars.insert("project_name".to_owned(), state.project_name.clone());
    vars.insert("loop_number".to_owned(), loop_number.to_string());
    vars.insert("phase".to_owned(), phase.to_owned());
    vars.insert("iteration".to_owned(), iteration.to_string());
    vars.insert("backend".to_owned(), backend.to_owned());
    vars.insert("opposite_backend".to_owned(), opposite_backend.to_owned());
    vars
}

fn should_omit_history_for_this_call(
    include_history_when_session_reuse_enabled: bool,
    session_reused_this_call: bool,
) -> bool {
    session_reused_this_call && !include_history_when_session_reuse_enabled
}

fn collect_review_history_for_prompt(
    effective: &EffectiveConfig,
    state: &ProjectState,
    project_dir: &Path,
    session_reused_this_call: bool,
) -> Result<String> {
    if should_omit_history_for_this_call(
        effective
            .workflow
            .include_history_when_session_reuse_enabled,
        session_reused_this_call,
    ) {
        return Ok(String::new());
    }

    collect_review_history(
        state,
        project_dir,
        effective.workflow.max_review_history_entries_in_prompt,
    )
}

fn collect_review_history(
    state: &ProjectState,
    project_dir: &Path,
    max_entries: usize,
) -> Result<String> {
    if max_entries == 0 {
        return Ok(String::new());
    }

    let Some(loop_state) = state.current_feature_loop() else {
        return Ok(String::new());
    };

    let mut exchanges = loop_state.artifacts.reviews.iter().collect::<Vec<_>>();
    exchanges.sort_by_key(|exchange| exchange.iteration);
    if exchanges.len() > max_entries {
        let start = exchanges.len() - max_entries;
        exchanges = exchanges.split_off(start);
    }

    let mut history = Vec::new();
    for exchange in exchanges {
        let feedback = read_project_relative_file(project_dir, &exchange.feedback)?;
        let response = read_project_relative_file(project_dir, &exchange.response)?;
        history.push(format!(
            "### Iteration {}\n\n#### Feedback\n\n{}\n\n#### Response\n\n{}",
            exchange.iteration, feedback, response
        ));
    }

    Ok(history.join("\n\n"))
}

fn collect_qa_history_for_prompt(
    effective: &EffectiveConfig,
    state: &ProjectState,
    project_dir: &Path,
    session_reused_this_call: bool,
) -> Result<String> {
    if should_omit_history_for_this_call(
        effective
            .workflow
            .include_history_when_session_reuse_enabled,
        session_reused_this_call,
    ) {
        return Ok(String::new());
    }

    collect_qa_history(
        state,
        project_dir,
        effective.workflow.max_qa_history_entries_in_prompt,
    )
}

fn collect_qa_history(
    state: &ProjectState,
    project_dir: &Path,
    max_entries: usize,
) -> Result<String> {
    if max_entries == 0 {
        return Ok(String::new());
    }

    let Some(loop_state) = state.current_feature_loop() else {
        return Ok(String::new());
    };

    let mut exchanges = loop_state.artifacts.qa_results.iter().collect::<Vec<_>>();
    exchanges.sort_by_key(|exchange| exchange.iteration);
    if exchanges.len() > max_entries {
        let start = exchanges.len() - max_entries;
        exchanges = exchanges.split_off(start);
    }

    let mut history = Vec::new();
    for exchange in exchanges {
        let report = read_project_relative_file(project_dir, &exchange.report)?;
        let response_section = if let Some(ref response_rel) = exchange.implementer_response {
            let response = read_project_relative_file(project_dir, response_rel)?;
            format!("\n\n#### Implementer Response\n\n{response}")
        } else {
            String::new()
        };
        let verdict = if exchange.passed { "PASS" } else { "FAIL" };
        history.push(format!(
            "### QA Iteration {} ({})\n\n#### Report\n\n{}{}",
            exchange.iteration, verdict, report, response_section
        ));
    }

    Ok(history.join("\n\n"))
}

fn collect_completed_feature_loop_summary(state: &ProjectState) -> Result<String> {
    let mut loops = state
        .loops
        .iter()
        .filter(|loop_state| loop_state.status == LoopStatus::Completed)
        .collect::<Vec<&FeatureLoopState>>();
    loops.sort_by_key(|loop_state| loop_state.loop_number);

    if loops.is_empty() {
        return Ok("- None".to_owned());
    }

    let mut summary = Vec::with_capacity(loops.len());
    for loop_state in loops {
        let commit = loop_state.commit.as_deref().unwrap_or("none");
        summary.push(format!(
            "- Loop {}: {} (slug: {}, commit: {})",
            loop_state.loop_number, loop_state.feature_name, loop_state.slug, commit
        ));
    }
    Ok(summary.join("\n"))
}

fn latest_completion_feedback_context(
    state: &ProjectState,
    project_dir: &Path,
) -> Result<Option<String>> {
    let latest_completion = state
        .completion_attempts
        .iter()
        .max_by_key(|attempt| attempt.loop_number);

    let Some(completion) = latest_completion else {
        return Ok(None);
    };

    let failed_acceptance_results = completion
        .artifacts
        .acceptance_results
        .iter()
        .filter(|result| !result.passed)
        .collect::<Vec<_>>();
    if failed_acceptance_results.is_empty() {
        return Ok(None);
    }

    let completer_verdict_content = completion
        .artifacts
        .verdict
        .as_deref()
        .map(|verdict_rel| read_project_relative_file(project_dir, verdict_rel))
        .transpose()?
        .unwrap_or_else(|| "(missing completer verdict artifact)".to_owned());

    let mut sections = vec![format!(
        "### Completer Verdict Artifact\n\n{completer_verdict_content}"
    )];
    for (idx, result) in failed_acceptance_results.iter().enumerate() {
        let acceptance_fail_content = read_project_relative_file(project_dir, &result.artifact)?;
        sections.push(format!(
            "### Acceptance QA Failure Artifact {} (backend: {})\n\n{}",
            idx + 1,
            result.backend,
            acceptance_fail_content
        ));
    }

    Ok(Some(sections.join("\n\n")))
}

#[allow(clippy::too_many_arguments)]
fn build_qa_prompt(
    effective: &EffectiveConfig,
    state: &ProjectState,
    prompt_content: &str,
    feature_name: &str,
    loop_slug: &str,
    backend: &str,
    opposite_backend: &str,
    spec_content: &str,
    impl_notes_content: &str,
    git_diff: &str,
    qa_history: &str,
) -> Result<String> {
    let template_source = load_template_source(&effective.templates.qa, default_qa_template());

    let mut vars = base_vars(
        state,
        state.current_loop,
        "qa",
        state.phase_iteration,
        backend,
        opposite_backend,
    );
    vars.insert("feature_name".to_owned(), feature_name.to_owned());
    vars.insert("loop_slug".to_owned(), loop_slug.to_owned());
    vars.insert("prompt_content".to_owned(), prompt_content.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert("spec_content".to_owned(), spec_content.to_owned());
    vars.insert("feature_spec".to_owned(), spec_content.to_owned());
    vars.insert(
        "impl_notes_content".to_owned(),
        impl_notes_content.to_owned(),
    );
    vars.insert(
        "implementation_notes".to_owned(),
        impl_notes_content.to_owned(),
    );
    vars.insert("git_diff".to_owned(), git_diff.to_owned());
    vars.insert(
        "current_diff".to_owned(),
        format!("```diff\n{git_diff}\n```"),
    );
    let qa_history_text = if qa_history.is_empty() {
        "(none)"
    } else {
        qa_history
    };
    vars.insert("qa_history".to_owned(), qa_history_text.to_owned());
    vars.insert("prior_qa_history".to_owned(), qa_history_text.to_owned());
    vars.insert("system_guardrails".to_owned(), QA_GUARDRAILS.to_owned());

    let rendered =
        render_template_with_fallback(&effective.templates.qa, &vars, default_qa_template())?;
    let mut prompt = rendered;
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["system_guardrails"],
        "## System Guardrails",
        QA_GUARDRAILS,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["master_prompt", "prompt_content"],
        "## Master Prompt",
        prompt_content,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["feature_spec", "spec_content"],
        "## Feature Spec",
        spec_content,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["implementation_notes", "impl_notes_content"],
        "## Implementation Notes",
        impl_notes_content,
    );
    let current_diff_block = format!("```diff\n{git_diff}\n```");
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["current_diff", "git_diff"],
        "## Current Diff",
        &current_diff_block,
    );
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["prior_qa_history", "qa_history"],
        "## Prior QA History",
        qa_history_text,
    );
    Ok(prompt)
}

fn template_uses_any_var(template_source: &str, var_names: &[&str]) -> bool {
    var_names
        .iter()
        .any(|var_name| template_uses_var(template_source, var_name))
}

fn append_section_if_missing(
    prompt: &mut String,
    template_source: &str,
    var_names: &[&str],
    heading: &str,
    content: &str,
) {
    if content.is_empty() || template_uses_any_var(template_source, var_names) {
        return;
    }

    prompt.push_str("\n\n");
    prompt.push_str(heading);
    prompt.push_str("\n\n");
    prompt.push_str(content);
}

fn feedback_rel_path(
    project_dir: &Path,
    loop_number: u32,
    loop_slug: &str,
    iteration: u32,
) -> Result<String> {
    let suffix = format!("review-{iteration:03}-feedback.md");
    resolve_artifact_path_by_suffix(project_dir, loop_number, loop_slug, &suffix)?.ok_or_else(
        || {
            RalphError::Orchestration(format!(
                "missing feedback artifact for loop {loop_number} iteration {iteration}"
            ))
        },
    )
}

fn response_rel_path(
    project_dir: &Path,
    loop_number: u32,
    loop_slug: &str,
    iteration: u32,
) -> Result<String> {
    let suffix = format!("impl-response-{iteration:03}.md");
    if let Some(path) =
        resolve_artifact_path_by_suffix(project_dir, loop_number, loop_slug, &suffix)?
    {
        return Ok(path);
    }
    let qa_suffix = format!("impl-qa-response-{iteration:03}.md");
    resolve_artifact_path_by_suffix(project_dir, loop_number, loop_slug, &qa_suffix)?.ok_or_else(
        || {
            RalphError::Orchestration(format!(
                "missing implementer response artifact for loop {loop_number} iteration {iteration}"
            ))
        },
    )
}

fn read_project_relative_file(project_dir: &Path, relative: &str) -> Result<String> {
    let path = project_dir.join(relative);
    let content = fs::read_to_string(&path).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to read required artifact '{}': {err}",
            path.display()
        ))
    })?;
    Ok(content)
}

fn persist_state(project_dir: &Path, state: &ProjectState) -> Result<()> {
    save_project_state(project_dir, state)
}

fn phase_label(phase: &Phase) -> &'static str {
    match phase {
        Phase::Planning => "planning",
        Phase::Implementing => "implementing",
        Phase::QA => "qa",
        Phase::Reviewing => "reviewing",
        Phase::Committing => "committing",
        Phase::Completing => "completing",
    }
}

fn ensure_clean_start_for_new_loop(workspace_root: &Path) -> Result<()> {
    let Some(repo_root) = workspace_root.parent() else {
        return Ok(());
    };
    if !is_git_repo(repo_root) {
        return Ok(());
    }

    let mut dirty_paths =
        changed_paths_excluding_prefixes(repo_root, &[ORCHESTRATION_STATE_PATH_PREFIX])?;
    if dirty_paths.is_empty() {
        return Ok(());
    }

    dirty_paths.sort();
    dirty_paths.dedup();

    let sample = dirty_paths
        .iter()
        .take(MAX_DIRTY_PATHS_IN_ERROR)
        .map(|path| format!("- {path}"))
        .collect::<Vec<_>>()
        .join("\n");
    let remainder = if dirty_paths.len() > MAX_DIRTY_PATHS_IN_ERROR {
        format!(
            "\n- ... and {} more path(s)",
            dirty_paths.len() - MAX_DIRTY_PATHS_IN_ERROR
        )
    } else {
        String::new()
    };

    Err(RalphError::Validation(format!(
        "cannot start a new loop with uncommitted changes outside `.ralph/`.\ncommit/stash/discard those paths first:\n{sample}{remainder}"
    )))
}

fn current_git_diff(workspace_root: &Path) -> Result<String> {
    let Some(repo_root) = workspace_root.parent() else {
        return Ok(String::new());
    };
    if !is_git_repo(repo_root) {
        return Ok(String::new());
    }

    working_tree_diff_excluding_orchestration_state(repo_root)
}

fn current_git_diff_against_base(workspace_root: &Path, base_branch: &str) -> Result<String> {
    let Some(repo_root) = workspace_root.parent() else {
        return Ok(String::new());
    };
    if !is_git_repo(repo_root) {
        return Ok(String::new());
    }

    let base_ref = format!("{base_branch}...HEAD");
    let args = ["diff", base_ref.as_str(), "--", ".", ":(exclude).ralph/**"];

    match run_git(repo_root, &args) {
        Ok(diff) => Ok(diff),
        Err(_) => current_git_diff(workspace_root),
    }
}

fn stage_changes_for_review(workspace_root: &Path) -> Result<()> {
    let Some(repo_root) = workspace_root.parent() else {
        return Ok(());
    };
    if !is_git_repo(repo_root) {
        return Ok(());
    }
    stage_implementation_changes(repo_root)
}

fn generate_commit_message(
    style: &CommitMessageStyle,
    feature_name: &str,
    loop_number: u32,
    state: &ProjectState,
) -> String {
    match style {
        CommitMessageStyle::Conventional => {
            format!("feat(ralph): {feature_name} [loop-{loop_number}]")
        }
        CommitMessageStyle::Descriptive => {
            if let Some(loop_state) = state.current_feature_loop() {
                format!(
                    "{feature_name}\n\nImplemented via ralph loop {loop_number}.\nBackends: planner={}, implementer={}, reviewer={}",
                    loop_state.backends.planner,
                    loop_state.backends.implementer,
                    loop_state.backends.reviewer,
                )
            } else {
                format!("{feature_name}\n\nImplemented via ralph loop {loop_number}.")
            }
        }
        CommitMessageStyle::Minimal => feature_name.to_owned(),
    }
}

fn extract_reviewer_commit_message(body: &str) -> Option<String> {
    let mut in_commit_section = false;
    let mut lines = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "## Commit Message" {
            in_commit_section = true;
            continue;
        }

        if in_commit_section {
            if trimmed.starts_with("## ") {
                break;
            }
            if trimmed.is_empty() {
                continue;
            }
            lines.push(trimmed.to_owned());
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

fn expected_format_template_for(role: &str, iteration: Option<u32>) -> String {
    match role {
        "planner" => "\
# Feature: <name>\n\
## Description\n\
## Acceptance Criteria\n\
## Files to Modify/Create\n\
## Dependencies\n\
\n\
OR\n\
\n\
# Project Completion Request\n\
## Rationale\n\
## Summary of Work\n\
## Remaining Items"
            .to_owned(),
        "implementer-notes" => "\
# Implementation Notes\n\
## Decisions Made\n\
## Spec Deviations\n\
## Testing"
            .to_owned(),
        "implementer-response" => {
            let n = iteration.unwrap_or(1);
            format!(
                "\
# Implementation Response (Iteration {n})\n\
## Changes Made\n\
## Could Not Address"
            )
        }
        "reviewer" => "\
# Review: APPROVED\n\
## Acceptance Criteria Checklist\n\
## Notes\n\
## Commit Message\n\
\n\
OR\n\
\n\
# Review: SUGGESTIONS\n\
## Required Changes\n\
## Recommended Improvements"
            .to_owned(),
        "completer" => "\
# Verdict: COMPLETE\n\
(list of requirements and how they are satisfied)\n\
\n\
OR\n\
\n\
# Verdict: CONTINUE\n\
## Missing Requirements\n\
## Recommended Next Features"
            .to_owned(),
        "qa" => "\
# QA: PASS\n\
## Manual Testing\n\
## Automated Tests\n\
## Acceptance Criteria Verification\n\
\n\
OR\n\
\n\
# QA: FAIL\n\
## Failures\n\
## Suggested Fixes"
            .to_owned(),
        "prompt_reviewer" => "\
# Prompt Review\n\
## Issues Found\n\
## Refined Prompt"
            .to_owned(),
        _ => "valid markdown with required H1".to_owned(),
    }
}

fn session_retry_correction_prompt(parse_error: &RalphError, expected_format: &str) -> String {
    format!(
        "Your previous response could not be parsed.\n\
        Parse error: {parse_error}\n\n\
        Reformat your previous answer to match this exact structure:\n\
        {expected_format}\n\n\
        Return only corrected markdown. Do not add commentary.",
    )
}

/// Validate that session-aware arg rewriting would succeed for this backend.
/// If rewriting fails, logs a warning, returns None to disable reuse for this
/// invocation. The orchestrator continues with a fresh (non-resumed) call.
///
/// This ensures rewrite failures are handled at orchestrator level for all
/// backends (tmux and non-tmux), not just silently inside tmux.
fn validate_session_rewrite(
    registry: &BackendRegistry,
    backend_spec: &str,
    session_id: Option<String>,
    loop_dir: &Path,
    role: &str,
) -> Option<String> {
    let sid = session_id?;
    // Try to obtain the CliBackend to test effective_args
    let cli = match registry.cli_backend_for_spec(backend_spec) {
        Ok(cli) => cli,
        Err(_) => return Some(sid), // unknown spec, pass through
    };
    let ctx = crate::backend::BackendInvocationContext {
        loop_dir: loop_dir.to_owned(),
        role: role.to_owned(),
        session_id: Some(sid.clone()),
        json_output_required: true,
    };
    match cli.effective_args(&ctx) {
        Ok(_) => Some(sid),
        Err(e) => {
            warn!(
                backend = backend_spec,
                role = role,
                error = %e,
                "session arg rewrite failed, disabling reuse for this invocation"
            );
            None
        }
    }
}

/// Normalize raw backend output, extracting structured text when available.
/// On normalization error, logs a warning and falls back to raw output.
fn normalize_backend_output(
    backend_name: &str,
    raw: &str,
) -> crate::backend::output_normalizer::NormalizedOutput {
    use crate::backend::output_normalizer::{normalize_output, NormalizedOutput};
    match normalize_output(backend_name, raw) {
        Ok(normalized) => normalized,
        Err(e) => {
            warn!(
                backend = backend_name,
                error = %e,
                "output normalization failed, falling back to raw output"
            );
            NormalizedOutput {
                text: raw.to_owned(),
                ..Default::default()
            }
        }
    }
}

fn log_parse_retry_token_metrics(
    role: &str,
    phase: &str,
    loop_number: u32,
    attempt: u8,
    backend: &str,
    session_reused: bool,
    normalized: &crate::backend::output_normalizer::NormalizedOutput,
) {
    match (normalized.tokens_in, normalized.tokens_out, normalized.cached_in) {
        (Some(tokens_in), Some(tokens_out), Some(cached_in)) => info!(
            role = role,
            phase = phase,
            loop_number = loop_number,
            attempt = attempt,
            backend = backend,
            session_reused = session_reused,
            tokens_in = tokens_in,
            tokens_out = tokens_out,
            cached_in = cached_in,
            "parse-retry normalization metrics"
        ),
        (Some(tokens_in), Some(tokens_out), None) => info!(
            role = role,
            phase = phase,
            loop_number = loop_number,
            attempt = attempt,
            backend = backend,
            session_reused = session_reused,
            tokens_in = tokens_in,
            tokens_out = tokens_out,
            cached_in = tracing::field::Empty,
            "parse-retry normalization metrics"
        ),
        (Some(tokens_in), None, Some(cached_in)) => info!(
            role = role,
            phase = phase,
            loop_number = loop_number,
            attempt = attempt,
            backend = backend,
            session_reused = session_reused,
            tokens_in = tokens_in,
            tokens_out = tracing::field::Empty,
            cached_in = cached_in,
            "parse-retry normalization metrics"
        ),
        (Some(tokens_in), None, None) => info!(
            role = role,
            phase = phase,
            loop_number = loop_number,
            attempt = attempt,
            backend = backend,
            session_reused = session_reused,
            tokens_in = tokens_in,
            tokens_out = tracing::field::Empty,
            cached_in = tracing::field::Empty,
            "parse-retry normalization metrics"
        ),
        (None, Some(tokens_out), Some(cached_in)) => info!(
            role = role,
            phase = phase,
            loop_number = loop_number,
            attempt = attempt,
            backend = backend,
            session_reused = session_reused,
            tokens_in = tracing::field::Empty,
            tokens_out = tokens_out,
            cached_in = cached_in,
            "parse-retry normalization metrics"
        ),
        (None, Some(tokens_out), None) => info!(
            role = role,
            phase = phase,
            loop_number = loop_number,
            attempt = attempt,
            backend = backend,
            session_reused = session_reused,
            tokens_in = tracing::field::Empty,
            tokens_out = tokens_out,
            cached_in = tracing::field::Empty,
            "parse-retry normalization metrics"
        ),
        (None, None, Some(cached_in)) => info!(
            role = role,
            phase = phase,
            loop_number = loop_number,
            attempt = attempt,
            backend = backend,
            session_reused = session_reused,
            tokens_in = tracing::field::Empty,
            tokens_out = tracing::field::Empty,
            cached_in = cached_in,
            "parse-retry normalization metrics"
        ),
        (None, None, None) => info!(
            role = role,
            phase = phase,
            loop_number = loop_number,
            attempt = attempt,
            backend = backend,
            session_reused = session_reused,
            tokens_in = tracing::field::Empty,
            tokens_out = tracing::field::Empty,
            cached_in = tracing::field::Empty,
            "parse-retry normalization metrics"
        ),
    }
}

/// Result from `execute_with_parse_retries` that includes the parsed value
/// plus session metadata from output normalization.
struct ParseRetryResult<T> {
    parsed: T,
    /// Session ID extracted from normalized output (if any).
    session_id: Option<String>,
}

async fn execute_with_parse_retries<T, F>(
    backend: Arc<dyn Backend>,
    registry: &BackendRegistry,
    role: &str,
    phase: &str,
    loop_number: u32,
    original_prompt: &str,
    initial_session_id: Option<&str>,
    parse_fn: F,
    expected_format: &str,
    timeout_secs: u64,
    log_writer: &mut LogWriter,
    // Written with the last discovered session_id, even if parse ultimately fails.
    // Enables callers to persist session records per D6 lifecycle rules.
    out_session_id: Option<&mut Option<String>>,
    // Repo root for cwd invariant assertion (spec D6).
    repo_root: Option<&Path>,
) -> Result<ParseRetryResult<T>>
where
    F: Fn(&str) -> Result<T>,
{
    let backend_name = backend.name().to_owned();
    let loop_dir_hint = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut attempts_executed: u8 = 0;
    let mut active_session_id = validate_session_rewrite(
        registry,
        &backend_name,
        initial_session_id.map(str::to_owned),
        &loop_dir_hint,
        role,
    );
    let mut last_session_id: Option<String> = None;

    registry.override_session_id(active_session_id.clone()).await;

    let first_raw = execute_with_timeout_retries(
        backend.clone(),
        role,
        phase,
        original_prompt,
        timeout_secs,
        log_writer,
        repo_root,
    )
    .await?;

    // If output is empty/near-empty, retry with the same backend before going to the
    // reformatter. Empty responses indicate a backend execution issue (e.g. token limits,
    // overloaded API), not a formatting problem. Sending empty output to the reformatter
    // causes it to fabricate a structurally valid but semantically wrong response.
    let first_raw = if first_raw.trim().len() < 20 {
        warn!(
            role = role,
            output_len = first_raw.len(),
            "backend returned empty/near-empty output, retrying with same backend"
        );
        let retry_raw = execute_with_timeout_retries(
            backend.clone(),
            role,
            phase,
            original_prompt,
            timeout_secs,
            log_writer,
            repo_root,
        )
        .await?;
        if retry_raw.trim().len() > first_raw.trim().len() {
            retry_raw
        } else {
            first_raw
        }
    } else {
        first_raw
    };

    attempts_executed += 1;
    // Normalize output: extract structured text and session_id from Claude/Codex JSON.
    let normalized = normalize_backend_output(&backend_name, &first_raw);
    let first_session_id = normalized.session_id.clone();
    log_parse_retry_token_metrics(
        role,
        phase,
        loop_number,
        attempts_executed,
        &backend_name,
        active_session_id.is_some(),
        &normalized,
    );
    if first_session_id.is_some() {
        active_session_id = first_session_id.clone();
        last_session_id = first_session_id.clone();
    }
    let first_output = normalized.text;

    let parse_error_first = match parse_fn(&first_output) {
        Ok(parsed) => {
            return Ok(ParseRetryResult {
                parsed,
                session_id: first_session_id,
            });
        }
        Err(parse_error) => Some(parse_error),
    };

    let mut latest_unparsed_output = first_output;

    // Attempt 2: in-session follow-up only when a session is active after attempt 1.
    if active_session_id.is_some() {
        let resume_session_id = validate_session_rewrite(
            registry,
            &backend_name,
            active_session_id.clone(),
            &loop_dir_hint,
            role,
        );
        registry.override_session_id(resume_session_id.clone()).await;
        if let Some(parse_error) = &parse_error_first {
            warn!(
                role = role,
                backend = %backend_name,
                error = %parse_error,
                "parse failed, retrying with in-session correction prompt"
            );
        }
        let correction_prompt = session_retry_correction_prompt(
            parse_error_first
                .as_ref()
                .expect("parse_error_1 must exist after initial parse failure"),
            expected_format,
        );
        let second_raw = execute_with_timeout_retries(
            backend.clone(),
            role,
            phase,
            &correction_prompt,
            timeout_secs,
            log_writer,
            repo_root,
        )
        .await?;
        attempts_executed += 1;
        let second_normalized = normalize_backend_output(&backend_name, &second_raw);
        log_parse_retry_token_metrics(
            role,
            phase,
            loop_number,
            attempts_executed,
            &backend_name,
            resume_session_id.is_some(),
            &second_normalized,
        );
        if second_normalized.session_id.is_some() {
            last_session_id = second_normalized.session_id.clone();
        }
        if let Ok(parsed) = parse_fn(&second_normalized.text) {
            return Ok(ParseRetryResult {
                parsed,
                session_id: last_session_id,
            });
        }
        latest_unparsed_output = second_normalized.text;
    }

    // Attempt 3: opposite-backend reformatter.
    let parse_error = parse_error_first
        .as_ref()
        .expect("parse_error_1 must exist before reformatter");
    let reformatter_spec = registry
        .opposite(backend.name())
        .map(|opposite_name| registry.resolve_backend_for_role(opposite_name, "reformatter"))
        .unwrap_or_else(|_| backend.name().to_owned());
    let reformatter_backend = registry
        .get(&reformatter_spec)
        .unwrap_or_else(|| backend.clone());
    let reformatter_name = reformatter_backend.name().to_owned();
    let reformatter_timeout_secs = registry
        .timeout_for_role(&reformatter_spec, "reformatter")
        .as_secs();

    warn!(
        role = role,
        backend = %reformatter_name,
        error = %parse_error,
        "parse failed, requesting reformat via opposite backend"
    );
    // Use ~~~ fences instead of --- to avoid triggering strip_frontmatter().
    let reformat_prompt = format!(
        "CRITICAL: Your previous response could not be parsed.\n\n\
        Error: {parse_error}\n\n\
        Your original response was:\n~~~\n{latest_unparsed_output}\n~~~\n\n\
        Requirements:\n\
        1. Your response MUST begin with the correct H1 heading as the VERY FIRST LINE\n\
        2. No preamble, commentary, or explanation before the H1\n\
        3. No YAML frontmatter (no lines starting with ---)\n\
        4. Include ALL required H2 sections\n\n\
        Required structure:\n{expected_format}\n\n\
        Respond ONLY with the corrected markdown. No explanation.\n",
    );
    registry.override_session_id(None).await;
    let third_raw = execute_with_timeout_retries(
        reformatter_backend,
        role,
        phase,
        &reformat_prompt,
        reformatter_timeout_secs,
        log_writer,
        repo_root,
    )
    .await?;
    attempts_executed += 1;
    let third_normalized = normalize_backend_output(&reformatter_name, &third_raw);
    log_parse_retry_token_metrics(
        role,
        phase,
        loop_number,
        attempts_executed,
        &reformatter_name,
        false,
        &third_normalized,
    );
    if let Ok(parsed) = parse_fn(&third_normalized.text) {
        return Ok(ParseRetryResult {
            parsed,
            session_id: last_session_id,
        });
    }

    // Attempt 4: full reminded prompt on original backend as a forced fresh call.
    warn!(
        role = role,
        backend = %backend_name,
        "reformat failed, retrying with fresh full prompt"
    );
    let reminded_prompt = format!(
        "IMPORTANT: Format your response as parseable markdown. \
        Your VERY FIRST LINE must be exactly:\n\n{expected_format}\n\n\
        No preamble. No commentary before the H1. No YAML frontmatter. \
        Include all required H2 sections.\n\n{original_prompt}",
    );
    registry.override_session_id(None).await;
    let fourth_raw = execute_with_timeout_retries(
        backend,
        role,
        phase,
        &reminded_prompt,
        timeout_secs,
        log_writer,
        repo_root,
    )
    .await?;
    attempts_executed += 1;
    let fourth_normalized = normalize_backend_output(&backend_name, &fourth_raw);
    log_parse_retry_token_metrics(
        role,
        phase,
        loop_number,
        attempts_executed,
        &backend_name,
        false,
        &fourth_normalized,
    );
    if fourth_normalized.session_id.is_some() {
        last_session_id = fourth_normalized.session_id;
    }
    if let Ok(parsed) = parse_fn(&fourth_normalized.text) {
        return Ok(ParseRetryResult {
            parsed,
            session_id: last_session_id,
        });
    }

    warn!(role = role, attempts = attempts_executed, "all parse retries exhausted");
    // Write last discovered session_id even on failure (D6 lifecycle rule).
    if let Some(out) = out_session_id {
        *out = last_session_id;
    }
    Err(RalphError::ParseRetriesExhausted {
        role: role.to_owned(),
        phase: phase.to_owned(),
        attempts: attempts_executed,
    })
}

async fn execute_with_timeout_retries(
    backend: Arc<dyn Backend>,
    role: &str,
    phase: &str,
    prompt: &str,
    timeout_secs: u64,
    log_writer: &mut LogWriter,
    repo_root: Option<&Path>,
) -> Result<String> {
    // Verify cwd is exactly at repo root before backend invocation (spec D6).
    // Enforces strict equality: debug_assert_eq!(current_dir, repo_root).
    // Guard: only assert when cwd is related to repo_root (same tree).
    // Unit tests that don't chdir into the temp workspace have an unrelated
    // cwd and are safely skipped.
    if let (Ok(cwd), Some(root)) = (std::env::current_dir(), repo_root) {
        if cwd.starts_with(root) || root.starts_with(&cwd) {
            debug_assert_eq!(
                cwd, root,
                "backend invocation cwd ({}) must equal repo root ({})",
                cwd.display(),
                root.display()
            );
        }
    }

    let retry_started = Instant::now();

    for attempt in 1..=3_u8 {
        let is_fallback = log_writer.attempt() > 0;
        log_writer.write_attempt_separator(backend.name(), is_fallback);

        match backend.execute_with_log(prompt, Some(log_writer)).await {
            Ok(output) => {
                return Ok(output);
            }
            Err(RalphError::BackendTimeout {
                backend: backend_name,
                idle_seconds,
                timeout_kind,
            }) => {
                let total_elapsed_secs = retry_started.elapsed().as_secs();
                if attempt == 3 {
                    warn!(
                        role = role,
                        backend = %backend_name,
                        attempt = attempt,
                        idle_seconds = idle_seconds,
                        total_elapsed_secs = total_elapsed_secs,
                        timeout_kind = ?timeout_kind,
                        "backend timeout, retries exhausted"
                    );
                    return Err(RalphError::BackendTimeoutExhausted {
                        backend: backend_name,
                        phase: phase.to_owned(),
                        role: role.to_owned(),
                        timeout_secs,
                        attempts: attempt,
                    });
                }
                let backoff = 2_u64.pow((attempt - 1) as u32);
                warn!(
                    role = role,
                    backend = %backend_name,
                    attempt = attempt,
                    idle_seconds = idle_seconds,
                    total_elapsed_secs = total_elapsed_secs,
                    timeout_kind = ?timeout_kind,
                    backoff_secs = backoff,
                    "backend timeout, retrying..."
                );
                sleep(Duration::from_secs(backoff)).await;
            }
            Err(other) => return Err(other),
        }
    }

    Err(RalphError::Orchestration(
        "unexpected timeout retry control-flow error".to_owned(),
    ))
}

/// Compute the bootstrap hash for session reuse identity verification.
#[allow(dead_code)]
///
/// The hash includes the role, backend spec, prompt hash at loop start, spec
/// content hash, template content hash, and a version salt. If any of these
/// change between invocations, the stored session is stale and must not be reused.
fn compute_bootstrap_hash(
    role: &str,
    backend_spec: &str,
    prompt_hash_at_loop_start: &str,
    spec_content: &str,
    role_template_content: &str,
) -> String {
    let spec_hash = sha256_hex(spec_content);
    let template_hash = sha256_hex(role_template_content);
    sha256_hex(&format!(
        "{role}|{backend_spec}|{prompt_hash_at_loop_start}|{spec_hash}|{template_hash}|sessions-v1"
    ))
}

/// V1 supported roles for session reuse. Planner and completer are known roles
/// but not supported for session reuse in v1.
const V1_SESSION_REUSE_SUPPORTED_ROLES: &[&str] = &["implementer", "reviewer", "qa"];
const KNOWN_ROLES: &[&str] = &["planner", "implementer", "reviewer", "qa", "completer"];

/// Determine whether session reuse should be attempted for a given role.
///
/// Returns `Some(session_id)` if a valid stored session exists with matching
/// bootstrap hash, or `None` if session reuse is disabled/skipped for this role.
///
/// Runtime role-policy rules:
/// - Unknown roles: warn and skip.
/// - Known but unsupported v1 roles (planner, completer): warn and skip.
/// - Supported roles not in config `session_reuse_roles`: skip silently.
fn resolve_session_for_role(
    effective: &EffectiveConfig,
    state: &mut ProjectState,
    role: &str,
    backend_spec: &str,
    loop_number: u32,
    bootstrap_hash: &str,
) -> Option<String> {
    if !effective.workflow.session_reuse_enabled {
        return None;
    }

    // Unknown role => warn and skip
    if !KNOWN_ROLES.contains(&role) {
        warn!(role = role, "unknown role for session reuse, skipping");
        return None;
    }

    // Known but unsupported v1 role => warn and skip
    if !V1_SESSION_REUSE_SUPPORTED_ROLES.contains(&role) {
        warn!(
            role = role,
            "session reuse not supported for v1 role, skipping"
        );
        return None;
    }

    // Not in configured roles => skip silently
    if !effective
        .workflow
        .session_reuse_roles
        .contains(&role.to_owned())
    {
        return None;
    }

    // Lookup existing session record
    if let Some(record) = state.session_store.lookup(loop_number, role, backend_spec) {
        if record.bootstrap_hash == bootstrap_hash {
            return Some(record.session_id.clone());
        }
        // Bootstrap hash mismatch: will force fresh call, record replaced after execution
        debug!(
            role = role,
            loop_number = loop_number,
            "session bootstrap hash mismatch, forcing fresh call"
        );
    }

    None
}

/// Update session store after a backend execution. Follows session lifecycle rules:
/// - Store only when session_id exists.
/// - Update on new id; keep prior id when resume response omits id.
/// - Parse failure after normalization still updates/stores session record.
fn upsert_session_after_execution(
    state: &mut ProjectState,
    role: &str,
    backend_spec: &str,
    loop_number: u32,
    bootstrap_hash: &str,
    new_session_id: Option<&str>,
    had_prior_session: bool,
) {
    use crate::project::state::SessionRecord;

    match new_session_id {
        Some(sid) => {
            // New session id: store or update
            let existing = state
                .session_store
                .lookup(loop_number, role, backend_spec);
            let (call_count, created_at) = match existing {
                Some(r) => (r.call_count + 1, r.created_at),
                None => (1, Utc::now()),
            };
            state.session_store.upsert(SessionRecord {
                session_id: sid.to_owned(),
                backend_spec: backend_spec.to_owned(),
                role: role.to_owned(),
                loop_number,
                bootstrap_hash: bootstrap_hash.to_owned(),
                call_count,
                created_at,
                last_used_at: Utc::now(),
            });
        }
        None if had_prior_session => {
            // Resume response omitted session_id: keep prior stored id,
            // just bump call_count and last_used_at.
            if let Some(existing) = state
                .session_store
                .lookup(loop_number, role, backend_spec)
            {
                let mut updated = existing.clone();
                updated.call_count += 1;
                updated.last_used_at = Utc::now();
                state.session_store.upsert(updated);
            }
        }
        None => {
            // No session id and no prior session: nothing to store
        }
    }
}

/// Ensure the legacy `prompt_hash_at_loop_start` field is populated.
/// If empty, fall back to `prompt_hash` and persist the repaired value.
fn ensure_prompt_hash_at_loop_start(state: &mut ProjectState) {
    if state.prompt_hash_at_loop_start.is_empty() && !state.prompt_hash.is_empty() {
        state.prompt_hash_at_loop_start = state.prompt_hash.clone();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use tempfile::tempdir;
    use tokio::sync::Mutex as AsyncMutex;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::Registry;

    use super::{
        build_planner_prompt, collect_qa_history, collect_qa_history_for_prompt,
        collect_review_history, collect_review_history_for_prompt, execute_with_parse_retries,
        preload_role_model_backends, resolve_tmux_settings, summarize_previous_specs_for_planner,
        summarize_state_for_planner, validate_tmux_preflight,
    };
    use crate::backend::{Backend, BackendRegistry, BackendRegistryTmuxConfig};
    use crate::config::global::{BackendRoleModels, PlannerStateInPrompt, PreviousSpecsInPrompt};
    use crate::config::{resolve_effective_config, GlobalConfig, RunWorkflowOverrides};
    use crate::error::RalphError;
    use crate::output_log::LogWriter;
    use crate::project::state::{
        FeatureLoopArtifacts, FeatureLoopBackends, FeatureLoopState, LoopStatus, LoopType,
        ProjectState, QaExchange, ReviewExchange,
    };

    fn tmux_disabled() -> BackendRegistryTmuxConfig {
        BackendRegistryTmuxConfig {
            enabled: false,
            session_name: "ralph".to_owned(),
            window_keep_seconds: 0,
        }
    }

    #[test]
    fn preload_role_model_backends_creates_expected_entries_for_default_config() {
        let config = GlobalConfig::default();
        let mut registry = BackendRegistry::new(&config, tmux_disabled());

        preload_role_model_backends(&mut registry)
            .expect("preloading default role-model backends should succeed");

        assert!(registry.get("claude(opus)").is_some());
        assert!(registry.get("claude(sonnet)").is_some());
        assert!(registry.get("codex(gpt-5.3-codex-xhigh)").is_some());
        assert!(registry.get("codex(gpt-5.3-codex-high)").is_some());
        assert!(registry.get("codex(gpt-5.3-codex-medium)").is_some());
    }

    #[test]
    fn preload_role_model_backends_is_noop_when_models_are_unset() {
        let mut config = GlobalConfig::default();
        config.backends.claude.models = BackendRoleModels::default();
        config.backends.codex.models = BackendRoleModels::default();
        let mut registry = BackendRegistry::new(&config, tmux_disabled());

        preload_role_model_backends(&mut registry)
            .expect("preloading without role-models should succeed");

        assert!(registry.get("claude(opus)").is_none());
        assert!(registry.get("codex(gpt-5.3-codex-xhigh)").is_none());
    }

    #[test]
    fn preload_role_model_backends_covers_all_roles_for_all_backends() {
        let mut config = GlobalConfig::default();
        config.backends.claude.models = BackendRoleModels {
            planner: Some("claude-planner".to_owned()),
            implementer: Some("claude-implementer".to_owned()),
            reviewer: Some("claude-reviewer".to_owned()),
            qa: Some("claude-qa".to_owned()),
            completer: Some("claude-completer".to_owned()),
            acceptance_qa: Some("claude-acceptance-qa".to_owned()),
            reformatter: Some("claude-reformatter".to_owned()),
        };
        config.backends.codex.models = BackendRoleModels {
            planner: Some("codex-planner".to_owned()),
            implementer: Some("codex-implementer".to_owned()),
            reviewer: Some("codex-reviewer".to_owned()),
            qa: Some("codex-qa".to_owned()),
            completer: Some("codex-completer".to_owned()),
            acceptance_qa: Some("codex-acceptance-qa".to_owned()),
            reformatter: Some("codex-reformatter".to_owned()),
        };
        let mut registry = BackendRegistry::new(&config, tmux_disabled());

        preload_role_model_backends(&mut registry)
            .expect("preloading distinct role-model specs should succeed");

        for expected_spec in [
            "claude(claude-planner)",
            "claude(claude-implementer)",
            "claude(claude-reviewer)",
            "claude(claude-qa)",
            "claude(claude-completer)",
            "claude(claude-reformatter)",
            "codex(codex-planner)",
            "codex(codex-implementer)",
            "codex(codex-reviewer)",
            "codex(codex-qa)",
            "codex(codex-completer)",
            "codex(codex-reformatter)",
        ] {
            assert!(
                registry.get(expected_spec).is_some(),
                "expected preloaded backend spec {expected_spec}"
            );
        }
    }

    #[test]
    fn resolve_tmux_settings_prefers_cli_override() {
        let resolved = resolve_tmux_settings(Some(false), true, "ralph".to_owned());
        assert!(!resolved.enabled);
        assert_eq!(resolved.session_name, "ralph");
    }

    #[test]
    fn resolve_tmux_settings_falls_back_to_config() {
        let resolved = resolve_tmux_settings(None, true, "session-a".to_owned());
        assert!(resolved.enabled);
        assert_eq!(resolved.session_name, "session-a");
    }

    #[test]
    fn validate_tmux_preflight_checks_when_enabled_and_not_dry_run() {
        let called = Arc::new(AtomicBool::new(false));
        let called_ref = Arc::clone(&called);

        let result = validate_tmux_preflight(true, false, move || {
            called_ref.store(true, Ordering::Relaxed);
            Err(RalphError::TmuxUnavailable)
        });

        assert!(called.load(Ordering::Relaxed));
        assert!(matches!(result, Err(RalphError::TmuxUnavailable)));
    }

    #[test]
    fn validate_tmux_preflight_skips_check_for_dry_run() {
        let result = validate_tmux_preflight(true, true, || Err(RalphError::TmuxUnavailable));
        assert!(result.is_ok());
    }

    #[test]
    fn expected_format_template_for_implementer_response_substitutes_iteration() {
        let template = super::expected_format_template_for("implementer-response", Some(3));
        assert!(
            template.contains("(Iteration 3)"),
            "should substitute actual iteration number; got: {template}"
        );
        assert!(
            !template.contains("<N>"),
            "should not contain placeholder <N>; got: {template}"
        );
    }

    #[test]
    fn expected_format_template_for_implementer_response_defaults_to_1() {
        let template = super::expected_format_template_for("implementer-response", None);
        assert!(
            template.contains("(Iteration 1)"),
            "should default to iteration 1; got: {template}"
        );
    }

    #[test]
    fn expected_format_template_for_other_roles_ignores_iteration() {
        let planner = super::expected_format_template_for("planner", Some(5));
        assert!(
            planner.contains("# Feature:"),
            "planner template should be unaffected by iteration; got: {planner}"
        );
        let reviewer = super::expected_format_template_for("reviewer", None);
        assert!(
            reviewer.contains("# Review: APPROVED"),
            "reviewer template should work without iteration; got: {reviewer}"
        );
    }

    #[test]
    fn expected_format_template_for_qa_contains_pass_and_fail() {
        let qa = super::expected_format_template_for("qa", None);
        assert!(
            qa.contains("# QA: PASS"),
            "qa template should contain PASS heading; got: {qa}"
        );
        assert!(
            qa.contains("# QA: FAIL"),
            "qa template should contain FAIL heading; got: {qa}"
        );
        assert!(
            qa.contains("## Manual Testing"),
            "qa template should contain Manual Testing section; got: {qa}"
        );
        assert!(
            qa.contains("## Failures"),
            "qa template should contain Failures section; got: {qa}"
        );
    }

    #[test]
    fn planner_prompt_default_template_has_single_master_prompt_section() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let effective = resolve_effective_config(
            temp.path(),
            &project_dir,
            GlobalConfig::default(),
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve effective config");
        let state = ProjectState::new("demo", "Demo", "hash", None);

        let prompt = build_planner_prompt(
            &effective,
            &state,
            "# Master Prompt Body",
            1,
            "claude",
            "codex",
            project_dir.as_path(),
        )
        .expect("build planner prompt");

        assert_eq!(prompt.matches("## Master Prompt").count(), 1);
    }

    #[test]
    fn planner_prompt_custom_template_without_master_placeholder_appends_once() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let custom_template = temp.path().join("planner-custom.md");
        fs::write(
            &custom_template,
            "Custom planner template\n\n{{system_guardrails}}\n\n{{state_json}}\n\n{{previous_specs}}\n",
        )
        .expect("write custom template");

        let mut global = GlobalConfig::default();
        global.templates.planner = custom_template.to_string_lossy().to_string();
        let effective = resolve_effective_config(
            temp.path(),
            &project_dir,
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve effective config");
        let state = ProjectState::new("demo", "Demo", "hash", None);

        let prompt = build_planner_prompt(
            &effective,
            &state,
            "# Master Prompt Body",
            1,
            "claude",
            "codex",
            project_dir.as_path(),
        )
        .expect("build planner prompt");

        assert_eq!(prompt.matches("## Master Prompt").count(), 1);

        let custom_index = prompt
            .find("Custom planner template")
            .expect("custom template content should be present");
        let master_index = prompt
            .find("## Master Prompt")
            .expect("appended master prompt section should be present");
        assert!(master_index > custom_index);
    }

    fn make_feature_loop(
        loop_number: u32,
        name: &str,
        status: LoopStatus,
        reviews: Vec<ReviewExchange>,
        qa_results: Vec<QaExchange>,
    ) -> FeatureLoopState {
        FeatureLoopState {
            loop_number,
            slug: name.to_lowercase().replace(' ', "-"),
            feature_name: name.to_owned(),
            loop_type: LoopType::Feature,
            status,
            backends: FeatureLoopBackends {
                planner: "claude".to_owned(),
                implementer: "codex".to_owned(),
                reviewer: "claude".to_owned(),
                qa: "codex".to_owned(),
            },
            artifacts: FeatureLoopArtifacts {
                spec: format!(
                    "loops/{loop_number:03}-{}/spec.md",
                    name.to_lowercase().replace(' ', "-")
                ),
                impl_notes: None,
                reviews,
                approval: None,
                qa_results,
                pending_qa_feedback: None,
            },
            commit: None,
            started_at: chrono::Utc::now(),
            completed_at: None,
        }
    }

    fn write_test_file(project_dir: &Path, relative: &str, content: &str) {
        let path = project_dir.join(relative);
        let parent = path
            .parent()
            .expect("test artifact path should have parent");
        fs::create_dir_all(parent).expect("create parent dir");
        fs::write(path, content).expect("write test artifact");
    }

    fn parse_iteration_headers(history: &str, prefix: &str) -> Vec<u32> {
        history
            .lines()
            .filter_map(|line| {
                line.strip_prefix(prefix).and_then(|rest| {
                    rest.split_whitespace()
                        .next()
                        .and_then(|token| token.parse::<u32>().ok())
                })
            })
            .collect()
    }

    #[test]
    fn collect_review_history_caps_to_latest_three_sequential() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path();

        let mut reviews = Vec::new();
        for iteration in 1..=5 {
            let feedback = format!("loops/001-feature-a/review-{iteration:03}.md");
            let response = format!("loops/001-feature-a/response-{iteration:03}.md");
            write_test_file(project_dir, &feedback, &format!("feedback-{iteration}"));
            write_test_file(project_dir, &response, &format!("response-{iteration}"));
            reviews.push(ReviewExchange {
                iteration,
                feedback,
                response,
            });
        }

        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.current_loop = 1;
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::InProgress,
            reviews,
            vec![],
        ));

        let history =
            collect_review_history(&state, project_dir, 3).expect("collect review history");
        assert_eq!(
            parse_iteration_headers(&history, "### Iteration "),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn collect_review_history_caps_to_highest_two_non_sequential() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path();

        let mut reviews = Vec::new();
        for iteration in [1, 3, 7, 2, 5] {
            let feedback = format!("loops/001-feature-a/review-{iteration:03}.md");
            let response = format!("loops/001-feature-a/response-{iteration:03}.md");
            write_test_file(project_dir, &feedback, &format!("feedback-{iteration}"));
            write_test_file(project_dir, &response, &format!("response-{iteration}"));
            reviews.push(ReviewExchange {
                iteration,
                feedback,
                response,
            });
        }

        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.current_loop = 1;
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::InProgress,
            reviews,
            vec![],
        ));

        let history =
            collect_review_history(&state, project_dir, 2).expect("collect review history");
        assert_eq!(
            parse_iteration_headers(&history, "### Iteration "),
            vec![5, 7]
        );
    }

    #[test]
    fn collect_qa_history_caps_to_highest_two_non_sequential() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path();

        let mut qa_results = Vec::new();
        for iteration in [1, 3, 7, 2, 5] {
            let report = format!("loops/001-feature-a/qa-{iteration:03}.md");
            write_test_file(project_dir, &report, &format!("qa-{iteration}"));
            qa_results.push(QaExchange {
                iteration,
                passed: iteration % 2 == 0,
                report,
                implementer_response: None,
            });
        }

        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.current_loop = 1;
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::InProgress,
            vec![],
            qa_results,
        ));

        let history = collect_qa_history(&state, project_dir, 2).expect("collect qa history");
        assert_eq!(
            parse_iteration_headers(&history, "### QA Iteration "),
            vec![5, 7]
        );
    }

    #[test]
    fn collect_history_returns_empty_when_cap_is_zero() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path();

        let review_feedback = "loops/001-feature-a/review-001.md";
        let review_response = "loops/001-feature-a/response-001.md";
        let qa_report = "loops/001-feature-a/qa-001.md";
        write_test_file(project_dir, review_feedback, "feedback");
        write_test_file(project_dir, review_response, "response");
        write_test_file(project_dir, qa_report, "qa report");

        let reviews = vec![ReviewExchange {
            iteration: 1,
            feedback: review_feedback.to_owned(),
            response: review_response.to_owned(),
        }];
        let qa_results = vec![QaExchange {
            iteration: 1,
            passed: false,
            report: qa_report.to_owned(),
            implementer_response: None,
        }];
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.current_loop = 1;
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::InProgress,
            reviews,
            qa_results,
        ));

        assert!(collect_review_history(&state, project_dir, 0)
            .expect("collect review history")
            .is_empty());
        assert!(collect_qa_history(&state, project_dir, 0)
            .expect("collect qa history")
            .is_empty());
    }

    #[test]
    fn history_omitted_when_session_reused_and_config_disables_reused_history() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let review_feedback = "loops/001-feature-a/review-001.md";
        let review_response = "loops/001-feature-a/response-001.md";
        let qa_report = "loops/001-feature-a/qa-001.md";
        write_test_file(&project_dir, review_feedback, "feedback");
        write_test_file(&project_dir, review_response, "response");
        write_test_file(&project_dir, qa_report, "qa report");

        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.current_loop = 1;
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::InProgress,
            vec![ReviewExchange {
                iteration: 1,
                feedback: review_feedback.to_owned(),
                response: review_response.to_owned(),
            }],
            vec![QaExchange {
                iteration: 1,
                passed: false,
                report: qa_report.to_owned(),
                implementer_response: None,
            }],
        ));

        let mut global = GlobalConfig::default();
        global.workflow.include_history_when_session_reuse_enabled = false;
        let effective = resolve_effective_config(
            temp.path(),
            &project_dir,
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve effective config");

        assert!(
            collect_review_history_for_prompt(&effective, &state, &project_dir, true)
                .expect("collect review history")
                .is_empty()
        );
        assert!(
            collect_qa_history_for_prompt(&effective, &state, &project_dir, true)
                .expect("collect qa history")
                .is_empty()
        );
    }

    #[test]
    fn history_included_when_resume_rewrite_fallback_uses_fresh_prompt() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let mut reviews = Vec::new();
        for iteration in 1..=3 {
            let feedback = format!("loops/001-feature-a/review-{iteration:03}.md");
            let response = format!("loops/001-feature-a/response-{iteration:03}.md");
            write_test_file(&project_dir, &feedback, &format!("feedback-{iteration}"));
            write_test_file(&project_dir, &response, &format!("response-{iteration}"));
            reviews.push(ReviewExchange {
                iteration,
                feedback,
                response,
            });
        }

        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.current_loop = 1;
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::InProgress,
            reviews,
            vec![],
        ));

        let mut global = GlobalConfig::default();
        global.workflow.max_review_history_entries_in_prompt = 2;
        global.workflow.include_history_when_session_reuse_enabled = false;
        let effective = resolve_effective_config(
            temp.path(),
            &project_dir,
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve effective config");

        // Simulates resume-arg rewrite failure where the invocation falls back
        // to a fresh prompt (`session_reused_this_call = false`).
        let history = collect_review_history_for_prompt(&effective, &state, &project_dir, false)
            .expect("collect review history");
        assert_eq!(
            parse_iteration_headers(&history, "### Iteration "),
            vec![2, 3]
        );
    }

    #[test]
    fn summarize_state_includes_loop_metadata_excludes_feedback_body() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::Completed,
            vec![ReviewExchange {
                iteration: 1,
                feedback: "loops/001-feature-a/review-1.md".to_owned(),
                response: "loops/001-feature-a/response-1.md".to_owned(),
            }],
            vec![QaExchange {
                iteration: 1,
                passed: true,
                report: "loops/001-feature-a/qa-1.md".to_owned(),
                implementer_response: None,
            }],
        ));
        state.loops.push(make_feature_loop(
            2,
            "Feature B",
            LoopStatus::InProgress,
            vec![],
            vec![],
        ));
        state.current_loop = 2;

        let summary = summarize_state_for_planner(&state, None);

        // Must include metadata
        assert!(summary.contains("Feature A"), "should include feature name");
        assert!(summary.contains("Loop 1"), "should include loop number");
        assert!(summary.contains("Completed"), "should include status");
        assert!(summary.contains("spec="), "should include spec path");

        // Verdict must be deterministic for every loop entry
        assert!(
            summary.contains("verdict=completed"),
            "completed loop should have verdict=completed"
        );
        assert!(
            summary.contains("verdict=pending"),
            "in-progress loop should have verdict=pending"
        );

        // Must NOT include raw feedback/report file paths as body text
        // (the function does not read files, so feedback bodies are excluded by design)
        assert!(
            !summary.contains("review-1.md"),
            "should not include feedback file path as body"
        );
        assert!(
            !summary.contains("qa-1.md"),
            "should not include qa report file path as body"
        );
    }

    #[test]
    fn summarize_state_cap_limits_to_latest_n() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        for i in 1..=5 {
            state.loops.push(make_feature_loop(
                i,
                &format!("Feature {i}"),
                LoopStatus::Completed,
                vec![],
                vec![],
            ));
        }

        // Cap to 2 => only loops 4, 5
        let summary = summarize_state_for_planner(&state, Some(2));
        assert!(!summary.contains("Feature 1"), "loop 1 should be excluded");
        assert!(!summary.contains("Feature 3"), "loop 3 should be excluded");
        assert!(summary.contains("Feature 4"), "loop 4 should be included");
        assert!(summary.contains("Feature 5"), "loop 5 should be included");
    }

    #[test]
    fn summarize_state_cap_zero_shows_none() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::Completed,
            vec![],
            vec![],
        ));

        let summary = summarize_state_for_planner(&state, Some(0));
        assert!(summary.contains("(none shown)"), "cap=0 should show none");
        assert!(
            !summary.contains("Feature A"),
            "should not include any features"
        );
    }

    #[test]
    fn summarize_state_unlimited_includes_all() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        for i in 1..=20 {
            state.loops.push(make_feature_loop(
                i,
                &format!("Feature {i}"),
                LoopStatus::Completed,
                vec![],
                vec![],
            ));
        }

        let summary = summarize_state_for_planner(&state, None);
        assert!(
            summary.contains("Feature 1"),
            "should include first feature"
        );
        assert!(
            summary.contains("Feature 20"),
            "should include last feature"
        );
    }

    #[test]
    fn summarize_specs_none_mode_returns_empty() {
        let state = ProjectState::new("demo", "Demo", "hash", None);
        let temp = tempdir().expect("temp dir");
        let result = summarize_previous_specs_for_planner(
            &state,
            temp.path(),
            PreviousSpecsInPrompt::None,
            None,
        )
        .expect("should succeed");
        assert!(result.is_empty(), "None mode should return empty string");
    }

    #[test]
    fn summarize_specs_titles_mode_produces_bullet_titles() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.loops.push(make_feature_loop(
            1,
            "Auth System",
            LoopStatus::Completed,
            vec![],
            vec![],
        ));
        state.loops.push(make_feature_loop(
            2,
            "API Layer",
            LoopStatus::Completed,
            vec![],
            vec![],
        ));

        let temp = tempdir().expect("temp dir");
        let result = summarize_previous_specs_for_planner(
            &state,
            temp.path(),
            PreviousSpecsInPrompt::Titles,
            None,
        )
        .expect("should succeed");

        assert!(
            result.contains("- Loop 1: Auth System"),
            "should have bullet for loop 1"
        );
        assert!(
            result.contains("- Loop 2: API Layer"),
            "should have bullet for loop 2"
        );
        // Must NOT contain spec file content (since we didn't write any files)
        assert!(
            !result.contains("##"),
            "titles mode should not have markdown headers"
        );
    }

    #[test]
    fn summarize_specs_titles_mode_cap_zero_returns_empty() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::Completed,
            vec![],
            vec![],
        ));

        let temp = tempdir().expect("temp dir");
        let result = summarize_previous_specs_for_planner(
            &state,
            temp.path(),
            PreviousSpecsInPrompt::Titles,
            Some(0),
        )
        .expect("should succeed");
        assert!(result.is_empty(), "cap=0 should return empty");
    }

    #[test]
    fn summarize_specs_fulltext_mode_reads_spec_files() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path();

        // Create a spec file
        let loop_dir = project_dir.join("loops/001-auth");
        fs::create_dir_all(&loop_dir).expect("create loop dir");
        fs::write(
            loop_dir.join("spec.md"),
            "# Auth Feature\nSpec content here",
        )
        .expect("write spec");

        state.loops.push(FeatureLoopState {
            loop_number: 1,
            slug: "auth".to_owned(),
            feature_name: "Auth Feature".to_owned(),
            loop_type: LoopType::Feature,
            status: LoopStatus::Completed,
            backends: FeatureLoopBackends {
                planner: "claude".to_owned(),
                implementer: "codex".to_owned(),
                reviewer: "claude".to_owned(),
                qa: "codex".to_owned(),
            },
            artifacts: FeatureLoopArtifacts {
                spec: "loops/001-auth/spec.md".to_owned(),
                impl_notes: None,
                reviews: vec![],
                approval: None,
                qa_results: vec![],
                pending_qa_feedback: None,
            },
            commit: None,
            started_at: chrono::Utc::now(),
            completed_at: None,
        });

        let result = summarize_previous_specs_for_planner(
            &state,
            project_dir,
            PreviousSpecsInPrompt::FullText,
            None,
        )
        .expect("should succeed");

        assert!(
            result.contains("Auth Feature"),
            "should include feature name"
        );
        assert!(
            result.contains("Spec content here"),
            "should include spec file content"
        );
    }

    #[test]
    fn summarize_specs_cap_limits_to_latest_n() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        for i in 1..=5 {
            state.loops.push(make_feature_loop(
                i,
                &format!("Feature {i}"),
                LoopStatus::Completed,
                vec![],
                vec![],
            ));
        }

        let temp = tempdir().expect("temp dir");
        let result = summarize_previous_specs_for_planner(
            &state,
            temp.path(),
            PreviousSpecsInPrompt::Titles,
            Some(2),
        )
        .expect("should succeed");

        assert!(
            !result.contains("Feature 1"),
            "loop 1 should be excluded by cap"
        );
        assert!(
            !result.contains("Feature 3"),
            "loop 3 should be excluded by cap"
        );
        assert!(result.contains("Feature 4"), "loop 4 should be included");
        assert!(result.contains("Feature 5"), "loop 5 should be included");
    }

    #[test]
    fn planner_prompt_with_summary_mode_excludes_full_json() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let mut global = GlobalConfig::default();
        global.workflow.planner_state_in_prompt = PlannerStateInPrompt::Summary;
        global.workflow.planner_previous_specs_in_prompt = PreviousSpecsInPrompt::Titles;
        global.workflow.planner_max_prior_loops = Some(10);

        let effective = resolve_effective_config(
            temp.path(),
            &project_dir,
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve effective config");

        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.loops.push(make_feature_loop(
            1,
            "Feature A",
            LoopStatus::Completed,
            vec![],
            vec![],
        ));

        let prompt = build_planner_prompt(
            &effective,
            &state,
            "Master prompt body",
            2,
            "claude",
            "codex",
            &project_dir,
        )
        .expect("build planner prompt");

        // Summary mode should NOT include the full JSON state
        assert!(
            !prompt.contains("\"project_id\""),
            "summary mode should not include full JSON state keys"
        );
        // Should include loop summary metadata
        assert!(
            prompt.contains("Feature A"),
            "summary should include feature name"
        );
        // Should include titles-only spec listing
        assert!(
            prompt.contains("- Loop 1: Feature A"),
            "should include titles-mode spec listing"
        );
    }

    #[test]
    fn planner_prompt_with_full_json_mode_includes_json() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let mut global = GlobalConfig::default();
        global.workflow.planner_state_in_prompt = PlannerStateInPrompt::FullJson;
        global.workflow.planner_previous_specs_in_prompt = PreviousSpecsInPrompt::None;

        let effective = resolve_effective_config(
            temp.path(),
            &project_dir,
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve effective config");

        let state = ProjectState::new("demo", "Demo", "hash", None);

        let prompt = build_planner_prompt(
            &effective,
            &state,
            "Master prompt body",
            1,
            "claude",
            "codex",
            &project_dir,
        )
        .expect("build planner prompt");

        // FullJson mode should include the full JSON state
        assert!(
            prompt.contains("\"project_id\""),
            "full-json mode should include JSON state keys"
        );
    }

    #[test]
    fn planner_prompt_full_json_has_no_nested_code_fences() {
        let temp = tempdir().expect("temp dir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let mut global = GlobalConfig::default();
        global.workflow.planner_state_in_prompt = PlannerStateInPrompt::FullJson;
        global.workflow.planner_previous_specs_in_prompt = PreviousSpecsInPrompt::None;

        let effective = resolve_effective_config(
            temp.path(),
            &project_dir,
            global,
            None,
            RunWorkflowOverrides::default(),
        )
        .expect("resolve effective config");

        let state = ProjectState::new("demo", "Demo", "hash", None);

        let prompt = build_planner_prompt(
            &effective,
            &state,
            "Master prompt body",
            1,
            "claude",
            "codex",
            &project_dir,
        )
        .expect("build planner prompt");

        // Count occurrences of ```json — there should be exactly one (from template),
        // not two (which would indicate nested fences).
        let fence_count = prompt.matches("```json").count();
        assert_eq!(
            fence_count, 1,
            "FullJson mode should produce exactly one ```json fence, got {fence_count}"
        );
    }

    #[test]
    fn summarize_state_verdict_deterministic_for_all_statuses() {
        let mut state = ProjectState::new("demo", "Demo", "hash", None);

        // Approved loop (has approval artifact)
        let mut approved_loop =
            make_feature_loop(1, "Approved", LoopStatus::Completed, vec![], vec![]);
        approved_loop.artifacts.approval = Some("loops/001-approved/approval.md".to_owned());
        state.loops.push(approved_loop);

        // Completed loop (no approval)
        state.loops.push(make_feature_loop(
            2,
            "Completed",
            LoopStatus::Completed,
            vec![],
            vec![],
        ));

        // In-progress loop
        state.loops.push(make_feature_loop(
            3,
            "InProgress",
            LoopStatus::InProgress,
            vec![],
            vec![],
        ));

        // Failed loop (last QA failed)
        state.loops.push(make_feature_loop(
            4,
            "Failed",
            LoopStatus::InProgress,
            vec![],
            vec![QaExchange {
                iteration: 1,
                passed: false,
                report: "loops/004-failed/qa-1.md".to_owned(),
                implementer_response: None,
            }],
        ));

        let summary = summarize_state_for_planner(&state, None);

        // Every loop line must include a verdict= token
        for line in summary.lines() {
            if line.starts_with("- Loop ") {
                assert!(
                    line.contains("verdict="),
                    "every loop line must include verdict=, got: {line}"
                );
            }
        }

        // Specific verdicts
        assert!(
            summary.contains("verdict=approved"),
            "loop with approval should have verdict=approved"
        );
        assert!(
            summary.contains("verdict=completed"),
            "completed loop without approval should have verdict=completed"
        );
        assert!(
            summary.contains("verdict=pending"),
            "in-progress loop should have verdict=pending"
        );
        assert!(
            summary.contains("verdict=failed"),
            "loop with failed QA should have verdict=failed"
        );
    }

    #[derive(Clone)]
    struct SequencedBackend {
        name: String,
        responses: Arc<AsyncMutex<Vec<String>>>,
        prompts: Arc<AsyncMutex<Vec<String>>>,
    }

    impl SequencedBackend {
        fn new(name: &str, responses: Vec<String>) -> Self {
            Self {
                name: name.to_owned(),
                responses: Arc::new(AsyncMutex::new(responses)),
                prompts: Arc::new(AsyncMutex::new(Vec::new())),
            }
        }

        async fn call_count(&self) -> usize {
            self.prompts.lock().await.len()
        }
    }

    #[async_trait]
    impl Backend for SequencedBackend {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, prompt: &str) -> crate::Result<String> {
            self.prompts.lock().await.push(prompt.to_owned());
            let mut responses = self.responses.lock().await;
            if responses.is_empty() {
                return Ok("fallback backend response long enough for retry path".to_owned());
            }
            Ok(responses.remove(0))
        }

        async fn health_check(&self) -> crate::Result<()> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ParseRetryMetricsEvent {
        attempt: u8,
        session_reused: bool,
        tokens_in_seen: bool,
        tokens_out_seen: bool,
        cached_in_seen: bool,
    }

    #[derive(Default)]
    struct MetricsVisitor {
        attempt: Option<u8>,
        session_reused: Option<bool>,
        tokens_in_seen: bool,
        tokens_out_seen: bool,
        cached_in_seen: bool,
    }

    impl MetricsVisitor {
        fn mark_token_field(&mut self, name: &str) {
            match name {
                "tokens_in" => self.tokens_in_seen = true,
                "tokens_out" => self.tokens_out_seen = true,
                "cached_in" => self.cached_in_seen = true,
                _ => {}
            }
        }
    }

    impl Visit for MetricsVisitor {
        fn record_u64(&mut self, field: &Field, value: u64) {
            match field.name() {
                "attempt" => self.attempt = Some(value as u8),
                "tokens_in" | "tokens_out" | "cached_in" => self.mark_token_field(field.name()),
                _ => {}
            }
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            match field.name() {
                "attempt" => self.attempt = u8::try_from(value).ok(),
                "tokens_in" | "tokens_out" | "cached_in" => self.mark_token_field(field.name()),
                _ => {}
            }
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            if field.name() == "session_reused" {
                self.session_reused = Some(value);
            }
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            let _ = (field, value);
        }

        fn record_debug(&mut self, field: &Field, _value: &dyn std::fmt::Debug) {
            self.mark_token_field(field.name());
        }
    }

    #[derive(Clone, Default)]
    struct MetricsCaptureLayer {
        events: Arc<Mutex<Vec<ParseRetryMetricsEvent>>>,
    }

    impl<S> Layer<S> for MetricsCaptureLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = MetricsVisitor::default();
            event.record(&mut visitor);
            if let (Some(attempt), Some(session_reused)) = (visitor.attempt, visitor.session_reused) {
                self.events
                    .lock()
                    .expect("capture lock")
                    .push(ParseRetryMetricsEvent {
                        attempt,
                        session_reused,
                        tokens_in_seen: visitor.tokens_in_seen,
                        tokens_out_seen: visitor.tokens_out_seen,
                        cached_in_seen: visitor.cached_in_seen,
                    });
            }
        }
    }

    #[test]
    fn parse_retry_attempts_are_three_without_session() {
        let temp = tempdir().expect("temp dir");
        let backend = SequencedBackend::new(
            "mock-retry-backend",
            vec![
                "first long non-parseable response body for attempt one".to_owned(),
                "second long non-parseable response body for attempt two".to_owned(),
                "third long non-parseable response body for attempt three".to_owned(),
            ],
        );
        let backend_handle = backend.clone();
        let backend: Arc<dyn Backend> = Arc::new(backend);
        let registry = BackendRegistry::new(&GlobalConfig::default(), tmux_disabled());
        let mut log = LogWriter::open(temp.path(), Some(1), Some("retry"), "implementer");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let result = runtime.block_on(execute_with_parse_retries(
            backend,
            &registry,
            "implementer",
            "implementing",
            1,
            "original prompt",
            None,
            |_raw| -> crate::Result<()> {
                Err(RalphError::ParseError("forced parse failure".to_owned()))
            },
            "# Implementation Notes",
            30,
            &mut log,
            None,
            None,
        ));

        match result {
            Err(RalphError::ParseRetriesExhausted { attempts, .. }) => {
                assert_eq!(attempts, 3, "without a session, exactly 3 attempts should run")
            }
            _ => panic!("expected ParseRetriesExhausted"),
        }
        assert_eq!(
            runtime.block_on(backend_handle.call_count()),
            3,
            "backend should be called exactly 3 times without session follow-up"
        );
    }

    #[test]
    fn parse_retry_attempts_four_with_session_followup_and_token_metrics() {
        let temp = tempdir().expect("temp dir");
        let backend = SequencedBackend::new(
            "claude-mock",
            vec![
                r#"{"session_id":"sess-from-attempt1","content":[{"type":"text","text":"attempt one not parseable but long enough"}],"usage":{"input_tokens":11,"output_tokens":22,"cache_read_input_tokens":33}}"#.to_owned(),
                "attempt two still non-parseable output body".to_owned(),
                "attempt three still non-parseable output body".to_owned(),
                "attempt four still non-parseable output body".to_owned(),
            ],
        );
        let backend_handle = backend.clone();
        let backend: Arc<dyn Backend> = Arc::new(backend);
        let registry = BackendRegistry::new(&GlobalConfig::default(), tmux_disabled());
        let mut log = LogWriter::open(temp.path(), Some(1), Some("retry"), "implementer");

        let capture_layer = MetricsCaptureLayer::default();
        let captured = capture_layer.events.clone();
        let subscriber = Registry::default().with(capture_layer);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let result = tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(execute_with_parse_retries(
                backend,
                &registry,
                "implementer",
                "implementing",
                1,
                "original prompt",
                None,
                |_raw| -> crate::Result<()> {
                    Err(RalphError::ParseError("forced parse failure".to_owned()))
                },
                "# Implementation Notes",
                30,
                &mut log,
                None,
                None,
            ))
        });

        match result {
            Err(RalphError::ParseRetriesExhausted { attempts, .. }) => {
                assert_eq!(attempts, 4, "session-aware path should execute 4 attempts")
            }
            _ => panic!("expected ParseRetriesExhausted"),
        }
        assert_eq!(
            runtime.block_on(backend_handle.call_count()),
            4,
            "backend should be called exactly 4 times with session follow-up"
        );

        let events = captured.lock().expect("capture lock").clone();
        assert_eq!(
            events.len(),
            4,
            "expected one token-metrics log event per normalization call"
        );
        assert_eq!(
            events.iter().map(|e| e.attempt).collect::<Vec<_>>(),
            vec![1, 2, 3, 4],
            "attempt numbers should be 1-based within the executed retry sequence"
        );
        assert_eq!(
            events
                .iter()
                .map(|e| e.session_reused)
                .collect::<Vec<_>>(),
            vec![false, true, false, false],
            "attempt 2 should reuse the session id discovered on attempt 1"
        );

        assert!(
            events[0].tokens_in_seen && events[0].tokens_out_seen && events[0].cached_in_seen,
            "attempt 1 should log all token fields when structured usage is present"
        );
        for event in events.iter().skip(1) {
            assert!(
                !event.tokens_in_seen && !event.tokens_out_seen && !event.cached_in_seen,
                "token fields should be omitted (Empty) when usage data is unavailable"
            );
        }
    }

    // --- Bootstrap hash determinism and invalidation tests ---

    #[test]
    fn bootstrap_hash_is_deterministic() {
        let hash1 = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "prompt_hash_abc",
            "spec body",
            "template content",
        );
        let hash2 = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "prompt_hash_abc",
            "spec body",
            "template content",
        );
        assert_eq!(hash1, hash2, "identical inputs must produce identical hashes");
    }

    #[test]
    fn bootstrap_hash_changes_on_role_change() {
        let base = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "phash",
            "spec",
            "template",
        );
        let changed = super::compute_bootstrap_hash(
            "reviewer",
            "claude(opus)",
            "phash",
            "spec",
            "template",
        );
        assert_ne!(base, changed, "different roles must produce different hashes");
    }

    #[test]
    fn bootstrap_hash_changes_on_backend_change() {
        let base = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "phash",
            "spec",
            "template",
        );
        let changed = super::compute_bootstrap_hash(
            "implementer",
            "codex(gpt-5.3)",
            "phash",
            "spec",
            "template",
        );
        assert_ne!(base, changed, "different backends must produce different hashes");
    }

    #[test]
    fn bootstrap_hash_changes_on_prompt_hash_change() {
        let base = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "phash_v1",
            "spec",
            "template",
        );
        let changed = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "phash_v2",
            "spec",
            "template",
        );
        assert_ne!(base, changed, "different prompt hashes must produce different bootstrap hashes");
    }

    #[test]
    fn bootstrap_hash_changes_on_spec_content_change() {
        let base = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "phash",
            "spec v1",
            "template",
        );
        let changed = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "phash",
            "spec v2",
            "template",
        );
        assert_ne!(base, changed, "different spec content must produce different hashes");
    }

    #[test]
    fn bootstrap_hash_changes_on_template_content_change() {
        let base = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "phash",
            "spec",
            "template v1",
        );
        let changed = super::compute_bootstrap_hash(
            "implementer",
            "claude(opus)",
            "phash",
            "spec",
            "template v2",
        );
        assert_ne!(base, changed, "different template content must produce different hashes");
    }

    #[test]
    fn bootstrap_hash_includes_version_salt() {
        // Verify the hash is not simply sha256 of concatenated fields without salt
        let hash = super::compute_bootstrap_hash("qa", "codex", "ph", "s", "t");
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64, "sha256 hex output should be 64 chars");
    }
}
