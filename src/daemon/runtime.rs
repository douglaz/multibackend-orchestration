use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::config::GlobalConfig;
use crate::daemon::bootstrap;
use crate::daemon::github::{self, PrMergeStatus};
use crate::daemon::rebase_agent::{
    classify_rebase_failure_pure, parse_rebase_agent_backend, RebaseAgentBackend, RebaseFailureKind,
};

use crate::daemon::interactive_prd::{self, PrdPollConfig};
use crate::daemon::oracle_review;
use crate::daemon::process;
use crate::daemon::refine;
use crate::daemon::worktree;
use crate::daemon::{format_task_id, TaskHandle};
use crate::error::RalphError;
use crate::Result;

/// Configuration for the daemon runtime loop.
#[derive(Clone)]
pub struct DaemonRuntimeConfig {
    pub owner: String,
    pub repo: String,
    pub base_branch: String,
    pub poll_seconds: u64,
    pub max_concurrent: u32,
    pub labels: Vec<String>,
    /// When true, the daemon runs exactly one iteration and exits.
    pub single_iteration: bool,
    /// When true, emit runtime diagnostics to stderr.
    pub verbose: bool,
    /// Root of the git repository (for worktree operations).
    pub repo_root: PathBuf,
    /// Prompt refinement feature toggle.
    pub refinement_enabled: bool,
    /// Backend spec used for prompt refinement.
    pub refinement_backend: String,
    /// Global config snapshot for runtime backend operations.
    pub global_config: GlobalConfig,
    /// Whether auto-rebase is enabled for PR-backed tasks.
    pub auto_rebase_enabled: bool,
    /// Minimum interval (seconds) between rebase attempts for the same task.
    pub rebase_interval_seconds: u64,
    /// Maximum rebase attempts per daemon cycle.
    pub max_rebases_per_cycle: u32,
    /// Per-attempt timeout (seconds) for rebase operations.
    pub rebase_timeout_seconds: u64,
    /// Backend spec string for AI-assisted rebase conflict recovery.
    /// Parsed internally by `resolve_rebase_conflicts`; supports "none",
    /// "claude", "claude(<model>)".
    pub rebase_agent_backend: String,
    /// Workspace root (`.ralph/` directory).
    pub workspace_root: PathBuf,
    /// Whether the interactive PRD workflow is enabled.
    pub prd_enabled: bool,
    /// Backend specs for PRD question generation (exactly 2).
    pub prd_question_backends: Vec<String>,
    /// Backend spec for PRD draft writer.
    pub prd_writer_backend: String,
    /// Backend spec for PRD draft reviewer.
    pub prd_reviewer_backend: String,
    /// Maximum internal writer/reviewer retries for PRD draft generation.
    pub prd_max_revisions: u32,
    /// Total wall-clock timeout (seconds) for backend calls within a single
    /// PRD state transition.
    pub prd_backend_timeout_secs: u64,
    /// Timeout (seconds) used when shutting down the PRD background task.
    pub prd_shutdown_timeout_secs: u64,
    /// Whether automated Oracle PR reviews are enabled.
    pub oracle_review_enabled: bool,
    /// Timeout (seconds) for a single Oracle invocation.
    pub oracle_review_timeout_secs: u64,
    /// Optional allowlist of GitHub authors eligible for Oracle review.
    pub oracle_review_authors: Vec<String>,
    /// Maximum successful Oracle reviews to post per daemon cycle.
    pub oracle_review_max_per_cycle: u32,
    /// Executable used for git invocations in interactive PRD.
    pub git_bin: String,
    /// Executable used for GitHub CLI invocations in interactive PRD.
    pub gh_bin: String,
    /// Maximum number of backend timeout retries per invocation.
    /// Threaded through to task params so orchestrators use a consistent value.
    pub max_backend_retries: Option<u8>,
    /// GitHub usernames whose PR review comments trigger amendments.
    pub pr_review_whitelist: Vec<String>,
}

pub async fn spawn_blocking_op<T, F>(op: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|err| RalphError::Orchestration(format!("blocking task join failure: {err}")))?
}

const ARTIFACT_WATCH_POLL_SECONDS: u64 = 2;
const DRAFT_PR_WATCH_POLL_SECONDS: u64 = 15;
pub(crate) const GITHUB_COMMENT_LIMIT: usize = 65_536;
const TRUNCATED_NOTE: &str = "\n\n[truncated]";
const COMPLETE_TASK_MAX_ATTEMPTS: u32 = 3;
const COMPLETE_TASK_RETRY_DELAY_SECS: u64 = 30;
const WATCHER_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum consecutive ahead-check failures before the watcher gives up.
const DRAFT_PR_WATCHER_MAX_CONSECUTIVE_FAILURES: u32 = 5;

fn draft_pr_watch_poll_seconds() -> u64 {
    std::env::var("RALPH_DRAFT_PR_WATCH_POLL_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DRAFT_PR_WATCH_POLL_SECONDS)
}

fn default_sleep(duration: Duration) -> impl Future<Output = ()> {
    tokio::time::sleep(duration)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftPrTransition {
    None,
    MarkReady,
    CloseNoDiff,
}

#[derive(Default)]
struct ArtifactWatcherState {
    quick_prd_posted: bool,
    final_prompt_posted: bool,
}

impl ArtifactWatcherState {
    fn is_complete(&self) -> bool {
        self.quick_prd_posted && self.final_prompt_posted
    }
}

#[async_trait]
trait ArtifactCommentClient: Send + Sync {
    async fn marker_exists(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u32,
        marker: &str,
    ) -> Result<bool>;

    async fn post_idempotent_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u32,
        task_id: &str,
        phase: &str,
        body_text: &str,
    ) -> Result<()>;
}

struct GitHubArtifactCommentClient;

#[async_trait]
impl ArtifactCommentClient for GitHubArtifactCommentClient {
    async fn marker_exists(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u32,
        marker: &str,
    ) -> Result<bool> {
        github::comment_marker_exists(owner, repo, issue_number, marker).await
    }

    async fn post_idempotent_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u32,
        task_id: &str,
        phase: &str,
        body_text: &str,
    ) -> Result<()> {
        github::post_idempotent_comment(owner, repo, issue_number, task_id, phase, body_text).await
    }
}

/// Draft-PR watcher: polls branch divergence on a fixed interval and creates
/// a draft PR when the branch first moves ahead of the base branch.
///
/// Uses `tokio::select!` with cancellation for immediate shutdown.
/// Only one draft creation attempt is active at a time (single-flight guard).
/// Performs an unconditional push before draft PR creation.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn draft_pr_watcher(
    owner: String,
    repo: String,
    base_branch: String,
    worktree_path: PathBuf,
    branch: String,
    task_id: String,
    issue_number: u32,
    cancel: CancellationToken,
    workspace_root: PathBuf,
) {
    draft_pr_watcher_with_sleep(
        owner,
        repo,
        base_branch,
        worktree_path,
        branch,
        task_id,
        issue_number,
        cancel,
        workspace_root,
        default_sleep,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn draft_pr_watcher_with_sleep<S, SFut>(
    owner: String,
    repo: String,
    base_branch: String,
    worktree_path: PathBuf,
    branch: String,
    task_id: String,
    issue_number: u32,
    cancel: CancellationToken,
    workspace_root: PathBuf,
    mut sleep_fn: S,
) where
    S: FnMut(Duration) -> SFut,
    SFut: Future<Output = ()>,
{
    let poll_interval = Duration::from_secs(draft_pr_watch_poll_seconds());
    let mut pr_created = false;
    let mut consecutive_failures: u32 = 0;

    loop {
        // Check if branch has commits ahead of base
        let has_ahead = {
            match github::has_commits_ahead_of_base(&worktree_path, &base_branch).await {
                Ok(v) => {
                    consecutive_failures = 0;
                    v
                }
                Err(err) => {
                    consecutive_failures += 1;
                    if consecutive_failures >= DRAFT_PR_WATCHER_MAX_CONSECUTIVE_FAILURES {
                        eprintln!(
                            "draft-pr-watcher: ahead-check failed {consecutive_failures} consecutive times \
                             for {task_id}, giving up: {err}"
                        );
                        break;
                    }
                    eprintln!(
                        "draft-pr-watcher: ahead-check failed for {task_id} \
                         ({consecutive_failures}/{DRAFT_PR_WATCHER_MAX_CONSECUTIVE_FAILURES}): {err}"
                    );
                    false
                }
            }
        };

        if has_ahead && !pr_created {
            // Step 1: push unconditionally before creating draft PR
            let push_ok = {
                match github::push_branch_with_retry(&worktree_path, &branch).await {
                    Ok(()) => {
                        eprintln!("draft-pr-watcher: pushed branch {branch} for {task_id}");
                        true
                    }
                    Err(err) => {
                        eprintln!(
                            "draft-pr-watcher: push failed for {task_id} branch {branch}: {err}"
                        );
                        false
                    }
                }
            };

            // Step 2: create draft PR
            if push_ok {
                let title = format!("[Draft] {task_id}");
                let body =
                    format!("Automated draft PR for task `{task_id}` (issue #{issue_number}).");
                match github::create_pr(&owner, &repo, &branch, &title, &body, true).await {
                    Ok(url) => {
                        eprintln!("draft-pr-watcher: created draft PR for {task_id}: {url}");
                        pr_created = true;
                        // Persist PR URL to durable storage for daemon restart recovery.
                        save_task_metadata(
                            &workspace_root,
                            &task_id,
                            &TaskMetadata { pr_url: Some(url) },
                        );
                    }
                    Err(err) => {
                        eprintln!(
                            "draft-pr-watcher: failed to create draft PR for {task_id}: {err}"
                        );
                        // Re-check: another process may have created the PR concurrently.
                        if let Ok(Some(url)) =
                            github::find_existing_pr(&owner, &repo, &branch).await
                        {
                            eprintln!("draft-pr-watcher: found existing PR for {task_id}: {url}");
                            pr_created = true;
                            save_task_metadata(
                                &workspace_root,
                                &task_id,
                                &TaskMetadata { pr_url: Some(url) },
                            );
                        }
                    }
                }
            }
        }

        // If PR is created, watcher job is done.
        if pr_created {
            break;
        }

        // Wait for next poll or cancellation
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }
            _ = sleep_fn(poll_interval) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn draft_pr_watcher_single_iteration_for_test(
    owner: String,
    repo: String,
    base_branch: String,
    worktree_path: PathBuf,
    branch: String,
    task_id: String,
    issue_number: u32,
    workspace_root: PathBuf,
) {
    let cancel = CancellationToken::new();
    cancel.cancel();
    draft_pr_watcher_with_sleep(
        owner,
        repo,
        base_branch,
        worktree_path,
        branch,
        task_id,
        issue_number,
        cancel,
        workspace_root,
        |_| async {},
    )
    .await;
}

pub(crate) async fn complete_task_with_retry_for_test<F, Fut, S, SFut>(
    mut attempt_op: F,
    mut sleep_fn: S,
) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
    S: FnMut(Duration) -> SFut,
    SFut: Future<Output = ()>,
{
    for attempt in 1..=COMPLETE_TASK_MAX_ATTEMPTS {
        match attempt_op().await {
            Ok(()) => return Ok(()),
            Err(err) => {
                if let Some(delay) = complete_task_retry_delay(&err, attempt) {
                    sleep_fn(delay).await;
                    continue;
                }
                return Err(err);
            }
        }
    }
    Err(RalphError::Orchestration(
        "complete_task retry loop exhausted unexpectedly".to_owned(),
    ))
}

async fn post_artifact_comments(
    owner: String,
    repo: String,
    issue_number: u32,
    task_id: String,
    worktree_path: PathBuf,
    child_start_time: SystemTime,
    watcher_cancel: CancellationToken,
) {
    let client = GitHubArtifactCommentClient;
    post_artifact_comments_with_client(
        &client,
        &owner,
        &repo,
        issue_number,
        &task_id,
        &worktree_path,
        child_start_time,
        watcher_cancel,
        Duration::from_secs(ARTIFACT_WATCH_POLL_SECONDS),
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn post_artifact_comments_with_client(
    client: &dyn ArtifactCommentClient,
    owner: &str,
    repo: &str,
    issue_number: u32,
    task_id: &str,
    worktree_path: &Path,
    child_start_time: SystemTime,
    watcher_cancel: CancellationToken,
    poll_interval: Duration,
) {
    let mut state = ArtifactWatcherState::default();
    let mut cancelled = false;

    loop {
        sweep_artifact_comments(
            client,
            owner,
            repo,
            issue_number,
            task_id,
            worktree_path,
            child_start_time,
            &mut state,
        )
        .await;

        if state.is_complete() {
            break;
        }

        tokio::select! {
            _ = watcher_cancel.cancelled() => {
                cancelled = true;
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {}
        }
    }

    if cancelled && !state.is_complete() {
        sweep_artifact_comments(
            client,
            owner,
            repo,
            issue_number,
            task_id,
            worktree_path,
            child_start_time,
            &mut state,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn sweep_artifact_comments(
    client: &dyn ArtifactCommentClient,
    owner: &str,
    repo: &str,
    issue_number: u32,
    task_id: &str,
    worktree_path: &Path,
    child_start_time: SystemTime,
    state: &mut ArtifactWatcherState,
) {
    if !state.quick_prd_posted {
        if let Some(content) = detect_quick_prd_artifact(worktree_path, child_start_time) {
            state.quick_prd_posted = try_post_artifact_comment(
                client,
                owner,
                repo,
                issue_number,
                task_id,
                "quick-prd",
                "### Quick PRD",
                &content,
            )
            .await;
        }
    }

    if !state.final_prompt_posted {
        if let Some(content) = detect_final_prompt_artifact(worktree_path, child_start_time) {
            state.final_prompt_posted = try_post_artifact_comment(
                client,
                owner,
                repo,
                issue_number,
                task_id,
                "final-prompt",
                "### Final Prompt (after review)",
                &content,
            )
            .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn try_post_artifact_comment(
    client: &dyn ArtifactCommentClient,
    owner: &str,
    repo: &str,
    issue_number: u32,
    task_id: &str,
    phase: &str,
    header: &str,
    content: &str,
) -> bool {
    let marker = format!("<!-- ralph:task:{task_id}:{phase} -->");
    let available_body_chars = GITHUB_COMMENT_LIMIT
        .saturating_sub(marker.chars().count())
        .saturating_sub(1);
    let formatted_body = format!("{header}\n\n{content}");
    let truncated_body = truncate_for_github(&formatted_body, available_body_chars);

    match client
        .marker_exists(owner, repo, issue_number, &marker)
        .await
    {
        Ok(true) => return true,
        Ok(false) => {}
        Err(err) => {
            eprintln!("warning: failed to check artifact marker for {task_id}/{phase}: {err}");
        }
    }

    if let Err(err) = client
        .post_idempotent_comment(owner, repo, issue_number, task_id, phase, &truncated_body)
        .await
    {
        eprintln!("warning: failed to post artifact comment for {task_id}/{phase}: {err}");
        return false;
    }

    match client
        .marker_exists(owner, repo, issue_number, &marker)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            eprintln!("warning: failed to verify artifact marker for {task_id}/{phase}: {err}");
            false
        }
    }
}

fn detect_quick_prd_artifact(worktree_path: &Path, child_start_time: SystemTime) -> Option<String> {
    let root = worktree_path.join(".ralph").join("quick-prd");
    let mut candidates = Vec::new();
    let entries = std::fs::read_dir(root).ok()?;

    for entry in entries.flatten() {
        let ty = match entry.file_type() {
            Ok(ty) => ty,
            Err(_) => continue,
        };
        if !ty.is_dir() {
            continue;
        }
        let meta_path = entry.path().join("meta.json");
        let modified = match std::fs::metadata(&meta_path).and_then(|meta| meta.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };
        if modified >= child_start_time {
            let spec_path = entry.path().join("SPEC.md");
            candidates.push((spec_path, modified));
        }
    }

    let path = newest_by_mtime(candidates)?;
    read_nonempty_artifact(&path)
}

fn detect_final_prompt_artifact(
    worktree_path: &Path,
    child_start_time: SystemTime,
) -> Option<String> {
    let root = worktree_path.join(".ralph").join("projects");
    let mut signals = Vec::new();
    let entries = std::fs::read_dir(root).ok()?;

    for entry in entries.flatten() {
        let ty = match entry.file_type() {
            Ok(ty) => ty,
            Err(_) => continue,
        };
        if !ty.is_dir() {
            continue;
        }
        let signal_path = entry.path().join("prompt-original.md");
        let modified = match std::fs::metadata(&signal_path).and_then(|meta| meta.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };
        if modified >= child_start_time {
            signals.push((signal_path, modified));
        }
    }

    let signal_path = newest_by_mtime(signals)?;
    let prompt_path = signal_path.parent()?.join("prompt.md");
    read_nonempty_artifact(&prompt_path)
}

fn newest_by_mtime(candidates: Vec<(PathBuf, SystemTime)>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .max_by(|(path_a, time_a), (path_b, time_b)| {
            match time_a
                .partial_cmp(time_b)
                .unwrap_or(std::cmp::Ordering::Equal)
            {
                std::cmp::Ordering::Equal => path_a.cmp(path_b),
                ordering => ordering,
            }
        })
        .map(|(path, _)| path)
}

pub(crate) fn truncate_for_github(body: &str, max_chars: usize) -> String {
    if body.chars().count() <= max_chars {
        return body.to_owned();
    }

    let note_chars = TRUNCATED_NOTE.chars().count();
    if max_chars <= note_chars {
        return TRUNCATED_NOTE.chars().take(max_chars).collect();
    }

    let keep_chars = max_chars - note_chars;
    let mut truncated: String = body.chars().take(keep_chars).collect();
    truncated.push_str(TRUNCATED_NOTE);
    truncated
}

fn read_nonempty_artifact(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        None
    } else {
        Some(content)
    }
}

/// Re-trigger a failed task by claiming its GitHub issue via label swap:
/// `ralph:failed` -> `ralph:ready`.
pub async fn retrigger_failed_task(owner: &str, repo: &str, issue_number: u32) -> Result<()> {
    // Verify current label state from GitHub
    let labels = github::fetch_issue_labels(owner, repo, issue_number).await?;
    let lifecycle = github::classify_lifecycle_labels(&labels);

    if !lifecycle.iter().any(|l| l == "ralph:failed") {
        return Err(RalphError::Validation(format!(
            "issue {owner}/{repo}#{issue_number} is not in failed state (labels: {})",
            lifecycle.join(", ")
        )));
    }

    // Swap failed -> ready so the daemon picks it up on next poll
    github::swap_lifecycle_label(owner, repo, issue_number, "ralph:failed", "ralph:ready").await?;

    eprintln!(
        "retrigger: event=success repo={owner}/{repo} issue_number={issue_number} transition=failed_to_ready"
    );
    Ok(())
}

/// Terminal cleanup policy:
/// - completed: cleanup worktree
/// - failed: preserve worktree for retry
fn should_cleanup_worktree(terminal_label: &str) -> bool {
    terminal_label == "ralph:completed"
}

/// Return the log file path for a task.
fn task_log_path(workspace_root: &Path, task_id: &str) -> PathBuf {
    workspace_root
        .join("tmp")
        .join("logs")
        .join(format!("{task_id}.log"))
}

/// Return the durable metadata file path for a task.
///
/// Stored under `.ralph/daemon/tasks/{task_id}.json` so it survives daemon
/// restarts while the workspace root persists.
fn task_metadata_path(workspace_root: &Path, task_id: &str) -> PathBuf {
    workspace_root
        .join("daemon")
        .join("tasks")
        .join(format!("{task_id}.json"))
}

/// Durable per-task metadata persisted across daemon restarts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TaskMetadata {
    #[serde(default)]
    pub pr_url: Option<String>,
}

/// Load task metadata from disk.  Returns `Default` if the file does not exist.
pub fn load_task_metadata(workspace_root: &Path, task_id: &str) -> TaskMetadata {
    let path = task_metadata_path(workspace_root, task_id);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => TaskMetadata::default(),
    }
}

/// Result of a strict task metadata load that distinguishes missing files
/// from corrupt/unreadable files.
pub enum TaskMetadataLoadResult {
    /// File exists and parsed successfully.
    Ok(TaskMetadata),
    /// File does not exist (definitively missing).
    NotFound,
    /// File exists but could not be read or parsed (transient/corrupt).
    Error(String),
}

/// Load task metadata with strict error handling.
///
/// Unlike [`load_task_metadata`], this variant distinguishes `NotFound`
/// (file absent) from parse/read errors so that callers can avoid
/// destructive actions (e.g. clearing staged amendments) when the metadata
/// file is corrupt rather than missing.
pub fn load_task_metadata_strict(workspace_root: &Path, task_id: &str) -> TaskMetadataLoadResult {
    let path = task_metadata_path(workspace_root, task_id);
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<TaskMetadata>(&content) {
            Ok(meta) => TaskMetadataLoadResult::Ok(meta),
            Err(err) => TaskMetadataLoadResult::Error(format!(
                "corrupt task metadata at {}: {err}",
                path.display()
            )),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => TaskMetadataLoadResult::NotFound,
        Err(err) => TaskMetadataLoadResult::Error(format!(
            "failed to read task metadata at {}: {err}",
            path.display()
        )),
    }
}

/// Persist task metadata to disk (best-effort: logs on failure).
///
/// Uses atomic temp-file + rename to prevent crash-interrupted writes from
/// leaving truncated/corrupt JSON that would silently reset metadata.
pub fn save_task_metadata(workspace_root: &Path, task_id: &str, meta: &TaskMetadata) {
    let path = task_metadata_path(workspace_root, task_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(meta) {
        Ok(json) => {
            let tmp_path = path.with_extension("json.tmp");
            if let Err(err) = std::fs::write(&tmp_path, &json) {
                eprintln!("warning: failed to write task metadata tmp for {task_id}: {err}");
                return;
            }
            if let Err(err) = std::fs::rename(&tmp_path, &path) {
                eprintln!("warning: failed to rename task metadata for {task_id}: {err}");
            }
        }
        Err(err) => {
            eprintln!("warning: failed to serialize task metadata for {task_id}: {err}");
        }
    }
}

/// Print the last 50 lines of a task's log file to stderr for diagnostics.
fn print_log_tail(task_id: &str, log_file: &Path) {
    if let Ok(content) = std::fs::read_to_string(log_file) {
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(50);
        eprintln!(
            "--- last output from {task_id} ({}) ---",
            log_file.display()
        );
        for line in &lines[start..] {
            eprintln!("  {line}");
        }
        eprintln!("--- end ---");
    }
}

/// Run the daemon loop: reconcile, then poll/claim/dispatch/collect.
///
/// All task state is in-memory (`children: HashMap<u32, TaskHandle>`).
/// GitHub lifecycle labels are the only durable task lifecycle source of truth.
pub async fn run(config: &DaemonRuntimeConfig) -> Result<()> {
    if let Err(err) = validate_daemon_branch_format(&config.global_config.git.branch_format) {
        eprintln!(
            "daemon runtime refused to start for {}/{}: {err}",
            config.owner, config.repo
        );
        return Err(err);
    }

    // Phase 0: Clean .ralph/tmp and recreate logs directory
    {
        let tmp_dir = config.workspace_root.join("tmp");
        if tmp_dir.exists() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }
        let logs_dir = tmp_dir.join("logs");
        std::fs::create_dir_all(&logs_dir).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to create logs directory {}: {err}",
                logs_dir.display()
            ))
        })?;
    }

    let repo_root_lock = Arc::new(Semaphore::new(1));

    // Phase 1: Startup reconciliation — reset all `ralph:in-progress` to `ralph:ready`.
    // Always queries `ralph:in-progress` regardless of configured poll labels.
    reconcile_in_progress_labels(&config.owner, &config.repo, config.verbose).await?;

    // Phase 2: PRD background task lifecycle
    let prd_cancel = CancellationToken::new();
    let prd_handle: Option<tokio::task::JoinHandle<()>> =
        if config.prd_enabled && !config.single_iteration {
            // Continuous mode: spawn background PRD task with immediate first tick
            let cancel = prd_cancel.clone();
            let prd_config = config.clone();
            let prd_lock = repo_root_lock.clone();
            Some(tokio::spawn(async move {
                // Immediate first tick
                if let Err(err) = run_prd_phase(&prd_config, &prd_lock).await {
                    eprintln!("warning: PRD background tick failed: {err}");
                }
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(prd_config.poll_seconds)) => {}
                    }
                    if cancel.is_cancelled() {
                        break;
                    }
                    if let Err(err) = run_prd_phase(&prd_config, &prd_lock).await {
                        eprintln!("warning: PRD background tick failed: {err}");
                    }
                }
            }))
        } else {
            None
        };

    // Phase 3: Main loop with in-memory child tracking
    let mut children: HashMap<u32, TaskHandle> = HashMap::new();
    let mut iteration: u64 = 0;

    loop {
        iteration = iteration.saturating_add(1);

        // Kill children whose issues were externally aborted (label changed
        // from ralph:in-progress to ralph:failed via CLI `daemon abort`).
        // Runs before collect_children so that a fast-finishing aborted task
        // is not mistakenly treated as a normal success.
        kill_aborted_children(config, &mut children, &repo_root_lock).await;

        // Collect finished children
        collect_children(config, &mut children, &repo_root_lock).await;

        // Auto-rebase phase: rebase eligible PR-backed child branches
        auto_rebase_phase(config, &mut children, &repo_root_lock).await;

        // PR review polling phase: detect review comments and resume completed projects
        if !config.pr_review_whitelist.is_empty() {
            if let Err(err) = pr_review_phase(config, &mut children, &repo_root_lock).await {
                eprintln!("warning: PR review polling failed: {err}");
            }
        }

        if let Err(err) = oracle_review::oracle_review_phase(config).await {
            eprintln!("warning: oracle review phase failed: {err}");
        }

        // Interactive PRD phase: in single-iteration mode, run exactly one
        // inline tick (no background task). In continuous mode, PRD runs as
        // a background task spawned above.
        if config.prd_enabled && config.single_iteration {
            if let Err(err) = run_prd_phase(config, &repo_root_lock).await {
                eprintln!("warning: interactive PRD phase failed: {err}");
            }
        }

        // Poll for new issues
        let active_count = children.len() as u32;
        let slots = config.max_concurrent.saturating_sub(active_count);
        if config.verbose {
            let planned_sleep_seconds = if config.single_iteration {
                0
            } else {
                config.poll_seconds
            };
            eprintln!(
                "verbose: poll-cycle iteration={iteration} active_children={active_count} available_slots={slots} planned_sleep_seconds={planned_sleep_seconds}"
            );
        }

        if slots > 0 {
            if let Err(err) = poll_and_claim(config, &mut children, slots, &repo_root_lock).await {
                eprintln!("warning: poll/claim cycle failed: {err}");
            }
        }

        // Collect again after spawning
        collect_children(config, &mut children, &repo_root_lock).await;

        if config.single_iteration {
            // In single-iteration mode, wait for all spawned children to
            // reach a terminal state so the outcome is deterministic.
            drain_all_children(config, &mut children, &repo_root_lock).await;
            break;
        }

        tokio::time::sleep(Duration::from_secs(config.poll_seconds)).await;
    }

    // Phase 4: PRD shutdown — cancel token, bounded await, explicit abort on timeout.
    //
    // We capture `abort_handle` before awaiting the `JoinHandle` because
    // `JoinHandle::abort()` consumes `self` while we need to await first.
    // `AbortHandle::abort()` is functionally equivalent — both signal the
    // tokio task to cancel — so this satisfies the spec requirement of
    // "call handle.abort() if timeout expires" without ownership issues.
    if let Some(handle) = prd_handle {
        prd_cancel.cancel();
        let timeout_dur = Duration::from_secs(config.prd_shutdown_timeout_secs);
        let abort_handle = handle.abort_handle();
        match tokio::time::timeout(timeout_dur, handle).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                eprintln!("warning: PRD background task panicked during shutdown: {err}");
            }
            Err(_) => {
                eprintln!(
                    "warning: PRD background task did not stop within {}s, aborting",
                    config.prd_shutdown_timeout_secs
                );
                abort_handle.abort();
            }
        }
    }

    Ok(())
}

/// Run the interactive PRD poll/advance phase.
///
/// Builds a `PrdPollConfig` from the runtime config and delegates to
/// `interactive_prd::poll_and_advance_prd` in a blocking task.
async fn run_prd_phase(
    config: &DaemonRuntimeConfig,
    repo_root_lock: &Arc<Semaphore>,
) -> Result<()> {
    // data_dir must be the root above owner/repo so that state_path()
    // constructs {data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue}.json
    // without duplicating the owner/repo segment already present in repo_root.
    let data_dir = config
        .repo_root
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.repo_root.clone());

    let prd_config = PrdPollConfig {
        owner: config.owner.clone(),
        repo: config.repo.clone(),
        data_dir,
        git_bin: config.git_bin.clone(),
        gh_bin: config.gh_bin.clone(),
        prd_enabled: config.prd_enabled,
        question_backends: config.prd_question_backends.clone(),
        writer_backend: config.prd_writer_backend.clone(),
        reviewer_backend: config.prd_reviewer_backend.clone(),
        max_revisions: config.prd_max_revisions,
        backend_timeout_secs: config.prd_backend_timeout_secs,
        global_config: config.global_config.clone(),
        verbose: config.verbose,
        max_concurrent: config.max_concurrent,
        worker_cwd: None,
    };

    // Acquire repo_root_lock so PRD git ops (fetch, reset) are serialized
    // with dispatch/rebase/cleanup phases that also hold this semaphore.
    let _permit = repo_root_lock.clone().acquire_owned().await.map_err(|e| {
        crate::error::RalphError::Orchestration(format!(
            "failed to acquire repo-root semaphore for PRD: {e}"
        ))
    })?;
    spawn_blocking_op(move || interactive_prd::poll_and_advance_prd(&prd_config)).await
}

/// Startup reconciliation: every issue currently labeled `ralph:in-progress`
/// is reset to `ralph:ready` (children map is empty on fresh daemon start).
///
/// Always queries `ralph:in-progress` directly rather than using configured
/// poll labels, ensuring stale issues are caught regardless of label config.
async fn reconcile_in_progress_labels(owner: &str, repo: &str, verbose: bool) -> Result<()> {
    // Always query ralph:in-progress explicitly to catch all stale issues
    let reconcile_labels = vec!["ralph:in-progress".to_owned()];
    let (issues, _overflow) = github::poll_issues(owner, repo, &reconcile_labels).await?;

    let mut reconciled = 0u32;
    for issue in &issues {
        let lifecycle = github::classify_lifecycle_labels(&issue.labels);
        if lifecycle.iter().any(|l| l == "ralph:in-progress") {
            if let Err(err) = github::swap_lifecycle_label(
                owner,
                repo,
                issue.number,
                "ralph:in-progress",
                "ralph:ready",
            )
            .await
            {
                eprintln!(
                    "reconcile: failed to reset issue #{} from in-progress to ready: {err}",
                    issue.number
                );
                continue;
            }
            reconciled += 1;
            if verbose {
                eprintln!(
                    "verbose: reconcile reset issue #{} in-progress->ready",
                    issue.number
                );
            }
        }
    }
    if reconciled > 0 {
        eprintln!("reconcile: reset {reconciled} in-progress issue(s) to ready");
    }
    Ok(())
}

/// A claimed issue ready for concurrent dispatch.
struct ClaimedIssue {
    issue_number: u32,
    raw_idea: String,
    issue_labels: Vec<String>,
}

/// Per-issue outcome from a dispatch worker, preserving `issue_number` identity
/// even when the worker panics, so rollback can always be applied.
enum DispatchOutcome {
    /// Dispatch succeeded — caller inserts the handle into `children`.
    Success {
        issue_number: u32,
        handle: Box<TaskHandle>,
    },
    /// Dispatch returned an error — per-issue rollback needed.
    Failure { issue_number: u32, detail: String },
    /// Dispatch panicked — per-issue rollback needed (same path as Failure).
    Panic { issue_number: u32, detail: String },
}

/// Per-issue outcome from a completion worker in `collect_children`.
/// Preserves `issue_number` so panic recovery can transition the issue
/// to a terminal failure state.
#[allow(dead_code)]
enum CompletionOutcome {
    /// Completion finished (success or handled error).
    Done { issue_number: u32, task_id: String },
    /// Completion worker panicked — explicit terminalization needed.
    Panic {
        issue_number: u32,
        task_id: String,
        detail: String,
    },
}

/// Poll for new issues, filter, claim, and dispatch.
async fn poll_and_claim(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, TaskHandle>,
    slots: u32,
    repo_root_lock: &Arc<Semaphore>,
) -> Result<()> {
    let (issues, overflow) =
        github::poll_issues(&config.owner, &config.repo, &config.labels).await?;

    if overflow {
        eprintln!("warning: gh issue list returned exactly 100 issues; results may be truncated");
    }

    // Stage 1: Sequential claim/label-swap and idea extraction
    let mut claimed_issues: Vec<ClaimedIssue> = Vec::new();
    for issue in &issues {
        if claimed_issues.len() as u32 >= slots {
            break;
        }

        // Classify lifecycle labels
        let lifecycle = github::classify_lifecycle_labels(&issue.labels);

        // Multi-lifecycle-label normalization
        if lifecycle.len() > 1 {
            match github::normalize_multi_lifecycle_labels(
                &config.owner,
                &config.repo,
                issue.number,
                &lifecycle,
            )
            .await
            {
                Ok(true) => {
                    eprintln!(
                        "poll: normalized multi-lifecycle issue #{} to ralph:failed, skipping",
                        issue.number
                    );
                    continue;
                }
                Ok(false) => {}
                Err(err) => {
                    eprintln!(
                        "warning: failed to normalize multi-lifecycle labels on #{}: {err}",
                        issue.number
                    );
                    continue;
                }
            }
        }

        // Only claim issues with `ralph:ready` and no other lifecycle labels
        if lifecycle.len() != 1 || lifecycle[0] != "ralph:ready" {
            continue;
        }

        // Skip issues carrying in-progress PRD labels (prevents dual workflow ownership)
        if interactive_prd::has_in_progress_prd_label(&issue.labels) {
            if config.verbose {
                eprintln!(
                    "verbose: skipping issue #{} — carries in-progress PRD label, handled by interactive PRD workflow",
                    issue.number
                );
            }
            continue;
        }

        // Skip if we already have a child for this issue
        if children.contains_key(&issue.number) {
            continue;
        }

        // Skip issues owned by pr_review_phase: if a resume-pending marker or
        // staged PR-review amendments exist AND pr_review_phase can actually
        // own this issue (task metadata with pr_url exists and that PR is
        // still open), then pr_review_phase exclusively owns this issue.
        // Without this guard, a failed PR-review resume that rolls back to
        // ralph:ready would let the claim path immediately re-dispatch as
        // DispatchOrigin::Claim in the same iteration, bypassing the
        // resume-only safety path.
        //
        // However, if task metadata is missing or the PR is closed/merged,
        // pr_review_phase cannot dispatch this issue — so we must NOT block
        // the claim path.  In that case we warn and clear stale artifacts.
        if !config.pr_review_whitelist.is_empty() {
            let task_id = super::format_task_id(&config.owner, &config.repo, issue.number);
            let has_marker_or_staged =
                super::pr_review::has_resume_pending_marker(&config.workspace_root, &task_id)
                    || super::pr_review::has_staged_amendments(&config.workspace_root, &task_id);
            if has_marker_or_staged {
                // Verify pr_review_phase can actually own this issue: task
                // metadata must exist with a pr_url, and that PR must be open.
                //
                // Use strict metadata loading to distinguish NotFound from
                // corrupt/unreadable files.  Corrupt metadata must NOT cause
                // clearing of staged amendments (data-loss risk).
                let meta_result = load_task_metadata_strict(&config.workspace_root, &task_id);
                // Tri-state: Some(true) = PR open, Some(false) = PR closed/missing,
                // None = transient error (unknown).
                let pr_check_result = match &meta_result {
                    TaskMetadataLoadResult::Error(err) => {
                        eprintln!(
                            "warning: {err}; deferring claim for issue #{} to avoid \
                             clearing staged amendments",
                            issue.number
                        );
                        None
                    }
                    TaskMetadataLoadResult::NotFound => Some(false),
                    TaskMetadataLoadResult::Ok(meta) => {
                        if let Some(pr_url) = &meta.pr_url {
                            if let Some(pr_number) = github::extract_pr_number(pr_url) {
                                match github::is_pr_open(
                                    &config.owner,
                                    &config.repo,
                                    pr_number,
                                    &config.gh_bin,
                                )
                                .await
                                {
                                    Ok(true) => Some(true),
                                    Ok(false) => Some(false),
                                    Err(err) => {
                                        eprintln!(
                                            "warning: transient error checking PR state for issue #{}: {err}; \
                                             deferring claim to avoid clearing staged amendments",
                                            issue.number
                                        );
                                        None
                                    }
                                }
                            } else {
                                Some(false)
                            }
                        } else {
                            Some(false)
                        }
                    }
                };

                match pr_check_result {
                    None => {
                        // Transient error — do not clear artifacts, skip this
                        // issue this cycle.
                        continue;
                    }
                    Some(true) => {
                        if config.verbose {
                            eprintln!(
                                "verbose: skipping issue #{} — PR-review marker/staged amendments present, \
                                 owned by pr_review_phase",
                                issue.number
                            );
                        }
                        continue;
                    }
                    Some(false) => {
                        // PR is definitively closed/missing or metadata
                        // unparseable — safe to clear stale artifacts.
                        eprintln!(
                            "warning: clearing stale PR-review artifacts for issue #{} — \
                             task metadata missing or PR not open; allowing normal claim dispatch",
                            issue.number
                        );
                        super::pr_review::clear_resume_pending_marker(
                            &config.workspace_root,
                            &task_id,
                        );
                        super::pr_review::clear_staged_amendments(&config.workspace_root, &task_id);
                    }
                }
            }
        }

        // Claim: ready -> in-progress
        if let Err(err) = github::swap_lifecycle_label(
            &config.owner,
            &config.repo,
            issue.number,
            "ralph:ready",
            "ralph:in-progress",
        )
        .await
        {
            eprintln!("warning: failed to claim issue #{}: {err}", issue.number);
            continue;
        }

        // Dispatch input selection: for prd-done issues, attempt to recover
        // the approved spec from issue comments; otherwise use title/body.
        let has_prd_done = issue.labels.iter().any(|l| l == "ralph:prd-done");
        let raw_idea = if has_prd_done {
            let gh_bin = config.gh_bin.clone();
            let owner_c = config.owner.clone();
            let repo_c = config.repo.clone();
            let issue_number = issue.number;
            let spec = spawn_blocking_op(move || {
                Ok(interactive_prd::extract_approved_spec(
                    &gh_bin,
                    &owner_c,
                    &repo_c,
                    issue_number,
                ))
            })
            .await
            .unwrap_or(None);

            match spec {
                Some(s) => {
                    eprintln!("prd-done: using approved spec for issue #{}", issue.number);
                    s
                }
                None => {
                    eprintln!(
                        "approved spec not found, falling back for issue #{}",
                        issue.number
                    );
                    compose_raw_idea(&issue.title, issue.body.as_deref())
                }
            }
        } else {
            compose_raw_idea(&issue.title, issue.body.as_deref())
        };

        claimed_issues.push(ClaimedIssue {
            issue_number: issue.number,
            raw_idea,
            issue_labels: issue.labels.clone(),
        });
    }

    if claimed_issues.is_empty() {
        return Ok(());
    }

    // Stage 2: Dispatch claimed issues concurrently up to available slots.
    //
    // Each spawned task catches panics via a nested `tokio::spawn` so the
    // per-issue `issue_number` is always available in the outcome — even
    // when the dispatch worker panics.
    let mut dispatch_set: JoinSet<DispatchOutcome> = JoinSet::new();

    for claimed in claimed_issues {
        let config = config.clone();
        let repo_root_lock = repo_root_lock.clone();
        dispatch_set.spawn(async move {
            let issue_number = claimed.issue_number;
            // Inner spawn isolates panics: if dispatch_task panics, the
            // JoinHandle returns Err(JoinError) while issue_number survives
            // in the outer task.
            let inner = tokio::spawn(async move {
                dispatch_task(
                    &config,
                    issue_number,
                    &claimed.raw_idea,
                    &claimed.issue_labels,
                    &repo_root_lock,
                    DispatchOrigin::Claim,
                )
                .await
            });
            match inner.await {
                Ok(Ok(handle)) => DispatchOutcome::Success {
                    issue_number,
                    handle: Box::new(handle),
                },
                Ok(Err(err)) => DispatchOutcome::Failure {
                    issue_number,
                    detail: format!("{err}"),
                },
                Err(join_err) => {
                    let detail = format!("{join_err}");
                    DispatchOutcome::Panic {
                        issue_number,
                        detail,
                    }
                }
            }
        });
    }

    // Stage 3: Collect dispatch results and apply to children
    while let Some(join_result) = dispatch_set.join_next().await {
        let outcome = match join_result {
            Ok(outcome) => outcome,
            Err(err) => {
                // JoinError without issue identity — should not happen since
                // we catch panics above, but log defensively.
                eprintln!("warning: dispatch JoinSet task failed unexpectedly: {err}");
                continue;
            }
        };

        match outcome {
            DispatchOutcome::Success {
                issue_number,
                handle,
            } => {
                children.insert(issue_number, *handle);
            }
            DispatchOutcome::Failure {
                issue_number,
                detail,
            } => {
                eprintln!("warning: failed to dispatch issue #{issue_number}: {detail}");
                // Per-issue rollback: swap ralph:in-progress -> ralph:failed
                if let Err(rollback_err) = github::swap_lifecycle_label(
                    &config.owner,
                    &config.repo,
                    issue_number,
                    "ralph:in-progress",
                    "ralph:failed",
                )
                .await
                {
                    eprintln!(
                        "warning: dispatch rollback failed for issue #{issue_number}: {rollback_err}"
                    );
                }
            }
            DispatchOutcome::Panic {
                issue_number,
                detail,
            } => {
                eprintln!("warning: dispatch worker panicked for issue #{issue_number}: {detail}");
                // Per-issue rollback: same path as Err — swap ralph:in-progress -> ralph:failed
                if let Err(rollback_err) = github::swap_lifecycle_label(
                    &config.owner,
                    &config.repo,
                    issue_number,
                    "ralph:in-progress",
                    "ralph:failed",
                )
                .await
                {
                    eprintln!(
                        "warning: dispatch panic rollback failed for issue #{issue_number}: {rollback_err}"
                    );
                }
            }
        }
    }

    Ok(())
}

fn project_prompt_path(worktree_path: &Path, project_id: &str) -> PathBuf {
    worktree_path
        .join(".ralph")
        .join("projects")
        .join(project_id)
        .join("prompt.md")
}

fn should_resume_issue_project(worktree_path: &Path, project_id: &str) -> bool {
    project_prompt_path(worktree_path, project_id).is_file()
}

fn detect_legacy_slug_branch(worktree_path: &Path) -> Result<Option<String>> {
    let output = std::process::Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/ralph/",
        ])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to scan local ralph/* branches in {}: {err}",
                worktree_path.display()
            ))
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    let mut branches: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    branches.sort();

    for branch in branches {
        if !branch.starts_with("ralph/") {
            continue;
        }
        if branch.starts_with("ralph/issue-") || branch.starts_with("ralph/daemon/") {
            continue;
        }
        return Ok(Some(branch));
    }

    Ok(None)
}

fn validate_daemon_branch_format(branch_format: &str) -> Result<()> {
    // Validate two distinct project IDs to reject constant formats (e.g. "ralph/issue-1")
    // that accidentally pass a single-ID check.
    for (project_id, expected) in [("issue-1", "ralph/issue-1"), ("issue-2", "ralph/issue-2")] {
        let rendered = crate::git::branch::resolve_branch_name(branch_format, project_id);
        if rendered != expected {
            return Err(RalphError::Validation(format!(
                "incompatible git.branch_format for daemon-managed issue dispatch: \
                 formatting project_id '{project_id}' must produce '{expected}', \
                 but '{branch_format}' produced '{rendered}'"
            )));
        }
    }
    Ok(())
}

/// Origin of a dispatch call, used to scope PR-review-specific logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchOrigin {
    /// Normal claim flow (poll_and_claim).
    Claim,
    /// PR-review resume flow (pr_review_phase).
    PrReviewResume,
}

/// Dispatch a single task: create worktree, spawn child, track in-memory.
async fn dispatch_task(
    config: &DaemonRuntimeConfig,
    issue_number: u32,
    raw_idea: &str,
    issue_labels: &[String],
    repo_root_lock: &Arc<Semaphore>,
    origin: DispatchOrigin,
) -> Result<TaskHandle> {
    let task_id = format_task_id(&config.owner, &config.repo, issue_number);
    let project_id = format!("issue-{issue_number}");
    let branch_name = crate::git::branch::resolve_branch_name(
        &config.global_config.git.branch_format,
        &project_id,
    );

    bootstrap::ensure_repo_ready(&config.repo_root, Some(repo_root_lock.clone())).await?;

    let workspace_root = config.workspace_root.clone();

    // Create worktree (reuses existing branch if present).
    let wt_path = {
        let _permit = repo_root_lock
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| {
                RalphError::Orchestration(format!("git root semaphore closed: {err}"))
            })?;
        let repo_root = config.repo_root.clone();
        let ws_root = workspace_root.clone();
        let tid = task_id.clone();
        let branch_name_clone = branch_name.clone();
        let lock = Some(repo_root_lock.clone());
        spawn_blocking_op(move || {
            worktree::create_worktree(&repo_root, &ws_root, &tid, &branch_name_clone, lock)
        })
        .await?
    };

    // Clean worktree of any dirty files from previous runs
    {
        let wt = wt_path.clone();
        spawn_blocking_op(move || worktree::clean_worktree(&wt)).await?;
    }

    // Remote-first project branch sync — must run BEFORE discovery so that
    // `ralph/issue-{n}` (which contains committed project data) is checked
    // out when we scan `.ralph/projects/`.
    {
        let _permit = repo_root_lock
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| {
                RalphError::Orchestration(format!("git root semaphore closed: {err}"))
            })?;
        let wt = wt_path.clone();
        let base_branch = config.base_branch.clone();
        match spawn_blocking_op(move || {
            crate::git::branch::sync_project_branch(&wt, issue_number, &base_branch)
        })
        .await
        {
            Ok(()) => {
                eprintln!(
                    "dispatch: remote-first sync completed for issue {issue_number} (task {task_id})"
                );
            }
            Err(err) => {
                eprintln!(
                    "dispatch: remote-first sync failed for issue {issue_number} (task {task_id}): {err}"
                );
                return Err(err);
            }
        }
    }

    let resume_existing_project = {
        let wt = wt_path.clone();
        let pid = project_id.clone();
        spawn_blocking_op(move || Ok(should_resume_issue_project(&wt, &pid))).await?
    };

    // PrReviewResume dispatches MUST resume an existing project.  If the
    // project state is missing or corrupt (no prompt.md on the branch),
    // fall back would start a fresh implementation cycle with a placeholder
    // prompt — which is never correct.  Fail fast *before* draining staged
    // amendments so pr_review_phase can roll back the label swap and
    // preserve staged amendments without side effects.
    if origin == DispatchOrigin::PrReviewResume && !resume_existing_project {
        return Err(RalphError::Orchestration(format!(
            "PrReviewResume dispatch for {task_id} cannot resume: project state not found in worktree \
             (prompt.md missing on branch {branch_name}); aborting to preserve staged amendments"
        )));
    }

    // Drain any staged PR review amendments into the project's amendment queue.
    // Both drain and purge are gated to PrReviewResume dispatches only — running
    // them on normal Claim paths would consume staged amendments without the
    // accompanying state reset, causing quick-dev short-circuits to skip
    // processing and lose staged feedback.
    let drained_count = if origin == DispatchOrigin::PrReviewResume {
        let ws = config.workspace_root.clone();
        let tid = task_id.clone();
        let pid = project_id.clone();
        let wt = wt_path.clone();
        let is_quick = issue_labels.iter().any(|l| l == "ralph:quick");
        let drained = spawn_blocking_op(move || {
            if !super::pr_review::has_staged_amendments(&ws, &tid) {
                return Ok(0);
            }
            let project_dir = wt.join(".ralph").join("projects").join(&pid);
            if project_dir.exists() {
                let count = super::pr_review::drain_staged_amendments(&ws, &tid, &project_dir)?;
                if count > 0 {
                    super::pr_review::reset_project_state_for_resume(&project_dir, is_quick)?;
                }
                Ok(count)
            } else {
                Ok(0)
            }
        })
        .await?;
        if drained > 0 {
            eprintln!("dispatch: drained {drained} staged PR review amendment(s) for {task_id}");
        }
        drained
    } else {
        0
    };

    if resume_existing_project {
        eprintln!("dispatch: event=project_resume task_id={task_id} project_id={project_id}");
    } else {
        let legacy_slug_branch = {
            let wt = wt_path.clone();
            spawn_blocking_op(move || detect_legacy_slug_branch(&wt)).await?
        };

        if let Some(legacy_branch) = legacy_slug_branch {
            eprintln!(
                "warning: dispatch detected legacy branch '{legacy_branch}' for issue #{issue_number}; \
                 starting fresh as '{project_id}' instead of resuming a slug project"
            );
        }
    }

    // Skip refinement, issue title/body updates, and refined-prompt comment for
    // resumed projects — the issue already has real content from the original
    // dispatch, and raw_idea may be a placeholder (e.g. PR review resume).
    let idea = if resume_existing_project {
        raw_idea.to_owned()
    } else {
        // Refine the prompt if enabled.
        // For prd-done issues, skip refinement entirely to preserve the exact
        // approved spec (or compose_raw_idea fallback) as the dispatch payload.
        let has_prd_done = issue_labels.iter().any(|l| l == "ralph:prd-done");
        let (idea, refined_title, cleaned_body) = if config.refinement_enabled && !has_prd_done {
            match refine::refine_prompt(raw_idea, &config.refinement_backend, &config.global_config)
                .await
            {
                Ok(refined) => (refined.body, refined.title, refined.cleaned_body),
                Err(err) => {
                    eprintln!(
                        "warning: refinement failed for task {task_id}, using raw idea: {err}"
                    );
                    (raw_idea.to_owned(), None, None)
                }
            }
        } else {
            (raw_idea.to_owned(), None, None)
        };

        // Update GitHub issue title with refined title (best-effort)
        if let Some(ref title) = refined_title {
            if let Err(err) =
                github::update_issue_title(&config.owner, &config.repo, issue_number, title).await
            {
                eprintln!("warning: failed to update issue title for {task_id}: {err}");
            }
        }

        // Update GitHub issue body with cleaned body (best-effort)
        if let Some(ref cleaned_body) = cleaned_body {
            if let Err(err) =
                github::update_issue_body(&config.owner, &config.repo, issue_number, cleaned_body)
                    .await
            {
                eprintln!("warning: failed to update issue body for {task_id}: {err}");
            }
        }

        // Post refined-prompt comment (best-effort)
        {
            let comment_body = match &refined_title {
                Some(title) => format!("**{title}**\n\n{idea}"),
                None => idea.clone(),
            };
            if let Err(err) = github::post_idempotent_comment(
                &config.owner,
                &config.repo,
                issue_number,
                &task_id,
                "refined-prompt",
                &comment_body,
            )
            .await
            {
                eprintln!("warning: failed to post refined-prompt comment for {task_id}: {err}");
            }
        }

        idea
    };

    // Create log file for child output
    let log_path = task_log_path(&config.workspace_root, &task_id);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Ignore stale artifacts left from prior runs.  Subtract 2 seconds to
    // tolerate filesystems that truncate mtime to whole-second granularity
    // (e.g. tmpfs in nix build sandboxes).
    let child_start_time = SystemTime::now() - Duration::from_secs(2);

    // Resolve PR URL: first check durable metadata (survives daemon restarts),
    // then fall back to GitHub API lookup by exact head-branch match.
    let persisted_meta = load_task_metadata(&config.workspace_root, &task_id);
    let pr_url: Option<String> = if persisted_meta.pr_url.is_some() {
        eprintln!(
            "dispatch: recovered persisted PR URL for {task_id}: {}",
            persisted_meta.pr_url.as_deref().unwrap_or("none")
        );
        persisted_meta.pr_url
    } else {
        match github::find_existing_pr(&config.owner, &config.repo, &branch_name).await {
            Ok(url) => url,
            Err(err) => {
                eprintln!("dispatch: PR URL lookup failed for {task_id}: {err}");
                None
            }
        }
    };

    eprintln!(
        "dispatch: resolved PR URL for {task_id}: {}",
        pr_url.as_deref().unwrap_or("none")
    );

    // Spawn in-process task — branch by `ralph:quick` label for quick-dev flow
    let is_quick = issue_labels.iter().any(|l| l == "ralph:quick");
    let cancel_token = CancellationToken::new();
    let wt = wt_path.clone();

    let join_handle = {
        let cancel = cancel_token.clone();
        match (is_quick, resume_existing_project) {
            (true, true) => {
                eprintln!(
                    "dispatch: task {task_id} resuming with quick-dev-run --project {project_id} pr_url={}",
                    pr_url.as_deref().unwrap_or("none")
                );
                let params = super::tasks::QuickDevRunTaskParams {
                    workspace_root: wt,
                    project: Some(project_id.clone()),
                    pr_url: pr_url.clone(),
                    cancel,
                    max_backend_retries: config.max_backend_retries,
                    implementer_backend: None,
                    reviewer_backend: None,
                    skip_commit: false,
                    max_review_iterations: None,
                    max_final_review_retries: None,
                };
                super::tasks::spawn_inprocess_task(
                    || super::tasks::run_quick_dev_run_task(params),
                    &log_path,
                )?
            }
            (true, false) => {
                eprintln!(
                    "dispatch: task {task_id} starting fresh with quick-dev-auto --project-id {project_id} pr_url={}",
                    pr_url.as_deref().unwrap_or("none")
                );
                let params = super::tasks::QuickDevAutoTaskParams {
                    workspace_root: wt,
                    idea: idea.clone(),
                    project_id: Some(project_id.clone()),
                    pr_url: pr_url.clone(),
                    cancel,
                    max_backend_retries: config.max_backend_retries,
                    implementer_backend: None,
                    reviewer_backend: None,
                    skip_commit: false,
                    max_review_iterations: None,
                    max_final_review_retries: None,
                };
                super::tasks::spawn_inprocess_task(
                    || super::tasks::run_quick_dev_auto_task(params),
                    &log_path,
                )?
            }
            (false, true) => {
                eprintln!(
                    "dispatch: task {task_id} resuming with run --project {project_id} pr_url={}",
                    pr_url.as_deref().unwrap_or("none")
                );
                let params = super::tasks::RunTaskParams {
                    workspace_root: wt,
                    project: Some(project_id.clone()),
                    pr_url: pr_url.clone(),
                    cancel,
                    max_backend_retries: config.max_backend_retries,
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
                };
                super::tasks::spawn_inprocess_task(
                    || super::tasks::run_run_task(params),
                    &log_path,
                )?
            }
            (false, false) => {
                eprintln!(
                    "dispatch: task {task_id} starting fresh with auto --project-id {project_id} pr_url={}",
                    pr_url.as_deref().unwrap_or("none")
                );
                let params = super::tasks::AutoTaskParams {
                    workspace_root: wt,
                    idea: idea.clone(),
                    project_id: Some(project_id.clone()),
                    pr_url: pr_url.clone(),
                    cancel,
                    max_backend_retries: config.max_backend_retries,
                    spec_writer: None,
                    spec_reviewer: None,
                    max_spec_revisions: 1,
                    backend: None,
                    planner_backend: None,
                    implementer_backend: None,
                    reviewer_backend: None,
                    qa_backend: None,
                    completer_backend: None,
                    tmux: None,
                    skip_commit: false,
                    skip_prompt_review: false,
                    dry_run: false,
                };
                super::tasks::spawn_inprocess_task(
                    || super::tasks::run_auto_task(params),
                    &log_path,
                )?
            }
        }
    };

    let watcher_cancel = CancellationToken::new();
    let watcher_handle = if !config.owner.is_empty() && !config.repo.is_empty() {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let tid = task_id.clone();
        let wt = wt_path.clone();
        let cancel = watcher_cancel.clone();
        Some(tokio::spawn(async move {
            post_artifact_comments(owner, repo, issue_number, tid, wt, child_start_time, cancel)
                .await;
        }))
    } else {
        None
    };

    // Start draft-PR watcher: polls for branch divergence and creates a
    // draft PR when the branch first moves ahead of the base branch.
    let draft_pr_cancel = CancellationToken::new();
    let draft_pr_handle = if !config.owner.is_empty() && !config.repo.is_empty() && pr_url.is_none()
    {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let base_branch = config.base_branch.clone();
        let wt = wt_path.clone();
        let branch = branch_name.clone();
        let cancel = draft_pr_cancel.clone();
        let tid = task_id.clone();
        let ws_root = config.workspace_root.clone();
        Some(tokio::spawn(async move {
            draft_pr_watcher(
                owner,
                repo,
                base_branch,
                wt,
                branch,
                tid,
                issue_number,
                cancel,
                ws_root,
            )
            .await;
        }))
    } else {
        None
    };

    // Staged amendment files were copied (not moved) during drain — now that
    // spawn succeeded, purge the originals so they are not re-drained on a
    // future cycle.  Only purge when we actually drained amendments.
    if drained_count > 0 {
        super::pr_review::purge_staged_amendments(&config.workspace_root, &task_id);
    }

    eprintln!("dispatched task {task_id} (in-process)");

    Ok(TaskHandle {
        join_handle,
        cancel_token,
        aborted_externally: Arc::new(AtomicBool::new(false)),
        watcher_cancel,
        watcher_handle,
        draft_pr_cancel,
        draft_pr_handle,
        branch: branch_name,
        log_file: log_path,
        last_rebase_at: None,
        last_rebase_failure_sha: None,
        pr_url,
    })
}

fn compose_raw_idea(title: &str, body: Option<&str>) -> String {
    format!("{title}\n\n{}", body.unwrap_or_default())
}

/// Extract the original title from a raw idea string.
pub fn extract_original_title(raw_idea: &str) -> Option<String> {
    let segment = match raw_idea.split_once("\n\n") {
        Some((before, _)) => before,
        None => raw_idea,
    };
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

async fn await_watcher_with_timeout(
    join_handle: tokio::task::JoinHandle<()>,
    watcher_name: &str,
    task_id: &str,
) {
    await_watcher_with_timeout_impl(join_handle, watcher_name, task_id, WATCHER_TEARDOWN_TIMEOUT)
        .await;
}

async fn await_watcher_with_timeout_impl(
    join_handle: tokio::task::JoinHandle<()>,
    watcher_name: &str,
    task_id: &str,
    timeout: Duration,
) {
    let mut join_handle = join_handle;
    match tokio::time::timeout(timeout, &mut join_handle).await {
        Ok(Ok(())) => {}
        Ok(Err(join_err)) => {
            eprintln!("warning: {watcher_name} join failed for {task_id}: {join_err}");
        }
        Err(_) => {
            join_handle.abort();
            eprintln!("warning: {watcher_name} teardown timed out for {task_id}, aborted");
        }
    }
}

/// Derive a terminal label from the result of awaiting a task's `JoinHandle`.
///
/// - `Ok(Ok(_))` → `"ralph:completed"` (task succeeded)
/// - `Ok(Err(Cancelled))` → `"ralph:failed"` (cooperative cancellation)
/// - `Ok(Err(_))` → `"ralph:failed"` (task error)
/// - `Err(JoinError)` → `"ralph:failed"` (task panicked)
fn derive_terminal_label(
    result: &std::result::Result<
        crate::Result<crate::workflow::orchestrator::OrchestrationResult>,
        tokio::task::JoinError,
    >,
) -> &'static str {
    match result {
        Ok(Ok(_)) => "ralph:completed",
        Ok(Err(_)) => "ralph:failed",
        Err(_) => "ralph:failed",
    }
}

/// Collect finished children and transition them to terminal states via labels.
async fn collect_children(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, TaskHandle>,
    repo_root_lock: &Arc<Semaphore>,
) {
    // Stage 1: Sequential task status scan via JoinHandle::is_finished()
    let mut finished: Vec<(u32, &'static str)> = Vec::new();
    let mut still_running = 0u32;

    for (issue_number, handle) in children.iter_mut() {
        if handle.join_handle.is_finished() {
            let task_id = format_task_id(&config.owner, &config.repo, *issue_number);
            if config.verbose {
                eprintln!("verbose: task terminal task_id={task_id}");
            }
            // We cannot await the join handle yet because we only have &mut.
            // Mark as finished; we'll resolve the Result in stage 2.
            finished.push((*issue_number, "pending_resolve"));
        } else {
            still_running = still_running.saturating_add(1);
        }
    }

    if config.verbose && still_running > 0 {
        eprintln!("verbose: child collection still_running={still_running}");
    }

    // Stage 2: Per-task teardown — resolve JoinHandle result to determine
    // terminal label, then tear down watchers.
    let mut completion_tasks: Vec<(u32, String, &'static str, bool)> = Vec::new();
    for (issue_number, _) in finished {
        let task_id = format_task_id(&config.owner, &config.repo, issue_number);
        let Some(mut handle) = children.remove(&issue_number) else {
            continue;
        };

        // Resolve the task result from the JoinHandle.
        let join_result = (&mut handle.join_handle).await;
        let was_externally_aborted = handle
            .aborted_externally
            .load(std::sync::atomic::Ordering::SeqCst);
        let terminal_label = if was_externally_aborted {
            eprintln!("collect: task {task_id} was externally aborted, forcing ralph:failed");
            "ralph:failed"
        } else {
            derive_terminal_label(&join_result)
        };
        match &join_result {
            Ok(Err(ref err)) => {
                if matches!(err, RalphError::Cancelled) {
                    eprintln!("collect: task {task_id} cancelled");
                } else {
                    eprintln!("collect: task {task_id} failed: {err}");
                }
            }
            Err(join_err) => {
                eprintln!("collect: task {task_id} panicked: {join_err}");
            }
            _ => {}
        }

        handle.watcher_cancel.cancel();
        if let Some(join_handle) = handle.watcher_handle.take() {
            await_watcher_with_timeout(join_handle, "artifact watcher", &task_id).await;
        }
        handle.draft_pr_cancel.cancel();
        if let Some(join_handle) = handle.draft_pr_handle.take() {
            await_watcher_with_timeout(join_handle, "draft PR watcher", &task_id).await;
        }
        if terminal_label == "ralph:failed" {
            print_log_tail(&task_id, &handle.log_file);
        }
        completion_tasks.push((
            issue_number,
            task_id,
            terminal_label,
            was_externally_aborted,
        ));
    }

    // Stage 3: Run complete_task concurrently across finished children via JoinSet.
    //
    // Each completion worker returns a `CompletionOutcome` carrying the
    // `issue_number` so panics/errors are tied to a specific issue.  If a
    // worker panics, the issue is explicitly transitioned to `ralph:failed`.
    if completion_tasks.is_empty() {
        return;
    }

    let mut complete_set: JoinSet<CompletionOutcome> = JoinSet::new();
    for (issue_number, task_id, terminal_label, externally_aborted) in completion_tasks {
        let config = config.clone();
        let repo_root_lock = repo_root_lock.clone();
        let tid = task_id.clone();
        complete_set.spawn(async move {
            // Inner spawn isolates panics so issue_number is preserved.
            let inner = tokio::spawn(async move {
                complete_task(
                    &config,
                    issue_number,
                    &tid,
                    terminal_label,
                    externally_aborted,
                    &repo_root_lock,
                )
                .await;
            });
            match inner.await {
                Ok(()) => CompletionOutcome::Done {
                    issue_number,
                    task_id,
                },
                Err(join_err) => CompletionOutcome::Panic {
                    issue_number,
                    task_id,
                    detail: format!("{join_err}"),
                },
            }
        });
    }

    while let Some(join_result) = complete_set.join_next().await {
        let outcome = match join_result {
            Ok(outcome) => outcome,
            Err(err) => {
                eprintln!("warning: completion JoinSet task failed unexpectedly: {err}");
                continue;
            }
        };

        match outcome {
            CompletionOutcome::Done { .. } => {}
            CompletionOutcome::Panic {
                issue_number,
                task_id,
                detail,
            } => {
                eprintln!(
                    "warning: complete_task panicked for {task_id} (issue #{issue_number}): {detail}"
                );
                // Explicitly transition to terminal failure state so the
                // issue does not remain stuck as ralph:in-progress.
                if let Err(rollback_err) = github::swap_lifecycle_label(
                    &config.owner,
                    &config.repo,
                    issue_number,
                    "ralph:in-progress",
                    "ralph:failed",
                )
                .await
                {
                    eprintln!(
                        "warning: completion panic rollback failed for {task_id} (issue #{issue_number}): {rollback_err}"
                    );
                }
            }
        }
    }
}

/// Kill running children whose issues have been externally aborted (e.g. via
/// `ralph daemon abort`).  The CLI abort swaps the issue label to
/// `ralph:failed` but cannot kill the process (no PID access).  This function
/// queries labels for each running child and terminates any that are no longer
/// `ralph:in-progress`.
async fn kill_aborted_children(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, TaskHandle>,
    _repo_root_lock: &Arc<Semaphore>,
) {
    let issue_numbers: Vec<u32> = children.keys().cloned().collect();
    let mut to_kill = Vec::new();

    // Query labels concurrently via JoinSet, capped at max(1, config.max_concurrent)
    let concurrency_cap = std::cmp::max(1, config.max_concurrent) as usize;
    let mut join_set: JoinSet<(u32, String, std::result::Result<Vec<String>, RalphError>)> =
        JoinSet::new();

    let mut pending = issue_numbers.into_iter();
    let mut in_flight = 0usize;

    loop {
        // Fill up to concurrency cap
        while in_flight < concurrency_cap {
            if let Some(issue_number) = pending.next() {
                let task_id = format_task_id(&config.owner, &config.repo, issue_number);
                let owner = config.owner.clone();
                let repo = config.repo.clone();
                join_set.spawn(async move {
                    let result = github::fetch_issue_labels(&owner, &repo, issue_number).await;
                    (issue_number, task_id, result)
                });
                in_flight += 1;
            } else {
                break;
            }
        }

        if in_flight == 0 {
            break;
        }

        // Await next completed label fetch
        if let Some(result) = join_set.join_next().await {
            in_flight -= 1;
            match result {
                Ok((issue_number, task_id, Ok(labels))) => {
                    if !labels.iter().any(|l| l == "ralph:in-progress") {
                        eprintln!(
                            "abort-check: task {task_id} no longer in-progress (labels: {}), killing",
                            labels.join(", ")
                        );
                        to_kill.push(issue_number);
                    }
                }
                Ok((_, task_id, Err(err))) => {
                    eprintln!("abort-check: failed to query labels for {task_id}: {err}");
                }
                Err(err) => {
                    eprintln!("abort-check: label fetch task panicked: {err}");
                }
            }
        }
    }

    // Cancel aborted tasks — cooperative cancellation via token.
    // Also cancel watchers immediately so they stop acting while the
    // task winds down.  Don't remove from children map; let
    // collect_children() observe the JoinHandle completion on the next
    // cycle.
    for issue_number in to_kill {
        if let Some(handle) = children.get(&issue_number) {
            let task_id = format_task_id(&config.owner, &config.repo, issue_number);
            handle
                .aborted_externally
                .store(true, std::sync::atomic::Ordering::SeqCst);
            handle.cancel_token.cancel();
            handle.watcher_cancel.cancel();
            handle.draft_pr_cancel.cancel();
            eprintln!("abort-check: cancelled task {task_id} (externally aborted)");
        }
    }
}

/// Wait until all active tasks have exited.
async fn drain_all_children(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, TaskHandle>,
    repo_root_lock: &Arc<Semaphore>,
) {
    drain_all_children_with_deadline(config, children, repo_root_lock, Duration::from_secs(7200))
        .await;
}

/// Inner implementation with a configurable drain deadline (testable).
async fn drain_all_children_with_deadline(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, TaskHandle>,
    repo_root_lock: &Arc<Semaphore>,
    drain_timeout: Duration,
) {
    // Cancel all tasks and their watchers to initiate cooperative shutdown.
    // Cancelling watcher tokens here prevents side effects (e.g. draft-PR
    // creation) from racing with task teardown during the drain period.
    for handle in children.values() {
        handle.cancel_token.cancel();
        handle.watcher_cancel.cancel();
        handle.draft_pr_cancel.cancel();
    }

    let deadline = tokio::time::Instant::now() + drain_timeout;

    while !children.is_empty() && tokio::time::Instant::now() < deadline {
        collect_children(config, children, repo_root_lock).await;
        if children.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Force-abort remaining tasks.  We abort the JoinHandle, then await
    // it with a bounded timeout so we only mark the task as failed after
    // the task has actually stopped executing.  This prevents labelling a
    // task as terminal while it is still mutating git state.
    if !children.is_empty() {
        let remaining: Vec<u32> = children.keys().cloned().collect();
        for issue_number in remaining {
            let task_id = format_task_id(&config.owner, &config.repo, issue_number);
            if let Some(mut handle) = children.remove(&issue_number) {
                eprintln!("warning: force-aborting task {task_id} (drain timeout)");
                handle.join_handle.abort();
                // Wait for the aborted task to actually resolve (up to 10s).
                // abort() is cooperative — it only takes effect at .await
                // points.  This bounded wait ensures we don't label the task
                // as failed while it is still running blocking code.
                let task_resolved =
                    tokio::time::timeout(Duration::from_secs(10), &mut handle.join_handle)
                        .await
                        .is_ok();
                if !task_resolved {
                    // The task is still running — we must NOT proceed with
                    // complete_task (which cleans up the worktree) because the
                    // task may still be mutating git state.  Only swap the
                    // label so the issue is not stuck as in-progress.
                    eprintln!(
                        "warning: task {task_id} did not resolve within 10s after abort, \
                         skipping worktree cleanup"
                    );
                    if let Err(err) = github::swap_lifecycle_label(
                        &config.owner,
                        &config.repo,
                        issue_number,
                        "ralph:in-progress",
                        "ralph:failed",
                    )
                    .await
                    {
                        eprintln!(
                            "warning: swap_lifecycle_label failed for {task_id}, \
                             falling back to add_label_with_retry: {err}"
                        );
                        let _ = github::add_label_with_retry(
                            &config.owner,
                            &config.repo,
                            issue_number,
                            "ralph:failed",
                        )
                        .await;
                    }
                    // Still cancel and await watcher tasks so in-flight GitHub
                    // operations (draft PR creation, artifact comments) don't
                    // complete after the issue is marked failed.
                    handle.watcher_cancel.cancel();
                    if let Some(join_handle) = handle.watcher_handle.take() {
                        await_watcher_with_timeout(join_handle, "artifact watcher", &task_id).await;
                    }
                    handle.draft_pr_cancel.cancel();
                    if let Some(join_handle) = handle.draft_pr_handle.take() {
                        await_watcher_with_timeout(join_handle, "draft PR watcher", &task_id).await;
                    }
                    continue;
                }
                handle.watcher_cancel.cancel();
                if let Some(join_handle) = handle.watcher_handle.take() {
                    await_watcher_with_timeout(join_handle, "artifact watcher", &task_id).await;
                }
                handle.draft_pr_cancel.cancel();
                if let Some(join_handle) = handle.draft_pr_handle.take() {
                    await_watcher_with_timeout(join_handle, "draft PR watcher", &task_id).await;
                }

                // Panic-isolate complete_task so that a panic in one task's
                // completion does not prevent subsequent tasks from completing
                // (matching the pattern used in collect_children).
                let config_clone = config.clone();
                let repo_root_lock_clone = repo_root_lock.clone();
                let tid = task_id.clone();
                let externally_aborted = handle
                    .aborted_externally
                    .load(std::sync::atomic::Ordering::SeqCst);
                let inner = tokio::spawn(async move {
                    complete_task(
                        &config_clone,
                        issue_number,
                        &tid,
                        "ralph:failed",
                        externally_aborted,
                        &repo_root_lock_clone,
                    )
                    .await;
                });
                if let Err(join_err) = inner.await {
                    eprintln!(
                        "warning: complete_task panicked for {task_id} during drain: {join_err}"
                    );
                    if let Err(rollback_err) = github::swap_lifecycle_label(
                        &config.owner,
                        &config.repo,
                        issue_number,
                        "ralph:in-progress",
                        "ralph:failed",
                    )
                    .await
                    {
                        eprintln!(
                            "warning: drain panic rollback failed for {task_id}: {rollback_err}"
                        );
                    }
                }
            }
        }
    }
}

/// Transition a task to terminal state via GitHub labels.
pub(crate) fn should_retry_complete_task(err: &RalphError, attempt: u32) -> bool {
    attempt < COMPLETE_TASK_MAX_ATTEMPTS && err.is_transient()
}

pub(crate) fn should_mark_draft_pr_ready(
    has_changes: bool,
    pr_is_draft: bool,
    terminal_label: &str,
) -> bool {
    has_changes && pr_is_draft && terminal_label == "ralph:completed"
}

pub(crate) fn should_close_no_diff_draft_pr(has_changes: bool, pr_is_draft: bool) -> bool {
    !has_changes && pr_is_draft
}

pub(crate) fn decide_draft_pr_transition(
    has_changes: bool,
    pr_is_draft: bool,
    terminal_label: &str,
) -> DraftPrTransition {
    if should_close_no_diff_draft_pr(has_changes, pr_is_draft) {
        return DraftPrTransition::CloseNoDiff;
    }
    if should_mark_draft_pr_ready(has_changes, pr_is_draft, terminal_label) {
        return DraftPrTransition::MarkReady;
    }
    DraftPrTransition::None
}

pub(crate) fn complete_task_retry_delay(err: &RalphError, attempt: u32) -> Option<Duration> {
    if should_retry_complete_task(err, attempt) {
        Some(Duration::from_secs(COMPLETE_TASK_RETRY_DELAY_SECS))
    } else {
        None
    }
}

async fn complete_task(
    config: &DaemonRuntimeConfig,
    issue_number: u32,
    task_id: &str,
    terminal_label: &str,
    externally_aborted: bool,
    repo_root_lock: &Arc<Semaphore>,
) {
    for attempt in 1..=COMPLETE_TASK_MAX_ATTEMPTS {
        match complete_task_attempt(
            config,
            issue_number,
            task_id,
            terminal_label,
            externally_aborted,
            repo_root_lock,
        )
        .await
        {
            Ok(()) => return,
            Err(err) => {
                if let Some(delay) = complete_task_retry_delay(&err, attempt) {
                    eprintln!(
                        "warning: complete_task transient failure for {task_id} on attempt {attempt}/{COMPLETE_TASK_MAX_ATTEMPTS}: {err}; retrying in {COMPLETE_TASK_RETRY_DELAY_SECS}s"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }

                eprintln!(
                    "warning: complete_task failed for {task_id} on attempt {attempt}/{COMPLETE_TASK_MAX_ATTEMPTS}: {err}"
                );
                return;
            }
        }
    }
}

async fn complete_task_attempt(
    config: &DaemonRuntimeConfig,
    issue_number: u32,
    task_id: &str,
    terminal_label: &str,
    externally_aborted: bool,
    repo_root_lock: &Arc<Semaphore>,
) -> Result<()> {
    // Post completion comment (best-effort, idempotent)
    {
        let phase = if externally_aborted {
            "aborted"
        } else {
            terminal_label.trim_start_matches("ralph:")
        };
        let comment_body = format!("Task `{task_id}` finished with status: **{phase}**.");
        github::post_idempotent_comment(
            &config.owner,
            &config.repo,
            issue_number,
            task_id,
            phase,
            &comment_body,
        )
        .await?;
    }

    // PR flow (only on success, never for externally aborted tasks)
    if terminal_label == "ralph:completed" && !externally_aborted {
        // Resolve actual worktree branch for PR creation
        let workspace_root = config.workspace_root.clone();
        let wt_path = worktree::task_worktree_path(&workspace_root, task_id);
        if wt_path.exists() {
            if let Err(err) = handle_pr_flow(config, task_id, issue_number, &wt_path).await {
                eprintln!("warning: PR flow failed for {task_id} (best-effort, continuing to label swap): {err}");
            }
        }
    }

    // Swap lifecycle label: in-progress -> terminal.
    // When externally aborted, the label was already changed outside the
    // daemon (e.g. `ralph:in-progress` removed).  Skip the swap to avoid
    // error-looping on a label that no longer exists.
    if externally_aborted {
        // Best-effort: ensure terminal label is present even if external
        // actor only removed in-progress without adding failed.
        if let Err(err) =
            github::add_label_with_retry(&config.owner, &config.repo, issue_number, terminal_label)
                .await
        {
            eprintln!(
                "warning: failed to ensure {terminal_label} for externally aborted {task_id}: {err}"
            );
        }
    } else {
        github::swap_lifecycle_label(
            &config.owner,
            &config.repo,
            issue_number,
            "ralph:in-progress",
            terminal_label,
        )
        .await?;
    }

    // Clear resume-pending marker now that the task has reached a terminal
    // state.  This is the durable source-of-truth: the marker persists from
    // pr_review_phase through dispatch and task execution, and is only cleared
    // here — ensuring crash recovery at any earlier point can still detect the
    // in-flight resume.
    super::pr_review::clear_resume_pending_marker(&config.workspace_root, task_id);

    // Worktree cleanup
    cleanup_worktree_for_terminal_state(config, task_id, terminal_label, repo_root_lock).await;

    let log_path = task_log_path(&config.workspace_root, task_id);
    eprintln!(
        "task {task_id} completed with label: {terminal_label} (log: {})",
        log_path.display()
    );

    Ok(())
}

async fn cleanup_worktree_for_terminal_state(
    config: &DaemonRuntimeConfig,
    task_id: &str,
    terminal_label: &str,
    repo_root_lock: &Arc<Semaphore>,
) {
    if should_cleanup_worktree(terminal_label) {
        eprintln!(
            "complete-task-terminal: cleaning worktree for {task_id} (label={terminal_label})"
        );
        cleanup_worktree(config, task_id, repo_root_lock).await;
        return;
    }

    eprintln!("complete-task-terminal: preserving worktree for {task_id} (label={terminal_label})");
}

/// Remove the worktree for a task (best-effort).
async fn cleanup_worktree(
    config: &DaemonRuntimeConfig,
    task_id: &str,
    repo_root_lock: &Arc<Semaphore>,
) {
    let workspace_root = config.workspace_root.clone();
    let repo_root = config.repo_root.clone();
    let tid = task_id.to_owned();
    let lock = repo_root_lock.clone();

    let _permit = repo_root_lock
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| RalphError::Orchestration(format!("git root semaphore closed: {err}")))
        .ok();
    if let Err(err) = spawn_blocking_op(move || {
        worktree::remove_worktree(&repo_root, &workspace_root, &tid, Some(lock));
        Ok(())
    })
    .await
    {
        eprintln!("warning: failed to cleanup worktree for {task_id}: {err}");
    }
}

// =============================================================================
// Auto-Rebase Phase
// =============================================================================

/// Run the auto-rebase phase: rebase eligible PR-backed task branches onto
/// their PR base branch.
///
/// Iterates active children in deterministic ascending issue_number order,
/// capped at `max_rebases_per_cycle`. Each rebase attempt is bounded by
/// `rebase_timeout_seconds`.
/// Candidate for rebase: contains all metadata needed to execute the rebase
/// concurrently without needing mutable access to `children`.
struct RebaseCandidate {
    issue_number: u32,
    task_id: String,
    branch: String,
    last_failure_sha: Option<String>,
    rebase_target: String,
    head_sha: String,
    pr_number: u32,
}

/// Outcome of a concurrent rebase operation.
enum RebaseOutcome {
    Success {
        issue_number: u32,
        task_id: String,
    },
    Failure {
        issue_number: u32,
        task_id: String,
        head_sha: String,
        last_failure_sha: Option<String>,
        pr_number: u32,
        error: String,
        is_lease: bool,
    },
}

/// PR review polling phase: detect review comments from whitelisted users on
/// open PRs, stage amendments, and re-dispatch completed projects.
///
/// In addition to newly-discovered amendments, this also retries dispatch for
/// tasks that have staged amendments from previous cycles (e.g. deferred due to
/// capacity constraints).
async fn pr_review_phase(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, TaskHandle>,
    repo_root_lock: &Arc<Semaphore>,
) -> Result<()> {
    // Per-cycle cache for PR open state shared between polling and dispatch phases
    // to avoid redundant GitHub API calls for the same PR.
    let mut pr_open_cache: HashMap<u32, bool> = HashMap::new();

    let poll_results =
        match super::pr_review::poll_pr_reviews(config, children, &mut pr_open_cache).await {
            Ok(results) => results,
            Err(err) => {
                eprintln!(
                    "warning: PR review polling failed, continuing with staged amendments: {err}"
                );
                Vec::new()
            }
        };

    // Build the set of task_ids that received new amendments this cycle.
    let newly_staged: std::collections::HashSet<String> =
        poll_results.iter().map(|r| r.task_id.clone()).collect();

    // Also discover tasks with previously-staged amendments (deferred on earlier
    // cycles due to capacity or other transient issues).
    let all_tasks = super::pr_review::discover_tasks_with_prs(
        &config.workspace_root,
        &config.owner,
        &config.repo,
    );

    // Merge: poll_results + tasks with staged amendments not already in poll_results.
    struct DispatchCandidate {
        task_id: String,
        issue_number: u32,
        pr_number: u32,
    }

    // poll_results candidates are known-open (poll_pr_reviews already checked and
    // populated the cache).
    let mut candidates: Vec<DispatchCandidate> = Vec::new();
    for r in &poll_results {
        candidates.push(DispatchCandidate {
            task_id: r.task_id.clone(),
            issue_number: r.issue_number,
            pr_number: r.pr_number,
        });
    }

    for task_info in &all_tasks {
        if newly_staged.contains(&task_info.task_id) {
            continue; // already a candidate from poll_results
        }
        // A task qualifies for re-dispatch if it has staged amendments OR a
        // resume-pending marker.  The marker case covers crash-after-dispatch:
        // staged files were purged but the task never reached terminal
        // completion, so the marker is the durable recovery signal.
        if super::pr_review::has_staged_amendments(&config.workspace_root, &task_info.task_id)
            || super::pr_review::has_resume_pending_marker(
                &config.workspace_root,
                &task_info.task_id,
            )
        {
            candidates.push(DispatchCandidate {
                task_id: task_info.task_id.clone(),
                issue_number: task_info.issue_number,
                pr_number: task_info.pr_number,
            });
        }
    }

    if candidates.is_empty() {
        return Ok(());
    }

    // For each candidate, check if it needs re-dispatch.
    for candidate in &candidates {
        // Skip if already running.
        if children.contains_key(&candidate.issue_number) {
            continue;
        }

        // Gate dispatch on PR still being open (use cache to avoid duplicate calls).
        if candidate.pr_number > 0 {
            let is_open = match pr_open_cache.get(&candidate.pr_number) {
                Some(&cached) => cached,
                None => {
                    let open = match github::is_pr_open(
                        &config.owner,
                        &config.repo,
                        candidate.pr_number,
                        &config.gh_bin,
                    )
                    .await
                    {
                        Ok(o) => o,
                        Err(err) => {
                            eprintln!(
                                "warning: failed to check PR #{} state for {}: {err}",
                                candidate.pr_number, candidate.task_id
                            );
                            continue;
                        }
                    };
                    pr_open_cache.insert(candidate.pr_number, open);
                    open
                }
            };
            if !is_open {
                continue;
            }
        }

        // Check capacity.
        let slots = config.max_concurrent.saturating_sub(children.len() as u32);
        if slots == 0 {
            eprintln!(
                "PR review amendments pending for {} but no capacity slots available; deferring",
                candidate.task_id
            );
            continue;
        }

        // Fetch issue labels to determine current state.
        let labels = match github::fetch_issue_labels_with_gh_bin(
            &config.gh_bin,
            &config.owner,
            &config.repo,
            candidate.issue_number,
        )
        .await
        {
            Ok(labels) => labels,
            Err(err) => {
                eprintln!(
                    "warning: failed to fetch labels for issue #{} ({}): {err}",
                    candidate.issue_number, candidate.task_id
                );
                continue;
            }
        };

        // Multi-lifecycle normalization: if the issue has more than one
        // lifecycle label, normalize to ralph:failed and skip this cycle.
        // This prevents resuming from an ambiguous state and mirrors the
        // same policy used in the claim flow (poll_and_claim).
        let lifecycle = github::classify_lifecycle_labels(&labels);
        if lifecycle.len() > 1 {
            match github::normalize_multi_lifecycle_labels(
                &config.owner,
                &config.repo,
                candidate.issue_number,
                &lifecycle,
            )
            .await
            {
                Ok(true) => {
                    eprintln!(
                        "pr-review: normalized multi-lifecycle issue #{} to ralph:failed, skipping",
                        candidate.issue_number
                    );
                }
                Ok(false) => {}
                Err(err) => {
                    eprintln!(
                        "warning: failed to normalize multi-lifecycle labels on #{} during PR review: {err}",
                        candidate.issue_number
                    );
                }
            }
            continue;
        }

        // Resume projects labeled ralph:completed, or ralph:ready when a
        // resume-pending marker OR staged amendments exist.  The marker case
        // covers restart-drift (label was swapped to in-progress but daemon
        // crashed before dispatch, then startup reconciliation converted
        // in-progress → ready).  The staged-amendments case covers comments
        // staged for a ralph:ready issue that never had a marker set yet.
        //
        // Recovery: when a resume-pending marker exists but NO lifecycle label
        // is present, a prior swap removed the label and both forward-add and
        // rollback-add failed, stranding the issue.  Re-add ralph:ready so the
        // normal swap path can proceed.
        let has_marker =
            super::pr_review::has_resume_pending_marker(&config.workspace_root, &candidate.task_id);
        let has_staged =
            super::pr_review::has_staged_amendments(&config.workspace_root, &candidate.task_id);
        let no_lifecycle = lifecycle.is_empty();

        let mut labels = labels;
        let from_label = if labels.iter().any(|l| l == "ralph:completed") {
            "ralph:completed"
        } else if labels.iter().any(|l| l == "ralph:ready") && (has_marker || has_staged) {
            "ralph:ready"
        } else if no_lifecycle && has_marker {
            // Stranded issue: marker present but no lifecycle label.
            // Re-add ralph:ready so the swap path can proceed.
            match github::add_label_with_retry(
                &config.owner,
                &config.repo,
                candidate.issue_number,
                "ralph:ready",
            )
            .await
            {
                Ok(()) => {
                    eprintln!(
                        "pr-review: recovered stranded issue #{} \
                         (no lifecycle label + marker present); re-added ralph:ready",
                        candidate.issue_number
                    );
                    labels.push("ralph:ready".to_string());
                    "ralph:ready"
                }
                Err(err) => {
                    eprintln!(
                        "warning: failed to recover stranded issue #{} \
                         (no lifecycle label + marker): {err}",
                        candidate.issue_number
                    );
                    continue;
                }
            }
        } else {
            continue;
        };

        eprintln!(
            "pr-review: resuming {from_label} task {} with staged amendment(s)",
            candidate.task_id
        );

        // Set resume-pending marker before label swap so that restart-drift
        // (completed → in-progress → [crash] → ready) can be detected.
        // Also set for ready+staged (no prior marker) so the same safety
        // property holds during the swap.
        if !has_marker {
            if let Err(err) = super::pr_review::set_resume_pending_marker(
                &config.workspace_root,
                &candidate.task_id,
            ) {
                eprintln!(
                    "warning: failed to set resume-pending marker for {}: {err}",
                    candidate.task_id
                );
                continue;
            }
        }

        // Swap label: ralph:{completed|ready} -> ralph:in-progress
        if let Err(swap_err) = github::swap_lifecycle_label(
            &config.owner,
            &config.repo,
            candidate.issue_number,
            from_label,
            "ralph:in-progress",
        )
        .await
        {
            eprintln!(
                "warning: failed to swap lifecycle label for {}: {swap_err}",
                candidate.task_id
            );
            // Label swap failed — no in-flight resume actually started.
            // Only clear the marker when:
            // 1. It was created in this cycle (!has_marker at entry), AND
            // 2. The original label was *confirmed* restored (Some(true)).
            //    - None: remove step failed — label *may* be absent due to
            //      concurrent removal, so we cannot assume it is still present.
            //    - Some(false): rollback explicitly failed — label is missing.
            //    In both ambiguous/failed cases, keep the marker so restart
            //    recovery can detect the stranded state and retry next cycle.
            let label_confirmed_restored = swap_err.from_label_restored == Some(true);
            if !has_marker && label_confirmed_restored {
                super::pr_review::clear_resume_pending_marker(
                    &config.workspace_root,
                    &candidate.task_id,
                );
            }
            continue;
        }

        // Compose a minimal raw_idea for dispatch (it won't be used for resumed projects
        // since should_resume_issue_project will return true).
        let raw_idea = format!("PR review amendments for issue #{}", candidate.issue_number);

        // Dispatch the task. State reset and amendment drain happen inside dispatch_task
        // after worktree creation and before task spawn.
        match dispatch_task(
            config,
            candidate.issue_number,
            &raw_idea,
            &labels,
            repo_root_lock,
            DispatchOrigin::PrReviewResume,
        )
        .await
        {
            Ok(handle) => {
                children.insert(candidate.issue_number, handle);
                // NOTE: resume-pending marker is intentionally NOT cleared here.
                // It persists until terminal completion (complete_task) so that a
                // daemon crash after dispatch but before amendment consumption
                // can still be recovered on restart.
                eprintln!(
                    "pr-review: dispatched task {} for PR review amendments",
                    candidate.task_id
                );
            }
            Err(err) => {
                eprintln!(
                    "warning: failed to dispatch task {} for PR review amendments: {err}",
                    candidate.task_id
                );
                // Revert label swap on dispatch failure.
                if let Err(rollback_err) = github::swap_lifecycle_label(
                    &config.owner,
                    &config.repo,
                    candidate.issue_number,
                    "ralph:in-progress",
                    from_label,
                )
                .await
                {
                    eprintln!(
                        "warning: pr-review dispatch rollback failed for {} (issue #{}): {rollback_err}; \
                         issue may be stuck in ralph:in-progress — will be recovered at next daemon restart",
                        candidate.task_id, candidate.issue_number
                    );
                    // Keep marker when rollback fails — restart recovery needs
                    // it to detect the in-flight resume that got stuck.
                } else {
                    // Rollback succeeded — issue is back to its original label
                    // and no in-flight resume is active.  Only clear the marker
                    // when it was created in this cycle (!has_marker at entry).
                    // For pre-existing markers the marker must persist so
                    // retries remain possible.
                    if !has_marker {
                        super::pr_review::clear_resume_pending_marker(
                            &config.workspace_root,
                            &candidate.task_id,
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

async fn auto_rebase_phase(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, TaskHandle>,
    repo_root_lock: &Arc<Semaphore>,
) {
    if !config.auto_rebase_enabled {
        eprintln!("auto-rebase: skipped (disabled by config)");
        return;
    }

    // Collect children sorted by issue number for deterministic processing
    let mut issue_numbers: Vec<u32> = children.keys().cloned().collect();
    issue_numbers.sort();

    // Stage 1: Sequential candidate discovery and merge-metadata queries
    let mut candidates: Vec<RebaseCandidate> = Vec::new();

    for issue_number in &issue_numbers {
        let (branch, last_rebase_at, last_failure_sha, cached_pr_url) =
            match children.get(issue_number) {
                Some(h) => {
                    // Skip tasks that are externally aborted or already cancelling —
                    // they should not trigger rebase activity.
                    if h.aborted_externally
                        .load(std::sync::atomic::Ordering::SeqCst)
                        || h.cancel_token.is_cancelled()
                    {
                        continue;
                    }
                    (
                        h.branch.clone(),
                        h.last_rebase_at,
                        h.last_rebase_failure_sha.clone(),
                        h.pr_url.clone(),
                    )
                }
                None => continue,
            };

        if candidates.len() as u32 >= config.max_rebases_per_cycle {
            eprintln!(
                "auto-rebase: per-cycle cap reached ({}/{})",
                candidates.len() as u32,
                config.max_rebases_per_cycle
            );
            break;
        }

        let task_id = format_task_id(&config.owner, &config.repo, *issue_number);

        // Honor per-task rebase cooldown
        let cooldown = Duration::from_secs(config.rebase_interval_seconds);
        if let Some(last) = last_rebase_at {
            if last.elapsed() < cooldown {
                eprintln!(
                    "auto-rebase: skip {task_id} — cooldown ({} of {}s remaining)",
                    cooldown.as_secs().saturating_sub(last.elapsed().as_secs()),
                    cooldown.as_secs(),
                );
                continue;
            }
        }

        let branch = &branch;

        // Use cached PR URL from TaskHandle when available; fall back to API.
        let pr_url = if let Some(url) = cached_pr_url {
            url
        } else {
            match github::find_existing_pr(&config.owner, &config.repo, branch).await {
                Ok(Some(url)) => {
                    // Back-fill cached PR URL so future cycles skip the lookup.
                    if let Some(h) = children.get_mut(issue_number) {
                        h.pr_url = Some(url.clone());
                    }
                    url
                }
                Ok(None) => {
                    eprintln!("auto-rebase: skip {task_id} — no PR URL");
                    continue;
                }
                Err(err) => {
                    eprintln!("auto-rebase: skip {task_id} — PR lookup failed: {err}");
                    continue;
                }
            }
        };

        let pr_number = match github::extract_pr_number(&pr_url) {
            Some(n) => n,
            None => {
                eprintln!("auto-rebase: skip {task_id} — unparsable PR URL: {pr_url}");
                continue;
            }
        };

        // Query PR merge info — on failure, break the loop (rate limit safety)
        let merge_info = {
            match github::query_pr_merge_info(&config.owner, &config.repo, pr_number).await {
                Ok(info) => info,
                Err(err) => {
                    eprintln!(
                        "auto-rebase: gh pr view failed for {task_id} (PR #{pr_number}): {err} — stopping rebase processing for this cycle"
                    );
                    break;
                }
            }
        };

        // Skip non-open PRs
        if merge_info.state != "OPEN" {
            eprintln!(
                "auto-rebase: skip {task_id} — PR state is {} (not OPEN)",
                merge_info.state
            );
            continue;
        }

        // Skip conflicting or unknown merge status
        match merge_info.merge_status {
            PrMergeStatus::Conflicting => {
                eprintln!("auto-rebase: skip {task_id} — PR merge status is Conflicting");
                continue;
            }
            PrMergeStatus::Unknown => {
                eprintln!("auto-rebase: skip {task_id} — PR merge status is Unknown");
                continue;
            }
            PrMergeStatus::Mergeable => {}
        }

        let rebase_target = format!("origin/{}", merge_info.base_branch);
        let head_sha = merge_info.head_oid.clone();

        eprintln!(
            "auto-rebase: rebasing {task_id} (branch={branch}, target={rebase_target}, head={head_sha})"
        );

        candidates.push(RebaseCandidate {
            issue_number: *issue_number,
            task_id,
            branch: branch.clone(),
            last_failure_sha,
            rebase_target,
            head_sha,
            pr_number,
        });
    }

    if candidates.is_empty() {
        return;
    }

    // Stage 2: Execute rebase operations concurrently via JoinSet
    let mut rebase_set: JoinSet<RebaseOutcome> = JoinSet::new();

    for candidate in candidates {
        let config = config.clone();
        let repo_root_lock = repo_root_lock.clone();
        rebase_set.spawn(async move {
            execute_rebase_candidate(&config, &candidate, &repo_root_lock).await
        });
    }

    // Stage 3: Collect results and apply child-state mutations sequentially
    let mut outcomes: Vec<RebaseOutcome> = Vec::new();
    while let Some(result) = rebase_set.join_next().await {
        match result {
            Ok(outcome) => outcomes.push(outcome),
            Err(err) => {
                eprintln!("auto-rebase: rebase task panicked: {err}");
            }
        }
    }

    // Sort outcomes by issue_number for deterministic application
    outcomes.sort_by_key(|o| match o {
        RebaseOutcome::Success { issue_number, .. } => *issue_number,
        RebaseOutcome::Failure { issue_number, .. } => *issue_number,
    });

    for outcome in outcomes {
        match outcome {
            RebaseOutcome::Success {
                issue_number,
                task_id,
            } => {
                eprintln!("auto-rebase: success for {task_id}");
                if let Some(h) = children.get_mut(&issue_number) {
                    h.last_rebase_at = Some(std::time::Instant::now());
                }
            }
            RebaseOutcome::Failure {
                issue_number,
                task_id,
                head_sha,
                last_failure_sha,
                pr_number,
                error,
                is_lease,
            } => {
                if is_lease {
                    eprintln!(
                        "auto-rebase: lease mismatch for {task_id} — skipping for this cycle"
                    );
                    continue;
                }

                eprintln!("auto-rebase: failure for {task_id}: {error}");

                // Skip duplicate failure comment for the same head SHA.
                if last_failure_sha.as_deref() == Some(head_sha.as_str()) {
                    eprintln!(
                        "auto-rebase: skipping duplicate failure comment for {task_id} (head={head_sha})"
                    );
                } else {
                    let marker = format!("<!-- ralph:rebase:{task_id}:failed:{head_sha} -->");
                    let body = format!(
                        "{marker}\nAuto-rebase failed for task `{task_id}` (head: `{head_sha}`).\n\nError: {error}"
                    );
                    let _ = github::post_pr_comment(&config.owner, &config.repo, pr_number, &body)
                        .await;
                    if let Some(h) = children.get_mut(&issue_number) {
                        h.last_rebase_failure_sha = Some(head_sha.clone());
                    }
                }
            }
        }
    }
}

/// Execute a single rebase candidate: worktree creation, fetch, rebase, cleanup.
async fn execute_rebase_candidate(
    config: &DaemonRuntimeConfig,
    candidate: &RebaseCandidate,
    repo_root_lock: &Arc<Semaphore>,
) -> RebaseOutcome {
    let task_id = &candidate.task_id;
    let branch = &candidate.branch;

    // Create worktree on the task's branch
    let wt_path = {
        let _permit = match repo_root_lock.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(err) => {
                return RebaseOutcome::Failure {
                    issue_number: candidate.issue_number,
                    task_id: task_id.clone(),
                    head_sha: candidate.head_sha.clone(),
                    last_failure_sha: candidate.last_failure_sha.clone(),
                    pr_number: candidate.pr_number,
                    error: format!("failed to acquire repo-root semaphore: {err}"),
                    is_lease: false,
                };
            }
        };
        let repo_root = config.repo_root.clone();
        let ws_root = config.workspace_root.clone();
        let tid = task_id.clone();
        let br = branch.clone();
        let lock = Some(repo_root_lock.clone());
        match spawn_blocking_op(move || {
            worktree::create_worktree_on_branch(&repo_root, &ws_root, &tid, &br, lock)
        })
        .await
        {
            Ok(path) => path,
            Err(err) => {
                return RebaseOutcome::Failure {
                    issue_number: candidate.issue_number,
                    task_id: task_id.clone(),
                    head_sha: candidate.head_sha.clone(),
                    last_failure_sha: candidate.last_failure_sha.clone(),
                    pr_number: candidate.pr_number,
                    error: format!("failed to create worktree: {err}"),
                    is_lease: false,
                };
            }
        }
    };

    // Single deadline for the entire rebase operation (fetch + rebase + push)
    // to prevent the total wall-clock from exceeding the configured limit.
    let deadline = Instant::now() + Duration::from_secs(config.rebase_timeout_seconds);

    // Fetch in the repo root with semaphore serialization.
    let fetch_result = {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wt = wt_path.clone();
        let _permit = match repo_root_lock.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(err) => {
                return RebaseOutcome::Failure {
                    issue_number: candidate.issue_number,
                    task_id: task_id.clone(),
                    head_sha: candidate.head_sha.clone(),
                    last_failure_sha: candidate.last_failure_sha.clone(),
                    pr_number: candidate.pr_number,
                    error: format!("failed to acquire repo-root semaphore for fetch: {err}"),
                    is_lease: false,
                };
            }
        };
        spawn_blocking_op(move || execute_rebase_fetch(&wt, remaining)).await
    };
    if let Err(err) = fetch_result {
        eprintln!("auto-rebase: fetch failed for {task_id}: {err}");
        cleanup_rebase_worktree(config, task_id, repo_root_lock).await;
        return RebaseOutcome::Failure {
            issue_number: candidate.issue_number,
            task_id: task_id.clone(),
            head_sha: candidate.head_sha.clone(),
            last_failure_sha: candidate.last_failure_sha.clone(),
            pr_number: candidate.pr_number,
            error: format!("fetch failed: {err}"),
            is_lease: false,
        };
    }

    // Rebase + push in the worktree without holding the root semaphore.
    let rebase_result = {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            cleanup_rebase_worktree(config, task_id, repo_root_lock).await;
            return RebaseOutcome::Failure {
                issue_number: candidate.issue_number,
                task_id: task_id.clone(),
                head_sha: candidate.head_sha.clone(),
                last_failure_sha: candidate.last_failure_sha.clone(),
                pr_number: candidate.pr_number,
                error: "rebase timeout budget exhausted after fetch".to_string(),
                is_lease: false,
            };
        }
        let wt = wt_path.clone();
        let target = candidate.rebase_target.clone();
        let br = branch.clone();
        let backend_str = config.rebase_agent_backend.clone();
        spawn_blocking_op(move || execute_rebase(&wt, &target, &br, remaining, &backend_str)).await
    };

    // Clean up rebase worktree (best-effort)
    cleanup_rebase_worktree(config, task_id, repo_root_lock).await;

    match rebase_result {
        Ok(()) => RebaseOutcome::Success {
            issue_number: candidate.issue_number,
            task_id: task_id.clone(),
        },
        Err(err) => {
            let err_msg = err.to_string();
            let is_lease = github::is_lease_rejection(&err_msg);
            RebaseOutcome::Failure {
                issue_number: candidate.issue_number,
                task_id: task_id.clone(),
                head_sha: candidate.head_sha.clone(),
                last_failure_sha: candidate.last_failure_sha.clone(),
                pr_number: candidate.pr_number,
                error: err_msg,
                is_lease,
            }
        }
    }
}

/// Best-effort cleanup of a rebase worktree.
async fn cleanup_rebase_worktree(
    config: &DaemonRuntimeConfig,
    task_id: &str,
    repo_root_lock: &Arc<Semaphore>,
) {
    let repo_root = config.repo_root.clone();
    let ws_root = config.workspace_root.clone();
    let tid = task_id.to_owned();
    let lock = Some(repo_root_lock.clone());
    let _permit = match repo_root_lock.clone().acquire_owned().await {
        Ok(permit) => permit,
        Err(err) => {
            eprintln!(
                "auto-rebase: failed to reacquire repo-root semaphore for {task_id} cleanup: {err}"
            );
            return;
        }
    };
    let _ = spawn_blocking_op(move || {
        worktree::remove_rebase_worktree(&repo_root, &ws_root, &tid, lock);
        Ok(())
    })
    .await;
}

/// Execute fetch + rebase + force-with-lease push in a worktree.
///
/// All three steps share a single `timeout` budget. Time consumed by earlier
/// steps is subtracted from the budget available for later steps, so the
/// total wall-clock time stays bounded to roughly `timeout`.
///
/// When rebase fails due to merge conflicts and an agent backend is configured,
/// the conflict recovery agent is invoked to resolve conflicts iteratively.
/// The `agent_backend` is a raw string spec ("none", "claude", "claude(<model>)")
/// parsed internally by `resolve_rebase_conflicts`.
fn execute_rebase(
    worktree_path: &Path,
    rebase_target: &str,
    branch: &str,
    timeout: Duration,
    agent_backend: &str,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;

    // Parse backend once at the top so all branches use the typed enum.
    let parsed_backend = parse_rebase_agent_backend(agent_backend)?;

    // Helper: remaining time or error if expired.
    let remaining = |label: &str| -> Result<Duration> {
        let now = std::time::Instant::now();
        if now >= deadline {
            return Err(RalphError::Orchestration(format!(
                "{label}: per-attempt timeout exceeded"
            )));
        }
        Ok(deadline - now)
    };

    // Helper: bounded abort — abort rebase with timeout from shared deadline.
    let bounded_abort = |wt: &Path| {
        let abort_budget = deadline
            .checked_duration_since(std::time::Instant::now())
            .unwrap_or(Duration::from_secs(10))
            .max(Duration::from_secs(5));
        let _ = process::run_command_with_timeout(
            std::process::Command::new("git")
                .args(["rebase", "--abort"])
                .current_dir(wt),
            abort_budget,
        );
    };

    // Rebase
    let rebase_budget = remaining("git rebase")?;
    let rebase_output = process::run_command_with_timeout(
        std::process::Command::new("git")
            .args(["rebase", rebase_target])
            .current_dir(worktree_path),
        rebase_budget,
    )?;

    if !rebase_output.status.success() {
        // Step 1: Pure criteria check (no I/O, unit-testable)
        let pure_kind = classify_rebase_failure_pure(
            rebase_output.status.code().unwrap_or(-1),
            &rebase_output.stderr,
        );

        // Step 2: If pure check says conflict, verify with timeout-bounded I/O probe
        let failure_kind = if pure_kind == RebaseFailureKind::Conflict {
            // Compute remaining budget before the I/O probe; fail immediately
            // if no time remains.
            let classify_budget = remaining("conflict classification")?;
            match crate::git::has_conflicts_with_timeout(worktree_path, classify_budget) {
                Ok(true) => RebaseFailureKind::Conflict,
                _ => RebaseFailureKind::Other,
            }
        } else {
            RebaseFailureKind::Other
        };

        match failure_kind {
            RebaseFailureKind::Conflict => {
                match parsed_backend {
                    RebaseAgentBackend::None => {
                        // Disabled: abort with bounded timeout and fail as before
                        bounded_abort(worktree_path);
                        let stderr = String::from_utf8_lossy(&rebase_output.stderr)
                            .trim()
                            .to_owned();
                        return Err(RalphError::Orchestration(format!(
                            "git rebase failed with merge conflicts (agent resolution was skipped/disabled): {stderr}"
                        )));
                    }
                    RebaseAgentBackend::Claude { .. } => {
                        eprintln!(
                            "rebase-agent: conflict detected, invoking AI agent for resolution"
                        );
                        // resolve_rebase_conflicts handles abort-on-failure internally
                        crate::daemon::rebase_agent::resolve_rebase_conflicts(
                            worktree_path,
                            rebase_target,
                            agent_backend,
                            deadline,
                        )?;
                        // Agent succeeded — fall through to push
                    }
                }
            }
            RebaseFailureKind::Other => {
                // Non-conflict failure: abort with bounded timeout and return error
                bounded_abort(worktree_path);
                let stderr = String::from_utf8_lossy(&rebase_output.stderr)
                    .trim()
                    .to_owned();
                return Err(RalphError::Orchestration(format!(
                    "git rebase failed: {stderr}"
                )));
            }
        }
    }

    // Push with --force-with-lease (also under remaining budget)
    let push_budget = remaining("git push")?;
    let push_output = process::run_command_with_timeout(
        std::process::Command::new("git")
            .args(["push", "--force-with-lease", "origin", branch])
            .current_dir(worktree_path),
        push_budget,
    )?;

    if !push_output.status.success() {
        let stderr = String::from_utf8_lossy(&push_output.stderr).to_string();
        return Err(RalphError::Orchestration(format!(
            "git push --force-with-lease failed for branch {branch}: {stderr}"
        )));
    }

    Ok(())
}

fn execute_rebase_fetch(worktree_path: &Path, timeout: Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;

    let remaining = |label: &str| -> Result<Duration> {
        let now = std::time::Instant::now();
        if now >= deadline {
            return Err(RalphError::Orchestration(format!(
                "{label}: per-attempt timeout exceeded"
            )));
        }
        Ok(deadline - now)
    };

    let fetch_budget = remaining("git fetch")?;
    let fetch_output = process::run_command_with_timeout(
        std::process::Command::new("git")
            .args(["fetch", "origin"])
            .current_dir(worktree_path),
        fetch_budget,
    )?;

    if !fetch_output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "git fetch failed: {}",
            String::from_utf8_lossy(&fetch_output.stderr).trim()
        )));
    }

    Ok(())
}

pub(crate) fn extract_project_ref(branch: &str) -> Option<String> {
    let mut parts = branch.split('/');
    let prefix = parts.next()?;
    if prefix != "ralph" {
        return None;
    }
    let second = parts.next()?;
    if second.is_empty() {
        return None;
    }
    // Handle both `ralph/{project_id}` and `ralph/daemon/{task_id}` formats.
    if second == "daemon" {
        let task_id = parts.next()?;
        if !task_id.is_empty() && parts.next().is_none() {
            return Some(task_id.to_owned());
        }
        return None;
    }
    if parts.next().is_none() {
        Some(second.to_owned())
    } else {
        None
    }
}

pub(crate) fn build_pr_title(raw: &str) -> String {
    let sanitized = raw.replace(['\n', '\r'], " ");
    let trimmed = sanitized.trim();
    if trimmed.chars().count() > 80 {
        let mut truncated: String = trimmed.chars().take(77).collect();
        truncated.push_str("...");
        truncated
    } else {
        trimmed.to_owned()
    }
}

pub(crate) fn extract_issue_body(raw_idea: Option<&str>) -> Option<String> {
    let raw = raw_idea?;
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let (_, body) = normalized.split_once("\n\n")?;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

pub(crate) fn build_pr_body(
    branch: &str,
    diff_stat: Option<&str>,
    issue_body: Option<&str>,
    task_id: &str,
    issue_number: u32,
) -> String {
    let mut body = format!("Automated PR for task `{task_id}`.\n\nCloses #{issue_number}\n\n");

    body.push_str("## Diff Stat\n");
    match diff_stat.map(str::trim).filter(|s| !s.is_empty()) {
        Some(stat) => {
            let mut lines = stat.lines();
            let first_hundred: Vec<&str> = lines.by_ref().take(100).collect();
            body.push_str("```text\n");
            body.push_str(&first_hundred.join("\n"));
            if lines.next().is_some() {
                if !first_hundred.is_empty() {
                    body.push('\n');
                }
                body.push_str("... (truncated)");
            }
            body.push_str("\n```\n");
        }
        None => {
            body.push_str("Diff stat unavailable.\n");
        }
    }

    body.push_str("\n## Issue Context\n");
    match issue_body.map(str::trim).filter(|s| !s.is_empty()) {
        Some(context) => {
            let capped: String = context.chars().take(4000).collect();
            if capped.is_empty() {
                body.push_str("Issue context unavailable.\n");
            } else {
                body.push_str(&capped);
                body.push('\n');
            }
        }
        None => {
            body.push_str("Issue context unavailable (legacy task or missing issue body).\n");
        }
    }

    body.push_str("\n---\n");
    match extract_project_ref(branch) {
        Some(project_ref) => {
            body.push_str(&format!("Project Ref: `{project_ref}`\n"));
        }
        None => {
            body.push_str(&format!(
                "Project Ref: unavailable (could not extract from branch `{branch}`).\n"
            ));
        }
    }

    body
}

/// Handle the PR creation/update flow for a completed task.
pub(crate) async fn handle_pr_flow(
    config: &DaemonRuntimeConfig,
    task_id: &str,
    issue_number: u32,
    wt_path: &Path,
) -> Result<()> {
    // Resolve branch from worktree
    let branch = {
        match github::current_branch(wt_path).await {
            Ok(b) => b,
            Err(err) => {
                eprintln!("warning: failed to read current branch for {task_id}: {err}");
                return Ok(());
            }
        }
    };

    // Step 0: Check for existing PR (used by both no-diff and update paths)
    let existing_pr_url = {
        match github::find_existing_pr(&config.owner, &config.repo, &branch).await {
            Ok(url) => url,
            Err(err) => {
                eprintln!("warning: failed to check for existing PR: {err}");
                None
            }
        }
    };

    // Step 1: Check if there's a diff against the configured base branch
    let has_changes = {
        match github::has_diff_with_base(wt_path, Some(&config.base_branch)).await {
            Ok(v) => v,
            Err(err) => {
                eprintln!("warning: failed to check diff for {task_id}: {err}");
                return Ok(());
            }
        }
    };

    if !has_changes {
        if let Some(url) = existing_pr_url.as_ref() {
            if let Some(pr_number) = github::extract_pr_number(url) {
                let pr_is_draft =
                    github::is_pr_draft(&config.owner, &config.repo, pr_number).await?;

                if decide_draft_pr_transition(has_changes, pr_is_draft, "ralph:completed")
                    == DraftPrTransition::CloseNoDiff
                {
                    github::close_pr(&config.owner, &config.repo, pr_number).await?;

                    // Clear persisted PR URL so future flows do not reuse closed draft PRs.
                    save_task_metadata(
                        &config.workspace_root,
                        task_id,
                        &TaskMetadata { pr_url: None },
                    );
                }
            } else {
                eprintln!(
                    "warning: failed to extract PR number from existing PR URL for {task_id}: {url}"
                );
            }
        }

        let mut body = format!("Task `{task_id}` completed with no code changes. No PR created.");
        if existing_pr_url.is_some() {
            body.push_str(" Existing draft PR was closed if it was still in draft state.");
        }

        if let Err(err) = github::post_idempotent_comment(
            &config.owner,
            &config.repo,
            issue_number,
            task_id,
            "no-diff",
            &body,
        )
        .await
        {
            eprintln!("warning: failed to post no-diff comment for {task_id}: {err}");
        }
        return Ok(());
    }

    // Skip push/PR flow when no origin remote exists
    {
        let has_origin = match github::has_origin_remote(wt_path).await {
            Ok(v) => v,
            Err(err) => {
                eprintln!(
                    "warning: failed to check origin remote for {task_id}; skipping push/PR: {err}"
                );
                return Ok(());
            }
        };
        if !has_origin {
            eprintln!("warning: origin remote missing for {task_id}; skipping push/PR flow");
            return Ok(());
        }
    }

    // Step 2: Push branch
    github::push_branch_with_retry(wt_path, &branch).await?;

    // Step 3: Gather context
    let diff_stat: Option<String> = {
        match github::diff_stat(wt_path).await {
            Ok(stat) => stat,
            Err(err) => {
                eprintln!("warning: diff stat failed for {task_id}: {err}; using fallback");
                None
            }
        }
    };

    // Fetch raw_idea from GitHub for PR body context
    let raw_idea = {
        match github::fetch_issue_body(&config.owner, &config.repo, issue_number).await {
            Ok((title, body)) => Some(compose_raw_idea(&title, body.as_deref())),
            Err(_) => None,
        }
    };
    let issue_body = extract_issue_body(raw_idea.as_deref());

    // Step 4: Build title and body
    let title = build_pr_title(&format!("ralph: {task_id}"));
    let pr_body = build_pr_body(
        &branch,
        diff_stat.as_deref(),
        issue_body.as_deref(),
        task_id,
        issue_number,
    );

    let body_file = match write_body_file(&pr_body) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("warning: failed to write PR body file for {task_id}: {err}");
            return Ok(());
        }
    };

    // Try to get refined title from GitHub issue
    let refined_title = {
        match github::fetch_issue_body(&config.owner, &config.repo, issue_number).await {
            Ok((title, _)) => {
                if title.is_empty() {
                    None
                } else {
                    Some(title)
                }
            }
            Err(_) => None,
        }
    };

    let title = refined_title
        .or_else(|| extract_original_title(raw_idea.as_deref().unwrap_or_default()))
        .unwrap_or(title);

    match existing_pr_url {
        Some(url) => {
            eprintln!("editing existing PR for {task_id}: {url}");
            let body_path = body_file.path().to_path_buf();
            if let Err(err) = github::edit_pr(&url, &title, &body_path).await {
                eprintln!(
                    "warning: failed to edit PR for {task_id} (continuing to mark-ready): {err}"
                );
            }

            if let Some(pr_number) = github::extract_pr_number(&url) {
                save_task_metadata(
                    &config.workspace_root,
                    task_id,
                    &TaskMetadata {
                        pr_url: Some(url.clone()),
                    },
                );
                let pr_is_draft =
                    github::is_pr_draft(&config.owner, &config.repo, pr_number).await?;

                if decide_draft_pr_transition(has_changes, pr_is_draft, "ralph:completed")
                    == DraftPrTransition::MarkReady
                {
                    github::mark_pr_ready(&config.owner, &config.repo, pr_number).await?;
                }
            }
        }
        None => {
            let body_path = body_file.path().to_path_buf();
            match github::create_pr_with_body_file(
                &config.owner,
                &config.repo,
                &branch,
                &title,
                &body_path,
                Some(&config.base_branch),
                false,
            )
            .await
            {
                Ok(url) => {
                    eprintln!("created PR for {task_id}: {url}");
                    save_task_metadata(
                        &config.workspace_root,
                        task_id,
                        &TaskMetadata {
                            pr_url: Some(url.clone()),
                        },
                    );
                }
                Err(err) => {
                    eprintln!(
                        "warning: failed to create PR for {task_id}; continuing to terminal state: {err}"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Write the PR body content to a `NamedTempFile` for `--body-file` usage.
fn write_body_file(body: &str) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new().map_err(|err| {
        RalphError::Orchestration(format!("failed to create temp file for PR body: {err}"))
    })?;
    tmp.write_all(body.as_bytes()).map_err(|err| {
        RalphError::Orchestration(format!("failed to write PR body to temp file: {err}"))
    })?;
    tmp.flush().map_err(|err| {
        RalphError::Orchestration(format!("failed to flush PR body temp file: {err}"))
    })?;
    Ok(tmp)
}

#[cfg(test)]
mod tests {
    use super::{
        await_watcher_with_timeout_impl, build_pr_body, build_pr_title, derive_terminal_label,
        detect_final_prompt_artifact, detect_quick_prd_artifact, extract_issue_body,
        extract_original_title, extract_project_ref, newest_by_mtime,
        post_artifact_comments_with_client, should_close_no_diff_draft_pr,
        should_mark_draft_pr_ready, should_resume_issue_project, should_retry_complete_task,
        sweep_artifact_comments, truncate_for_github, validate_daemon_branch_format,
        write_body_file, ArtifactCommentClient, ArtifactWatcherState, TRUNCATED_NOTE,
    };
    use crate::error::RalphError;
    use crate::Result;
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tokio_util::sync::CancellationToken;

    #[test]
    fn extract_original_title_with_body() {
        assert_eq!(
            extract_original_title("Fix bug\n\nDetails"),
            Some("Fix bug".to_owned())
        );
    }

    #[test]
    fn build_pr_title_sanitizes_newlines() {
        let title = build_pr_title("  Fix login\nflow\rissue  ");
        assert_eq!(title, "Fix login flow issue");
    }

    #[test]
    fn build_pr_title_truncates_long_title() {
        let input = "a".repeat(120);
        let title = build_pr_title(&input);
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with("..."));
    }

    #[test]
    fn build_pr_body_no_context_legacy_task() {
        let body = build_pr_body("feature/no-project-ref", None, None, "task-1", 42);
        assert!(body.contains("Automated PR for task `task-1`."));
        assert!(body.contains("Closes #42"));
        assert!(body.contains("Diff stat unavailable."));
        assert!(body.contains("Issue context unavailable (legacy task or missing issue body)."));
        assert!(body.contains(
            "Project Ref: unavailable (could not extract from branch `feature/no-project-ref`)."
        ));
    }

    #[test]
    fn extract_original_title_no_body() {
        assert_eq!(
            extract_original_title("Fix bug"),
            Some("Fix bug".to_owned())
        );
    }

    #[test]
    fn extract_project_ref_success() {
        assert_eq!(
            extract_project_ref("ralph/my-project"),
            Some("my-project".to_owned())
        );
    }

    #[test]
    fn extract_project_ref_daemon_format() {
        assert_eq!(
            extract_project_ref("ralph/daemon/acme-widgets-901"),
            Some("acme-widgets-901".to_owned())
        );
    }

    #[test]
    fn extract_original_title_empty() {
        assert_eq!(extract_original_title(""), None);
    }

    #[test]
    fn extract_original_title_body_only() {
        assert_eq!(extract_original_title("\n\nBody only"), None);
    }

    #[test]
    fn extract_project_ref_non_matching_branches() {
        assert_eq!(extract_project_ref("main"), None);
        assert_eq!(extract_project_ref("feature/foo"), None);
        assert_eq!(extract_project_ref("ralph/"), None);
        assert_eq!(extract_project_ref("ralph/daemon/"), None);
        assert_eq!(extract_project_ref("ralph/daemon/a/b"), None);
    }

    #[test]
    fn extract_issue_body_reads_body_after_title_separator() {
        let raw = Some("Issue title\n\nIssue body content");
        assert_eq!(
            extract_issue_body(raw),
            Some("Issue body content".to_owned())
        );
    }

    #[test]
    fn extract_issue_body_handles_missing_or_empty_body() {
        assert_eq!(extract_issue_body(None), None);
        assert_eq!(extract_issue_body(Some("Title only")), None);
        assert_eq!(extract_issue_body(Some("Title\n\n   ")), None);
    }

    #[test]
    fn runtime_pr_diff_stat_failure_fallback() {
        let body = build_pr_body(
            "ralph/my-project",
            None,
            Some("Issue body context here"),
            "acme-widgets-1",
            1,
        );
        assert!(body.contains("Diff stat unavailable."));
        assert!(body.contains("Issue body context here"));
        assert!(body.contains("Project Ref: `my-project`"));
        assert!(body.contains("Automated PR for task `acme-widgets-1`."));
    }

    #[test]
    fn build_pr_body_full_metadata_assembly() {
        // Verify the full PR metadata assembly with all fields populated,
        // matching the format that would be submitted to GitHub.
        // Branch format is "ralph/{project_ref}" for project ref extraction.
        let body = build_pr_body(
            "ralph/issue-901",
            Some(" src/main.rs | 10 +++++-----\n 1 file changed, 5 insertions(+), 5 deletions(-)"),
            Some("E2E PR metadata verification issue."),
            "acme-widgets-901",
            901,
        );
        assert!(body.contains("Automated PR for task `acme-widgets-901`."));
        assert!(body.contains("Closes #901"));
        assert!(body.contains("src/main.rs | 10 +++++-----"));
        assert!(body.contains("1 file changed, 5 insertions(+), 5 deletions(-)"));
        assert!(body.contains("E2E PR metadata verification issue."));
        assert!(body.contains("Project Ref: `issue-901`"));
        // Verify the body does NOT contain raw error messages
        assert!(!body.contains("unavailable"));
    }

    #[test]
    fn build_pr_title_daemon_task_format() {
        // The daemon assembles the title as "ralph: {task_id}".
        let title = build_pr_title("ralph: acme-widgets-901");
        assert_eq!(title, "ralph: acme-widgets-901");
    }

    #[test]
    fn should_retry_complete_task_retries_only_transient_errors_under_cap() {
        let transient =
            RalphError::Orchestration("network timeout while posting comment".to_owned());
        assert!(should_retry_complete_task(&transient, 1));
        assert!(should_retry_complete_task(&transient, 2));
        assert!(!should_retry_complete_task(&transient, 3));

        let terminal = RalphError::BranchMismatch {
            expected: "ralph/issue-93".to_owned(),
            actual: "main".to_owned(),
        };
        assert!(!should_retry_complete_task(&terminal, 1));
    }

    #[test]
    fn should_mark_draft_pr_ready_only_on_completed_with_changes() {
        assert!(should_mark_draft_pr_ready(true, true, "ralph:completed"));
        assert!(!should_mark_draft_pr_ready(false, true, "ralph:completed"));
        assert!(!should_mark_draft_pr_ready(true, false, "ralph:completed"));
        assert!(!should_mark_draft_pr_ready(true, true, "ralph:failed"));
    }

    #[test]
    fn should_close_no_diff_draft_pr_only_for_no_diff_drafts() {
        assert!(should_close_no_diff_draft_pr(false, true));
        assert!(!should_close_no_diff_draft_pr(true, true));
        assert!(!should_close_no_diff_draft_pr(false, false));
    }

    #[test]
    fn complete_task_retry_policy_has_required_cap_and_delay() {
        assert_eq!(super::COMPLETE_TASK_MAX_ATTEMPTS, 3);
        assert_eq!(super::COMPLETE_TASK_RETRY_DELAY_SECS, 30);
    }

    #[test]
    fn write_body_file_creates_readable_temp() {
        let content = "Test PR body\n\nWith multiple lines";
        let tmp = write_body_file(content).expect("write_body_file should succeed");
        let read_back = std::fs::read_to_string(tmp.path()).expect("read temp file");
        assert_eq!(read_back, content);
    }

    #[test]
    fn build_pr_body_diff_stat_cap() {
        let lines: Vec<String> = (1..=150).map(|i| format!("file{i}.rs | 1 +")).collect();
        let stat = lines.join("\n");
        let body = build_pr_body("ralph/proj", Some(&stat), None, "task-2", 2);
        assert!(body.contains("file1.rs | 1 +"));
        assert!(body.contains("file100.rs | 1 +"));
        assert!(body.contains("... (truncated)"));
        assert!(!body.contains("file101.rs"));
    }

    #[test]
    fn build_pr_body_context_cap() {
        let long_context = "\u{2603}".repeat(5000);
        let body = build_pr_body("ralph/proj", None, Some(&long_context), "task-3", 3);
        let snowman_count = body.matches('\u{2603}').count();
        assert_eq!(
            snowman_count, 4000,
            "issue context should be capped at 4000 chars, got {snowman_count}"
        );
    }

    #[test]
    fn resume_decision_requires_issue_prompt_md() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let worktree = tmp.path().join("repo");
        let project_id = "issue-42";
        let prompt_path = worktree
            .join(".ralph")
            .join("projects")
            .join(project_id)
            .join("prompt.md");

        assert!(!should_resume_issue_project(&worktree, project_id));

        std::fs::create_dir_all(prompt_path.parent().expect("prompt parent"))
            .expect("create project dir");
        std::fs::write(&prompt_path, "# prompt").expect("write prompt");
        assert!(should_resume_issue_project(&worktree, project_id));

        std::fs::remove_file(&prompt_path).expect("remove prompt");
        assert!(!should_resume_issue_project(&worktree, project_id));
    }

    #[test]
    fn daemon_branch_format_validation_accepts_default() {
        validate_daemon_branch_format("ralph/{project_id}")
            .expect("default daemon branch format should be accepted");
    }

    #[test]
    fn daemon_branch_format_validation_rejects_incompatible_format() {
        let err = validate_daemon_branch_format("feature/{project_id}")
            .expect_err("incompatible branch format should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("git.branch_format"),
            "expected branch format validation message, got: {msg}"
        );
        assert!(
            msg.contains("ralph/issue-1"),
            "expected expected-branch hint, got: {msg}"
        );
    }

    #[test]
    fn daemon_branch_format_validation_rejects_constant_format() {
        let err = validate_daemon_branch_format("ralph/issue-1")
            .expect_err("constant branch format should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("git.branch_format"),
            "expected branch format validation message, got: {msg}"
        );
        assert!(
            msg.contains("ralph/issue-2"),
            "expected second project_id hint, got: {msg}"
        );
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct PostedComment {
        phase: String,
        marker: String,
        body: String,
    }

    #[derive(Default)]
    struct MockArtifactCommentState {
        markers: HashSet<String>,
        posted_comments: Vec<PostedComment>,
        fail_post_attempts_remaining: usize,
        post_attempts: usize,
    }

    #[derive(Clone, Default)]
    struct MockArtifactCommentClient {
        state: Arc<Mutex<MockArtifactCommentState>>,
    }

    impl MockArtifactCommentClient {
        fn with_failures(fail_post_attempts: usize) -> Self {
            let state = MockArtifactCommentState {
                fail_post_attempts_remaining: fail_post_attempts,
                ..MockArtifactCommentState::default()
            };
            Self {
                state: Arc::new(Mutex::new(state)),
            }
        }

        fn posted_comments(&self) -> Vec<PostedComment> {
            self.state
                .lock()
                .expect("lock posted comments")
                .posted_comments
                .clone()
        }

        fn post_attempts(&self) -> usize {
            self.state.lock().expect("lock post attempts").post_attempts
        }
    }

    #[async_trait]
    impl ArtifactCommentClient for MockArtifactCommentClient {
        async fn marker_exists(
            &self,
            _owner: &str,
            _repo: &str,
            _issue_number: u32,
            marker: &str,
        ) -> Result<bool> {
            Ok(self
                .state
                .lock()
                .expect("lock marker exists")
                .markers
                .contains(marker))
        }

        async fn post_idempotent_comment(
            &self,
            _owner: &str,
            _repo: &str,
            _issue_number: u32,
            task_id: &str,
            phase: &str,
            body_text: &str,
        ) -> Result<()> {
            let mut state = self.state.lock().expect("lock post");
            state.post_attempts = state.post_attempts.saturating_add(1);
            if state.fail_post_attempts_remaining > 0 {
                state.fail_post_attempts_remaining -= 1;
                return Err(RalphError::Orchestration(
                    "mock transient post failure".to_owned(),
                ));
            }

            let marker = format!("<!-- ralph:task:{task_id}:{phase} -->");
            if state.markers.insert(marker.clone()) {
                state.posted_comments.push(PostedComment {
                    phase: phase.to_owned(),
                    marker,
                    body: body_text.to_owned(),
                });
            }
            Ok(())
        }
    }

    fn write_quick_prd(worktree_path: &std::path::Path, slug: &str, content: &str) -> PathBuf {
        let cache_dir = worktree_path.join(".ralph").join("quick-prd").join(slug);
        std::fs::create_dir_all(&cache_dir).expect("create spec dir");
        let spec_path = cache_dir.join("SPEC.md");
        std::fs::write(&spec_path, content).expect("write spec");
        std::fs::write(cache_dir.join("meta.json"), "{}").expect("write meta");
        spec_path
    }

    fn write_final_prompt(
        worktree_path: &std::path::Path,
        project_id: &str,
        prompt_content: &str,
    ) -> (PathBuf, PathBuf) {
        let project_dir = worktree_path
            .join(".ralph")
            .join("projects")
            .join(project_id);
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let signal = project_dir.join("prompt-original.md");
        let prompt = project_dir.join("prompt.md");
        std::fs::write(&signal, "reviewed").expect("write signal");
        std::fs::write(&prompt, prompt_content).expect("write prompt");
        (signal, prompt)
    }

    #[tokio::test]
    async fn quick_prd_detection_posts_correct_marker_header_and_body() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_quick_prd(tmp.path(), "001-task", "Quick PRD body");

        let client = MockArtifactCommentClient::default();
        let mut state = ArtifactWatcherState::default();
        sweep_artifact_comments(
            &client,
            "acme",
            "widgets",
            7,
            "acme-widgets-7",
            tmp.path(),
            SystemTime::now() - Duration::from_secs(1),
            &mut state,
        )
        .await;

        let comments = client.posted_comments();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].phase, "quick-prd");
        assert_eq!(
            comments[0].marker,
            "<!-- ralph:task:acme-widgets-7:quick-prd -->"
        );
        assert!(comments[0].body.starts_with("### Quick PRD"));
        assert!(comments[0].body.contains("Quick PRD body"));
        assert!(state.quick_prd_posted);
    }

    #[tokio::test]
    async fn final_prompt_uses_prompt_original_signal_and_reads_prompt_md() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join(".ralph").join("projects").join("proj-a");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        std::fs::write(project_dir.join("prompt.md"), "Final prompt body only")
            .expect("write prompt");

        let client = MockArtifactCommentClient::default();
        let mut state = ArtifactWatcherState::default();
        let child_start = SystemTime::now() - Duration::from_secs(1);

        sweep_artifact_comments(
            &client,
            "acme",
            "widgets",
            9,
            "acme-widgets-9",
            tmp.path(),
            child_start,
            &mut state,
        )
        .await;
        assert!(
            client.posted_comments().is_empty(),
            "prompt.md without prompt-original.md signal must not post"
        );

        std::fs::write(project_dir.join("prompt-original.md"), "signal").expect("write signal");
        sweep_artifact_comments(
            &client,
            "acme",
            "widgets",
            9,
            "acme-widgets-9",
            tmp.path(),
            child_start,
            &mut state,
        )
        .await;

        let comments = client.posted_comments();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].phase, "final-prompt");
        assert_eq!(
            comments[0].marker,
            "<!-- ralph:task:acme-widgets-9:final-prompt -->"
        );
        assert!(comments[0]
            .body
            .starts_with("### Final Prompt (after review)"));
        assert!(comments[0].body.contains("Final prompt body only"));
        assert!(state.final_prompt_posted);
    }

    #[tokio::test]
    async fn stale_artifacts_older_than_child_start_are_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_quick_prd(tmp.path(), "001-old", "Old quick PRD");
        write_final_prompt(tmp.path(), "proj-old", "Old prompt");
        std::thread::sleep(Duration::from_millis(20));

        let child_start = SystemTime::now();
        assert_eq!(detect_quick_prd_artifact(tmp.path(), child_start), None);
        assert_eq!(detect_final_prompt_artifact(tmp.path(), child_start), None);

        let client = MockArtifactCommentClient::default();
        let mut state = ArtifactWatcherState::default();
        sweep_artifact_comments(
            &client,
            "acme",
            "widgets",
            11,
            "acme-widgets-11",
            tmp.path(),
            child_start,
            &mut state,
        )
        .await;
        assert!(client.posted_comments().is_empty());
    }

    #[test]
    fn multiple_spec_candidates_choose_newest_then_lexical_tiebreak() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_quick_prd(tmp.path(), "001", "Older");
        std::thread::sleep(Duration::from_millis(10));
        write_quick_prd(tmp.path(), "002", "Newer");

        let detected =
            detect_quick_prd_artifact(tmp.path(), SystemTime::now() - Duration::from_secs(1))
                .expect("detect quick prd");
        assert_eq!(detected, "Newer");

        let t = UNIX_EPOCH + Duration::from_secs(1234);
        let tied = newest_by_mtime(vec![
            (PathBuf::from("alpha"), t),
            (PathBuf::from("beta"), t),
        ])
        .expect("tied candidate");
        assert_eq!(tied, PathBuf::from("beta"));
    }

    #[test]
    fn truncate_for_github_appends_note_within_limit() {
        let truncated = truncate_for_github("abcdefghijklmnopqrstuvwxyz", 16);
        assert_eq!(truncated.chars().count(), 16);
        assert!(truncated.ends_with(TRUNCATED_NOTE));
    }

    #[tokio::test]
    async fn await_watcher_with_timeout_impl_aborts_stuck_task() {
        let counter = Arc::new(AtomicU64::new(0));
        let counter_for_task = counter.clone();
        let join_handle = tokio::spawn(async move {
            loop {
                counter_for_task.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        let before_timeout = counter.load(Ordering::SeqCst);
        assert!(
            before_timeout > 0,
            "counter should be incrementing before timeout"
        );

        await_watcher_with_timeout_impl(
            join_handle,
            "artifact watcher",
            "acme-widgets-77",
            Duration::from_millis(25),
        )
        .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        let snapshot_after = counter.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let snapshot_later = counter.load(Ordering::SeqCst);
        assert_eq!(
            snapshot_after, snapshot_later,
            "counter should stop changing after task is aborted (after={snapshot_after}, later={snapshot_later})"
        );
    }

    #[tokio::test]
    async fn cancellation_triggers_final_sweep_without_missing_artifact() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let client = MockArtifactCommentClient::default();
        let child_start = SystemTime::now();
        let cancel = CancellationToken::new();

        let watch_client = client.clone();
        let watch_cancel = cancel.clone();
        let worktree_path = tmp.path().to_path_buf();
        let watcher = tokio::spawn(async move {
            post_artifact_comments_with_client(
                &watch_client,
                "acme",
                "widgets",
                15,
                "acme-widgets-15",
                &worktree_path,
                child_start,
                watch_cancel,
                Duration::from_secs(30),
            )
            .await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        write_quick_prd(tmp.path(), "boundary", "Boundary content");
        cancel.cancel();
        watcher.await.expect("watcher join");

        let comments = client.posted_comments();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].phase, "quick-prd");
    }

    #[tokio::test]
    async fn github_post_failure_retries_without_panic() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_quick_prd(tmp.path(), "retry", "Retry quick prd");
        let client = MockArtifactCommentClient::with_failures(1);
        let cancel = CancellationToken::new();

        let watch_client = client.clone();
        let watch_cancel = cancel.clone();
        let worktree_path = tmp.path().to_path_buf();
        let watcher = tokio::spawn(async move {
            post_artifact_comments_with_client(
                &watch_client,
                "acme",
                "widgets",
                21,
                "acme-widgets-21",
                &worktree_path,
                SystemTime::now() - Duration::from_secs(1),
                watch_cancel,
                Duration::from_millis(20),
            )
            .await;
        });

        tokio::time::sleep(Duration::from_millis(120)).await;
        cancel.cancel();
        watcher.await.expect("watcher join");

        let comments = client.posted_comments();
        assert!(
            client.post_attempts() >= 2,
            "expected retry attempts after transient failure"
        );
        assert_eq!(
            comments.iter().filter(|c| c.phase == "quick-prd").count(),
            1
        );
    }

    #[tokio::test]
    async fn single_watcher_run_posts_both_artifact_comments() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_quick_prd(tmp.path(), "combined", "Combined quick prd");
        write_final_prompt(tmp.path(), "proj-combined", "Combined final prompt");
        let client = MockArtifactCommentClient::default();

        tokio::time::timeout(
            Duration::from_secs(2),
            post_artifact_comments_with_client(
                &client,
                "acme",
                "widgets",
                33,
                "acme-widgets-33",
                tmp.path(),
                SystemTime::now() - Duration::from_secs(1),
                CancellationToken::new(),
                Duration::from_millis(20),
            ),
        )
        .await
        .expect("watcher should complete after posting both artifacts");

        let comments = client.posted_comments();
        let phases: HashSet<String> = comments.into_iter().map(|c| c.phase).collect();
        assert!(phases.contains("quick-prd"));
        assert!(phases.contains("final-prompt"));
        assert_eq!(phases.len(), 2);
    }

    #[tokio::test]
    async fn watcher_idempotency_prevents_duplicate_comments_on_redispatch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_quick_prd(tmp.path(), "idempotent", "Idempotent quick");
        write_final_prompt(tmp.path(), "proj-idempotent", "Idempotent final prompt");
        let client = MockArtifactCommentClient::default();
        let child_start = SystemTime::now() - Duration::from_secs(1);

        tokio::time::timeout(
            Duration::from_secs(2),
            post_artifact_comments_with_client(
                &client,
                "acme",
                "widgets",
                44,
                "acme-widgets-44",
                tmp.path(),
                child_start,
                CancellationToken::new(),
                Duration::from_millis(20),
            ),
        )
        .await
        .expect("first watcher should complete");

        tokio::time::timeout(
            Duration::from_secs(2),
            post_artifact_comments_with_client(
                &client,
                "acme",
                "widgets",
                44,
                "acme-widgets-44",
                tmp.path(),
                child_start,
                CancellationToken::new(),
                Duration::from_millis(20),
            ),
        )
        .await
        .expect("second watcher should complete");

        let comments = client.posted_comments();
        assert_eq!(
            comments.iter().filter(|c| c.phase == "quick-prd").count(),
            1
        );
        assert_eq!(
            comments
                .iter()
                .filter(|c| c.phase == "final-prompt")
                .count(),
            1
        );
    }

    // -----------------------------------------------------------------------
    // collect_children result mapping via derive_terminal_label
    // -----------------------------------------------------------------------

    #[test]
    fn derive_terminal_label_ok_result_is_completed() {
        use crate::workflow::orchestrator::OrchestrationResult;
        let result: std::result::Result<
            crate::Result<OrchestrationResult>,
            tokio::task::JoinError,
        > = Ok(Ok(OrchestrationResult {
            summary: "done".to_owned(),
            loop_number: Some(1),
        }));
        assert_eq!(derive_terminal_label(&result), "ralph:completed");
    }

    #[test]
    fn derive_terminal_label_cancelled_is_failed() {
        let result: std::result::Result<
            crate::Result<crate::workflow::orchestrator::OrchestrationResult>,
            tokio::task::JoinError,
        > = Ok(Err(RalphError::Cancelled));
        assert_eq!(derive_terminal_label(&result), "ralph:failed");
    }

    #[test]
    fn derive_terminal_label_error_is_failed() {
        let result: std::result::Result<
            crate::Result<crate::workflow::orchestrator::OrchestrationResult>,
            tokio::task::JoinError,
        > = Ok(Err(RalphError::Orchestration("boom".to_owned())));
        assert_eq!(derive_terminal_label(&result), "ralph:failed");
    }

    #[tokio::test]
    async fn derive_terminal_label_panic_join_error_is_failed() {
        // Spawn a task that panics, then check the JoinError maps to failed.
        let handle = tokio::spawn(async {
            panic!("simulated task panic");
            #[allow(unreachable_code)]
            Ok::<crate::workflow::orchestrator::OrchestrationResult, crate::error::RalphError>(
                crate::workflow::orchestrator::OrchestrationResult {
                    summary: String::new(),
                    loop_number: None,
                },
            )
        });
        let result = handle.await;
        assert!(result.is_err(), "should be JoinError from panic");
        assert_eq!(derive_terminal_label(&result), "ralph:failed");
    }

    // -----------------------------------------------------------------------
    // drain_all_children timeout-abort behavior
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn drain_all_children_aborts_stuck_tasks_after_timeout() {
        use super::{drain_all_children, DaemonRuntimeConfig};
        use crate::daemon::TaskHandle;
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::Semaphore;

        // Create a config with short poll interval (drain uses 7200s by default;
        // we'll test that stuck tasks are eventually aborted).
        let config = DaemonRuntimeConfig {
            owner: "test".to_owned(),
            repo: "repo".to_owned(),
            base_branch: "main".to_owned(),
            poll_seconds: 1,
            max_concurrent: 1,
            labels: vec![],
            single_iteration: true,
            verbose: false,
            repo_root: PathBuf::from("/tmp/test-drain"),
            refinement_enabled: false,
            refinement_backend: "claude".to_owned(),
            global_config: crate::config::GlobalConfig::default(),
            auto_rebase_enabled: false,
            rebase_interval_seconds: 300,
            max_rebases_per_cycle: 0,
            rebase_timeout_seconds: 60,
            rebase_agent_backend: "none".to_owned(),
            workspace_root: PathBuf::from("/tmp/test-drain/.ralph"),
            prd_enabled: false,
            prd_question_backends: vec![],
            prd_writer_backend: "claude".to_owned(),
            prd_reviewer_backend: "claude".to_owned(),
            prd_max_revisions: 1,
            prd_backend_timeout_secs: 60,
            prd_shutdown_timeout_secs: 10,
            oracle_review_enabled: false,
            oracle_review_timeout_secs: 900,
            oracle_review_authors: vec![],
            oracle_review_max_per_cycle: 3,
            git_bin: "git".to_owned(),
            gh_bin: "gh".to_owned(),
            max_backend_retries: None,
            pr_review_whitelist: vec![],
        };

        let cancel = CancellationToken::new();
        let cancel_inner = cancel.clone();

        // Spawn a task that blocks until cancelled (simulating a stuck task).
        let handle = tokio::spawn(async move {
            cancel_inner.cancelled().await;
            Err::<crate::workflow::orchestrator::OrchestrationResult, _>(RalphError::Cancelled)
        });

        let mut children: HashMap<u32, TaskHandle> = HashMap::new();
        children.insert(
            999,
            TaskHandle {
                join_handle: handle,
                cancel_token: cancel,
                aborted_externally: Arc::new(AtomicBool::new(false)),
                watcher_cancel: CancellationToken::new(),
                watcher_handle: None,
                draft_pr_cancel: CancellationToken::new(),
                draft_pr_handle: None,
                branch: "ralph/test".to_owned(),
                log_file: PathBuf::from("/tmp/test-drain.log"),
                last_rebase_at: None,
                last_rebase_failure_sha: None,
                pr_url: None,
            },
        );

        let repo_root_lock = Arc::new(Semaphore::new(1));

        // drain_all_children should cancel the token and then collect the task.
        // We use a timeout to ensure this test doesn't hang.
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            drain_all_children(&config, &mut children, &repo_root_lock),
        )
        .await;

        assert!(result.is_ok(), "drain_all_children should complete");
        assert!(
            children.is_empty(),
            "all children should be drained after drain_all_children"
        );
    }

    /// Verifies the force-abort path in `drain_all_children_with_deadline`.
    /// Spawns a genuinely non-cooperative task that blocks a thread (not
    /// just an async sleep) and ignores cancellation.  With a short drain
    /// deadline, the function must escalate to `join_handle.abort()` and
    /// remove the task from the map.  We also assert that no post-drain
    /// side-effects occur (the task does not continue writing after abort).
    #[tokio::test]
    async fn drain_all_children_force_aborts_non_cooperative_task() {
        use super::{drain_all_children_with_deadline, DaemonRuntimeConfig};
        use crate::daemon::TaskHandle;
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::Semaphore;

        let config = DaemonRuntimeConfig {
            owner: "test".to_owned(),
            repo: "repo".to_owned(),
            base_branch: "main".to_owned(),
            poll_seconds: 1,
            max_concurrent: 1,
            labels: vec![],
            single_iteration: true,
            verbose: false,
            repo_root: PathBuf::from("/tmp/test-drain-abort"),
            refinement_enabled: false,
            refinement_backend: "claude".to_owned(),
            global_config: crate::config::GlobalConfig::default(),
            auto_rebase_enabled: false,
            rebase_interval_seconds: 300,
            max_rebases_per_cycle: 0,
            rebase_timeout_seconds: 60,
            rebase_agent_backend: "none".to_owned(),
            workspace_root: PathBuf::from("/tmp/test-drain-abort/.ralph"),
            prd_enabled: false,
            prd_question_backends: vec![],
            prd_writer_backend: "claude".to_owned(),
            prd_reviewer_backend: "claude".to_owned(),
            prd_max_revisions: 1,
            prd_backend_timeout_secs: 60,
            prd_shutdown_timeout_secs: 10,
            oracle_review_enabled: false,
            oracle_review_timeout_secs: 900,
            oracle_review_authors: vec![],
            oracle_review_max_per_cycle: 3,
            git_bin: "git".to_owned(),
            gh_bin: "gh".to_owned(),
            max_backend_retries: None,
            pr_review_whitelist: vec![],
        };

        // Counter that the task increments; if it continues running after
        // drain completes, the counter will keep growing.
        let side_effect_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counter_clone = side_effect_counter.clone();

        // Spawn a non-cooperative task: alternates between a blocking
        // thread sleep (genuinely non-cooperative — abort cannot preempt
        // it) and an .await point where abort can take effect.
        let handle = tokio::spawn(async move {
            loop {
                counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // Genuine blocking sleep — not abort-cooperative.
                tokio::task::spawn_blocking(|| {
                    std::thread::sleep(Duration::from_millis(50));
                })
                .await
                .ok();
            }
            // Unreachable but satisfies the return type.
            #[allow(unreachable_code)]
            Ok::<crate::workflow::orchestrator::OrchestrationResult, _>(
                crate::workflow::orchestrator::OrchestrationResult {
                    summary: String::new(),
                    loop_number: None,
                },
            )
        });

        let cancel = CancellationToken::new();
        let mut children: HashMap<u32, TaskHandle> = HashMap::new();
        children.insert(
            888,
            TaskHandle {
                join_handle: handle,
                cancel_token: cancel,
                aborted_externally: Arc::new(AtomicBool::new(false)),
                watcher_cancel: CancellationToken::new(),
                watcher_handle: None,
                draft_pr_cancel: CancellationToken::new(),
                draft_pr_handle: None,
                branch: "ralph/test-abort".to_owned(),
                log_file: PathBuf::from("/tmp/test-drain-abort.log"),
                last_rebase_at: None,
                last_rebase_failure_sha: None,
                pr_url: None,
            },
        );

        let repo_root_lock = Arc::new(Semaphore::new(1));

        // Use a very short drain deadline so the non-cooperative task hits
        // the force-abort path (deadline expires, then join_handle.abort()).
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            drain_all_children_with_deadline(
                &config,
                &mut children,
                &repo_root_lock,
                Duration::from_millis(500), // short deadline forces abort path
            ),
        )
        .await;

        assert!(
            result.is_ok(),
            "drain_all_children_with_deadline should complete within outer timeout"
        );
        assert!(
            children.is_empty(),
            "all children should be drained (force-aborted) after drain deadline"
        );

        // Snapshot the counter, wait, and verify it does not advance —
        // proving the task is truly stopped, not just removed from the map.
        let snapshot = side_effect_counter.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let after = side_effect_counter.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            snapshot, after,
            "task should not produce side effects after force-abort (counter advanced from {snapshot} to {after})"
        );
    }
}
