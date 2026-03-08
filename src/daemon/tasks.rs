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
use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::prd::quick::{QuickPrdOptions, QuickPrdPipeline};
use crate::project::lifecycle::{create_project, CreateProjectOptions, PromptSource};
use crate::workflow::orchestrator::{Orchestrator, OrchestrationResult, RunOptions};
use crate::workflow::quick_dev_orchestrator::{QuickDevOrchestrator, QuickDevRunOptions};
use crate::workspace::Workspace;
use crate::Result;

// ---------------------------------------------------------------------------
// Task parameter structs
// ---------------------------------------------------------------------------

/// Parameters for the `auto` dispatch variant (fresh project with quick-prd).
pub struct AutoTaskParams {
    pub workspace_root: PathBuf,
    pub idea: String,
    pub project_id: Option<String>,
    pub pr_url: Option<String>,
    pub global_config: GlobalConfig,
    pub cancel: CancellationToken,
}

/// Parameters for the `run` dispatch variant (resume existing project).
pub struct RunTaskParams {
    pub workspace_root: PathBuf,
    pub project_id: String,
    pub pr_url: Option<String>,
    pub cancel: CancellationToken,
}

/// Parameters for the `quick-dev-auto` dispatch variant.
pub struct QuickDevAutoTaskParams {
    pub workspace_root: PathBuf,
    pub idea: String,
    pub project_id: Option<String>,
    pub pr_url: Option<String>,
    pub global_config: GlobalConfig,
    pub cancel: CancellationToken,
}

/// Parameters for the `quick-dev-run` dispatch variant.
pub struct QuickDevRunTaskParams {
    pub workspace_root: PathBuf,
    pub project_id: String,
    pub pr_url: Option<String>,
    pub cancel: CancellationToken,
}

// ---------------------------------------------------------------------------
// Library entry points
// ---------------------------------------------------------------------------

/// Run `auto` flow: quick-prd → create project → orchestrate until complete.
pub async fn run_auto_task(params: AutoTaskParams) -> Result<OrchestrationResult> {
    let workspace = load_workspace(&params.workspace_root)?;
    let repo_root = workspace
        .root
        .parent()
        .map(|p| p.to_owned())
        .unwrap_or_else(|| params.workspace_root.clone());

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
        result = quick_prd.run_in(repo_root) => result?,
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
            starting_backend: None,
        },
    )?;

    // Orchestrate
    let workspace = load_workspace(&params.workspace_root)?;
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator.run(RunOptions {
        project: Some(project_id),
        loops: None,
        until_review: false,
        until_complete: true,
        dry_run: false,
        backend: None,
        planner_backend: None,
        implementer_backend: None,
        reviewer_backend: None,
        qa_backend: None,
        completer_backend: None,
        tmux: None,
        on_prompt_change: None,
        skip_commit: false,
        skip_prompt_review: false,
        pr_url: params.pr_url,
        cancel: params.cancel,
        max_backend_retries: None,
    })
    .await
}

/// Run `run` flow: resume existing project, orchestrate until complete.
pub async fn run_run_task(params: RunTaskParams) -> Result<OrchestrationResult> {
    let workspace = load_workspace(&params.workspace_root)?;

    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator.run(RunOptions {
        project: Some(params.project_id),
        loops: None,
        until_review: false,
        until_complete: true,
        dry_run: false,
        backend: None,
        planner_backend: None,
        implementer_backend: None,
        reviewer_backend: None,
        qa_backend: None,
        completer_backend: None,
        tmux: None,
        on_prompt_change: None,
        skip_commit: false,
        skip_prompt_review: false,
        pr_url: params.pr_url,
        cancel: params.cancel,
        max_backend_retries: None,
    })
    .await
}

/// Run `quick-dev-auto` flow: quick-prd → create project → quick-dev orchestrate.
pub async fn run_quick_dev_auto_task(params: QuickDevAutoTaskParams) -> Result<OrchestrationResult> {
    let workspace = load_workspace(&params.workspace_root)?;
    let repo_root = workspace
        .root
        .parent()
        .map(|p| p.to_owned())
        .unwrap_or_else(|| params.workspace_root.clone());

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
        result = quick_prd.run_in(repo_root) => result?,
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
            starting_backend: None,
        },
    )?;

    // Orchestrate via quick-dev
    let mut orchestrator = QuickDevOrchestrator::new(workspace);
    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id),
            implementer_backend: None,
            reviewer_backend: None,
            pr_url: params.pr_url,
            skip_commit: false,
            max_review_iterations: None,
            max_final_review_retries: None,
            cancel: params.cancel,
            max_backend_retries: None,
        })
        .await?;

    Ok(OrchestrationResult {
        summary: result.summary,
        loop_number: result.loop_number,
    })
}

/// Run `quick-dev-run` flow: resume existing project via quick-dev orchestrator.
pub async fn run_quick_dev_run_task(params: QuickDevRunTaskParams) -> Result<OrchestrationResult> {
    let workspace = load_workspace(&params.workspace_root)?;

    let mut orchestrator = QuickDevOrchestrator::new(workspace);
    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(params.project_id),
            implementer_backend: None,
            reviewer_backend: None,
            pr_url: params.pr_url,
            skip_commit: false,
            max_review_iterations: None,
            max_final_review_retries: None,
            cancel: params.cancel,
            max_backend_retries: None,
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
                    eprintln!(
                        "warning: failed to inspect trailing newline for log file {}: {err}",
                        log_file.display()
                    );
                    format_retrigger_separator(&timestamp, None)
                }
            }
        };
        if let Err(err) = file.write_all(separator.as_bytes()) {
            eprintln!(
                "warning: failed to write retrigger separator to {}: {err}",
                log_file.display()
            );
        }
    }

    Ok(file)
}

fn has_content_for_separator(log_file: &Path, metadata_len: std::io::Result<u64>) -> (bool, bool) {
    match metadata_len {
        Ok(len) => (len > 0, false),
        Err(err) => {
            eprintln!(
                "warning: failed to inspect log file {} metadata: {err}",
                log_file.display()
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
    if ralph_dir.join("ralph.toml").is_file() {
        Workspace::load(ralph_dir)
    } else {
        crate::cli::init::create_workspace(&ralph_dir)
    }
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
