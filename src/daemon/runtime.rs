use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::GlobalConfig;
use crate::daemon::bootstrap;
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
    /// When true, emit runtime diagnostics to stderr.
    pub verbose: bool,
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
pub async fn run(store: &TaskStore, config: &DaemonRuntimeConfig) -> Result<()> {
    // Phase 1: Startup reconciliation
    {
        let store = store.clone();
        let verbose = config.verbose;
        spawn_blocking_op(move || reconcile_tasks(&store, verbose)).await?;
    }
    {
        let store = store.clone();
        let config = config.clone();
        spawn_blocking_op(move || reconcile_worktrees(&store, &config)).await?;
    }

    // Phase 2: Main loop
    let mut children: HashMap<String, ActiveChild> = HashMap::new();
    let mut iteration: u64 = 0;

    // Re-adopt pending tasks from reconciliation
    adopt_pending_tasks(store, config, &mut children).await?;

    loop {
        iteration = iteration.saturating_add(1);

        // Collect finished children
        collect_children(store, config, &mut children).await;

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
fn reconcile_tasks(store: &TaskStore, verbose: bool) -> Result<()> {
    store.with_exclusive_tasks(|tasks| {
        let mut reconciled = 0u32;
        for task in tasks.iter_mut() {
            if task.state == TaskState::InProgress {
                task.state = TaskState::Pending;
                task.child_pid = None;
                task.child_pgid = None;
                task.updated_at = now_iso8601();
                reconciled += 1;
                if verbose {
                    eprintln!(
                        "verbose: reconcile reset task_id={} in_progress->pending",
                        task.task_id
                    );
                }
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
    worktree::reconcile_worktrees(&config.repo_root, workspace_root, &active_ids);
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
        if config.verbose {
            eprintln!(
                "verbose: adopt pending task_id={} action=re-adopt",
                task.task_id
            );
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
            complete_task(store, config, &task.task_id, TaskState::Failed).await;
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
            refined_title: None,
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
            complete_task(store, config, &task_id, TaskState::Failed).await;
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
    bootstrap::ensure_repo_ready(&config.repo_root).await?;

    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| RalphError::Orchestration("cannot derive workspace root".into()))?
        .to_path_buf();

    // Create worktree (reuses existing branch if present)
    let wt_path = {
        let repo_root = config.repo_root.clone();
        let ws_root = workspace_root.clone();
        let tid = task.task_id.clone();
        spawn_blocking_op(move || worktree::create_worktree(&repo_root, &ws_root, &tid)).await?
    };

    // Clean worktree of any dirty files from previous runs or backend
    // side-effects. This prevents the orchestrator from aborting due to
    // uncommitted changes outside `.ralph/`.
    {
        let wt = wt_path.clone();
        spawn_blocking_op(move || worktree::clean_worktree(&wt)).await?;
    }

    // Resolve raw idea. Legacy tasks are hydrated from GitHub if `raw_idea`
    // is missing.
    let raw_idea = match &task.raw_idea {
        Some(idea) => idea.clone(),
        None => {
            let store_clone = store.clone();
            let task_clone = task.clone();
            spawn_blocking_op(move || fetch_and_persist_raw_idea(&store_clone, &task_clone)).await?
        }
    };

    // Refine the prompt if enabled, falling back to raw idea on failure.
    let (idea, refined_title) = if config.refinement_enabled {
        match refine::refine_prompt(&raw_idea, &config.refinement_backend, &config.global_config)
            .await
        {
            Ok(refined) => (refined.body, refined.title),
            Err(err) => {
                eprintln!(
                    "warning: refinement failed for task {}, using raw idea: {err}",
                    task.task_id
                );
                (raw_idea, None)
            }
        }
    } else {
        (raw_idea, None)
    };

    // Persist refined_title best-effort (do not abort dispatch on failure).
    // Always write (even None) to clear any stale title from a previous attempt.
    {
        let store_clone = store.clone();
        let tid = task.task_id.clone();
        let title_clone = refined_title.clone();
        if let Err(err) = spawn_blocking_op(move || {
            store_clone.update_task(&tid, |t| {
                t.refined_title = title_clone.clone();
                Ok(())
            })
        })
        .await
        {
            eprintln!(
                "warning: failed to persist refined_title for {}: {err}",
                task.task_id
            );
        }
    }

    // Post refined-prompt comment (best-effort, never aborts dispatch).
    {
        let owner = task.owner.clone();
        let repo = task.repo.clone();
        let issue_number = task.issue_number;
        let tid = task.task_id.clone();
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
    let verbose = config.verbose;
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
                    if verbose {
                        eprintln!(
                            "verbose: dispatch abort-race task_id={task_id_owned} terminal_state={} spawned_pid={pid}",
                            t.state
                        );
                    }
                    return Ok(false);
                }

                t.state = TaskState::InProgress;
                t.child_pid = Some(pid);
                t.child_pgid = Some(pgid);
                t.updated_at = crate::util::time::now_iso8601();
                if verbose {
                    eprintln!(
                        "verbose: dispatch transition task_id={task_id_owned} pending->in_progress pid={pid}"
                    );
                }
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

/// Extract the original title from a raw idea string.
///
/// Takes the segment before the first `\n\n`, trims it, and returns `None` if
/// empty; otherwise `Some(title)`.
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
    let mut still_running = 0u32;

    for (task_id, active) in children.iter_mut() {
        match active.child.try_wait() {
            Ok(Some(status)) => {
                if config.verbose {
                    let exit_code = status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "signal".to_owned());
                    eprintln!(
                        "verbose: child terminal task_id={task_id} pid={} exit_status={} exit_code={exit_code}",
                        active.pid, status
                    );
                }
                let terminal_state = if status.success() {
                    TaskState::Completed
                } else {
                    TaskState::Failed
                };
                finished.push((task_id.clone(), terminal_state));
            }
            Ok(None) => {
                still_running = still_running.saturating_add(1);
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

    if config.verbose && still_running > 0 {
        eprintln!("verbose: child collection still_running={still_running}");
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
    let verbose = config.verbose;
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
                    if verbose {
                        eprintln!(
                            "verbose: complete preserve-terminal task_id={task_id_owned} state={}",
                            task.state
                        );
                    }
                    return Ok(None);
                }

                let prior_state = task.state.clone();
                task.state = ts.clone();
                task.child_pid = None;
                task.child_pgid = None;
                task.updated_at = now_iso8601();
                if verbose {
                    eprintln!(
                        "verbose: complete transition task_id={task_id_owned} {prior_state}->{}",
                        task.state
                    );
                }
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
            if let Ok(actual_branch) = spawn_blocking_op(move || github::current_branch(&wt)).await
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
        handle_pr_flow(store, config, &task).await;
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

/// Handle the PR creation/reuse flow for a completed task.
async fn handle_pr_flow(store: &TaskStore, _config: &DaemonRuntimeConfig, task: &DaemonTask) {
    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let branch = match &task.branch {
        Some(b) => b.clone(),
        None => return,
    };

    let wt_path = worktree::task_worktree_path(&workspace_root, &task.task_id);

    // Check if there's a diff
    let has_changes = {
        let wt = wt_path.clone();
        match spawn_blocking_op(move || github::has_diff(&wt)).await {
            Ok(v) => v,
            Err(err) => {
                eprintln!("warning: failed to check diff for {}: {err}", task.task_id);
                return;
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
            github::post_idempotent_comment(&owner, &repo, issue_number, &tid, "no-diff", &body)
        })
        .await
        {
            eprintln!(
                "warning: failed to post no-diff comment for {}: {err}",
                task.task_id
            );
        }
        return;
    }

    // Skip push/PR flow when no origin remote exists in the task worktree.
    {
        let wt = wt_path.clone();
        let has_origin = match spawn_blocking_op(move || github::has_origin_remote(&wt)).await {
            Ok(v) => v,
            Err(err) => {
                eprintln!(
                    "warning: failed to check origin remote for {}; skipping push/PR: {err}",
                    task.task_id
                );
                return;
            }
        };
        if !has_origin {
            eprintln!(
                "warning: origin remote missing for {}; skipping push/PR flow",
                task.task_id
            );
            return;
        }
    }

    // Push branch to remote before PR creation
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
                return;
            }
        }
    }

    // Check for existing PR
    {
        let owner = task.owner.clone();
        let repo = task.repo.clone();
        let br = branch.clone();
        match spawn_blocking_op(move || github::find_existing_pr(&owner, &repo, &br)).await {
            Ok(Some(url)) => {
                eprintln!("reusing existing PR for {}: {url}", task.task_id);
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
                return;
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("warning: failed to check for existing PR: {err}");
            }
        }
    }

    // Create new PR — title precedence: refined_title -> original title -> fallback
    let title = task
        .refined_title
        .clone()
        .or_else(|| {
            extract_original_title(task.raw_idea.as_deref().unwrap_or_default())
        })
        .unwrap_or_else(|| format!("ralph: {}", task.task_id));
    let body = format!(
        "Automated PR for task `{}`.\n\nCloses #{}",
        task.task_id, task.issue_number
    );
    {
        let owner = task.owner.clone();
        let repo = task.repo.clone();
        let br = branch.clone();
        let title = title.clone();
        let body = body.clone();
        match spawn_blocking_op(move || github::create_pr(&owner, &repo, &br, &title, &body)).await
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

#[cfg(test)]
mod tests {
    use super::extract_original_title;

    #[test]
    fn extract_original_title_with_body() {
        assert_eq!(
            extract_original_title("Fix bug\n\nDetails"),
            Some("Fix bug".to_owned())
        );
    }

    #[test]
    fn extract_original_title_no_body() {
        assert_eq!(
            extract_original_title("Fix bug"),
            Some("Fix bug".to_owned())
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
}
