use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::config::GlobalConfig;
use crate::daemon::bootstrap;
use crate::daemon::github::{self, PrMergeStatus};
use crate::daemon::rebase_agent::{
    classify_rebase_failure_pure, parse_rebase_agent_backend, RebaseAgentBackend, RebaseFailureKind,
};

use crate::daemon::interactive_prd::{self, PrdPollConfig};
use crate::daemon::process;
use crate::daemon::refine;
use crate::daemon::worktree;
use crate::daemon::{format_task_id, ChildHandle};
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
    /// Path to the ralph binary for spawning daemon child commands.
    pub ralph_bin: PathBuf,
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
const GITHUB_COMMENT_LIMIT: usize = 65_536;
const TRUNCATED_NOTE: &str = "\n\n[truncated]";

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
        let owner = owner.to_owned();
        let repo = repo.to_owned();
        let marker = marker.to_owned();
        spawn_blocking_op(move || {
            github::comment_marker_exists(&owner, &repo, issue_number, &marker)
        })
        .await
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
        let owner = owner.to_owned();
        let repo = repo.to_owned();
        let task_id = task_id.to_owned();
        let phase = phase.to_owned();
        let body_text = body_text.to_owned();
        spawn_blocking_op(move || {
            github::post_idempotent_comment(
                &owner,
                &repo,
                issue_number,
                &task_id,
                &phase,
                &body_text,
            )
        })
        .await
    }
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

fn truncate_for_github(body: &str, max_chars: usize) -> String {
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
pub fn retrigger_failed_task(owner: &str, repo: &str, issue_number: u32) -> Result<()> {
    // Verify current label state from GitHub
    let labels = github::fetch_issue_labels(owner, repo, issue_number)?;
    let lifecycle = github::classify_lifecycle_labels(&labels);

    if !lifecycle.iter().any(|l| l == "ralph:failed") {
        return Err(RalphError::Validation(format!(
            "issue {owner}/{repo}#{issue_number} is not in failed state (labels: {})",
            lifecycle.join(", ")
        )));
    }

    // Swap failed -> ready so the daemon picks it up on next poll
    github::swap_lifecycle_label(owner, repo, issue_number, "ralph:failed", "ralph:ready")?;

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
/// All task state is in-memory (`children: HashMap<u32, ChildHandle>`).
/// GitHub lifecycle labels are the only durable task lifecycle source of truth.
pub async fn run(config: &DaemonRuntimeConfig) -> Result<()> {
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

    // Phase 1: Startup reconciliation — reset all `ralph:in-progress` to `ralph:ready`.
    // Always queries `ralph:in-progress` regardless of configured poll labels.
    {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let verbose = config.verbose;
        spawn_blocking_op(move || reconcile_in_progress_labels(&owner, &repo, verbose)).await?;
    }

    // Phase 2: Main loop with in-memory child tracking
    let mut children: HashMap<u32, ChildHandle> = HashMap::new();
    let mut iteration: u64 = 0;

    loop {
        iteration = iteration.saturating_add(1);

        // Kill children whose issues were externally aborted (label changed
        // from ralph:in-progress to ralph:failed via CLI `daemon abort`).
        // Runs before collect_children so that a fast-finishing aborted task
        // is not mistakenly treated as a normal success.
        kill_aborted_children(config, &mut children).await;

        // Collect finished children
        collect_children(config, &mut children).await;

        // Auto-rebase phase: rebase eligible PR-backed child branches
        auto_rebase_phase(config, &mut children).await;

        // Interactive PRD phase: advance PRD-labeled issues (before claim/dispatch
        // to prevent dual workflow ownership).
        if config.prd_enabled {
            if let Err(err) = run_prd_phase(config).await {
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
            if let Err(err) = poll_and_claim(config, &mut children, slots).await {
                eprintln!("warning: poll/claim cycle failed: {err}");
            }
        }

        // Collect again after spawning
        collect_children(config, &mut children).await;

        if config.single_iteration {
            // In single-iteration mode, wait for all spawned children to
            // reach a terminal state so the outcome is deterministic.
            drain_all_children(config, &mut children).await;
            break;
        }

        tokio::time::sleep(Duration::from_secs(config.poll_seconds)).await;
    }

    Ok(())
}

/// Run the interactive PRD poll/advance phase.
///
/// Builds a `PrdPollConfig` from the runtime config and delegates to
/// `interactive_prd::poll_and_advance_prd` in a blocking task.
async fn run_prd_phase(config: &DaemonRuntimeConfig) -> Result<()> {
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

    spawn_blocking_op(move || interactive_prd::poll_and_advance_prd(&prd_config)).await
}

/// Startup reconciliation: every issue currently labeled `ralph:in-progress`
/// is reset to `ralph:ready` (children map is empty on fresh daemon start).
///
/// Always queries `ralph:in-progress` directly rather than using configured
/// poll labels, ensuring stale issues are caught regardless of label config.
fn reconcile_in_progress_labels(owner: &str, repo: &str, verbose: bool) -> Result<()> {
    // Always query ralph:in-progress explicitly to catch all stale issues
    let reconcile_labels = vec!["ralph:in-progress".to_owned()];
    let (issues, _overflow) = github::poll_issues(owner, repo, &reconcile_labels)?;

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
            ) {
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

/// Poll for new issues, filter, claim, and dispatch.
async fn poll_and_claim(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, ChildHandle>,
    slots: u32,
) -> Result<()> {
    let (issues, overflow) = {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let labels = config.labels.clone();
        spawn_blocking_op(move || github::poll_issues(&owner, &repo, &labels)).await?
    };

    if overflow {
        eprintln!("warning: gh issue list returned exactly 100 issues; results may be truncated");
    }

    let mut claimed = 0u32;
    for issue in &issues {
        if claimed >= slots {
            break;
        }

        // Classify lifecycle labels
        let lifecycle = github::classify_lifecycle_labels(&issue.labels);

        // Multi-lifecycle-label normalization
        if lifecycle.len() > 1 {
            let owner = config.owner.clone();
            let repo = config.repo.clone();
            let issue_number = issue.number;
            let lifecycle_clone = lifecycle.clone();
            match spawn_blocking_op(move || {
                github::normalize_multi_lifecycle_labels(
                    &owner,
                    &repo,
                    issue_number,
                    &lifecycle_clone,
                )
            })
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

        // Skip issues carrying any PRD label (prevents dual workflow ownership)
        if interactive_prd::has_prd_label(&issue.labels) {
            if config.verbose {
                eprintln!(
                    "verbose: skipping issue #{} — carries PRD label, handled by interactive PRD workflow",
                    issue.number
                );
            }
            continue;
        }

        // Skip if we already have a child for this issue
        if children.contains_key(&issue.number) {
            continue;
        }

        // Claim: ready -> in-progress
        {
            let owner = config.owner.clone();
            let repo = config.repo.clone();
            let issue_number = issue.number;
            if let Err(err) = spawn_blocking_op(move || {
                github::swap_lifecycle_label(
                    &owner,
                    &repo,
                    issue_number,
                    "ralph:ready",
                    "ralph:in-progress",
                )
            })
            .await
            {
                eprintln!("warning: failed to claim issue #{}: {err}", issue.number);
                continue;
            }
        }

        // Dispatch
        let raw_idea = format!(
            "{}\n\n{}",
            issue.title,
            issue.body.as_deref().unwrap_or_default()
        );
        if let Err(err) = dispatch_task(config, children, issue.number, &raw_idea).await {
            eprintln!("warning: failed to dispatch issue #{}: {err}", issue.number);
            // Mark as failed since we already claimed it
            let owner = config.owner.clone();
            let repo = config.repo.clone();
            let issue_number = issue.number;
            let _ = spawn_blocking_op(move || {
                github::swap_lifecycle_label(
                    &owner,
                    &repo,
                    issue_number,
                    "ralph:in-progress",
                    "ralph:failed",
                )
            })
            .await;
        }

        claimed += 1;
    }

    Ok(())
}

/// Scan a task worktree for valid projects under `.ralph/projects/*/prompt.md`.
fn discover_project_ids(worktree_path: &Path) -> Vec<String> {
    let projects_dir = worktree_path.join(".ralph").join("projects");
    let entries = match std::fs::read_dir(&projects_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let prompt_path = entry.path().join("prompt.md");
        if prompt_path.is_file() {
            found.push(name);
        }
    }
    found
}

/// Scan remote `ralph/*` branches (excluding `daemon/` and `issue-` branches)
/// for project data.  If found, check out that branch in the worktree and
/// return the discovered project ID.
///
/// This handles retriggers of tasks originally dispatched via `ralph auto
/// --idea`, where project commits land on `ralph/{project_id}` instead of
/// `ralph/issue-<n>`.
fn discover_project_from_remote_branches(
    worktree_path: &Path,
    task_id: &str,
) -> Result<Option<String>> {
    let output = std::process::Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/remotes/origin/ralph/",
        ])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "discover_project_from_remote_branches: git for-each-ref failed: {err}"
            ))
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for remote_branch in stdout.lines() {
        let remote_branch = remote_branch.trim();
        if remote_branch.is_empty() {
            continue;
        }
        // Skip daemon worktree branches and issue branches (already checked
        // by sync_project_branch).
        let short = remote_branch.strip_prefix("origin/ralph/").unwrap_or("");
        if short.starts_with("daemon/") || short.starts_with("issue-") {
            continue;
        }

        // Check if this branch has .ralph/projects/ content
        let ls = std::process::Command::new("git")
            .args(["ls-tree", "--name-only", remote_branch, ".ralph/projects/"])
            .current_dir(worktree_path)
            .output();
        let has_projects = ls
            .as_ref()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false);

        if !has_projects {
            continue;
        }

        // Checkout this branch and try discovery
        let local_branch = match remote_branch.strip_prefix("origin/") {
            Some(b) => b,
            None => continue,
        };
        let checkout = crate::git::run_git(
            worktree_path,
            &["checkout", "-B", local_branch, remote_branch],
        );
        if checkout.is_err() {
            continue;
        }

        let project_ids = discover_project_ids(worktree_path);
        if let Some(project_id) = project_ids.into_iter().next() {
            eprintln!(
                "dispatch: event=project_branch_fallback task_id={task_id} branch={local_branch} project_id={project_id}"
            );
            return Ok(Some(project_id));
        }
    }

    Ok(None)
}

/// Find the most recently modified project directory by prompt mtime.
/// Returns `None` if no projects exist.
#[cfg(test)]
fn discover_latest_project_id(worktree_path: &Path) -> Option<String> {
    let projects_dir = worktree_path.join(".ralph").join("projects");
    let entries = match std::fs::read_dir(&projects_dir) {
        Ok(entries) => entries,
        Err(_) => return None,
    };

    let mut best: Option<(String, std::time::SystemTime)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let prompt_path = entry.path().join("prompt.md");
        let modified = match std::fs::metadata(prompt_path).and_then(|meta| meta.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };
        let dominated = match &best {
            Some((_, prev)) => modified > *prev,
            None => true,
        };
        if dominated {
            best = Some((name, modified));
        }
    }
    best.map(|(id, _)| id)
}

/// Dispatch a single task: create worktree, spawn child, track in-memory.
async fn dispatch_task(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, ChildHandle>,
    issue_number: u32,
    raw_idea: &str,
) -> Result<()> {
    let task_id = format_task_id(&config.owner, &config.repo, issue_number);

    bootstrap::ensure_repo_ready(&config.repo_root).await?;

    let workspace_root = config.workspace_root.clone();

    // Create worktree (reuses existing branch if present)
    let wt_path = {
        let repo_root = config.repo_root.clone();
        let ws_root = workspace_root.clone();
        let tid = task_id.clone();
        spawn_blocking_op(move || worktree::create_worktree(&repo_root, &ws_root, &tid)).await?
    };

    // Clean worktree of any dirty files from previous runs
    {
        let wt = wt_path.clone();
        spawn_blocking_op(move || worktree::clean_worktree(&wt)).await?;
    }

    // Dispatch-time project discovery
    let effective_project_id = {
        let wt = wt_path.clone();
        let mut discovered = spawn_blocking_op(move || Ok(discover_project_ids(&wt))).await?;

        // If no projects found on the issue branch, scan remote ralph/*
        // branches for project data.  This handles retriggers of tasks that
        // were originally dispatched via `ralph auto --idea` (where project
        // commits land on `ralph/{project_id}`, not `ralph/issue-<n>`).
        if discovered.is_empty() {
            let wt = wt_path.clone();
            let tid = task_id.clone();
            if let Ok(Some(project_id)) =
                spawn_blocking_op(move || discover_project_from_remote_branches(&wt, &tid)).await
            {
                discovered.push(project_id);
            }
        }

        match discovered.len() {
            0 => None,
            1 => {
                let project_id = discovered.into_iter().next().unwrap();
                eprintln!(
                    "dispatch: event=project_backfill task_id={task_id} discovered_project_id={project_id}"
                );
                Some(project_id)
            }
            n => {
                eprintln!(
                    "dispatch: event=project_discovery_ambiguous task_id={task_id} count={n} projects={:?}",
                    discovered
                );
                None
            }
        }
    };

    // Remote-first project branch sync
    {
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

    // Checkout project branch if resuming
    if let Some(ref project_id) = effective_project_id {
        let wt = wt_path.clone();
        let branch = format!("ralph/{project_id}");
        let branch_clone = branch.clone();
        let checkout_result =
            spawn_blocking_op(move || worktree::checkout_branch_in_worktree(&wt, &branch_clone))
                .await;
        match checkout_result {
            Ok(()) => {
                eprintln!("dispatch: checked out project branch {branch} for task {task_id}");
            }
            Err(err) => {
                eprintln!(
                    "dispatch: project branch {branch} checkout failed for task {task_id} (may not exist yet): {err}"
                );
            }
        }
    }

    // Refine the prompt if enabled
    let (idea, refined_title, cleaned_body) = if config.refinement_enabled {
        match refine::refine_prompt(raw_idea, &config.refinement_backend, &config.global_config)
            .await
        {
            Ok(refined) => (refined.body, refined.title, refined.cleaned_body),
            Err(err) => {
                eprintln!("warning: refinement failed for task {task_id}, using raw idea: {err}");
                (raw_idea.to_owned(), None, None)
            }
        }
    } else {
        (raw_idea.to_owned(), None, None)
    };

    // Update GitHub issue title with refined title (best-effort)
    if let Some(ref title) = refined_title {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let title = title.clone();
        if let Err(err) = spawn_blocking_op(move || {
            github::update_issue_title(&owner, &repo, issue_number, &title)
        })
        .await
        {
            eprintln!("warning: failed to update issue title for {task_id}: {err}");
        }
    }

    // Update GitHub issue body with cleaned body (best-effort)
    if let Some(ref cleaned_body) = cleaned_body {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let cb = cleaned_body.clone();
        if let Err(err) =
            spawn_blocking_op(move || github::update_issue_body(&owner, &repo, issue_number, &cb))
                .await
        {
            eprintln!("warning: failed to update issue body for {task_id}: {err}");
        }
    }

    // Post refined-prompt comment (best-effort)
    {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let tid = task_id.clone();
        let comment_body = match &refined_title {
            Some(title) => format!("**{title}**\n\n{idea}"),
            None => idea.clone(),
        };
        if let Err(err) = spawn_blocking_op(move || {
            github::post_idempotent_comment(
                &owner,
                &repo,
                issue_number,
                &tid,
                "refined-prompt",
                &comment_body,
            )
        })
        .await
        {
            eprintln!("warning: failed to post refined-prompt comment for {task_id}: {err}");
        }
    }

    // Create log file for child output
    let log_path = task_log_path(&config.workspace_root, &task_id);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Determine branch name for the child handle
    let branch_name = format!("ralph/daemon/{task_id}");

    // Ignore stale artifacts left from prior runs.  Subtract 2 seconds to
    // tolerate filesystems that truncate mtime to whole-second granularity
    // (e.g. tmpfs in nix build sandboxes).
    let child_start_time = SystemTime::now() - Duration::from_secs(2);

    // Spawn child process
    let spawned = {
        let ralph_bin = config.ralph_bin.clone();
        let wt = wt_path.clone();
        let idea_clone = idea.clone();
        match effective_project_id.as_deref() {
            Some(project_id) => {
                eprintln!(
                    "dispatch: task {task_id} has project_id={project_id}; using ralph run --project"
                );
                process::spawn_ralph_run(&ralph_bin, &wt, project_id, &log_path).await?
            }
            None => {
                eprintln!(
                    "dispatch: task {task_id} has no project_id; using ralph auto --idea (fresh dispatch)"
                );
                process::spawn_ralph_auto(&ralph_bin, &wt, &idea_clone, &log_path).await?
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

    children.insert(
        issue_number,
        ChildHandle {
            pid: spawned.pid,
            pgid: spawned.pgid,
            child: spawned.child,
            watcher_cancel,
            watcher_handle,
            branch: branch_name,
            log_file: log_path,
            last_rebase_at: None,
            last_rebase_failure_sha: None,
        },
    );

    eprintln!("dispatched task {task_id} (pid={})", spawned.pid);

    Ok(())
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

/// Collect finished children and transition them to terminal states via labels.
async fn collect_children(config: &DaemonRuntimeConfig, children: &mut HashMap<u32, ChildHandle>) {
    let mut finished = Vec::new();
    let mut still_running = 0u32;

    for (issue_number, handle) in children.iter_mut() {
        match handle.child.try_wait() {
            Ok(Some(status)) => {
                let task_id = format_task_id(&config.owner, &config.repo, *issue_number);
                if config.verbose {
                    let exit_code = status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "signal".to_owned());
                    eprintln!(
                        "verbose: child terminal task_id={task_id} pid={} exit_status={} exit_code={exit_code}",
                        handle.pid, status
                    );
                }
                let terminal_label = if status.success() {
                    "ralph:completed"
                } else {
                    "ralph:failed"
                };
                finished.push((*issue_number, terminal_label));
            }
            Ok(None) => {
                still_running = still_running.saturating_add(1);
            }
            Err(err) => {
                let task_id = format_task_id(&config.owner, &config.repo, *issue_number);
                eprintln!(
                    "warning: failed to check child for {task_id} (pid={} pgid={}): {err}",
                    handle.pid, handle.pgid
                );
                finished.push((*issue_number, "ralph:failed"));
            }
        }
    }

    if config.verbose && still_running > 0 {
        eprintln!("verbose: child collection still_running={still_running}");
    }

    for (issue_number, terminal_label) in finished {
        let task_id = format_task_id(&config.owner, &config.repo, issue_number);
        let Some(mut handle) = children.remove(&issue_number) else {
            continue;
        };
        handle.watcher_cancel.cancel();
        if let Some(join_handle) = handle.watcher_handle.take() {
            if let Err(err) = join_handle.await {
                eprintln!("warning: artifact watcher join failed for {task_id}: {err}");
            }
        }
        if terminal_label == "ralph:failed" {
            print_log_tail(&task_id, &handle.log_file);
        }
        complete_task(config, issue_number, &task_id, terminal_label).await;
    }
}

/// Kill running children whose issues have been externally aborted (e.g. via
/// `ralph daemon abort`).  The CLI abort swaps the issue label to
/// `ralph:failed` but cannot kill the process (no PID access).  This function
/// queries labels for each running child and terminates any that are no longer
/// `ralph:in-progress`.
async fn kill_aborted_children(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, ChildHandle>,
) {
    let issue_numbers: Vec<u32> = children.keys().cloned().collect();
    let mut to_kill = Vec::new();

    for issue_number in issue_numbers {
        let task_id = format_task_id(&config.owner, &config.repo, issue_number);
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        match spawn_blocking_op(move || github::fetch_issue_labels(&owner, &repo, issue_number))
            .await
        {
            Ok(labels) => {
                if !labels.iter().any(|l| l == "ralph:in-progress") {
                    eprintln!(
                        "abort-check: task {task_id} no longer in-progress (labels: {}), killing",
                        labels.join(", ")
                    );
                    to_kill.push(issue_number);
                }
            }
            Err(err) => {
                eprintln!("abort-check: failed to query labels for {task_id}: {err}");
            }
        }
    }

    for issue_number in to_kill {
        if let Some(mut handle) = children.remove(&issue_number) {
            let task_id = format_task_id(&config.owner, &config.repo, issue_number);
            crate::daemon::process::terminate_process_group_blocking(
                handle.pgid,
                Duration::from_secs(10),
            );
            handle.watcher_cancel.cancel();
            if let Some(join_handle) = handle.watcher_handle.take() {
                if let Err(err) = join_handle.await {
                    eprintln!("warning: artifact watcher join failed for {task_id}: {err}");
                }
            }
            eprintln!(
                "abort-check: killed {task_id} (pid={} pgid={})",
                handle.pid, handle.pgid
            );
        }
    }
}

/// Wait until all active children have exited.
async fn drain_all_children(
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<u32, ChildHandle>,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(7200);

    while !children.is_empty() && tokio::time::Instant::now() < deadline {
        collect_children(config, children).await;
        if children.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Force-kill remaining children
    if !children.is_empty() {
        let remaining: Vec<u32> = children.keys().cloned().collect();
        for issue_number in remaining {
            let task_id = format_task_id(&config.owner, &config.repo, issue_number);
            if let Some(mut handle) = children.remove(&issue_number) {
                eprintln!(
                    "warning: force-killing child for {task_id} (pid={} pgid={}, drain timeout)",
                    handle.pid, handle.pgid
                );
                if let Err(err) = handle.child.kill().await {
                    eprintln!("warning: failed to kill child for {task_id}: {err}");
                }
                if let Err(err) = handle.child.wait().await {
                    eprintln!("warning: failed to wait child for {task_id}: {err}");
                }
                handle.watcher_cancel.cancel();
                if let Some(join_handle) = handle.watcher_handle.take() {
                    if let Err(err) = join_handle.await {
                        eprintln!("warning: artifact watcher join failed for {task_id}: {err}");
                    }
                }
            }
            complete_task(config, issue_number, &task_id, "ralph:failed").await;
        }
    }
}

/// Transition a task to terminal state via GitHub labels.
async fn complete_task(
    config: &DaemonRuntimeConfig,
    issue_number: u32,
    task_id: &str,
    terminal_label: &str,
) {
    // Post completion comment (best-effort, idempotent)
    {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let tid = task_id.to_owned();
        let phase = terminal_label.trim_start_matches("ralph:").to_owned();
        let comment_body = format!("Task `{tid}` finished with status: **{phase}**.");
        if let Err(err) = spawn_blocking_op(move || {
            github::post_idempotent_comment(
                &owner,
                &repo,
                issue_number,
                &tid,
                &phase,
                &comment_body,
            )
        })
        .await
        {
            eprintln!("warning: failed to post completion comment for {task_id}: {err}");
        }
    }

    // PR flow (only on success)
    if terminal_label == "ralph:completed" {
        // Resolve actual worktree branch for PR creation
        let workspace_root = config.workspace_root.clone();
        let wt_path = worktree::task_worktree_path(&workspace_root, task_id);
        if wt_path.exists() {
            if let Err(err) = handle_pr_flow(config, task_id, issue_number, &wt_path).await {
                eprintln!("warning: PR flow failed for {task_id}: {err}");
            }
        }
    }

    // Swap lifecycle label: in-progress -> terminal
    {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let label = terminal_label.to_owned();
        if let Err(err) = spawn_blocking_op(move || {
            github::swap_lifecycle_label(&owner, &repo, issue_number, "ralph:in-progress", &label)
        })
        .await
        {
            eprintln!("warning: failed to update labels for {task_id}: {err}");
        }
    }

    // Worktree cleanup
    cleanup_worktree_for_terminal_state(config, task_id, terminal_label).await;

    let log_path = task_log_path(&config.workspace_root, task_id);
    eprintln!(
        "task {task_id} completed with label: {terminal_label} (log: {})",
        log_path.display()
    );
}

async fn cleanup_worktree_for_terminal_state(
    config: &DaemonRuntimeConfig,
    task_id: &str,
    terminal_label: &str,
) {
    if should_cleanup_worktree(terminal_label) {
        eprintln!(
            "complete-task-terminal: cleaning worktree for {task_id} (label={terminal_label})"
        );
        cleanup_worktree(config, task_id).await;
        return;
    }

    eprintln!("complete-task-terminal: preserving worktree for {task_id} (label={terminal_label})");
}

/// Remove the worktree for a task (best-effort).
async fn cleanup_worktree(config: &DaemonRuntimeConfig, task_id: &str) {
    let workspace_root = config.workspace_root.clone();
    let repo_root = config.repo_root.clone();
    let tid = task_id.to_owned();
    if let Err(err) = spawn_blocking_op(move || {
        worktree::remove_worktree(&repo_root, &workspace_root, &tid);
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
async fn auto_rebase_phase(config: &DaemonRuntimeConfig, children: &mut HashMap<u32, ChildHandle>) {
    if !config.auto_rebase_enabled {
        eprintln!("auto-rebase: skipped (disabled by config)");
        return;
    }

    // Collect children sorted by issue number for deterministic processing
    let mut issue_numbers: Vec<u32> = children.keys().cloned().collect();
    issue_numbers.sort();

    let mut rebase_count = 0u32;

    for issue_number in &issue_numbers {
        let (branch, last_rebase_at, last_failure_sha) = match children.get(issue_number) {
            Some(h) => (
                h.branch.clone(),
                h.last_rebase_at,
                h.last_rebase_failure_sha.clone(),
            ),
            None => continue,
        };

        if rebase_count >= config.max_rebases_per_cycle {
            eprintln!(
                "auto-rebase: per-cycle cap reached ({}/{})",
                rebase_count, config.max_rebases_per_cycle
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

        // Check if there's an existing PR for this branch
        let pr_url = {
            let owner = config.owner.clone();
            let repo = config.repo.clone();
            let br = branch.clone();
            match spawn_blocking_op(move || github::find_existing_pr(&owner, &repo, &br)).await {
                Ok(Some(url)) => url,
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
            let owner = config.owner.clone();
            let repo = config.repo.clone();
            match spawn_blocking_op(move || github::query_pr_merge_info(&owner, &repo, pr_number))
                .await
            {
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

        // Perform rebase
        let rebase_target = format!("origin/{}", merge_info.base_branch);
        let head_sha = merge_info.head_oid.clone();

        eprintln!(
            "auto-rebase: rebasing {task_id} (branch={branch}, target={rebase_target}, head={head_sha})"
        );

        // Create worktree on the task's branch
        let wt_path = {
            let repo_root = config.repo_root.clone();
            let ws_root = config.workspace_root.clone();
            let tid = task_id.clone();
            let br = branch.clone();
            match spawn_blocking_op(move || {
                worktree::create_worktree_on_branch(&repo_root, &ws_root, &tid, &br)
            })
            .await
            {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("auto-rebase: failed to create worktree for {task_id}: {err}");
                    rebase_count += 1;
                    continue;
                }
            }
        };

        // Fetch, rebase, push with timeout
        let timeout = Duration::from_secs(config.rebase_timeout_seconds);
        let rebase_result = {
            let wt = wt_path.clone();
            let target = rebase_target.clone();
            let br = branch.clone();
            let timeout_dur = timeout;
            let backend_str = config.rebase_agent_backend.clone();
            spawn_blocking_op(move || execute_rebase(&wt, &target, &br, timeout_dur, &backend_str))
                .await
        };

        // Clean up rebase worktree (best-effort)
        {
            let repo_root = config.repo_root.clone();
            let ws_root = config.workspace_root.clone();
            let tid = task_id.clone();
            let _ = spawn_blocking_op(move || {
                worktree::remove_rebase_worktree(&repo_root, &ws_root, &tid);
                Ok(())
            })
            .await;
        }

        rebase_count += 1;

        match rebase_result {
            Ok(()) => {
                eprintln!("auto-rebase: success for {task_id}");
                if let Some(h) = children.get_mut(issue_number) {
                    h.last_rebase_at = Some(std::time::Instant::now());
                }
            }
            Err(err) => {
                let err_msg = err.to_string();
                let is_lease = github::is_lease_rejection(&err_msg);

                if is_lease {
                    eprintln!(
                        "auto-rebase: lease mismatch for {task_id} — skipping for this cycle"
                    );
                    continue;
                }

                eprintln!("auto-rebase: failure for {task_id}: {err_msg}");

                // Skip duplicate failure comment for the same head SHA.
                if last_failure_sha.as_deref() == Some(head_sha.as_str()) {
                    eprintln!(
                        "auto-rebase: skipping duplicate failure comment for {task_id} (head={head_sha})"
                    );
                } else {
                    let marker = format!("<!-- ralph:rebase:{task_id}:failed:{head_sha} -->");
                    let body = format!(
                        "{marker}\nAuto-rebase failed for task `{task_id}` (head: `{head_sha}`).\n\nError: {err_msg}"
                    );
                    let owner = config.owner.clone();
                    let repo = config.repo.clone();
                    let _ = spawn_blocking_op(move || {
                        github::post_pr_comment(&owner, &repo, pr_number, &body)
                    })
                    .await;
                    if let Some(h) = children.get_mut(issue_number) {
                        h.last_rebase_failure_sha = Some(head_sha.clone());
                    }
                }
            }
        }
    }
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

    // Fetch
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

pub(crate) fn extract_project_ref(branch: &str) -> Option<String> {
    let mut parts = branch.split('/');
    let prefix = parts.next()?;
    let project_id = parts.next()?;
    if prefix == "ralph" && !project_id.is_empty() && parts.next().is_none() {
        Some(project_id.to_owned())
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
async fn handle_pr_flow(
    config: &DaemonRuntimeConfig,
    task_id: &str,
    issue_number: u32,
    wt_path: &Path,
) -> Result<()> {
    // Resolve branch from worktree
    let branch = {
        let wt = wt_path.to_path_buf();
        match spawn_blocking_op(move || github::current_branch(&wt)).await {
            Ok(b) => b,
            Err(err) => {
                eprintln!("warning: failed to read current branch for {task_id}: {err}");
                return Ok(());
            }
        }
    };

    // Step 1: Check if there's a diff against the configured base branch
    let has_changes = {
        let wt = wt_path.to_path_buf();
        let base = config.base_branch.clone();
        match spawn_blocking_op(move || github::has_diff_with_base(&wt, Some(&base))).await {
            Ok(v) => v,
            Err(err) => {
                eprintln!("warning: failed to check diff for {task_id}: {err}");
                return Ok(());
            }
        }
    };

    if !has_changes {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let tid = task_id.to_owned();
        let body = format!("Task `{task_id}` completed with no code changes. No PR created.");
        if let Err(err) = spawn_blocking_op(move || {
            github::post_idempotent_comment(&owner, &repo, issue_number, &tid, "no-diff", &body)
        })
        .await
        {
            eprintln!("warning: failed to post no-diff comment for {task_id}: {err}");
        }
        return Ok(());
    }

    // Skip push/PR flow when no origin remote exists
    {
        let wt = wt_path.to_path_buf();
        let has_origin = match spawn_blocking_op(move || github::has_origin_remote(&wt)).await {
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
    {
        let wt = wt_path.to_path_buf();
        let br = branch.clone();
        match spawn_blocking_op(move || github::push_branch(&wt, &br)).await {
            Ok(()) => {}
            Err(err) => {
                eprintln!("warning: failed to push branch {branch} for {task_id}: {err}");
                return Ok(());
            }
        }
    }

    // Step 3: Gather context
    let diff_stat: Option<String> = {
        let wt = wt_path.to_path_buf();
        match spawn_blocking_op(move || github::diff_stat(&wt)).await {
            Ok(stat) => stat,
            Err(err) => {
                eprintln!("warning: diff stat failed for {task_id}: {err}; using fallback");
                None
            }
        }
    };

    // Fetch raw_idea from GitHub for PR body context
    let raw_idea = {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        match spawn_blocking_op(move || github::fetch_issue_body(&owner, &repo, issue_number)).await
        {
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

    // Step 5: Check for existing PR
    let existing_pr_url = {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let br = branch.clone();
        match spawn_blocking_op(move || github::find_existing_pr(&owner, &repo, &br)).await {
            Ok(url) => url,
            Err(err) => {
                eprintln!("warning: failed to check for existing PR: {err}");
                None
            }
        }
    };

    // Try to get refined title from GitHub issue
    let refined_title = {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        match spawn_blocking_op(move || github::fetch_issue_body(&owner, &repo, issue_number)).await
        {
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
            let url_for_edit = url.clone();
            let title_clone = title.clone();
            let body_path = body_file.path().to_path_buf();
            match spawn_blocking_op(move || {
                github::edit_pr(&url_for_edit, &title_clone, &body_path)
            })
            .await
            {
                Ok(()) => {}
                Err(err) => {
                    return Err(RalphError::Orchestration(format!(
                        "failed to edit PR for {task_id}: {err}"
                    )));
                }
            }
        }
        None => {
            let owner = config.owner.clone();
            let repo = config.repo.clone();
            let br = branch.clone();
            let title_clone = title.clone();
            let body_path = body_file.path().to_path_buf();
            let base = config.base_branch.clone();
            match spawn_blocking_op(move || {
                github::create_pr_with_body_file(
                    &owner,
                    &repo,
                    &br,
                    &title_clone,
                    &body_path,
                    Some(&base),
                )
            })
            .await
            {
                Ok(url) => {
                    eprintln!("created PR for {task_id}: {url}");
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
        build_pr_body, build_pr_title, detect_final_prompt_artifact, detect_quick_prd_artifact,
        discover_latest_project_id, extract_issue_body, extract_original_title,
        extract_project_ref, newest_by_mtime, post_artifact_comments_with_client,
        sweep_artifact_comments, truncate_for_github, write_body_file, ArtifactCommentClient,
        ArtifactWatcherState, TRUNCATED_NOTE,
    };
    use crate::error::RalphError;
    use crate::Result;
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::path::PathBuf;
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
        assert!(body.contains("Closes #1"));
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
    fn discover_latest_project_id_prefers_newest_created_at() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let worktree = tmp.path().join("repo");
        let projects_root = worktree.join(".ralph").join("projects");
        std::fs::create_dir_all(&projects_root).expect("create projects root");

        let project_a = projects_root.join("acme-project-a");
        let project_b = projects_root.join("acme-project-b");
        let project_c = projects_root.join("acme-project-c");
        std::fs::create_dir_all(&project_a).expect("mkdir a");
        std::fs::create_dir_all(&project_b).expect("mkdir b");
        std::fs::create_dir_all(&project_c).expect("mkdir c");

        std::fs::write(project_a.join("prompt.md"), "a").expect("write a");
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(project_c.join("prompt.md"), "c").expect("write c");
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(project_b.join("prompt.md"), "b").expect("write b");

        assert_eq!(
            discover_latest_project_id(&worktree),
            Some("acme-project-b".to_owned())
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
}
