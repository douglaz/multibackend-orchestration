use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::backend::tmux_backend::TmuxExecutionContext;
use crate::backend::{tmux, Backend, BackendRegistry, BackendRegistryTmuxConfig, RoleOverrides};
use crate::config::{
    resolve_effective_config, CommitMessageStyle, EffectiveConfig, PromptChangeAction,
    RunWorkflowOverrides,
};
use crate::error::RalphError;
use crate::git::branch::{branch_exists, checkout_branch, merge_base_branch, resolve_branch_name};
use crate::git::commit::{
    changed_paths_excluding_prefixes, commit_feature_loop, reset_and_clean_working_tree,
    stage_implementation_changes, working_tree_diff_excluding_orchestration_state,
    ORCHESTRATION_STATE_PATH_PREFIX,
};
use crate::git::{is_git_repo, run_git};
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
use crate::workspace::index::ProjectLifecycleStatus;
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
        let project_id = if let Some(id) = options.project.as_ref() {
            id.clone()
        } else {
            self.workspace
                .index
                .active_project
                .clone()
                .ok_or(RalphError::ActiveProjectNotSet)?
        };

        let project_dir = self.workspace.project_dir(&project_id);
        if !project_dir.exists() {
            return Err(RalphError::ProjectNotFound(project_id));
        }

        // When --project is explicitly specified, update the active project
        if explicit_project {
            self.workspace.index.set_active_project(&project_id)?;
            self.workspace.save_index()?;
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
            persist_state_and_index(&mut self.workspace, &project_id, &project_dir, &state)?;
        }

        // Skip-flag handling: mark completed immediately so future resumes also skip.
        if options.skip_prompt_review && !state.prompt_review_completed {
            info!("--skip-prompt-review: marking prompt review as completed");
            state.prompt_review_completed = true;
            persist_state_and_index(&mut self.workspace, &project_id, &project_dir, &state)?;
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
            let pr_backend = registry.get_or_create_for_spec(pr_backend_spec)?;

            registry
                .set_tmux_context(TmuxExecutionContext {
                    loop_number: None,
                    role: Some("prompt_reviewer".to_owned()),
                })
                .await;

            info!(backend = pr_backend.name(), "invoking prompt reviewer...");
            let decision = execute_with_parse_retries(
                pr_backend,
                &registry,
                "prompt_reviewer",
                "prompt_review",
                &prompt_reviewer_prompt,
                parse_prompt_reviewer_output,
                &expected_format_template_for("prompt_reviewer", None),
            )
            .await?;

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
            persist_state_and_index(&mut self.workspace, &project_id, &project_dir, &state)?;
            info!("prompt review completed; prompt file updated");
        }

        let feature_target = options.loops.unwrap_or(1);
        let mut completed_feature_loops = 0_u32;
        let mut logs: Vec<String> = Vec::new();

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
                        registry.get_or_create_for_spec(&feature_backends.planner)?;

                    let planner_prompt = build_planner_prompt(
                        &effective,
                        &state,
                        &prompt_content,
                        loop_number,
                        planner_backend.name(),
                        &feature_backends.implementer,
                        &project_dir,
                    )?;

                    registry
                        .set_tmux_context(TmuxExecutionContext {
                            loop_number: Some(loop_number),
                            role: Some("planner".to_owned()),
                        })
                        .await;

                    info!(
                        loop = loop_number,
                        backend = planner_backend.name(),
                        "invoking planner..."
                    );
                    let planner_decision = execute_with_parse_retries(
                        planner_backend,
                        &registry,
                        "planner",
                        "planning",
                        &planner_prompt,
                        parse_planner_output,
                        &expected_format_template_for("planner", None),
                    )
                    .await?;
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
                        registry.get_or_create_for_spec(&implementer_backend_name)?;

                    let spec_content = read_project_relative_file(&project_dir, &spec_rel)?;
                    let git_diff = current_git_diff(&self.workspace.root)?;
                    let iteration = state.phase_iteration;

                    if impl_notes_rel.is_none() {
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
                        )?;

                        registry
                            .set_tmux_context(TmuxExecutionContext {
                                loop_number: Some(loop_number),
                                role: Some("impl".to_owned()),
                            })
                            .await;

                        info!(
                            loop = loop_number,
                            backend = implementer_backend.name(),
                            "invoking implementer..."
                        );
                        let decision = execute_with_parse_retries(
                            implementer_backend,
                            &registry,
                            "implementer",
                            "implementing",
                            &impl_prompt,
                            |raw| parse_implementer_output(raw, None),
                            &expected_format_template_for("implementer-notes", None),
                        )
                        .await?;
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
                        )?;

                        registry
                            .set_tmux_context(TmuxExecutionContext {
                                loop_number: Some(loop_number),
                                role: Some("impl".to_owned()),
                            })
                            .await;

                        info!(
                            loop = loop_number,
                            backend = implementer_backend.name(),
                            iteration = iteration,
                            "invoking implementer for QA feedback response..."
                        );
                        let decision = execute_with_parse_retries(
                            implementer_backend,
                            &registry,
                            "implementer",
                            "implementing",
                            &impl_prompt,
                            |raw| parse_implementer_output(raw, Some(iteration)),
                            &expected_format_template_for("implementer-response", Some(iteration)),
                        )
                        .await?;

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
                        )?;

                        registry
                            .set_tmux_context(TmuxExecutionContext {
                                loop_number: Some(loop_number),
                                role: Some("impl".to_owned()),
                            })
                            .await;

                        info!(
                            loop = loop_number,
                            backend = implementer_backend.name(),
                            iteration = iteration,
                            "invoking implementer for feedback response..."
                        );
                        let decision = execute_with_parse_retries(
                            implementer_backend,
                            &registry,
                            "implementer",
                            "implementing",
                            &impl_prompt,
                            |raw| parse_implementer_output(raw, Some(iteration)),
                            &expected_format_template_for("implementer-response", Some(iteration)),
                        )
                        .await?;

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
                        persist_state_and_index(
                            &mut self.workspace,
                            &project_id,
                            &project_dir,
                            &state,
                        )?;
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

                    let qa_backend = registry.get_or_create_for_spec(&qa_backend_name)?;

                    let spec_content = read_project_relative_file(&project_dir, &spec_rel)?;
                    let impl_notes_rel = impl_notes_rel.ok_or_else(|| {
                        RalphError::Orchestration(
                            "cannot run QA before impl-notes artifact exists".to_owned(),
                        )
                    })?;
                    let impl_notes_content =
                        read_project_relative_file(&project_dir, &impl_notes_rel)?;
                    let git_diff = current_git_diff(&self.workspace.root)?;
                    let qa_history = collect_qa_history(&state, &project_dir)?;

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
                        })
                        .await;

                    info!(
                        loop = loop_number,
                        backend = qa_backend.name(),
                        iteration = state.phase_iteration,
                        "invoking QA..."
                    );
                    let qa_decision = execute_with_parse_retries(
                        qa_backend,
                        &registry,
                        "qa",
                        "qa",
                        &qa_prompt,
                        parse_qa_output,
                        &expected_format_template_for("qa", None),
                    )
                    .await?;
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
                            registry.get_or_create_for_spec(&reviewer_backend_name)?;

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
                        )?;

                        registry
                            .set_tmux_context(TmuxExecutionContext {
                                loop_number: Some(loop_number),
                                role: Some("reviewer".to_owned()),
                            })
                            .await;

                        info!(
                            loop = loop_number,
                            backend = reviewer_backend.name(),
                            "invoking reviewer..."
                        );
                        let reviewer_decision = execute_with_parse_retries(
                            reviewer_backend,
                            &registry,
                            "reviewer",
                            "reviewing",
                            &reviewer_prompt,
                            parse_reviewer_output,
                            &expected_format_template_for("reviewer", None),
                        )
                        .await?;
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
                        registry.get_or_create_for_spec(&completer_backend_name)?;

                    let termination_content =
                        read_project_relative_file(&project_dir, &termination_rel)?;
                    let previous_specs = collect_previous_specs(&state, &project_dir)?;

                    let completer_prompt = build_completer_prompt(
                        &effective,
                        &state,
                        &prompt_content,
                        completer_backend.name(),
                        &planner_backend_name,
                        &termination_content,
                        &previous_specs,
                    )?;

                    registry
                        .set_tmux_context(TmuxExecutionContext {
                            loop_number: Some(loop_number),
                            role: Some("completer".to_owned()),
                        })
                        .await;

                    info!(
                        loop = loop_number,
                        backend = completer_backend.name(),
                        "invoking completer..."
                    );
                    let completer_decision: CompleterDecision = execute_with_parse_retries(
                        completer_backend,
                        &registry,
                        "completer",
                        "completing",
                        &completer_prompt,
                        parse_completer_output,
                        &expected_format_template_for("completer", None),
                    )
                    .await?;
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
                                    .map(|family| registry.resolve_backend_for_role(family, "qa"))
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
                                        .get_or_create_for_spec(acceptance_qa_backend_name)?;
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
                                        })
                                        .await;

                                    info!(
                                        loop = loop_number,
                                        backend = acceptance_qa_backend.name(),
                                        "invoking acceptance QA..."
                                    );
                                    let acceptance_decision = execute_with_parse_retries(
                                        acceptance_qa_backend,
                                        &registry,
                                        "qa",
                                        "completing",
                                        &acceptance_prompt,
                                        parse_qa_output,
                                        &expected_format_template_for("qa", None),
                                    )
                                    .await?;
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
                persist_state_and_index(&mut self.workspace, &project_id, &project_dir, &state)?;
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

            persist_state_and_index(&mut self.workspace, &project_id, &project_dir, &state)?;

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
    }
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
    if let Some(index_entry) = workspace.index.get_project(&state.project_id) {
        if index_entry.parent_project != state.parent_project {
            eprintln!(
                "warning: parent_project mismatch for {}: index={:?} state={:?}",
                state.project_id, index_entry.parent_project, state.parent_project
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
            rollback_current_loop(state, project_dir, workspace_root)?;
            state.prompt_hash = new_prompt_hash.to_owned();
            state.prompt_hash_at_loop_start = new_prompt_hash.to_owned();
            Ok(())
        }
    }
}

fn collect_previous_specs(state: &ProjectState, project_dir: &Path) -> Result<String> {
    let mut parts = Vec::new();
    let mut loops = state.loops.iter().collect::<Vec<&FeatureLoopState>>();
    loops.sort_by_key(|l| l.loop_number);
    for loop_state in loops {
        if let Ok(spec) = read_project_relative_file(project_dir, &loop_state.artifacts.spec) {
            parts.push(format!(
                "## Loop {}: {}\n\n{}",
                loop_state.loop_number, loop_state.feature_name, spec
            ));
        }
    }

    Ok(parts.join("\n\n"))
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
    let mut vars = base_vars(state, loop_number, "planning", 1, backend, opposite_backend);
    let state_json = serde_json::to_string_pretty(state).unwrap_or_default();
    vars.insert("prompt_content".to_owned(), prompt_content.to_owned());
    vars.insert("state_content".to_owned(), state_json.clone());
    vars.insert(
        "previous_specs".to_owned(),
        collect_previous_specs(state, project_dir)?,
    );

    let rendered = render_template_with_fallback(
        &effective.templates.planner,
        &vars,
        default_planner_template(),
    )?;
    let mut prompt = format!(
        "{rendered}\n\n## System Guardrails\n\n{PLANNER_GUARDRAILS}\n\n## Master Prompt\n\n{prompt_content}\n\n## Current State\n\n```json\n{state_json}\n```\n"
    );

    if let Some(completion_feedback) = latest_completion_feedback_context(state, project_dir)? {
        prompt.push_str("\n## Completion Feedback\n\n");
        prompt.push_str(&completion_feedback);
        prompt.push('\n');
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
) -> Result<String> {
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
    vars.insert("spec_content".to_owned(), spec_content.to_owned());
    vars.insert("git_diff".to_owned(), git_diff.to_owned());
    vars.insert(
        "review_feedback_content".to_owned(),
        review_feedback.unwrap_or("").to_owned(),
    );
    vars.insert(
        "review_history".to_owned(),
        collect_review_history(state, project_dir)?,
    );

    let rendered = render_template_with_fallback(
        &effective.templates.implementer,
        &vars,
        default_implementer_template(),
    )?;
    Ok(format!(
        "{rendered}\n\n## System Guardrails\n\n{IMPLEMENTER_GUARDRAILS}\n\n## Master Prompt\n\n{prompt_content}\n\n## Feature Spec\n\n{spec_content}\n\n## Current Diff\n\n```diff\n{git_diff}\n```\n\n## Review Feedback\n\n{}\n",
        review_feedback.unwrap_or("(none)")
    ))
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
) -> Result<String> {
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
    vars.insert("spec_content".to_owned(), spec_content.to_owned());
    vars.insert(
        "impl_notes_content".to_owned(),
        impl_notes_content.to_owned(),
    );
    vars.insert("git_diff".to_owned(), git_diff.to_owned());
    vars.insert(
        "impl_response_content".to_owned(),
        impl_response_content.unwrap_or("").to_owned(),
    );
    vars.insert(
        "review_history".to_owned(),
        collect_review_history(state, project_dir)?,
    );

    let rendered = render_template_with_fallback(
        &effective.templates.reviewer,
        &vars,
        default_reviewer_template(),
    )?;
    Ok(format!(
        "{rendered}\n\n## System Guardrails\n\n{REVIEWER_GUARDRAILS}\n\n## Master Prompt\n\n{prompt_content}\n\n## Feature Spec\n\n{spec_content}\n\n## Implementation Notes\n\n{impl_notes_content}\n\n## Latest Implementation Response\n\n{}\n\n## Current Diff\n\n```diff\n{git_diff}\n```\n",
        impl_response_content.unwrap_or("(none)")
    ))
}

fn build_completer_prompt(
    effective: &EffectiveConfig,
    state: &ProjectState,
    prompt_content: &str,
    backend: &str,
    opposite_backend: &str,
    termination_request_content: &str,
    previous_specs: &str,
) -> Result<String> {
    let mut vars = base_vars(
        state,
        state.current_loop,
        "completing",
        1,
        backend,
        opposite_backend,
    );
    let state_json = serde_json::to_string_pretty(state).unwrap_or_default();
    vars.insert("prompt_content".to_owned(), prompt_content.to_owned());
    vars.insert(
        "termination_request_content".to_owned(),
        termination_request_content.to_owned(),
    );
    vars.insert("previous_specs".to_owned(), previous_specs.to_owned());
    vars.insert("state_content".to_owned(), state_json.clone());

    let rendered = render_template_with_fallback(
        &effective.templates.completer,
        &vars,
        default_completer_template(),
    )?;
    Ok(format!(
        "{rendered}\n\n## Master Prompt\n\n{prompt_content}\n\n## Completion Request\n\n{termination_request_content}\n\n## Prior Specs\n\n{previous_specs}\n\n## State\n\n```json\n{state_json}\n```\n"
    ))
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
## Tests Run
## Verification Summary

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

fn collect_review_history(state: &ProjectState, project_dir: &Path) -> Result<String> {
    let Some(loop_state) = state.current_feature_loop() else {
        return Ok(String::new());
    };

    let mut history = Vec::new();
    for exchange in &loop_state.artifacts.reviews {
        let feedback = read_project_relative_file(project_dir, &exchange.feedback)?;
        let response = read_project_relative_file(project_dir, &exchange.response)?;
        history.push(format!(
            "### Iteration {}\n\n#### Feedback\n\n{}\n\n#### Response\n\n{}",
            exchange.iteration, feedback, response
        ));
    }

    Ok(history.join("\n\n"))
}

fn collect_qa_history(state: &ProjectState, project_dir: &Path) -> Result<String> {
    let Some(loop_state) = state.current_feature_loop() else {
        return Ok(String::new());
    };

    let mut history = Vec::new();
    for exchange in &loop_state.artifacts.qa_results {
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
    vars.insert("spec_content".to_owned(), spec_content.to_owned());
    vars.insert(
        "impl_notes_content".to_owned(),
        impl_notes_content.to_owned(),
    );
    vars.insert("git_diff".to_owned(), git_diff.to_owned());
    vars.insert("qa_history".to_owned(), qa_history.to_owned());

    let rendered =
        render_template_with_fallback(&effective.templates.qa, &vars, default_qa_template())?;
    Ok(format!(
        "{rendered}\n\n## System Guardrails\n\n{QA_GUARDRAILS}\n\n## Master Prompt\n\n{prompt_content}\n\n## Feature Spec\n\n{spec_content}\n\n## Implementation Notes\n\n{impl_notes_content}\n\n## Current Diff\n\n```diff\n{git_diff}\n```\n\n## Prior QA History\n\n{}\n",
        if qa_history.is_empty() { "(none)" } else { qa_history }
    ))
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

fn persist_state_and_index(
    workspace: &mut Workspace,
    project_id: &str,
    project_dir: &Path,
    state: &ProjectState,
) -> Result<()> {
    save_project_state(project_dir, state)?;

    if let Some(project) = workspace.index.get_project_mut(project_id) {
        project.last_loop_number = state.last_loop_number();
        project.total_feature_loops = state
            .loops
            .iter()
            .filter(|loop_state| loop_state.status == LoopStatus::Completed)
            .count() as u32;
        project.total_completion_attempts = state
            .completion_attempts
            .iter()
            .filter(|attempt| attempt.status == LoopStatus::Completed)
            .count() as u32;

        project.status = match state.status {
            ProjectStatus::Pending => ProjectLifecycleStatus::Pending,
            ProjectStatus::InProgress => ProjectLifecycleStatus::InProgress,
            ProjectStatus::Completed => ProjectLifecycleStatus::Completed,
        };

        project.completed_at = if state.status == ProjectStatus::Completed {
            Some(Utc::now())
        } else {
            None
        };
    }

    workspace.save_index()?;
    Ok(())
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
## Tests Run\n\
## Verification Summary\n\
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

async fn execute_with_parse_retries<T, F>(
    backend: Arc<dyn Backend>,
    registry: &BackendRegistry,
    role: &str,
    phase: &str,
    original_prompt: &str,
    parse_fn: F,
    expected_format: &str,
) -> Result<T>
where
    F: Fn(&str) -> Result<T>,
{
    let first_output =
        execute_with_timeout_retries(backend.clone(), role, phase, original_prompt).await?;
    match parse_fn(&first_output) {
        Ok(parsed) => Ok(parsed),
        Err(parse_error_1) => {
            let reformatter_spec = registry
                .opposite(backend.name())
                .map(|opposite_name| {
                    registry.resolve_backend_for_role(opposite_name, "reformatter")
                })
                .unwrap_or_else(|_| backend.name().to_owned());
            let reformatter_backend = registry
                .get(&reformatter_spec)
                .unwrap_or_else(|| backend.clone());
            let reformatter_name = reformatter_backend.name().to_owned();

            warn!(
                role = role,
                backend = %reformatter_name,
                error = %parse_error_1,
                "parse failed, requesting reformat via {reformatter_name} (attempt 1/3)"
            );
            // Use ~~~ fences instead of --- to avoid triggering strip_frontmatter()
            let reformat_prompt = format!(
                "CRITICAL: Your previous response could not be parsed.\n\n\
                Error: {parse_error_1}\n\n\
                Your original response was:\n~~~\n{first_output}\n~~~\n\n\
                Requirements:\n\
                1. Your response MUST begin with the correct H1 heading as the VERY FIRST LINE\n\
                2. No preamble, commentary, or explanation before the H1\n\
                3. No YAML frontmatter (no lines starting with ---)\n\
                4. Include ALL required H2 sections\n\n\
                Required structure:\n{expected_format}\n\n\
                Respond ONLY with the corrected markdown. No explanation.\n",
            );

            let second_output =
                execute_with_timeout_retries(reformatter_backend, role, phase, &reformat_prompt)
                    .await?;
            if let Ok(parsed) = parse_fn(&second_output) {
                return Ok(parsed);
            }

            warn!(
                role = role,
                "reformat failed, retrying with format reminder (attempt 2/3)"
            );
            let reminded_prompt = format!(
                "IMPORTANT: Format your response as parseable markdown. \
                Your VERY FIRST LINE must be exactly:\n\n{expected_format}\n\n\
                No preamble. No commentary before the H1. No YAML frontmatter. \
                Include all required H2 sections.\n\n{original_prompt}",
            );
            let third_output =
                execute_with_timeout_retries(backend, role, phase, &reminded_prompt).await?;
            if let Ok(parsed) = parse_fn(&third_output) {
                return Ok(parsed);
            }

            warn!(role = role, "all parse retries exhausted (attempt 3/3)");
            Err(RalphError::ParseRetriesExhausted {
                role: role.to_owned(),
                phase: phase.to_owned(),
                attempts: 3,
            })
        }
    }
}

async fn execute_with_timeout_retries(
    backend: Arc<dyn Backend>,
    role: &str,
    phase: &str,
    prompt: &str,
) -> Result<String> {
    for attempt in 1..=3_u8 {
        match backend.execute(prompt).await {
            Ok(output) => return Ok(output),
            Err(RalphError::BackendTimeout {
                backend: backend_name,
            }) => {
                if attempt == 3 {
                    warn!(
                        role = role,
                        backend = %backend_name,
                        "backend timeout, retries exhausted"
                    );
                    return Err(RalphError::BackendTimeoutExhausted {
                        backend: backend_name,
                        phase: phase.to_owned(),
                        attempts: attempt,
                    });
                }
                let backoff = 2_u64.pow((attempt - 1) as u32);
                warn!(
                    role = role,
                    backend = %backend_name,
                    attempt = attempt,
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::{preload_role_model_backends, resolve_tmux_settings, validate_tmux_preflight};
    use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
    use crate::config::global::BackendRoleModels;
    use crate::config::GlobalConfig;
    use crate::error::RalphError;

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
            reformatter: Some("claude-reformatter".to_owned()),
        };
        config.backends.codex.models = BackendRoleModels {
            planner: Some("codex-planner".to_owned()),
            implementer: Some("codex-implementer".to_owned()),
            reviewer: Some("codex-reviewer".to_owned()),
            qa: Some("codex-qa".to_owned()),
            completer: Some("codex-completer".to_owned()),
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
            qa.contains("## Tests Run"),
            "qa template should contain Tests Run section; got: {qa}"
        );
        assert!(
            qa.contains("## Failures"),
            "qa template should contain Failures section; got: {qa}"
        );
    }
}
