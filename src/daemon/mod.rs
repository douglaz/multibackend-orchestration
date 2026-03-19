pub mod bootstrap;
pub mod github;
pub mod interactive_prd;
pub mod oracle_review;
pub mod pr_review;
pub mod process;
pub mod rebase_agent;
pub mod refine;
pub mod runtime;
pub mod tasks;
pub mod worktree;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::Result;

/// In-memory task handle tracked by the daemon runtime.
///
/// Replaces the former `ChildHandle` that held a `tokio::process::Child`.
/// Orchestration now runs as in-process tokio tasks instead of forked
/// subprocesses.
pub struct TaskHandle {
    /// Join handle for the in-process orchestration task.
    pub join_handle: JoinHandle<crate::Result<crate::workflow::orchestrator::OrchestrationResult>>,
    /// Cancellation token for cooperative task shutdown.
    pub cancel_token: CancellationToken,
    /// Flag set by `kill_aborted_children` when the issue was externally
    /// aborted (e.g. label removed from GitHub).  When set, `collect_children`
    /// forces the terminal label to `ralph:failed` and skips PR flow, even if
    /// the task's `JoinHandle` resolved `Ok`.
    pub aborted_externally: Arc<AtomicBool>,
    pub watcher_cancel: CancellationToken,
    pub watcher_handle: Option<JoinHandle<()>>,
    /// Cancellation token for the draft-PR watcher task.
    pub draft_pr_cancel: CancellationToken,
    /// Join handle for the draft-PR watcher task.
    pub draft_pr_handle: Option<JoinHandle<()>>,
    pub branch: String,
    pub log_file: PathBuf,
    pub last_rebase_at: Option<Instant>,
    /// Head SHA of the last rebase failure comment posted for this task,
    /// used to avoid spamming duplicate comments on persistent failures.
    pub last_rebase_failure_sha: Option<String>,
    /// PR URL for this task (resolved at spawn or created by draft-PR watcher).
    pub pr_url: Option<String>,
}

pub fn format_task_id(owner: &str, repo: &str, issue_number: u32) -> String {
    format!("{owner}-{repo}-{issue_number}")
}

/// Abort a running task by issue number: cancel the task's cancellation token
/// and update labels from `ralph:in-progress` to `ralph:failed`.
///
/// The caller is responsible for removing the task from the in-memory map.
pub async fn abort_task_by_labels(
    owner: &str,
    repo: &str,
    issue_number: u32,
    cancel_token: Option<&CancellationToken>,
) -> Result<()> {
    if let Some(token) = cancel_token {
        token.cancel();
    }

    // Swap label: ralph:in-progress -> ralph:failed
    github::swap_lifecycle_label(
        owner,
        repo,
        issue_number,
        "ralph:in-progress",
        "ralph:failed",
    )
    .await
    .map_err(|swap_err| {
        crate::error::RalphError::Orchestration(format!(
            "failed to update labels for abort of {owner}/{repo}#{issue_number}: {swap_err}"
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::format_task_id;

    #[test]
    fn format_task_id_constructs_expected_string() {
        assert_eq!(format_task_id("acme", "widgets", 42), "acme-widgets-42");
    }
}
