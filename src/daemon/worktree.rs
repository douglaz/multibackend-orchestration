use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::RalphError;
use crate::Result;

/// Return the base directory for daemon worktrees.
pub fn worktrees_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("daemon").join("worktrees")
}

/// Return the worktree path for a specific task.
pub fn task_worktree_path(workspace_root: &Path, task_id: &str) -> PathBuf {
    worktrees_dir(workspace_root).join(task_id)
}

/// Return the daemon branch name for a task.
pub fn daemon_branch_name(task_id: &str) -> String {
    format!("ralph/daemon/{task_id}")
}

/// Create a git worktree for the given task.
///
/// Creates a new branch `ralph/daemon/<task_id>` in a worktree at
/// `.ralph/daemon/worktrees/<task_id>/`.
pub fn create_worktree(repo_root: &Path, workspace_root: &Path, task_id: &str) -> Result<PathBuf> {
    let wt_path = task_worktree_path(workspace_root, task_id);

    if wt_path.exists() {
        return Ok(wt_path);
    }

    if let Some(parent) = wt_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let branch_name = daemon_branch_name(task_id);
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            &branch_name,
            &wt_path.to_string_lossy(),
            "HEAD",
        ])
        .current_dir(repo_root)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to create worktree for {task_id}: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "git worktree add failed for {task_id}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(wt_path)
}

/// Remove a task worktree and its local branch.
///
/// Cleanup order:
/// 1) If worktree path exists: `git worktree remove --force <path>`
/// 2) Always: `git worktree prune`
/// 3) Always: `git show-ref --verify refs/heads/<branch>`
/// 4) If branch exists: `git branch -D <branch>`
pub fn remove_worktree(
    repo_root: &Path,
    workspace_root: &Path,
    task_id: &str,
    branch: &str,
) -> Result<()> {
    let wt_path = task_worktree_path(workspace_root, task_id);
    let wt_path_string = wt_path.to_string_lossy().into_owned();
    if wt_path.exists() {
        let output = Command::new("git")
            .args(["worktree", "remove", "--force", &wt_path_string])
            .current_dir(repo_root)
            .output()
            .map_err(|err| {
                RalphError::Orchestration(format!(
                    "failed to run `git worktree remove --force {}` for task {task_id}: {err}",
                    wt_path.display()
                ))
            })?;

        if !output.status.success() {
            return Err(RalphError::Orchestration(format!(
                "`git worktree remove --force {}` failed for task {task_id}: {}",
                wt_path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }

    let prune_output = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo_root)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run `git worktree prune` while cleaning task {task_id}: {err}"
            ))
        })?;
    if !prune_output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "`git worktree prune` failed while cleaning task {task_id}: {}",
            String::from_utf8_lossy(&prune_output.stderr).trim()
        )));
    }

    let branch_ref = format!("refs/heads/{branch}");
    let show_ref_output = Command::new("git")
        .args(["show-ref", "--verify", &branch_ref])
        .current_dir(repo_root)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run `git show-ref --verify {branch_ref}` while cleaning task {task_id}: {err}"
            ))
        })?;

    // `show-ref --verify` exits non-zero when branch doesn't exist.
    if !show_ref_output.status.success() {
        return Ok(());
    }

    let delete_output = Command::new("git")
        .args(["branch", "-D", branch])
        .current_dir(repo_root)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run `git branch -D {branch}` while cleaning task {task_id}: {err}"
            ))
        })?;
    if !delete_output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "`git branch -D {branch}` failed while cleaning task {task_id}: {}",
            String::from_utf8_lossy(&delete_output.stderr).trim()
        )));
    }

    Ok(())
}

/// Reconcile orphaned and stale worktrees at startup.
///
/// Scans `.ralph/daemon/worktrees/` and removes directories that are either:
/// - Not associated with any known task ID (orphaned), or
/// - Associated with a task in a terminal state (stale).
///
/// `active_task_ids` should contain only IDs of non-terminal tasks that will
/// be re-adopted by the daemon.
///
/// `task_branches` maps task IDs to their branch names.
pub fn reconcile_worktrees(
    repo_root: &Path,
    workspace_root: &Path,
    active_task_ids: &[String],
    task_branches: &std::collections::HashMap<String, String>,
) -> Result<()> {
    let wt_dir = worktrees_dir(workspace_root);
    let mut on_disk_task_ids = std::collections::HashSet::new();

    let entries = match fs::read_dir(&wt_dir) {
        Ok(entries) => Some(entries),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => {
            return Err(RalphError::Orchestration(format!(
                "failed to read daemon worktrees directory {}: {err}",
                wt_dir.display()
            )));
        }
    };

    if let Some(entries) = entries {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            on_disk_task_ids.insert(name.clone());

            if !active_task_ids.contains(&name) {
                let branch = task_branches
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| daemon_branch_name(&name));
                eprintln!("reconcile: removing stale/orphaned worktree {name}");
                remove_worktree(repo_root, workspace_root, &name, &branch)?;
            }
        }
    }

    // If an active task has no worktree directory, clean any stale local
    // daemon branch before redispatch.
    for task_id in active_task_ids {
        if on_disk_task_ids.contains(task_id) {
            continue;
        }

        let branch = task_branches
            .get(task_id)
            .cloned()
            .unwrap_or_else(|| daemon_branch_name(task_id));
        remove_worktree(repo_root, workspace_root, task_id, &branch)?;
    }

    Ok(())
}
