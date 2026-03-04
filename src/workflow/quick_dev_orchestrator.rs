use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{info, warn};

use crate::backend::{Backend, BackendRegistry, BackendRegistryTmuxConfig};
use crate::config::{
    resolve_effective_config, EffectiveConfig, RunWorkflowOverrides,
};
use crate::error::RalphError;
use crate::git::commit::{
    changed_paths_excluding_prefixes, commit_and_push_phase_transition,
    working_tree_diff_excluding_orchestration_state,
};
use crate::git::is_git_repo;
use crate::output_log::LogWriter;
use crate::project::artifacts::{
    resolve_artifact_path_by_suffix, strip_backend_frontmatter, write_artifact,
    write_project_scoped_artifact, ArtifactKind, ArtifactWriteInput,
    ProjectScopedArtifactWriteInput,
};
use crate::project::lifecycle::reconstruct_project_state;
use crate::project::load_project_config_if_exists;
use crate::project::state::{Phase, ProjectState, ProjectStatus, QuickDevPhase};
use crate::prompts::quick_dev::{
    build_quick_dev_apply_fixes_prompt, build_quick_dev_codex_review_prompt,
    build_quick_dev_final_review_prompt, build_quick_dev_plan_implement_prompt,
};
use crate::util::lock::ProjectLock;
use crate::workflow::parser::{
    parse_codex_review_output, parse_quick_final_review_output, CodexReviewDecision,
    QuickFinalReviewDecision,
};
use crate::workspace::Workspace;
use crate::Result;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct QuickDevRunOptions {
    pub project: Option<String>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub pr_url: Option<String>,
    pub skip_commit: bool,
    pub max_review_iterations: Option<u32>,
    pub max_final_review_retries: Option<u32>,
}

const DEFAULT_MAX_REVIEW_ITERATIONS: u32 = 5;
const DEFAULT_MAX_FINAL_REVIEW_RETRIES: u32 = 2;

#[derive(Debug, Clone)]
pub struct OrchestrationResult {
    pub summary: String,
    pub loop_number: Option<u32>,
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

pub struct QuickDevOrchestrator {
    workspace: Workspace,
}

impl QuickDevOrchestrator {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }

    pub async fn run(&mut self, options: QuickDevRunOptions) -> Result<OrchestrationResult> {
        let project_id = self
            .workspace
            .resolve_project_id(options.project.as_deref())?;

        let project_dir = self.workspace.project_dir(&project_id);
        if !project_dir.exists() {
            return Err(RalphError::ProjectNotFound(project_id));
        }

        let _lock = ProjectLock::acquire(&project_dir, &project_id)?;

        let project_config = load_project_config_if_exists(&project_dir)?;
        let effective = resolve_effective_config(
            &self.workspace.root,
            &project_dir,
            self.workspace.config.clone(),
            project_config,
            RunWorkflowOverrides {
                implementer_backend: options.implementer_backend.as_deref(),
                reviewer_backend: options.reviewer_backend.as_deref(),
                ..Default::default()
            },
        )?;

        // --- Backend resolution ---
        let implementer_spec = resolve_implementer_backend(&options, &effective)?;
        let reviewer_spec = resolve_reviewer_backend(&options, &effective)?;
        validate_distinct_backends(&implementer_spec, &reviewer_spec)?;

        let mut registry = BackendRegistry::new(
            &effective.global,
            BackendRegistryTmuxConfig {
                enabled: false,
                session_name: String::new(),
                window_keep_seconds: effective.global.workspace.tmux_window_keep_seconds,
            },
        );

        // Health check both backends
        let _impl_backend = registry.get_or_create_for_role(&implementer_spec, "implementer")?;
        let _rev_backend = registry.get_or_create_for_role(&reviewer_spec, "reviewer")?;

        let mut state = reconstruct_project_state(&self.workspace, &project_id)?;

        // Propagate PR URL
        if let Some(ref url) = options.pr_url {
            if state.pr_url.as_ref() != Some(url) {
                state.pr_url = Some(url.clone());
            }
        }

        let repo_root: Option<PathBuf> = self.workspace.root.parent().map(|p| p.to_owned());
        let log_dir = self.workspace.root.join("tmp").join("logs");

        let max_review_iterations = options
            .max_review_iterations
            .unwrap_or(DEFAULT_MAX_REVIEW_ITERATIONS);
        let max_final_review_retries = options
            .max_final_review_retries
            .unwrap_or(DEFAULT_MAX_FINAL_REVIEW_RETRIES);

        // If the project was already completed (normal or force-complete),
        // do not restart from PlanAndImplement.
        if state.status == ProjectStatus::Completed && state.quick_dev_phase.is_none() {
            return Ok(OrchestrationResult {
                summary: "quick-dev already completed".to_owned(),
                loop_number: Some(state.current_loop.max(1)),
            });
        }

        // Determine the starting quick-dev phase from persisted state
        let starting_phase = state
            .quick_dev_phase
            .clone()
            .unwrap_or(QuickDevPhase::PlanAndImplement);

        // Ensure project has a loop for artifact writing
        let loop_number = if state.current_loop == 0 {
            let ln = state.next_loop_number();
            // Register a simple feature loop for artifact storage
            let now = chrono::Utc::now();
            state.register_feature_loop(
                ln,
                "quick-dev".to_owned(),
                "Quick Dev".to_owned(),
                crate::project::state::FeatureLoopBackends {
                    planner: implementer_spec.clone(),
                    implementer: implementer_spec.clone(),
                    reviewer: reviewer_spec.clone(),
                    qa: String::new(),
                },
                String::new(), // no spec artifact yet
                now,
            );
            ln
        } else {
            state.current_loop
        };

        let loop_slug = state
            .current_feature_loop()
            .map(|l| l.slug.clone())
            .unwrap_or_else(|| "quick-dev".to_owned());

        // Initial checkpoint: start -> PlanAndImplement (Planning -> Implementing).
        // Only emitted on fresh start when no persisted quick_dev_phase exists.
        if state.quick_dev_phase.is_none()
            && matches!(starting_phase, QuickDevPhase::PlanAndImplement)
        {
            checkpoint_if_enabled(
                &self.workspace,
                &project_id,
                loop_number,
                Phase::Planning,
                Phase::Implementing,
                effective.workflow.auto_commit,
                options.skip_commit,
                effective.global.git.sign_commits,
            )?;
        }

        let result = self
            .run_phase_machine(
                &mut state,
                &project_dir,
                &project_id,
                &log_dir,
                &effective,
                &mut registry,
                &implementer_spec,
                &reviewer_spec,
                starting_phase,
                loop_number,
                &loop_slug,
                max_review_iterations,
                max_final_review_retries,
                options.skip_commit,
                repo_root.as_deref(),
            )
            .await;

        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_phase_machine(
        &self,
        state: &mut ProjectState,
        project_dir: &Path,
        project_id: &str,
        log_dir: &Path,
        effective: &EffectiveConfig,
        registry: &mut BackendRegistry,
        implementer_spec: &str,
        reviewer_spec: &str,
        starting_phase: QuickDevPhase,
        loop_number: u32,
        loop_slug: &str,
        max_review_iterations: u32,
        max_final_review_retries: u32,
        skip_commit: bool,
        _repo_root: Option<&Path>,
    ) -> Result<OrchestrationResult> {
        let mut current_qd_phase = starting_phase;
        let mut review_iteration: u32 = state.quick_dev_review_iteration;
        let mut final_review_attempts: u32 = state.quick_dev_final_review_attempts;

        // Read prompt content
        let prompt_path = project_dir.join(&state.prompt_file);
        let prompt_content = if prompt_path.exists() {
            fs::read_to_string(&prompt_path).map_err(|e| {
                RalphError::Orchestration(format!(
                    "failed to read prompt file '{}': {e}",
                    prompt_path.display()
                ))
            })?
        } else {
            String::new()
        };

        // Read spec content if available
        let spec_content = state
            .current_feature_loop()
            .and_then(|l| {
                if l.artifacts.spec.is_empty() {
                    None
                } else {
                    let path = project_dir.join(&l.artifacts.spec);
                    fs::read_to_string(&path).ok()
                }
            })
            .unwrap_or_default();

        let mut last_review_feedback = String::new();

        // When resuming at ApplyFixes, reconstruct reviewer feedback from the
        // latest changes-requested artifact so the apply-fixes prompt is not
        // empty.  The artifact was written during the prior CodexReview phase
        // and persists on disk across process restarts.
        if matches!(current_qd_phase, QuickDevPhase::ApplyFixes) {
            last_review_feedback =
                load_latest_review_feedback(project_dir, loop_number, loop_slug);
        }

        // Phase machine loop (bounded to prevent infinite loops)
        for _step in 0..100 {
            // Persist state before executing phase action
            persist_quick_dev_state(
                state,
                &current_qd_phase,
                compute_phase_iteration(&current_qd_phase, review_iteration),
                review_iteration,
                final_review_attempts,
            );
            save_state_to_disk(state, project_dir)?;

            match current_qd_phase {
                QuickDevPhase::PlanAndImplement => {
                    info!(loop_number, "quick-dev: PlanAndImplement phase");

                    let git_diff = current_git_diff(&self.workspace.root)?;
                    let prompt = build_plan_implement_prompt(
                        effective,
                        &prompt_content,
                        &spec_content,
                        &git_diff,
                    )?;

                    let impl_backend =
                        registry.get_or_create_for_role(implementer_spec, "implementer")?;

                    let mut impl_log =
                        LogWriter::open(log_dir, project_id, Some(loop_number), "quick-dev-impl");
                    let raw = execute_backend(
                        impl_backend.clone(),
                        &prompt,
                        &mut impl_log,
                        registry.timeout_for_role(implementer_spec, "implementer").as_secs(),
                    )
                    .await?;

                    // Write artifact
                    write_artifact(
                        project_dir,
                        ArtifactWriteInput {
                            project_id,
                            loop_number,
                            loop_slug,
                            backend: implementer_spec,
                            role: "implementer",
                            kind: ArtifactKind::QuickDevPlanImplement,
                            body: &raw,
                        },
                    )?;

                    // Transition: PlanAndImplement -> CodexReview
                    let from_phase = Phase::Implementing;
                    let to_phase = Phase::Reviewing;
                    checkpoint_if_enabled(
                        &self.workspace,
                        project_id,
                        loop_number,
                        from_phase,
                        to_phase,
                        effective.workflow.auto_commit,
                        skip_commit,
                        effective.global.git.sign_commits,
                    )?;

                    review_iteration = 0;
                    current_qd_phase = QuickDevPhase::CodexReview;
                }

                QuickDevPhase::CodexReview => {
                    info!(
                        loop_number,
                        review_iteration, "quick-dev: CodexReview phase"
                    );

                    let git_diff = current_git_diff(&self.workspace.root)?;
                    let prompt = build_codex_review_prompt(
                        effective,
                        &prompt_content,
                        &spec_content,
                        &git_diff,
                    )?;

                    let rev_backend =
                        registry.get_or_create_for_role(reviewer_spec, "reviewer")?;

                    let mut rev_log =
                        LogWriter::open(log_dir, project_id, Some(loop_number), "quick-dev-review");
                    let raw = execute_backend(
                        rev_backend.clone(),
                        &prompt,
                        &mut rev_log,
                        registry.timeout_for_role(reviewer_spec, "reviewer").as_secs(),
                    )
                    .await?;

                    let decision = parse_codex_review_output(&raw)?;

                    match decision {
                        CodexReviewDecision::ReviewSatisfied { body } => {
                            info!(loop_number, "quick-dev: review satisfied");
                            write_artifact(
                                project_dir,
                                ArtifactWriteInput {
                                    project_id,
                                    loop_number,
                                    loop_slug,
                                    backend: reviewer_spec,
                                    role: "reviewer",
                                    kind: ArtifactKind::QuickDevCodexReview { satisfied: true },
                                    body: &body,
                                },
                            )?;

                            // Transition: CodexReview -> FinalReview
                            let from_phase = Phase::Reviewing;
                            let to_phase = Phase::FinalReview;
                            checkpoint_if_enabled(
                                &self.workspace,
                                project_id,
                                loop_number,
                                from_phase,
                                to_phase,
                                effective.workflow.auto_commit,
                                skip_commit,
                                effective.global.git.sign_commits,
                            )?;

                            current_qd_phase = QuickDevPhase::FinalReview;
                        }
                        CodexReviewDecision::ChangesRequested { body } => {
                            info!(loop_number, "quick-dev: changes requested");
                            write_artifact(
                                project_dir,
                                ArtifactWriteInput {
                                    project_id,
                                    loop_number,
                                    loop_slug,
                                    backend: reviewer_spec,
                                    role: "reviewer",
                                    kind: ArtifactKind::QuickDevCodexReview { satisfied: false },
                                    body: &body,
                                },
                            )?;

                            last_review_feedback = body;
                            review_iteration += 1;

                            // Guard: max review iterations
                            if review_iteration >= max_review_iterations {
                                warn!(
                                    loop_number,
                                    review_iteration,
                                    max_review_iterations,
                                    "quick-dev: max review iterations reached, skipping to FinalReview"
                                );

                                // Write warning artifact
                                write_project_scoped_artifact(
                                    project_dir,
                                    ProjectScopedArtifactWriteInput {
                                        artifact: "quick-dev-review-limit-warning",
                                        file_name: "quick-dev-review-limit-warning.md",
                                        project_id,
                                        backend: reviewer_spec,
                                        role: "reviewer",
                                        body: &format!(
                                            "# Review Iteration Limit Reached\n\nMax review iterations ({}) reached. Skipping to FinalReview.",
                                            max_review_iterations
                                        ),
                                    },
                                )?;

                                // Persist target phase before transition
                                persist_quick_dev_state(
                                    state,
                                    &QuickDevPhase::FinalReview,
                                    1,
                                    review_iteration,
                                    final_review_attempts,
                                );
                                save_state_to_disk(state, project_dir)?;

                                let from_phase = Phase::Reviewing;
                                let to_phase = Phase::FinalReview;
                                checkpoint_if_enabled(
                                    &self.workspace,
                                    project_id,
                                    loop_number,
                                    from_phase,
                                    to_phase,
                                    effective.workflow.auto_commit,
                                    skip_commit,
                                    effective.global.git.sign_commits,
                                )?;

                                current_qd_phase = QuickDevPhase::FinalReview;
                                continue;
                            }

                            // Transition: CodexReview -> ApplyFixes
                            let from_phase = Phase::Reviewing;
                            let to_phase = Phase::Implementing;
                            checkpoint_if_enabled(
                                &self.workspace,
                                project_id,
                                loop_number,
                                from_phase,
                                to_phase,
                                effective.workflow.auto_commit,
                                skip_commit,
                                effective.global.git.sign_commits,
                            )?;

                            current_qd_phase = QuickDevPhase::ApplyFixes;
                        }
                    }
                }

                QuickDevPhase::ApplyFixes => {
                    info!(
                        loop_number,
                        review_iteration, "quick-dev: ApplyFixes phase"
                    );

                    let git_diff = current_git_diff(&self.workspace.root)?;
                    let prompt = build_apply_fixes_prompt(
                        effective,
                        &prompt_content,
                        &spec_content,
                        &last_review_feedback,
                        &git_diff,
                    )?;

                    let impl_backend =
                        registry.get_or_create_for_role(implementer_spec, "implementer")?;

                    let mut impl_log = LogWriter::open(
                        log_dir,
                        project_id,
                        Some(loop_number),
                        &format!("quick-dev-fix-{review_iteration:03}"),
                    );
                    let raw = execute_backend(
                        impl_backend.clone(),
                        &prompt,
                        &mut impl_log,
                        registry.timeout_for_role(implementer_spec, "implementer").as_secs(),
                    )
                    .await?;

                    write_artifact(
                        project_dir,
                        ArtifactWriteInput {
                            project_id,
                            loop_number,
                            loop_slug,
                            backend: implementer_spec,
                            role: "implementer",
                            kind: ArtifactKind::QuickDevApplyFixes {
                                iteration: review_iteration,
                            },
                            body: &raw,
                        },
                    )?;

                    // Transition: ApplyFixes -> CodexReview
                    let from_phase = Phase::Implementing;
                    let to_phase = Phase::Reviewing;
                    checkpoint_if_enabled(
                        &self.workspace,
                        project_id,
                        loop_number,
                        from_phase,
                        to_phase,
                        effective.workflow.auto_commit,
                        skip_commit,
                        effective.global.git.sign_commits,
                    )?;

                    current_qd_phase = QuickDevPhase::CodexReview;
                }

                QuickDevPhase::FinalReview => {
                    info!(
                        loop_number,
                        final_review_attempts, "quick-dev: FinalReview phase"
                    );

                    let git_diff = current_git_diff(&self.workspace.root)?;

                    // Two sequential independent calls (implementer then reviewer)
                    // Each with fresh context (no session reuse)

                    // --- Implementer final review ---
                    let impl_final_prompt = build_final_review_prompt(
                        effective,
                        &prompt_content,
                        &spec_content,
                        &git_diff,
                    )?;
                    let impl_backend =
                        registry.get_or_create_for_role(implementer_spec, "implementer")?;
                    let mut impl_fr_log = LogWriter::open(
                        log_dir,
                        project_id,
                        Some(loop_number),
                        &format!("quick-dev-final-review-impl-{final_review_attempts:03}"),
                    );
                    let impl_raw = execute_backend(
                        impl_backend.clone(),
                        &impl_final_prompt,
                        &mut impl_fr_log,
                        registry.timeout_for_role(implementer_spec, "implementer").as_secs(),
                    )
                    .await?;
                    let impl_decision = parse_quick_final_review_output(&impl_raw)?;

                    let impl_complete =
                        matches!(impl_decision, QuickFinalReviewDecision::Complete { .. });
                    let impl_body = match &impl_decision {
                        QuickFinalReviewDecision::Complete { body } => body.clone(),
                        QuickFinalReviewDecision::IssuesFound { body } => body.clone(),
                    };
                    write_artifact(
                        project_dir,
                        ArtifactWriteInput {
                            project_id,
                            loop_number,
                            loop_slug,
                            backend: implementer_spec,
                            role: "implementer",
                            kind: ArtifactKind::QuickDevFinalReview {
                                role: "implementer".to_owned(),
                                complete: impl_complete,
                            },
                            body: &impl_body,
                        },
                    )?;

                    // --- Reviewer final review (fresh context) ---
                    let rev_final_prompt = build_final_review_prompt(
                        effective,
                        &prompt_content,
                        &spec_content,
                        &git_diff,
                    )?;
                    let rev_backend =
                        registry.get_or_create_for_role(reviewer_spec, "reviewer")?;
                    let mut rev_fr_log = LogWriter::open(
                        log_dir,
                        project_id,
                        Some(loop_number),
                        &format!("quick-dev-final-review-rev-{final_review_attempts:03}"),
                    );
                    let rev_raw = execute_backend(
                        rev_backend.clone(),
                        &rev_final_prompt,
                        &mut rev_fr_log,
                        registry.timeout_for_role(reviewer_spec, "reviewer").as_secs(),
                    )
                    .await?;
                    let rev_decision = parse_quick_final_review_output(&rev_raw)?;

                    let rev_complete =
                        matches!(rev_decision, QuickFinalReviewDecision::Complete { .. });
                    let rev_body = match &rev_decision {
                        QuickFinalReviewDecision::Complete { body } => body.clone(),
                        QuickFinalReviewDecision::IssuesFound { body } => body.clone(),
                    };
                    write_artifact(
                        project_dir,
                        ArtifactWriteInput {
                            project_id,
                            loop_number,
                            loop_slug,
                            backend: reviewer_spec,
                            role: "reviewer",
                            kind: ArtifactKind::QuickDevFinalReview {
                                role: "reviewer".to_owned(),
                                complete: rev_complete,
                            },
                            body: &rev_body,
                        },
                    )?;

                    if impl_complete && rev_complete {
                        // Both complete -> mark project completed
                        info!(loop_number, "quick-dev: both final reviews COMPLETE");
                        state.status = ProjectStatus::Completed;
                        state.current_phase = Phase::Completing;
                        state.quick_dev_phase = None;
                        save_state_to_disk(state, project_dir)?;

                        let from_phase = Phase::FinalReview;
                        let to_phase = Phase::Completing;
                        checkpoint_if_enabled(
                            &self.workspace,
                            project_id,
                            loop_number,
                            from_phase,
                            to_phase,
                            effective.workflow.auto_commit,
                            skip_commit,
                            effective.global.git.sign_commits,
                        )?;

                        return Ok(OrchestrationResult {
                            summary: "quick-dev completed successfully".to_owned(),
                            loop_number: Some(loop_number),
                        });
                    }

                    // Issues found: increment counter and check guard
                    final_review_attempts += 1;

                    if final_review_attempts >= max_final_review_retries {
                        warn!(
                            loop_number,
                            final_review_attempts,
                            max_final_review_retries,
                            "quick-dev: max final review retries reached, force-completing"
                        );

                        // Write force-complete artifact
                        write_project_scoped_artifact(
                            project_dir,
                            ProjectScopedArtifactWriteInput {
                                artifact: "quick-dev-force-complete",
                                file_name: "quick-dev-force-complete.md",
                                project_id,
                                backend: implementer_spec,
                                role: "orchestrator",
                                body: &format!(
                                    "# Quick-Dev Force Complete\n\nMax final review retries ({}) reached. Force-completing project.",
                                    max_final_review_retries
                                ),
                            },
                        )?;

                        state.status = ProjectStatus::Completed;
                        state.current_phase = Phase::Completing;
                        state.quick_dev_phase = None;
                        save_state_to_disk(state, project_dir)?;

                        let from_phase = Phase::FinalReview;
                        let to_phase = Phase::Completing;
                        checkpoint_if_enabled(
                            &self.workspace,
                            project_id,
                            loop_number,
                            from_phase,
                            to_phase,
                            effective.workflow.auto_commit,
                            skip_commit,
                            effective.global.git.sign_commits,
                        )?;

                        return Ok(OrchestrationResult {
                            summary: format!(
                                "quick-dev force-completed after {} final review retries",
                                max_final_review_retries
                            ),
                            loop_number: Some(loop_number),
                        });
                    }

                    // Transition: FinalReview -> PlanAndImplement (reloop)
                    info!(
                        loop_number,
                        final_review_attempts,
                        "quick-dev: issues found in final review, re-entering PlanAndImplement"
                    );

                    let from_phase = Phase::FinalReview;
                    let to_phase = Phase::Implementing;
                    checkpoint_if_enabled(
                        &self.workspace,
                        project_id,
                        loop_number,
                        from_phase,
                        to_phase,
                        effective.workflow.auto_commit,
                        skip_commit,
                        effective.global.git.sign_commits,
                    )?;

                    current_qd_phase = QuickDevPhase::PlanAndImplement;
                }
            }
        }

        Err(RalphError::Orchestration(
            "quick-dev: exceeded maximum phase transitions (100)".to_owned(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Backend resolution helpers
// ---------------------------------------------------------------------------

fn resolve_implementer_backend(
    options: &QuickDevRunOptions,
    effective: &EffectiveConfig,
) -> Result<String> {
    if let Some(ref spec) = options.implementer_backend {
        return Ok(spec.clone());
    }
    if let Some(ref spec) = effective.workflow.implementer_backend {
        return Ok(spec.clone());
    }
    Ok(effective.workflow.starting_backend.clone())
}

fn resolve_reviewer_backend(
    options: &QuickDevRunOptions,
    effective: &EffectiveConfig,
) -> Result<String> {
    if let Some(ref spec) = options.reviewer_backend {
        return Ok(spec.clone());
    }
    if let Some(ref spec) = effective.workflow.reviewer_backend {
        return Ok(spec.clone());
    }
    Err(RalphError::Validation(
        "quick-dev requires a second backend for review".to_owned(),
    ))
}

fn validate_distinct_backends(implementer: &str, reviewer: &str) -> Result<()> {
    if implementer == reviewer {
        return Err(RalphError::Validation(format!(
            "quick-dev requires distinct implementer and reviewer backends, but both resolved to '{implementer}'"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// State persistence helpers
// ---------------------------------------------------------------------------

fn persist_quick_dev_state(
    state: &mut ProjectState,
    phase: &QuickDevPhase,
    phase_iteration: u32,
    review_iteration: u32,
    final_review_attempts: u32,
) {
    state.quick_dev_phase = Some(phase.clone());
    state.current_phase = phase.to_current_phase();
    state.phase_iteration = phase_iteration;
    state.status = ProjectStatus::InProgress;
    state.quick_dev_review_iteration = review_iteration;
    state.quick_dev_final_review_attempts = final_review_attempts;
}

fn compute_phase_iteration(phase: &QuickDevPhase, review_iteration: u32) -> u32 {
    match phase {
        QuickDevPhase::PlanAndImplement
        | QuickDevPhase::CodexReview
        | QuickDevPhase::FinalReview => 1,
        QuickDevPhase::ApplyFixes => review_iteration.max(1),
    }
}

/// Load the body of the latest `quick-dev-codex-review-changes-requested.md`
/// artifact for the given loop.  Used to reconstruct reviewer feedback when
/// resuming at the `ApplyFixes` phase after a process restart.
fn load_latest_review_feedback(project_dir: &Path, loop_number: u32, loop_slug: &str) -> String {
    let suffix = ArtifactKind::QuickDevCodexReview { satisfied: false }.file_name();
    let artifact_rel = match resolve_artifact_path_by_suffix(
        project_dir,
        loop_number,
        loop_slug,
        &suffix,
    ) {
        Ok(Some(rel)) => rel,
        _ => return String::new(),
    };
    let artifact_path = project_dir.join(&artifact_rel);
    match fs::read_to_string(&artifact_path) {
        Ok(content) => {
            // Strip frontmatter (between leading `---` lines) to get the body.
            strip_backend_frontmatter(&content)
        }
        Err(_) => String::new(),
    }
}

fn save_state_to_disk(state: &ProjectState, project_dir: &Path) -> Result<()> {
    let state_path = project_dir.join("state.json");
    let content = serde_json::to_string_pretty(state)?;
    fs::write(&state_path, content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

fn current_git_diff(workspace_root: &Path) -> Result<String> {
    let Some(repo_root) = workspace_root.parent() else {
        return Ok(String::new());
    };
    if !is_git_repo(repo_root) {
        return Ok(String::new());
    }
    working_tree_diff_excluding_orchestration_state(repo_root)
}

fn checkpoint_if_enabled(
    workspace: &Workspace,
    project_id: &str,
    loop_number: u32,
    from_phase: Phase,
    to_phase: Phase,
    auto_commit: bool,
    skip_commit: bool,
    sign_commits: bool,
) -> Result<()> {
    if !auto_commit || skip_commit {
        return Ok(());
    }
    let Some(repo_root) = workspace.root.parent() else {
        return Ok(());
    };
    if !is_git_repo(repo_root) {
        return Ok(());
    }

    // Check if there are actual changes to commit beyond orchestration state
    // (no empty transition commits when only .ralph/ changed)
    let changed = changed_paths_excluding_prefixes(
        repo_root,
        &[crate::git::commit::ORCHESTRATION_STATE_PATH_PREFIX],
    )?;
    if changed.is_empty() {
        return Ok(());
    }

    let branch =
        crate::git::branch::resolve_branch_name(&workspace.config.git.branch_format, project_id);
    commit_and_push_phase_transition(
        repo_root,
        project_id,
        loop_number,
        from_phase,
        to_phase,
        &branch,
        sign_commits,
    )
}

// ---------------------------------------------------------------------------
// Prompt builders (thin wrappers)
// ---------------------------------------------------------------------------

fn build_plan_implement_prompt(
    effective: &EffectiveConfig,
    prompt_content: &str,
    spec_content: &str,
    git_diff: &str,
) -> Result<String> {
    let mut vars = BTreeMap::new();
    vars.insert("system_guardrails".to_owned(), QUICK_DEV_IMPLEMENTER_GUARDRAILS.to_owned());
    vars.insert("feature_spec".to_owned(), spec_content.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert("current_diff".to_owned(), git_diff.to_owned());
    build_quick_dev_plan_implement_prompt(&effective.templates.quick_dev_plan_implement, &vars)
}

fn build_codex_review_prompt(
    effective: &EffectiveConfig,
    prompt_content: &str,
    spec_content: &str,
    git_diff: &str,
) -> Result<String> {
    let mut vars = BTreeMap::new();
    vars.insert("system_guardrails".to_owned(), QUICK_DEV_REVIEWER_GUARDRAILS.to_owned());
    vars.insert("feature_spec".to_owned(), spec_content.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert("current_diff".to_owned(), git_diff.to_owned());
    build_quick_dev_codex_review_prompt(&effective.templates.quick_dev_codex_review, &vars)
}

fn build_apply_fixes_prompt(
    effective: &EffectiveConfig,
    prompt_content: &str,
    spec_content: &str,
    review_feedback: &str,
    git_diff: &str,
) -> Result<String> {
    let mut vars = BTreeMap::new();
    vars.insert("system_guardrails".to_owned(), QUICK_DEV_IMPLEMENTER_GUARDRAILS.to_owned());
    vars.insert("feature_spec".to_owned(), spec_content.to_owned());
    vars.insert("review_feedback".to_owned(), review_feedback.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert("current_diff".to_owned(), git_diff.to_owned());
    build_quick_dev_apply_fixes_prompt(&effective.templates.quick_dev_apply_fixes, &vars)
}

fn build_final_review_prompt(
    effective: &EffectiveConfig,
    prompt_content: &str,
    spec_content: &str,
    git_diff: &str,
) -> Result<String> {
    let mut vars = BTreeMap::new();
    vars.insert("system_guardrails".to_owned(), QUICK_DEV_REVIEWER_GUARDRAILS.to_owned());
    vars.insert("feature_spec".to_owned(), spec_content.to_owned());
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());
    vars.insert("current_diff".to_owned(), git_diff.to_owned());
    build_quick_dev_final_review_prompt(&effective.templates.quick_dev_final_review, &vars)
}

// ---------------------------------------------------------------------------
// Backend execution
// ---------------------------------------------------------------------------

async fn execute_backend(
    backend: Arc<dyn Backend>,
    prompt: &str,
    log_writer: &mut LogWriter,
    timeout_secs: u64,
) -> Result<String> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        backend.execute_with_log(prompt, Some(log_writer)),
    )
    .await;

    match result {
        Ok(inner) => inner,
        Err(_) => Err(RalphError::BackendTimeout {
            backend: backend.name().to_owned(),
            idle_seconds: timeout_secs,
            timeout_kind: crate::error::TimeoutKind::Walltime,
        }),
    }
}

// ---------------------------------------------------------------------------
// Guardrails
// ---------------------------------------------------------------------------

const QUICK_DEV_IMPLEMENTER_GUARDRAILS: &str = r#"- Keep edits scoped to this loop's feature and acceptance criteria.
- In review responses, address each required change explicitly.
- If a required change is already satisfied, cite concrete evidence (files/tests) instead of unrelated edits."#;

const QUICK_DEV_REVIEWER_GUARDRAILS: &str = r#"- Treat `.ralph/**` as orchestration runtime state; it is out of scope for feature review.
- Focus on acceptance criteria and actual behavior, not whether code was first introduced in this loop.
- If criteria are already satisfied and no code change is required, return the satisfied verdict with evidence."#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::state::{Phase, ProjectState, QuickDevPhase};

    #[test]
    fn resolve_implementer_backend_cli_override_takes_precedence() {
        let options = QuickDevRunOptions {
            project: None,
            implementer_backend: Some("cli-impl".to_owned()),
            reviewer_backend: None,
            pr_url: None,
            skip_commit: false,
            max_review_iterations: None,
            max_final_review_retries: None,
        };
        let effective = make_test_effective(
            Some("eff-impl".to_owned()),
            Some("eff-rev".to_owned()),
            "starting".to_owned(),
        );
        let result = resolve_implementer_backend(&options, &effective).unwrap();
        assert_eq!(result, "cli-impl");
    }

    #[test]
    fn resolve_implementer_backend_effective_fallback() {
        let options = QuickDevRunOptions {
            project: None,
            implementer_backend: None,
            reviewer_backend: None,
            pr_url: None,
            skip_commit: false,
            max_review_iterations: None,
            max_final_review_retries: None,
        };
        let effective = make_test_effective(
            Some("eff-impl".to_owned()),
            Some("eff-rev".to_owned()),
            "starting".to_owned(),
        );
        let result = resolve_implementer_backend(&options, &effective).unwrap();
        assert_eq!(result, "eff-impl");
    }

    #[test]
    fn resolve_implementer_backend_starting_fallback() {
        let options = QuickDevRunOptions {
            project: None,
            implementer_backend: None,
            reviewer_backend: None,
            pr_url: None,
            skip_commit: false,
            max_review_iterations: None,
            max_final_review_retries: None,
        };
        let effective = make_test_effective(None, Some("eff-rev".to_owned()), "starting".to_owned());
        let result = resolve_implementer_backend(&options, &effective).unwrap();
        assert_eq!(result, "starting");
    }

    #[test]
    fn resolve_reviewer_backend_cli_override() {
        let options = QuickDevRunOptions {
            project: None,
            implementer_backend: None,
            reviewer_backend: Some("cli-rev".to_owned()),
            pr_url: None,
            skip_commit: false,
            max_review_iterations: None,
            max_final_review_retries: None,
        };
        let effective =
            make_test_effective(None, Some("eff-rev".to_owned()), "starting".to_owned());
        let result = resolve_reviewer_backend(&options, &effective).unwrap();
        assert_eq!(result, "cli-rev");
    }

    #[test]
    fn resolve_reviewer_backend_effective_fallback() {
        let options = QuickDevRunOptions {
            project: None,
            implementer_backend: None,
            reviewer_backend: None,
            pr_url: None,
            skip_commit: false,
            max_review_iterations: None,
            max_final_review_retries: None,
        };
        let effective =
            make_test_effective(None, Some("eff-rev".to_owned()), "starting".to_owned());
        let result = resolve_reviewer_backend(&options, &effective).unwrap();
        assert_eq!(result, "eff-rev");
    }

    #[test]
    fn resolve_reviewer_backend_missing_fails_with_exact_message() {
        let options = QuickDevRunOptions {
            project: None,
            implementer_backend: None,
            reviewer_backend: None,
            pr_url: None,
            skip_commit: false,
            max_review_iterations: None,
            max_final_review_retries: None,
        };
        let effective = make_test_effective(None, None, "starting".to_owned());
        let err = resolve_reviewer_backend(&options, &effective).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid input: quick-dev requires a second backend for review"
        );
    }

    #[test]
    fn validate_distinct_backends_rejects_equal() {
        let err = validate_distinct_backends("claude(opus)", "claude(opus)").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("distinct"));
        assert!(msg.contains("claude(opus)"));
    }

    #[test]
    fn validate_distinct_backends_accepts_different() {
        validate_distinct_backends("claude(opus)", "codex(gpt-5)").unwrap();
    }

    #[test]
    fn persist_quick_dev_state_sets_fields_correctly() {
        let mut state = ProjectState::new("test", "Test", "hash", None);
        persist_quick_dev_state(&mut state, &QuickDevPhase::CodexReview, 1, 2, 1);

        assert_eq!(state.quick_dev_phase, Some(QuickDevPhase::CodexReview));
        assert_eq!(state.current_phase, Phase::Reviewing);
        assert_eq!(state.phase_iteration, 1);
        assert_eq!(state.status, ProjectStatus::InProgress);
        assert_eq!(state.quick_dev_review_iteration, 2);
        assert_eq!(state.quick_dev_final_review_attempts, 1);
    }

    #[test]
    fn persist_quick_dev_state_plan_and_implement() {
        let mut state = ProjectState::new("test", "Test", "hash", None);
        persist_quick_dev_state(&mut state, &QuickDevPhase::PlanAndImplement, 1, 0, 0);

        assert_eq!(
            state.quick_dev_phase,
            Some(QuickDevPhase::PlanAndImplement)
        );
        assert_eq!(state.current_phase, Phase::Implementing);
    }

    #[test]
    fn persist_quick_dev_state_apply_fixes() {
        let mut state = ProjectState::new("test", "Test", "hash", None);
        persist_quick_dev_state(&mut state, &QuickDevPhase::ApplyFixes, 3, 3, 0);

        assert_eq!(state.quick_dev_phase, Some(QuickDevPhase::ApplyFixes));
        assert_eq!(state.current_phase, Phase::Implementing);
        assert_eq!(state.phase_iteration, 3);
    }

    #[test]
    fn persist_quick_dev_state_final_review() {
        let mut state = ProjectState::new("test", "Test", "hash", None);
        persist_quick_dev_state(&mut state, &QuickDevPhase::FinalReview, 1, 0, 0);

        assert_eq!(state.quick_dev_phase, Some(QuickDevPhase::FinalReview));
        assert_eq!(state.current_phase, Phase::FinalReview);
    }

    #[test]
    fn compute_phase_iteration_returns_correct_values() {
        assert_eq!(compute_phase_iteration(&QuickDevPhase::PlanAndImplement, 0), 1);
        assert_eq!(compute_phase_iteration(&QuickDevPhase::CodexReview, 5), 1);
        assert_eq!(compute_phase_iteration(&QuickDevPhase::FinalReview, 3), 1);
        assert_eq!(compute_phase_iteration(&QuickDevPhase::ApplyFixes, 3), 3);
        assert_eq!(compute_phase_iteration(&QuickDevPhase::ApplyFixes, 0), 1);
    }

    #[test]
    fn default_max_values() {
        assert_eq!(DEFAULT_MAX_REVIEW_ITERATIONS, 5);
        assert_eq!(DEFAULT_MAX_FINAL_REVIEW_RETRIES, 2);
    }

    #[test]
    fn save_state_to_disk_roundtrips() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut state = ProjectState::new("test", "Test", "hash", None);
        state.quick_dev_phase = Some(QuickDevPhase::CodexReview);

        save_state_to_disk(&state, temp.path()).unwrap();

        let content = fs::read_to_string(temp.path().join("state.json")).unwrap();
        let loaded: ProjectState = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.quick_dev_phase, Some(QuickDevPhase::CodexReview));
        assert_eq!(loaded.project_id, "test");
    }

    #[test]
    fn resume_from_none_starts_at_plan_and_implement() {
        let state = ProjectState::new("test", "Test", "hash", None);
        let starting = state
            .quick_dev_phase
            .clone()
            .unwrap_or(QuickDevPhase::PlanAndImplement);
        assert_eq!(starting, QuickDevPhase::PlanAndImplement);
    }

    #[test]
    fn resume_from_codex_review() {
        let mut state = ProjectState::new("test", "Test", "hash", None);
        state.quick_dev_phase = Some(QuickDevPhase::CodexReview);
        let starting = state
            .quick_dev_phase
            .clone()
            .unwrap_or(QuickDevPhase::PlanAndImplement);
        assert_eq!(starting, QuickDevPhase::CodexReview);
    }

    #[test]
    fn resume_from_final_review() {
        let mut state = ProjectState::new("test", "Test", "hash", None);
        state.quick_dev_phase = Some(QuickDevPhase::FinalReview);
        let starting = state
            .quick_dev_phase
            .clone()
            .unwrap_or(QuickDevPhase::PlanAndImplement);
        assert_eq!(starting, QuickDevPhase::FinalReview);
    }

    #[test]
    fn quick_dev_phase_display() {
        assert_eq!(QuickDevPhase::PlanAndImplement.to_string(), "plan_and_implement");
        assert_eq!(QuickDevPhase::CodexReview.to_string(), "codex_review");
        assert_eq!(QuickDevPhase::ApplyFixes.to_string(), "apply_fixes");
        assert_eq!(QuickDevPhase::FinalReview.to_string(), "final_review");
    }

    #[test]
    fn quick_dev_phase_to_current_phase_mapping() {
        assert_eq!(QuickDevPhase::PlanAndImplement.to_current_phase(), Phase::Implementing);
        assert_eq!(QuickDevPhase::CodexReview.to_current_phase(), Phase::Reviewing);
        assert_eq!(QuickDevPhase::ApplyFixes.to_current_phase(), Phase::Implementing);
        assert_eq!(QuickDevPhase::FinalReview.to_current_phase(), Phase::FinalReview);
    }

    // Helper to build a minimal EffectiveConfig for testing
    fn make_test_effective(
        implementer: Option<String>,
        reviewer: Option<String>,
        starting: String,
    ) -> EffectiveConfig {
        use crate::config::global::GlobalConfig;
        use crate::config::{
            EffectiveDaemonConfig, EffectiveTemplateConfig, EffectiveWorkflowConfig,
        };
        use std::path::PathBuf;

        EffectiveConfig {
            workflow: EffectiveWorkflowConfig {
                starting_backend: starting,
                prompt_review_enabled: false,
                prompt_review_backends: vec![],
                prompt_review_min_reviewers: 0,
                planner_backend: None,
                implementer_backend: implementer,
                reviewer_backend: reviewer,
                qa_backend: None,
                completer_backend: None,
                final_review_enabled: false,
                final_review_backends: vec![],
                final_review_arbiter_backend: String::new(),
                final_review_min_reviewers: 0,
                final_review_consensus_threshold: 0.0,
                max_final_review_restarts: 0,
                completion_backends: vec![],
                completion_min_completers: 0,
                completion_consensus_threshold: 0.0,
                qa_enabled: false,
                max_qa_iterations: 0,
                max_review_iterations: 5,
                auto_commit: false,
                commit_message_style: crate::config::CommitMessageStyle::default(),
                commit_tag_format: String::new(),
                prompt_change_action: crate::config::PromptChangeAction::default(),
                planner_state_in_prompt: crate::config::PlannerStateInPrompt::default(),
                planner_previous_specs_in_prompt:
                    crate::config::PreviousSpecsInPrompt::default(),
                planner_max_prior_loops: None,
                max_review_history_entries_in_prompt: 0,
                max_qa_history_entries_in_prompt: 0,
                include_history_when_session_reuse_enabled: false,
                session_reuse_enabled: false,
                session_reuse_roles: vec![],
                session_reuse_reset_on_prompt_change: false,
                session_reuse_reset_on_rollback: false,
            },
            templates: EffectiveTemplateConfig {
                planner: PathBuf::new(),
                implementer: PathBuf::new(),
                reviewer: PathBuf::new(),
                prompt_reviewer: PathBuf::new(),
                prompt_review_validator: PathBuf::new(),
                completer: PathBuf::new(),
                qa: PathBuf::new(),
                final_reviewer: PathBuf::new(),
                quick_dev_plan_implement: PathBuf::new(),
                quick_dev_codex_review: PathBuf::new(),
                quick_dev_apply_fixes: PathBuf::new(),
                quick_dev_final_review: PathBuf::new(),
                planner_position: PathBuf::new(),
                vote: PathBuf::new(),
                arbiter: PathBuf::new(),
            },
            daemon: EffectiveDaemonConfig {
                poll_seconds: 30,
                max_concurrent: 1,
                labels: vec![],
                repo: None,
                refinement_enabled: false,
                refinement_backend: String::new(),
                auto_rebase_enabled: false,
                rebase_interval_seconds: 0,
                max_rebases_per_cycle: 0,
                rebase_timeout_seconds: 0,
                rebase_agent_backend: String::new(),
                prd_enabled: false,
                prd_question_backends: vec![],
                prd_writer_backend: String::new(),
                prd_reviewer_backend: String::new(),
                prd_max_revisions: 0,
                prd_backend_timeout_secs: 0,
                prd_shutdown_timeout_secs: 0,
            },
            global: GlobalConfig::default(),
            project: None,
        }
    }
}
