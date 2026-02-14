use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::GlobalConfig;
use crate::daemon::github;
use crate::daemon::process;
use crate::daemon::refine;
use crate::daemon::worktree;
use crate::daemon::{format_task_id, DaemonTask, TaskState, TaskStore};
use crate::error::RalphError;
use crate::util::time::now_iso8601;
use crate::Result;

/// Configuration for the daemon runtime loop.
#[derive(Clone)]
pub struct DaemonRuntimeConfig {
    pub owner: String,
    pub repo: String,
    pub poll_seconds: u64,
    pub max_concurrent: u32,
    pub labels: Vec<String>,
    /// When true, the daemon runs exactly one iteration and exits.
    pub single_iteration: bool,
    /// Path to the ralph binary for spawning `ralph auto` children.
    pub ralph_bin: PathBuf,
    /// Root of the git repository (for worktree operations).
    pub repo_root: PathBuf,
    /// Prompt refinement feature toggle (plumbed for upcoming loops).
    pub refinement_enabled: bool,
    /// Backend spec used for prompt refinement (plumbed for upcoming loops).
    pub refinement_backend: String,
    /// Global config snapshot for runtime backend operations.
    pub global_config: GlobalConfig,
}

/// Active child process handle tracked by the runtime.
struct ActiveChild {
    pid: u32,
    pgid: u32,
    child: tokio::process::Child,
    log_file: PathBuf,
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

/// Return the log file path for a task.
fn task_log_path(store: &TaskStore, task_id: &str) -> PathBuf {
    store
        .path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("logs")
        .join(format!("{task_id}.log"))
}

/// Print the last 50 lines of a task's log file to stderr for diagnostics.
fn print_log_tail(task_id: &str, log_file: &Path) {
    if let Ok(content) = std::fs::read_to_string(log_file) {
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(50);
        eprintln!("--- last output from {task_id} ({}) ---", log_file.display());
        for line in &lines[start..] {
            eprintln!("  {line}");
        }
        eprintln!("--- end ---");
    }
}

/// Run the daemon loop: reconcile, then poll/claim/dispatch/collect.
pub async fn run(store: &TaskStore, config: &DaemonRuntimeConfig) -> Result<()> {
    // Phase 1: Startup reconciliation
    {
        let store = store.clone();
        spawn_blocking_op(move || reconcile_tasks(&store)).await?;
    }
    {
        let store = store.clone();
        let config = config.clone();
        spawn_blocking_op(move || reconcile_worktrees(&store, &config)).await?;
    }

    // Phase 2: Main loop
    let mut children: HashMap<String, ActiveChild> = HashMap::new();

    // Re-adopt pending tasks from reconciliation
    adopt_pending_tasks(store, config, &mut children).await?;

    loop {
        // Collect finished children
        collect_children(store, config, &mut children).await;

        // Poll for new issues
        let active_count = children.len() as u32;
        let slots = config.max_concurrent.saturating_sub(active_count);

        if slots > 0 {
            if let Err(err) = poll_and_claim(store, config, &mut children, slots).await {
                eprintln!("warning: poll/claim cycle failed: {err}");
            }
        }

        // Collect again after spawning
        collect_children(store, config, &mut children).await;

        if config.single_iteration {
            // In single-iteration mode, wait for all spawned children to
            // reach a terminal state so the outcome is deterministic.
            drain_all_children(store, config, &mut children).await;
            break;
        }

        tokio::time::sleep(Duration::from_secs(config.poll_seconds)).await;
    }

    Ok(())
}

/// Reconcile task state on startup: move all `in_progress` tasks to `pending`
/// and clear their PID/PGID.
fn reconcile_tasks(store: &TaskStore) -> Result<()> {
    store.with_exclusive_tasks(|tasks| {
        let mut reconciled = 0u32;
        for task in tasks.iter_mut() {
            if task.state == TaskState::InProgress {
                task.state = TaskState::Pending;
                task.child_pid = None;
                task.child_pgid = None;
                task.updated_at = now_iso8601();
                reconciled += 1;
            }
        }
        if reconciled > 0 {
            eprintln!("reconcile: reset {reconciled} in_progress task(s) to pending");
        }
        Ok(())
    })
}

/// Reconcile orphaned/stale worktrees at startup.
///
/// Only non-terminal task IDs are considered "active" — worktrees for
/// terminal tasks (completed, failed, aborted) are cleaned up.
fn reconcile_worktrees(store: &TaskStore, config: &DaemonRuntimeConfig) -> Result<()> {
    let tasks = store.load()?;
    let active_ids: Vec<String> = tasks
        .iter()
        .filter(|t| !t.state.is_terminal())
        .map(|t| t.task_id.clone())
        .collect();
    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| RalphError::Orchestration("cannot derive workspace root".into()))?;
    worktree::reconcile_worktrees(
        &config.repo_root,
        workspace_root,
        &active_ids,
    );
    Ok(())
}

/// Re-adopt pending tasks by spawning children for them.
async fn adopt_pending_tasks(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
) -> Result<()> {
    let pending: Vec<DaemonTask> = {
        let store = store.clone();
        spawn_blocking_op(move || {
            let tasks = store.load()?;
            Ok(tasks
                .iter()
                .filter(|t| t.state == TaskState::Pending)
                .cloned()
                .collect())
        })
        .await?
    };

    for mut task in pending {
        if children.len() as u32 >= config.max_concurrent {
            break;
        }
        if task.raw_idea.is_none() {
            let store_clone = store.clone();
            let task_clone = task.clone();
            match spawn_blocking_op(move || fetch_and_persist_raw_idea(&store_clone, &task_clone))
                .await
            {
                Ok(raw_idea) => task.raw_idea = Some(raw_idea),
                Err(err) => {
                    eprintln!(
                        "warning: failed to hydrate raw idea for pending task {}: {err}",
                        task.task_id
                    );
                    continue;
                }
            }
        }
        if let Err(err) = dispatch_task(store, config, children, &task).await {
            eprintln!("warning: failed to re-adopt task {}: {err}", task.task_id);
        }
    }

    Ok(())
}

/// Poll for new issues, filter, claim, and dispatch.
async fn poll_and_claim(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
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

    let claimable = github::filter_claimable(issues);

    let existing_ids: Vec<String> = {
        let store = store.clone();
        spawn_blocking_op(move || {
            let existing_tasks = store.load()?;
            Ok(existing_tasks.iter().map(|t| t.task_id.clone()).collect())
        })
        .await?
    };

    let mut claimed = 0u32;
    for issue in claimable {
        if claimed >= slots {
            break;
        }

        let task_id = format_task_id(&config.owner, &config.repo, issue.number);

        // Skip if we already have a task for this issue
        if existing_ids.contains(&task_id) {
            continue;
        }

        // Claim on GitHub
        {
            let owner = config.owner.clone();
            let repo = config.repo.clone();
            let issue_number = issue.number;
            if let Err(err) =
                spawn_blocking_op(move || github::claim_issue(&owner, &repo, issue_number)).await
            {
                eprintln!("warning: failed to claim issue #{}: {err}", issue.number);
                continue;
            }
        }

        // Create task record
        let now = now_iso8601();
        let task = DaemonTask {
            task_id: task_id.clone(),
            state: TaskState::Pending,
            issue_number: issue.number,
            owner: config.owner.clone(),
            repo: config.repo.clone(),
            raw_idea: Some(format!(
                "{}\n\n{}",
                issue.title,
                issue.body.unwrap_or_default()
            )),
            child_pid: None,
            child_pgid: None,
            branch: Some(format!("ralph/daemon/{task_id}")),
            pr_url: None,
            created_at: now.clone(),
            updated_at: now,
        };

        {
            let store = store.clone();
            let task_id = task_id.clone();
            let task_for_store = task.clone();
            spawn_blocking_op(move || {
                store.with_exclusive_tasks(|tasks| {
                    // Double-check no duplicate
                    if !tasks.iter().any(|t| t.task_id == task_id) {
                        tasks.push(task_for_store.clone());
                    }
                    Ok(())
                })
            })
            .await?;
        }

        // Dispatch
        if let Err(err) = dispatch_task(store, config, children, &task).await {
            eprintln!("warning: failed to dispatch task {}: {err}", task_id);
        }

        claimed += 1;
    }

    Ok(())
}

/// Dispatch a single task: create worktree, spawn child, update state.
///
/// Uses a CAS-style transition: after spawning the child, the task is only
/// moved to `in_progress` if it is still in a non-terminal state. If another
/// process (e.g. `ralph daemon abort`) already moved the task to a terminal
/// state, the just-spawned child is immediately killed and cleaned up, and
/// the terminal state is preserved.
async fn dispatch_task(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
    task: &DaemonTask,
) -> Result<()> {
    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| RalphError::Orchestration("cannot derive workspace root".into()))?
        .to_path_buf();

    // Create worktree
    let wt_path = {
        let repo_root = config.repo_root.clone();
        let ws_root = workspace_root.clone();
        let tid = task.task_id.clone();
        spawn_blocking_op(move || worktree::create_worktree(&repo_root, &ws_root, &tid)).await?
    };

    // Resolve raw idea. Legacy tasks are hydrated from GitHub if `raw_idea`
    // is missing.
    let raw_idea = match &task.raw_idea {
        Some(idea) => idea.clone(),
        None => {
            let store_clone = store.clone();
            let task_clone = task.clone();
            spawn_blocking_op(move || fetch_and_persist_raw_idea(&store_clone, &task_clone))
                .await?
        }
    };

    // Refine the prompt if enabled, falling back to raw idea on failure.
    let idea = if config.refinement_enabled {
        match refine::refine_prompt(&raw_idea, &config.refinement_backend, &config.global_config)
            .await
        {
            Ok(refined) => refined,
            Err(err) => {
                eprintln!(
                    "warning: refinement failed for task {}, using raw idea: {err}",
                    task.task_id
                );
                raw_idea
            }
        }
    } else {
        raw_idea
    };

    // Post refined-prompt comment (best-effort, never aborts dispatch).
    {
        let owner = task.owner.clone();
        let repo = task.repo.clone();
        let issue_number = task.issue_number;
        let tid = task.task_id.clone();
        let idea_clone = idea.clone();
        if let Err(err) = spawn_blocking_op(move || {
            github::post_idempotent_comment(
                &owner,
                &repo,
                issue_number,
                &tid,
                "refined-prompt",
                &idea_clone,
            )
        })
        .await
        {
            eprintln!(
                "warning: failed to post refined-prompt comment for {}: {err}",
                task.task_id
            );
        }
    }

    // Create log file for child output
    let log_path = task_log_path(store, &task.task_id);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Spawn child process
    let spawned = {
        let ralph_bin = config.ralph_bin.clone();
        let wt = wt_path.clone();
        let idea_clone = idea.clone();
        process::spawn_ralph_auto(&ralph_bin, &wt, &idea_clone, &log_path).await?
    };

    // CAS-style update: only transition to in_progress if task is not
    // already terminal (e.g. concurrently aborted).
    let task_id_owned = task.task_id.clone();
    let pid = spawned.pid;
    let pgid = spawned.pgid;
    let activated = {
        let store = store.clone();
        let task_id_owned = task_id_owned.clone();
        spawn_blocking_op(move || {
            store.with_exclusive_tasks(|tasks| {
                let t = tasks
                    .iter_mut()
                    .find(|t| t.task_id == task_id_owned)
                    .ok_or_else(|| {
                        RalphError::Validation(format!("task not found: {task_id_owned}"))
                    })?;

                if t.state.is_terminal() {
                    // Task was moved to a terminal state (e.g. aborted) between
                    // when we read it and now. Preserve terminal state.
                    eprintln!(
                        "dispatch: task {task_id_owned} already terminal ({}); killing just-spawned child (pid={pid})",
                        t.state
                    );
                    return Ok(false);
                }

                t.state = TaskState::InProgress;
                t.child_pid = Some(pid);
                t.child_pgid = Some(pgid);
                t.updated_at = crate::util::time::now_iso8601();
                Ok(true)
            })
        })
        .await?
    };

    if !activated {
        // Task was already terminal — kill the just-spawned child and
        // clean up the worktree.
        let mut child = spawned.child;
        if let Err(err) = child.kill().await {
            eprintln!(
                "warning: failed to kill child for terminal-race task {} (pid={}): {err}",
                task.task_id, pid
            );
        }
        if let Err(err) = child.wait().await {
            eprintln!(
                "warning: failed to wait child for terminal-race task {} (pid={}): {err}",
                task.task_id, pid
            );
        }
        let repo_root = config.repo_root.clone();
        let tid = task.task_id.clone();
        // Best-effort worktree cleanup
        if let Err(err) = spawn_blocking_op(move || {
            worktree::remove_worktree(&repo_root, &workspace_root, &tid);
            Ok(())
        })
        .await
        {
            eprintln!(
                "warning: failed to cleanup worktree for terminal-race task {}: {err}",
                task.task_id
            );
        }
        return Ok(());
    }

    children.insert(
        task.task_id.clone(),
        ActiveChild {
            pid: spawned.pid,
            pgid: spawned.pgid,
            child: spawned.child,
            log_file: log_path,
        },
    );

    eprintln!("dispatched task {} (pid={})", task.task_id, spawned.pid);

    Ok(())
}

fn fetch_and_persist_raw_idea(store: &TaskStore, task: &DaemonTask) -> Result<String> {
    let raw_idea = match github::fetch_issue_body(&task.owner, &task.repo, task.issue_number) {
        Ok((title, body)) => compose_raw_idea(&title, body.as_deref()),
        Err(err) => {
            eprintln!(
                "warning: failed to fetch issue title/body for task {}: {err}; using metadata fallback",
                task.task_id
            );
            metadata_fallback_raw_idea(task)
        }
    };
    let raw_idea_for_store = raw_idea.clone();
    store.update_task(&task.task_id, |t| {
        t.raw_idea = Some(raw_idea_for_store.clone());
        Ok(())
    })?;
    Ok(raw_idea)
}

fn compose_raw_idea(title: &str, body: Option<&str>) -> String {
    format!("{title}\n\n{}", body.unwrap_or_default())
}

fn metadata_fallback_raw_idea(task: &DaemonTask) -> String {
    let title = format!(
        "Issue #{} ({}/{})",
        task.issue_number, task.owner, task.repo
    );
    let body = "Issue body unavailable from GitHub; using daemon task metadata.";
    compose_raw_idea(&title, Some(body))
}

/// Collect finished children and transition them to terminal states.
///
/// If a task has already been aborted (e.g. via `ralph daemon abort`), the
/// aborted state is preserved — we only perform cleanup, not a state
/// transition.
async fn collect_children(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
) {
    let mut finished = Vec::new();

    for (task_id, active) in children.iter_mut() {
        match active.child.try_wait() {
            Ok(Some(status)) => {
                let terminal_state = if status.success() {
                    TaskState::Completed
                } else {
                    TaskState::Failed
                };
                finished.push((task_id.clone(), terminal_state));
            }
            Ok(None) => {
                // Still running
            }
            Err(err) => {
                eprintln!(
                    "warning: failed to check child for {task_id} (pid={} pgid={}): {err}",
                    active.pid, active.pgid
                );
                finished.push((task_id.clone(), TaskState::Failed));
            }
        }
    }

    for (task_id, terminal_state) in finished {
        if terminal_state == TaskState::Failed {
            if let Some(active) = children.get(&task_id) {
                print_log_tail(&task_id, &active.log_file);
            }
        }
        children.remove(&task_id);
        // complete_task uses an atomic CAS — if the task was already moved
        // to a terminal state (e.g. aborted), it will preserve that state
        // and only perform cleanup.
        complete_task(store, config, &task_id, terminal_state).await;
    }
}

/// Wait until all active children have exited, collecting each into its
/// terminal state. Used by `--single-iteration` mode to guarantee
/// deterministic outcomes.
async fn drain_all_children(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(7200);

    while !children.is_empty() && tokio::time::Instant::now() < deadline {
        collect_children(store, config, children).await;
        if children.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // If any children are still running after the deadline, forcibly
    // terminate them and mark as failed.
    if !children.is_empty() {
        let remaining: Vec<String> = children.keys().cloned().collect();
        for task_id in remaining {
            if let Some(mut active) = children.remove(&task_id) {
                eprintln!(
                    "warning: force-killing child for {task_id} (pid={} pgid={}, drain timeout)",
                    active.pid, active.pgid
                );
                if let Err(err) = active.child.kill().await {
                    eprintln!("warning: failed to kill child for {task_id}: {err}");
                }
                if let Err(err) = active.child.wait().await {
                    eprintln!("warning: failed to wait child for {task_id}: {err}");
                }
            }
            complete_task(store, config, &task_id, TaskState::Failed).await;
        }
    }
}

/// Transition a task to terminal state, handle GitHub completion, cleanup worktree.
///
/// Uses an atomic CAS-style update: the transition only occurs if the task is
/// not already in a terminal state. This prevents the runtime from overwriting
/// an externally-set `aborted` state.
async fn complete_task(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    task_id: &str,
    terminal_state: TaskState,
) {
    // Atomic CAS: only transition if not already terminal.
    let task_id_owned = task_id.to_owned();
    let ts = terminal_state.clone();
    let updated = {
        let store = store.clone();
        let task_id_owned = task_id_owned.clone();
        spawn_blocking_op(move || {
            store.with_exclusive_tasks(|tasks| {
                let task = tasks
                    .iter_mut()
                    .find(|t| t.task_id == task_id_owned)
                    .ok_or_else(|| {
                        RalphError::Validation(format!("task not found: {task_id_owned}"))
                    })?;

                if task.state.is_terminal() {
                    // Already terminal (e.g. aborted externally) — preserve that
                    // state, just clear PID/PGID.
                    task.child_pid = None;
                    task.child_pgid = None;
                    task.updated_at = now_iso8601();
                    eprintln!(
                        "task {task_id_owned} child exited; already in terminal state: {}",
                        task.state
                    );
                    return Ok(None);
                }

                task.state = ts;
                task.child_pid = None;
                task.child_pgid = None;
                task.updated_at = now_iso8601();
                Ok(Some(task.clone()))
            })
        })
        .await
    };

    let task = match updated {
        Ok(Some(t)) => t,
        Ok(None) => {
            // Was already terminal — still cleanup worktree
            cleanup_worktree(store, config, task_id).await;
            return;
        }
        Err(err) => {
            eprintln!("warning: failed to update task {task_id} to terminal state: {err}");
            return;
        }
    };

    // GitHub completion flow
    let terminal_label = match terminal_state {
        TaskState::Completed => "ralph:completed",
        TaskState::Failed => "ralph:failed",
        TaskState::Aborted => "ralph:aborted",
        _ => return,
    };

    // Post completion comment (best-effort, idempotent)
    {
        let owner = task.owner.clone();
        let repo = task.repo.clone();
        let issue_number = task.issue_number;
        let tid = task_id.to_owned();
        let phase = terminal_state.as_str().to_owned();
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
    if terminal_state == TaskState::Completed {
        // The orchestrator may have switched the worktree to a different
        // branch (e.g. ralph/{project_id}) from the one the daemon created
        // (ralph/daemon/{task_id}). Resolve the actual branch so push and
        // PR creation target the right ref.
        let mut task = task.clone();
        let workspace_root = store
            .path()
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let wt_path = worktree::task_worktree_path(&workspace_root, task_id);
        if wt_path.exists() {
            let wt = wt_path.clone();
            if let Ok(actual_branch) =
                spawn_blocking_op(move || github::current_branch(&wt)).await
            {
                if task.branch.as_deref() != Some(&actual_branch) {
                    eprintln!(
                        "task {task_id}: worktree branch changed from {:?} to {actual_branch}",
                        task.branch
                    );
                    task.branch = Some(actual_branch.clone());
                    let store_clone = store.clone();
                    let tid = task_id.to_owned();
                    let _ = spawn_blocking_op(move || {
                        store_clone.update_task(&tid, |t| {
                            t.branch = Some(actual_branch.clone());
                            Ok(())
                        })
                    })
                    .await;
                }
            }
        }
        if let Err(err) = handle_pr_flow(store, config, &task).await {
            eprintln!(
                "warning: PR flow failed for {}: {err}",
                task.task_id
            );
        }
    }

    // Update labels (best-effort)
    {
        let owner = task.owner.clone();
        let repo = task.repo.clone();
        let issue_number = task.issue_number;
        let label = terminal_label.to_owned();
        if let Err(err) = spawn_blocking_op(move || {
            github::update_terminal_labels_best_effort(&owner, &repo, issue_number, &label);
            Ok(())
        })
        .await
        {
            eprintln!("warning: failed to update labels for {task_id}: {err}");
        }
    }

    // Cleanup worktree (best-effort)
    cleanup_worktree(store, config, task_id).await;

    let log_path = task_log_path(store, task_id);
    eprintln!(
        "task {task_id} completed with state: {terminal_state} (log: {})",
        log_path.display()
    );
}

/// Remove the worktree for a task (best-effort).
async fn cleanup_worktree(store: &TaskStore, config: &DaemonRuntimeConfig, task_id: &str) {
    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."))
        .to_path_buf();

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
///
/// Deterministic flow:
/// 1. Check for diff; if none, post no-diff comment and return.
/// 2. Push branch to remote.
/// 3. Gather context (diff stat, issue body) — failures degrade gracefully.
/// 4. Build title via `build_pr_title` and body via `build_pr_body`.
/// 5. Check for existing PR via `find_existing_pr`.
/// 6. If existing PR: attempt `edit_pr` only; on failure, return error, do NOT create.
/// 7. If no existing PR: create via `create_pr_with_body_file`, persist URL.
async fn handle_pr_flow(store: &TaskStore, _config: &DaemonRuntimeConfig, task: &DaemonTask) -> Result<()> {
    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let branch = match &task.branch {
        Some(b) => b.clone(),
        None => return Ok(()),
    };

    let wt_path = worktree::task_worktree_path(&workspace_root, &task.task_id);

    // Step 1: Check if there's a diff
    let has_changes = {
        let wt = wt_path.clone();
        match spawn_blocking_op(move || github::has_diff(&wt)).await {
            Ok(v) => v,
            Err(err) => {
                eprintln!(
                    "warning: failed to check diff for {}: {err}",
                    task.task_id
                );
                return Ok(());
            }
        }
    };

    if !has_changes {
        // No diff: post idempotent "no changes" comment (best-effort)
        let owner = task.owner.clone();
        let repo = task.repo.clone();
        let issue_number = task.issue_number;
        let tid = task.task_id.clone();
        let body = format!(
            "Task `{}` completed with no code changes. No PR created.",
            task.task_id
        );
        if let Err(err) = spawn_blocking_op(move || {
            github::post_idempotent_comment(
                &owner,
                &repo,
                issue_number,
                &tid,
                "no-diff",
                &body,
            )
        })
        .await
        {
            eprintln!(
                "warning: failed to post no-diff comment for {}: {err}",
                task.task_id
            );
        }
        return Ok(());
    }

    // Step 2: Push branch to remote before PR creation/edit
    {
        let wt = wt_path.clone();
        let br = branch.clone();
        match spawn_blocking_op(move || github::push_branch(&wt, &br)).await {
            Ok(()) => {}
            Err(err) => {
                eprintln!(
                    "warning: failed to push branch {} for {}: {err}",
                    branch, task.task_id
                );
                return Ok(());
            }
        }
    }

    // Step 3: Gather context for PR body
    // Diff stat — failure produces fallback text, does not abort.
    let diff_stat: Option<String> = {
        let wt = wt_path.clone();
        match spawn_blocking_op(move || github::diff_stat(&wt)).await {
            Ok(stat) => stat,
            Err(err) => {
                eprintln!(
                    "warning: diff stat failed for {}: {err}; using fallback",
                    task.task_id
                );
                None
            }
        }
    };

    // Issue body context from raw_idea
    let issue_body = extract_issue_body(task.raw_idea.as_deref());

    // Step 4: Build title and body via pure helpers
    let title = build_pr_title(&format!("ralph: {}", task.task_id));
    let pr_body = build_pr_body(
        &branch,
        diff_stat.as_deref(),
        issue_body.as_deref(),
        &task.task_id,
        task.issue_number,
    );

    // Write body to a temp file for --body-file usage
    let body_file = match write_body_file(&pr_body) {
        Ok(f) => f,
        Err(err) => {
            eprintln!(
                "warning: failed to write PR body file for {}: {err}",
                task.task_id
            );
            return Ok(());
        }
    };

    // Step 5: Check for existing PR
    let existing_pr_url = {
        let owner = task.owner.clone();
        let repo = task.repo.clone();
        let br = branch.clone();
        match spawn_blocking_op(move || github::find_existing_pr(&owner, &repo, &br)).await {
            Ok(url) => url,
            Err(err) => {
                eprintln!("warning: failed to check for existing PR: {err}");
                None
            }
        }
    };

    match existing_pr_url {
        Some(url) => {
            // Step 6: Existing PR — edit only, never fall through to create
            eprintln!("editing existing PR for {}: {url}", task.task_id);
            let url_for_edit = url.clone();
            let title_clone = title.clone();
            let body_path = body_file.path().to_path_buf();
            match spawn_blocking_op(move || {
                github::edit_pr(&url_for_edit, &title_clone, &body_path)
            })
            .await
            {
                Ok(()) => {
                    // Persist the PR URL
                    let store = store.clone();
                    let tid = task.task_id.clone();
                    let url_clone = url.clone();
                    if let Err(err) = spawn_blocking_op(move || {
                        store.update_task(&tid, |t| {
                            t.pr_url = Some(url_clone.clone());
                            Ok(())
                        })
                    })
                    .await
                    {
                        eprintln!(
                            "warning: failed to persist PR URL for {}: {err}",
                            task.task_id
                        );
                    }
                }
                Err(err) => {
                    // Edit failed — return error, do NOT fall through to create
                    return Err(RalphError::Orchestration(format!(
                        "failed to edit PR for {}: {err}",
                        task.task_id
                    )));
                }
            }
        }
        None => {
            // Step 7: No existing PR — create new
            let owner = task.owner.clone();
            let repo = task.repo.clone();
            let br = branch.clone();
            let title_clone = title.clone();
            let body_path = body_file.path().to_path_buf();
            match spawn_blocking_op(move || {
                github::create_pr_with_body_file(&owner, &repo, &br, &title_clone, &body_path)
            })
            .await
            {
                Ok(url) => {
                    eprintln!("created PR for {}: {url}", task.task_id);
                    let store = store.clone();
                    let tid = task.task_id.clone();
                    let url_clone = url.clone();
                    if let Err(err) = spawn_blocking_op(move || {
                        store.update_task(&tid, |t| {
                            t.pr_url = Some(url_clone.clone());
                            Ok(())
                        })
                    })
                    .await
                    {
                        eprintln!(
                            "warning: failed to persist PR URL for {}: {err}",
                            task.task_id
                        );
                    }
                }
                Err(err) => {
                    eprintln!(
                        "warning: failed to create PR for {}; continuing to terminal state: {err}",
                        task.task_id
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
        build_pr_body, build_pr_title, extract_issue_body, extract_project_ref, write_body_file,
    };

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
        assert!(
            body.contains(
                "Project Ref: unavailable (could not extract from branch `feature/no-project-ref`)."
            )
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

    /// Verify that when diff stat is None (failure/unavailable), build_pr_body
    /// still produces a valid body with fallback text. This exercises the
    /// "diff stat failure → fallback" path at the helper level.
    #[test]
    fn runtime_pr_diff_stat_failure_fallback() {
        // Simulate: diff stat generation failed (None), but we have issue context
        let body = build_pr_body(
            "ralph/my-project",
            None, // diff stat unavailable
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

    /// Verify that write_body_file produces a temp file whose content matches
    /// the provided body string. This is a building block of the --body-file
    /// integration.
    #[test]
    fn write_body_file_creates_readable_temp() {
        let content = "Test PR body\n\nWith multiple lines";
        let tmp = write_body_file(content).expect("write_body_file should succeed");
        let read_back = std::fs::read_to_string(tmp.path()).expect("read temp file");
        assert_eq!(read_back, content);
    }

    /// Verify that build_pr_body with diff stat present respects the 100-line cap.
    #[test]
    fn build_pr_body_diff_stat_cap() {
        let lines: Vec<String> = (1..=150).map(|i| format!("file{i}.rs | 1 +")).collect();
        let stat = lines.join("\n");
        let body = build_pr_body("ralph/proj", Some(&stat), None, "task-2", 2);
        // Should contain first 100 lines and truncation marker
        assert!(body.contains("file1.rs | 1 +"));
        assert!(body.contains("file100.rs | 1 +"));
        assert!(body.contains("... (truncated)"));
        // Should NOT contain line 101+
        assert!(!body.contains("file101.rs"));
    }

    /// Verify that build_pr_body caps issue context to 4000 chars.
    #[test]
    fn build_pr_body_context_cap() {
        // Use a character that doesn't appear in the template to avoid
        // counting template content.
        let long_context = "\u{2603}".repeat(5000); // snowman
        let body = build_pr_body("ralph/proj", None, Some(&long_context), "task-3", 3);
        let snowman_count = body.matches('\u{2603}').count();
        assert_eq!(
            snowman_count, 4000,
            "issue context should be capped at 4000 chars, got {snowman_count}"
        );
    }
}
