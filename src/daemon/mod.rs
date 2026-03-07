pub mod bootstrap;
pub mod github;
pub mod interactive_prd;
pub mod process;
pub mod rebase_agent;
pub mod refine;
pub mod runtime;
pub mod worktree;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::error::RalphError;
use crate::Result;

use self::process as daemon_process;

/// In-memory child process handle tracked by the daemon runtime.
///
/// No daemon task metadata is durably persisted. Issue metadata is fetched
/// from GitHub on demand.
pub struct ChildHandle {
    pub pid: u32,
    pub pgid: u32,
    pub child: tokio::process::Child,
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

/// Abort a running task by issue number: kill the process group (if any PID
/// is provided) and update labels from `ralph:in-progress` to `ralph:failed`.
///
/// The caller is responsible for removing the child from the in-memory map.
/// This function only performs process termination and label updates.
pub async fn abort_task_by_labels(
    owner: &str,
    repo: &str,
    issue_number: u32,
    child_pid: Option<u32>,
    child_pgid: Option<u32>,
) -> Result<()> {
    terminate_process_group_if_present(child_pid, child_pgid).await;

    // Swap label: ralph:in-progress -> ralph:failed
    github::swap_lifecycle_label(
        owner,
        repo,
        issue_number,
        "ralph:in-progress",
        "ralph:failed",
    )
    .await
    .map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to update labels for abort of {owner}/{repo}#{issue_number}: {err}"
        ))
    })?;

    Ok(())
}

async fn terminate_process_group_if_present(child_pid: Option<u32>, child_pgid: Option<u32>) {
    // Prefer killing by process group; fall back to single PID.
    if let Some(pgid) = child_pgid.filter(|v| *v > 0) {
        daemon_process::terminate_process_group(pgid, Duration::from_secs(10)).await;
        return;
    }
    if let Some(pid) = child_pid.filter(|v| *v > 0) {
        // No PGID available — treat the single PID as a one-member "group".
        daemon_process::terminate_process_group(pid, Duration::from_secs(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::format_task_id;

    #[test]
    fn format_task_id_constructs_expected_string() {
        assert_eq!(format_task_id("acme", "widgets", 42), "acme-widgets-42");
    }
}
