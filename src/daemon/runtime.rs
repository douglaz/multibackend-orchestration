use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread;
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
    child: std::process::Child,
}

/// Run the daemon loop: reconcile, then poll/claim/dispatch/collect.
pub fn run(store: &TaskStore, config: &DaemonRuntimeConfig) -> Result<()> {
    // Phase 1: Startup reconciliation
    reconcile_tasks(store)?;
    reconcile_worktrees(store, config)?;

    // Phase 2: Main loop
    let mut children: HashMap<String, ActiveChild> = HashMap::new();

    // Re-adopt pending tasks from reconciliation
    adopt_pending_tasks(store, config, &mut children)?;

    loop {
        // Collect finished children
        collect_children(store, config, &mut children);

        // Poll for new issues
        let active_count = children.len() as u32;
        let slots = config.max_concurrent.saturating_sub(active_count);

        if slots > 0 {
            if let Err(err) = poll_and_claim(store, config, &mut children, slots) {
                eprintln!("warning: poll/claim cycle failed: {err}");
            }
        }

        // Collect again after spawning
        collect_children(store, config, &mut children);

        if config.single_iteration {
            // In single-iteration mode, wait for all spawned children to
            // reach a terminal state so the outcome is deterministic.
            drain_all_children(store, config, &mut children);
            break;
        }

        thread::sleep(Duration::from_secs(config.poll_seconds));
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
    worktree::reconcile_worktrees(
        &config.repo_root,
        &store
            .path()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf(),
        &active_ids,
    );
    Ok(())
}

/// Re-adopt pending tasks by spawning children for them.
fn adopt_pending_tasks(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
) -> Result<()> {
    let tasks = store.load()?;
    let pending: Vec<DaemonTask> = tasks
        .iter()
        .cloned()
        .filter(|t| t.state == TaskState::Pending)
        .collect();

    for mut task in pending {
        if children.len() as u32 >= config.max_concurrent {
            break;
        }
        if task.raw_idea.is_none() {
            match fetch_and_persist_raw_idea(store, &task) {
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
        if let Err(err) = dispatch_task(store, config, children, &task) {
            eprintln!("warning: failed to re-adopt task {}: {err}", task.task_id);
        }
    }

    Ok(())
}

/// Poll for new issues, filter, claim, and dispatch.
fn poll_and_claim(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
    slots: u32,
) -> Result<()> {
    let (issues, overflow) = github::poll_issues(&config.owner, &config.repo, &config.labels)?;

    if overflow {
        eprintln!("warning: gh issue list returned exactly 100 issues; results may be truncated");
    }

    let claimable = github::filter_claimable(issues);

    let existing_tasks = store.load()?;
    let existing_ids: Vec<String> = existing_tasks.iter().map(|t| t.task_id.clone()).collect();

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
        if let Err(err) = github::claim_issue(&config.owner, &config.repo, issue.number) {
            eprintln!("warning: failed to claim issue #{}: {err}", issue.number);
            continue;
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

        store.with_exclusive_tasks(|tasks| {
            // Double-check no duplicate
            if !tasks.iter().any(|t| t.task_id == task_id) {
                tasks.push(task.clone());
            }
            Ok(())
        })?;

        // Dispatch
        if let Err(err) = dispatch_task(store, config, children, &task) {
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
fn dispatch_task(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
    task: &DaemonTask,
) -> Result<()> {
    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| RalphError::Orchestration("cannot derive workspace root".into()))?;

    // Create worktree
    let wt_path = worktree::create_worktree(&config.repo_root, workspace_root, &task.task_id)?;

    // Resolve raw idea. Legacy tasks are hydrated from GitHub if `raw_idea`
    // is missing.
    let raw_idea = match &task.raw_idea {
        Some(idea) => idea.clone(),
        None => fetch_and_persist_raw_idea(store, task)?,
    };

    // Refine the prompt if enabled, falling back to raw idea on failure.
    let idea = if config.refinement_enabled {
        match refine::refine_prompt(&raw_idea, &config.refinement_backend, &config.global_config) {
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
    if let Err(err) = github::post_idempotent_comment(
        &task.owner,
        &task.repo,
        task.issue_number,
        &task.task_id,
        "refined-prompt",
        &idea,
    ) {
        eprintln!(
            "warning: failed to post refined-prompt comment for {}: {err}",
            task.task_id
        );
    }

    // Spawn child process
    let spawned = process::spawn_ralph_auto(&config.ralph_bin, &wt_path, &idea)?;

    // CAS-style update: only transition to in_progress if task is not
    // already terminal (e.g. concurrently aborted).
    let task_id_owned = task.task_id.clone();
    let pid = spawned.pid;
    let pgid = spawned.pgid;
    let activated = store.with_exclusive_tasks(|tasks| {
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
    })?;

    if !activated {
        // Task was already terminal — kill the just-spawned child and
        // clean up the worktree.
        let mut child = spawned.child;
        let _ = child.kill();
        let _ = child.wait();
        worktree::remove_worktree(&config.repo_root, workspace_root, &task.task_id);
        return Ok(());
    }

    children.insert(
        task.task_id.clone(),
        ActiveChild {
            child: spawned.child,
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
fn collect_children(
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
                eprintln!("warning: failed to check child for {task_id}: {err}");
                finished.push((task_id.clone(), TaskState::Failed));
            }
        }
    }

    for (task_id, terminal_state) in finished {
        children.remove(&task_id);
        // complete_task uses an atomic CAS — if the task was already moved
        // to a terminal state (e.g. aborted), it will preserve that state
        // and only perform cleanup.
        complete_task(store, config, &task_id, terminal_state);
    }
}

/// Block until all active children have exited, collecting each into its
/// terminal state. Used by `--single-iteration` mode to guarantee
/// deterministic outcomes.
fn drain_all_children(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    children: &mut HashMap<String, ActiveChild>,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(300);

    while !children.is_empty() && std::time::Instant::now() < deadline {
        collect_children(store, config, children);
        if children.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    // If any children are still running after the deadline, forcibly
    // terminate them and mark as failed.
    if !children.is_empty() {
        let remaining: Vec<String> = children.keys().cloned().collect();
        for task_id in remaining {
            if let Some(mut active) = children.remove(&task_id) {
                eprintln!("warning: force-killing child for {task_id} (drain timeout)");
                let _ = active.child.kill();
                let _ = active.child.wait();
            }
            complete_task(store, config, &task_id, TaskState::Failed);
        }
    }
}

/// Transition a task to terminal state, handle GitHub completion, cleanup worktree.
///
/// Uses an atomic CAS-style update: the transition only occurs if the task is
/// not already in a terminal state. This prevents the runtime from overwriting
/// an externally-set `aborted` state.
fn complete_task(
    store: &TaskStore,
    config: &DaemonRuntimeConfig,
    task_id: &str,
    terminal_state: TaskState,
) {
    // Atomic CAS: only transition if not already terminal.
    let task_id_owned = task_id.to_owned();
    let ts = terminal_state.clone();
    let updated = store.with_exclusive_tasks(|tasks| {
        let task = tasks
            .iter_mut()
            .find(|t| t.task_id == task_id_owned)
            .ok_or_else(|| RalphError::Validation(format!("task not found: {task_id_owned}")))?;

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
    });

    let task = match updated {
        Ok(Some(t)) => t,
        Ok(None) => {
            // Was already terminal — still cleanup worktree
            cleanup_worktree(store, config, task_id);
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

    // Post completion comment (idempotent)
    let phase = terminal_state.as_str();
    let comment_body = format!("Task `{task_id}` finished with status: **{phase}**.");
    if let Err(err) = github::post_idempotent_comment(
        &task.owner,
        &task.repo,
        task.issue_number,
        task_id,
        phase,
        &comment_body,
    ) {
        eprintln!("warning: failed to post completion comment for {task_id}: {err}");
    }

    // PR flow (only on success)
    if terminal_state == TaskState::Completed {
        handle_pr_flow(store, config, &task);
    }

    // Update labels
    github::update_terminal_labels_best_effort(
        &task.owner,
        &task.repo,
        task.issue_number,
        terminal_label,
    );

    // Cleanup worktree
    cleanup_worktree(store, config, task_id);

    eprintln!("task {task_id} completed with state: {terminal_state}");
}

/// Remove the worktree for a task (best-effort).
fn cleanup_worktree(store: &TaskStore, config: &DaemonRuntimeConfig, task_id: &str) {
    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."));

    worktree::remove_worktree(&config.repo_root, workspace_root, task_id);
}

/// Handle the PR creation/reuse flow for a completed task.
fn handle_pr_flow(store: &TaskStore, _config: &DaemonRuntimeConfig, task: &DaemonTask) {
    let workspace_root = store
        .path()
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."));

    let branch = match &task.branch {
        Some(b) => b.clone(),
        None => return,
    };

    let wt_path = worktree::task_worktree_path(workspace_root, &task.task_id);

    // Check if there's a diff
    let has_changes = match github::has_diff(&wt_path) {
        Ok(v) => v,
        Err(_) => {
            // Worktree may already be gone; no diff means no PR
            false
        }
    };

    if !has_changes {
        // No diff: post idempotent "no changes" comment
        let _ = github::post_idempotent_comment(
            &task.owner,
            &task.repo,
            task.issue_number,
            &task.task_id,
            "no-diff",
            &format!(
                "Task `{}` completed with no code changes. No PR created.",
                task.task_id
            ),
        );
        return;
    }

    // Push branch to remote before PR creation
    if let Err(err) = github::push_branch(&wt_path, &branch) {
        eprintln!(
            "warning: failed to push branch {} for {}: {err}",
            branch, task.task_id
        );
        return;
    }

    // Check for existing PR
    match github::find_existing_pr(&task.owner, &task.repo, &branch) {
        Ok(Some(url)) => {
            eprintln!("reusing existing PR for {}: {url}", task.task_id);
            let url_clone = url.clone();
            let _ = store.update_task(&task.task_id, |t| {
                t.pr_url = Some(url_clone.clone());
                Ok(())
            });
            return;
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!("warning: failed to check for existing PR: {err}");
        }
    }

    // Create new PR
    let title = format!("ralph: {}", task.task_id);
    let body = format!(
        "Automated PR for task `{}`.\n\nCloses #{}",
        task.task_id, task.issue_number
    );
    match github::create_pr(&task.owner, &task.repo, &branch, &title, &body) {
        Ok(url) => {
            eprintln!("created PR for {}: {url}", task.task_id);
            let url_clone = url.clone();
            let _ = store.update_task(&task.task_id, |t| {
                t.pr_url = Some(url_clone.clone());
                Ok(())
            });
        }
        Err(err) => {
            eprintln!(
                "warning: failed to create PR for {}; continuing to terminal state: {err}",
                task.task_id
            );
        }
    }
}
