//! Library entry points for in-process daemon task dispatch.
//!
//! Each function mirrors the corresponding CLI `execute()` handler but takes
//! explicit parameters (no CLI arg parsing, no `current_dir()`).  Output goes
//! through `tracing` events routed to a per-task file subscriber via
//! `WithSubscriber`, ensuring log isolation across concurrent tasks.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::instrument::WithSubscriber;
use tracing_subscriber::fmt;

use crate::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use crate::cli::backend_spec;
use crate::config::{validate_required_backend_spec, PromptChangeAction};
use crate::error::RalphError;
use crate::prd::quick::{QuickPrdOptions, QuickPrdPipeline};
use crate::project::lifecycle::{create_project, CreateProjectOptions, PromptSource};
use crate::workflow::orchestrator::{Orchestrator, OrchestrationResult, RunOptions};
use crate::workflow::quick_dev_orchestrator::{
    self, QuickDevOrchestrator, QuickDevRunOptions,
};
use crate::workspace::Workspace;
use crate::Result;

// ---------------------------------------------------------------------------
// Task parameter structs
// ---------------------------------------------------------------------------

/// Parameters for the `auto` dispatch variant (fresh project with quick-prd).
///
/// Both daemon and CLI callers use this struct.  Fields that are CLI-only
/// (e.g. `dry_run`, per-role backend overrides) default to `None`/`false`
/// when constructed by the daemon dispatcher.
pub struct AutoTaskParams {
    pub workspace_root: PathBuf,
    pub idea: String,
    pub project_id: Option<String>,
    pub pr_url: Option<String>,
    pub cancel: CancellationToken,
    pub max_backend_retries: Option<u8>,
    // PRD options — `None` falls back to workspace config.
    pub spec_writer: Option<String>,
    pub spec_reviewer: Option<String>,
    pub max_spec_revisions: u32,
    // Orchestrator options
    pub backend: Option<String>,
    pub planner_backend: Option<String>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub qa_backend: Option<String>,
    pub completer_backend: Option<String>,
    pub tmux: Option<bool>,
    pub skip_commit: bool,
    pub skip_prompt_review: bool,
    pub dry_run: bool,
}

/// Parameters for the `run` dispatch variant (resume existing project).
///
/// Mirrors `RunOptions` but carries the workspace root explicitly.
pub struct RunTaskParams {
    pub workspace_root: PathBuf,
    pub project: Option<String>,
    pub pr_url: Option<String>,
    pub cancel: CancellationToken,
    pub max_backend_retries: Option<u8>,
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

/// Parameters for the `quick-dev-auto` dispatch variant.
pub struct QuickDevAutoTaskParams {
    pub workspace_root: PathBuf,
    pub idea: String,
    pub project_id: Option<String>,
    pub pr_url: Option<String>,
    pub cancel: CancellationToken,
    pub max_backend_retries: Option<u8>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub skip_commit: bool,
    pub max_review_iterations: Option<u32>,
    pub max_final_review_retries: Option<u32>,
}

/// Parameters for the `quick-dev-run` dispatch variant.
pub struct QuickDevRunTaskParams {
    pub workspace_root: PathBuf,
    pub project: Option<String>,
    pub pr_url: Option<String>,
    pub cancel: CancellationToken,
    pub max_backend_retries: Option<u8>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub skip_commit: bool,
    pub max_review_iterations: Option<u32>,
    pub max_final_review_retries: Option<u32>,
}

// ---------------------------------------------------------------------------
// Library entry points
// ---------------------------------------------------------------------------

/// Run `auto` flow: quick-prd → create project → orchestrate until complete.
///
/// Shared entry point for both CLI (`ralph auto`) and daemon dispatch.
pub async fn run_auto_task(params: AutoTaskParams) -> Result<OrchestrationResult> {
    let workspace = load_workspace(&params.workspace_root)?;
    let repo_root = workspace
        .root
        .parent()
        .map(|p| p.to_owned())
        .unwrap_or_else(|| params.workspace_root.clone());

    // Quick-prd phase — use explicit overrides or fall back to workspace config.
    let writer_spec = params
        .spec_writer
        .unwrap_or_else(|| workspace.config.workspace.daemon_prd_writer_backend.clone());
    let reviewer_spec = params
        .spec_reviewer
        .unwrap_or_else(|| {
            workspace
                .config
                .workspace
                .daemon_prd_reviewer_backend
                .clone()
        });

    let mut registry = BackendRegistry::new(
        &workspace.config,
        BackendRegistryTmuxConfig {
            enabled: false,
            session_name: workspace.config.workspace.tmux_session.clone(),
            window_keep_seconds: workspace.config.workspace.tmux_window_keep_seconds,
        },
    );
    registry.set_cwd(Some(repo_root.clone()));

    backend_spec::validate_backend_spec(&writer_spec, &workspace.config)?;
    backend_spec::validate_backend_spec(&reviewer_spec, &workspace.config)?;

    let writer = registry.get_or_create_for_spec(&writer_spec)?;
    let reviewer = registry.get_or_create_for_spec(&reviewer_spec)?;
    writer.health_check().await?;
    reviewer.health_check().await?;

    let quick_prd = QuickPrdPipeline::new(
        writer,
        reviewer,
        QuickPrdOptions {
            idea: params.idea.clone(),
            writer_spec,
            reviewer_spec,
            max_revisions: params.max_spec_revisions,
            dry_run: false,
        },
    );
    let quick_prd_result = tokio::select! {
        result = quick_prd.run(repo_root) => result?,
        _ = params.cancel.cancelled() => return Err(RalphError::Cancelled),
    };

    if params.cancel.is_cancelled() {
        return Err(RalphError::Cancelled);
    }

    tracing::info!(
        spec = %quick_prd_result.spec_path.display(),
        revisions = quick_prd_result.revision_count,
        "quick-prd completed"
    );

    // Handle dry-run: return the spec content as the summary.
    if params.dry_run {
        let spec = std::fs::read_to_string(&quick_prd_result.spec_path).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to read spec file {}: {err}",
                quick_prd_result.spec_path.display()
            ))
        })?;
        return Ok(OrchestrationResult {
            summary: spec,
            loop_number: None,
        });
    }

    // Validate orchestrator backend specs (fail-fast before project creation).
    for spec in [
        params.backend.as_deref(),
        params.planner_backend.as_deref(),
        params.implementer_backend.as_deref(),
        params.reviewer_backend.as_deref(),
        params.qa_backend.as_deref(),
        params.completer_backend.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        backend_spec::validate_backend_spec(spec, &workspace.config)?;
    }

    // Create project
    let project_id =
        params
            .project_id
            .unwrap_or_else(|| crate::cli::auto::slugify_idea_public(&params.idea));
    if project_id.is_empty() {
        return Err(RalphError::Validation(
            "derived project id from idea is empty".to_owned(),
        ));
    }
    let project_name = params.idea.chars().take(60).collect::<String>();
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: project_name,
            source: PromptSource::File(quick_prd_result.spec_path),
            starting_backend: params.backend.clone(),
        },
    )?;

    tracing::info!(project_id = %project_id, "project created");

    // Orchestrate
    let workspace = load_workspace(&params.workspace_root)?;
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(RunOptions {
            project: Some(project_id),
            loops: None,
            until_review: false,
            until_complete: true,
            dry_run: false,
            backend: params.backend,
            planner_backend: params.planner_backend,
            implementer_backend: params.implementer_backend,
            reviewer_backend: params.reviewer_backend,
            qa_backend: params.qa_backend,
            completer_backend: params.completer_backend,
            tmux: params.tmux,
            on_prompt_change: None,
            skip_commit: params.skip_commit,
            skip_prompt_review: params.skip_prompt_review,
            pr_url: params.pr_url,
            cancel: params.cancel,
            max_backend_retries: params.max_backend_retries,
        })
        .await
}

/// Run `run` flow: resume existing project, orchestrate until complete.
///
/// Shared entry point for both CLI (`ralph run`) and daemon dispatch.
pub async fn run_run_task(params: RunTaskParams) -> Result<OrchestrationResult> {
    let workspace = load_workspace(&params.workspace_root)?;

    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(RunOptions {
            project: params.project,
            loops: params.loops,
            until_review: params.until_review,
            until_complete: params.until_complete,
            dry_run: params.dry_run,
            backend: params.backend,
            planner_backend: params.planner_backend,
            implementer_backend: params.implementer_backend,
            reviewer_backend: params.reviewer_backend,
            qa_backend: params.qa_backend,
            completer_backend: params.completer_backend,
            tmux: params.tmux,
            on_prompt_change: params.on_prompt_change,
            skip_commit: params.skip_commit,
            skip_prompt_review: params.skip_prompt_review,
            pr_url: params.pr_url,
            cancel: params.cancel,
            max_backend_retries: params.max_backend_retries,
        })
        .await
}

/// Run `quick-dev-auto` flow: quick-prd → create project → quick-dev orchestrate.
///
/// Shared entry point for both CLI (`ralph quick-dev-auto`) and daemon dispatch.
pub async fn run_quick_dev_auto_task(params: QuickDevAutoTaskParams) -> Result<OrchestrationResult> {
    let workspace = load_workspace(&params.workspace_root)?;
    let repo_root = workspace
        .root
        .parent()
        .map(|p| p.to_owned())
        .unwrap_or_else(|| params.workspace_root.clone());

    // --- Quick-dev backend preflight validation (fail-fast before side effects) ---
    // Mirror the orchestrator's resolution chain: CLI -> workflow config -> starting_backend.
    // Since the project doesn't exist yet, project-level overrides don't apply.
    let preflight_implementer = params
        .implementer_backend
        .as_deref()
        .or(workspace.config.workflow.implementer_backend.as_deref())
        .unwrap_or(&workspace.config.workspace.default_backend);
    let preflight_reviewer = params
        .reviewer_backend
        .as_deref()
        .or(workspace.config.workflow.reviewer_backend.as_deref());

    match preflight_reviewer {
        None => {
            return Err(RalphError::Validation(
                "quick-dev requires a second backend for review".to_owned(),
            ));
        }
        Some(rev) => {
            quick_dev_orchestrator::validate_distinct_backends(preflight_implementer, rev)?;
            validate_required_backend_spec(
                &workspace.config,
                preflight_implementer,
                "quick-dev implementer backend",
            )?;
            validate_required_backend_spec(
                &workspace.config,
                rev,
                "quick-dev reviewer backend",
            )?;
        }
    }

    // Quick-prd phase
    let writer_spec = workspace.config.workspace.daemon_prd_writer_backend.clone();
    let reviewer_spec = workspace
        .config
        .workspace
        .daemon_prd_reviewer_backend
        .clone();

    let mut registry = BackendRegistry::new(
        &workspace.config,
        BackendRegistryTmuxConfig {
            enabled: false,
            session_name: workspace.config.workspace.tmux_session.clone(),
            window_keep_seconds: workspace.config.workspace.tmux_window_keep_seconds,
        },
    );
    registry.set_cwd(Some(repo_root.clone()));

    backend_spec::validate_backend_spec(&writer_spec, &workspace.config)?;
    backend_spec::validate_backend_spec(&reviewer_spec, &workspace.config)?;

    let writer = registry.get_or_create_for_spec(&writer_spec)?;
    let reviewer = registry.get_or_create_for_spec(&reviewer_spec)?;
    writer.health_check().await?;
    reviewer.health_check().await?;

    let quick_prd = QuickPrdPipeline::new(
        writer,
        reviewer,
        QuickPrdOptions {
            idea: params.idea.clone(),
            writer_spec,
            reviewer_spec,
            max_revisions: 1,
            dry_run: false,
        },
    );
    let quick_prd_result = tokio::select! {
        result = quick_prd.run(repo_root) => result?,
        _ = params.cancel.cancelled() => return Err(RalphError::Cancelled),
    };

    if params.cancel.is_cancelled() {
        return Err(RalphError::Cancelled);
    }

    tracing::info!(
        spec = %quick_prd_result.spec_path.display(),
        revisions = quick_prd_result.revision_count,
        "quick-prd completed"
    );

    // Create project
    let project_id =
        params
            .project_id
            .unwrap_or_else(|| crate::cli::auto::slugify_idea_public(&params.idea));
    if project_id.is_empty() {
        return Err(RalphError::Validation(
            "derived project id from idea is empty".to_owned(),
        ));
    }
    let project_name = params.idea.chars().take(60).collect::<String>();
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: project_name,
            source: PromptSource::File(quick_prd_result.spec_path),
            starting_backend: params.implementer_backend.clone(),
        },
    )?;

    tracing::info!(project_id = %project_id, "project created");

    // Reload workspace after project creation so the orchestrator sees the
    // newly created project state (matches run_auto_task behaviour).
    let workspace = load_workspace(&params.workspace_root)?;

    // Orchestrate via quick-dev
    let mut orchestrator = QuickDevOrchestrator::new(workspace);
    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id),
            implementer_backend: params.implementer_backend,
            reviewer_backend: params.reviewer_backend,
            pr_url: params.pr_url,
            skip_commit: params.skip_commit,
            max_review_iterations: params.max_review_iterations,
            max_final_review_retries: params.max_final_review_retries,
            cancel: params.cancel,
            max_backend_retries: params.max_backend_retries,
        })
        .await?;

    Ok(OrchestrationResult {
        summary: result.summary,
        loop_number: result.loop_number,
    })
}

/// Run `quick-dev-run` flow: resume existing project via quick-dev orchestrator.
///
/// Shared entry point for both CLI (`ralph quick-dev-run`) and daemon dispatch.
pub async fn run_quick_dev_run_task(params: QuickDevRunTaskParams) -> Result<OrchestrationResult> {
    let workspace = load_workspace(&params.workspace_root)?;

    let mut orchestrator = QuickDevOrchestrator::new(workspace);
    let result = orchestrator
        .run(QuickDevRunOptions {
            project: params.project,
            implementer_backend: params.implementer_backend,
            reviewer_backend: params.reviewer_backend,
            pr_url: params.pr_url,
            skip_commit: params.skip_commit,
            max_review_iterations: params.max_review_iterations,
            max_final_review_retries: params.max_final_review_retries,
            cancel: params.cancel,
            max_backend_retries: params.max_backend_retries,
        })
        .await?;

    Ok(OrchestrationResult {
        summary: result.summary,
        loop_number: result.loop_number,
    })
}

// ---------------------------------------------------------------------------
// Per-task log subscriber & spawn helper
// ---------------------------------------------------------------------------

/// Spawn an in-process orchestration task with its own per-task tracing
/// subscriber writing to `log_path`. Returns the join handle.
///
/// The `CancellationToken` is created by the caller and shared with the
/// task params, so the caller can cancel the task cooperatively.
pub fn spawn_inprocess_task<F, Fut>(
    task_fn: F,
    log_path: &Path,
) -> Result<JoinHandle<Result<OrchestrationResult>>>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<OrchestrationResult>> + Send + 'static,
{
    let file = open_log_file_append(log_path)?;
    let subscriber = fmt::Subscriber::builder()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(false)
        .finish();
    let dispatch = tracing::dispatcher::Dispatch::new(subscriber);

    let handle = tokio::spawn(task_fn().with_subscriber(dispatch));
    Ok(handle)
}

// ---------------------------------------------------------------------------
// Log file helpers (moved from process.rs)
// ---------------------------------------------------------------------------

/// Open a log file in append mode, writing a retrigger separator if the file
/// already has content.
pub fn open_log_file_append(log_file: &Path) -> Result<std::fs::File> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(log_file)
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to open log file {} for append: {err}",
                log_file.display()
            ))
        })?;

    let (has_content, force_conservative_separator) =
        has_content_for_separator(log_file, file.metadata().map(|meta| meta.len()));

    if has_content {
        let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let separator = if force_conservative_separator {
            format_retrigger_separator(&timestamp, None)
        } else {
            let ends_with_newline = file.seek(SeekFrom::End(-1)).and_then(|_| {
                let mut last = [0_u8; 1];
                file.read_exact(&mut last).map(|_| last[0] == b'\n')
            });
            match ends_with_newline {
                Ok(ends_with_newline) => {
                    format_retrigger_separator(&timestamp, Some(ends_with_newline))
                }
                Err(err) => {
                    tracing::warn!(
                        path = %log_file.display(),
                        error = %err,
                        "failed to inspect trailing newline for log file"
                    );
                    format_retrigger_separator(&timestamp, None)
                }
            }
        };
        if let Err(err) = file.write_all(separator.as_bytes()) {
            tracing::warn!(
                path = %log_file.display(),
                error = %err,
                "failed to write retrigger separator"
            );
        }
    }

    Ok(file)
}

fn has_content_for_separator(log_file: &Path, metadata_len: std::io::Result<u64>) -> (bool, bool) {
    match metadata_len {
        Ok(len) => (len > 0, false),
        Err(err) => {
            tracing::warn!(
                path = %log_file.display(),
                error = %err,
                "failed to inspect log file metadata"
            );
            (true, true)
        }
    }
}

fn format_retrigger_separator(timestamp: &str, ends_with_newline: Option<bool>) -> String {
    match ends_with_newline {
        Some(true) => format!("\n--- retrigger at {timestamp} ---\n\n"),
        Some(false) | None => format!("\n\n--- retrigger at {timestamp} ---\n\n"),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_workspace(workspace_root: &Path) -> Result<Workspace> {
    let ralph_dir = workspace_root.join(".ralph");
    // Daemon dispatch must always use strict load — the workspace must
    // already be initialized (the worktree setup copies `.ralph/`).
    // Auto-initializing here would silently run with default config if
    // the worktree config copy failed, violating the spec.
    Workspace::load(ralph_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RalphError;
    use crate::workflow::orchestrator::OrchestrationResult;
    use std::fs;
    use tempfile::tempdir;

    // -----------------------------------------------------------------------
    // Per-task log isolation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn spawn_inprocess_task_log_isolation_no_cross_contamination() {
        let tmp = tempdir().expect("tempdir");
        let log1 = tmp.path().join("task-1.log");
        let log2 = tmp.path().join("task-2.log");

        let handle1 = spawn_inprocess_task(
            || async {
                tracing::info!("TASK_ONE_MARKER");
                Ok(OrchestrationResult {
                    summary: "task1".to_owned(),
                    loop_number: Some(1),
                })
            },
            &log1,
        )
        .expect("spawn task 1");

        let handle2 = spawn_inprocess_task(
            || async {
                tracing::info!("TASK_TWO_MARKER");
                Ok(OrchestrationResult {
                    summary: "task2".to_owned(),
                    loop_number: Some(1),
                })
            },
            &log2,
        )
        .expect("spawn task 2");

        handle1.await.expect("task 1 join").expect("task 1 result");
        handle2.await.expect("task 2 join").expect("task 2 result");

        let content1 = fs::read_to_string(&log1).expect("read log1");
        let content2 = fs::read_to_string(&log2).expect("read log2");

        assert!(
            content1.contains("TASK_ONE_MARKER"),
            "log1 should contain TASK_ONE_MARKER, got: {content1}"
        );
        assert!(
            !content1.contains("TASK_TWO_MARKER"),
            "log1 should NOT contain TASK_TWO_MARKER (cross-contamination), got: {content1}"
        );
        assert!(
            content2.contains("TASK_TWO_MARKER"),
            "log2 should contain TASK_TWO_MARKER, got: {content2}"
        );
        assert!(
            !content2.contains("TASK_ONE_MARKER"),
            "log2 should NOT contain TASK_ONE_MARKER (cross-contamination), got: {content2}"
        );
    }

    // -----------------------------------------------------------------------
    // Cancellation behavior — Err(Cancelled) path
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn spawn_inprocess_task_returns_cancelled_on_token_cancel() {
        let tmp = tempdir().expect("tempdir");
        let log_path = tmp.path().join("cancel-test.log");
        let cancel = CancellationToken::new();
        let cancel_inner = cancel.clone();

        let handle = spawn_inprocess_task(
            move || async move {
                // Wait until cancelled
                cancel_inner.cancelled().await;
                Err(RalphError::Cancelled)
            },
            &log_path,
        )
        .expect("spawn task");

        // Cancel after a short delay
        cancel.cancel();

        let result = handle.await.expect("join should succeed");
        assert!(
            matches!(result, Err(RalphError::Cancelled)),
            "expected Err(Cancelled), got: {result:?}"
        );
    }
}
