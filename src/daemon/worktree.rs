use std::fs;
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

/// Create a git worktree for the given task.
///
/// Creates a new branch `ralph/daemon/<task_id>` in a worktree at
/// `.ralph/daemon/worktrees/<task_id>/`. If the branch already exists
/// (e.g. from a previous failed run), reuses it instead of passing `-b`.
pub fn create_worktree(repo_root: &Path, workspace_root: &Path, task_id: &str) -> Result<PathBuf> {
    let wt_path = task_worktree_path(workspace_root, task_id);

    if wt_path.exists() {
        return Ok(wt_path);
    }

    if let Some(parent) = wt_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let branch_name = format!("ralph/daemon/{task_id}");

    // Check if the branch already exists (e.g. from a previous failed run
    // where the worktree was cleaned up but the branch was not).
    let branch_exists = Command::new("git")
        .args(["rev-parse", "--verify", &format!("refs/heads/{branch_name}")])
        .current_dir(repo_root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let output = if branch_exists {
        // Reuse existing branch
        Command::new("git")
            .args([
                "worktree",
                "add",
                &wt_path.to_string_lossy(),
                &branch_name,
            ])
            .current_dir(repo_root)
            .output()
    } else {
        // Create new branch
        Command::new("git")
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
    }
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

/// Clean a worktree of any dirty files outside `.ralph/`, restoring it to a
/// pristine state matching the branch HEAD. This prevents the orchestrator
/// from aborting due to uncommitted changes left by a previous run or by
/// backend side-effects (e.g. codex writing files to the cwd).
pub fn clean_worktree(wt_path: &Path) -> Result<()> {
    // Discard modifications to tracked files (excluding .ralph/)
    let checkout = Command::new("git")
        .args(["checkout", "--", "."])
        .current_dir(wt_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run git checkout in worktree {}: {err}",
                wt_path.display()
            ))
        })?;
    if !checkout.status.success() {
        eprintln!(
            "warning: git checkout in worktree {} failed: {}",
            wt_path.display(),
            String::from_utf8_lossy(&checkout.stderr).trim()
        );
    }

    // Remove untracked files (excluding .ralph/)
    let clean = Command::new("git")
        .args(["clean", "-fd", "--exclude=.ralph"])
        .current_dir(wt_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run git clean in worktree {}: {err}",
                wt_path.display()
            ))
        })?;
    if !clean.status.success() {
        eprintln!(
            "warning: git clean in worktree {} failed: {}",
            wt_path.display(),
            String::from_utf8_lossy(&clean.stderr).trim()
        );
    }

    Ok(())
}

/// Create or reuse a rebase worktree and check out the requested branch.
///
/// Uses a dedicated worktree path `.ralph/daemon/worktrees/rebase-<task_id>/`.
pub fn create_worktree_on_branch(
    repo_root: &Path,
    workspace_root: &Path,
    task_id: &str,
    branch: &str,
) -> Result<PathBuf> {
    let wt_path = worktrees_dir(workspace_root).join(format!("rebase-{task_id}"));

    if !wt_path.exists() {
        if let Some(parent) = wt_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "--force",
                &wt_path.to_string_lossy(),
                "HEAD",
            ])
            .current_dir(repo_root)
            .output()
            .map_err(|err| {
                RalphError::Orchestration(format!(
                    "failed to create rebase worktree for {task_id}: {err}"
                ))
            })?;

        if !output.status.success() {
            return Err(RalphError::Orchestration(format!(
                "git worktree add failed for rebase-{task_id}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }

    checkout_branch_in_worktree(&wt_path, branch)?;
    Ok(wt_path)
}

/// Return the rebase worktree path for a task.
pub fn rebase_worktree_path(workspace_root: &Path, task_id: &str) -> PathBuf {
    worktrees_dir(workspace_root).join(format!("rebase-{task_id}"))
}

/// Remove a rebase worktree. Best-effort: logs warning on failure.
pub fn remove_rebase_worktree(repo_root: &Path, workspace_root: &Path, task_id: &str) {
    let wt_path = rebase_worktree_path(workspace_root, task_id);
    if !wt_path.exists() {
        return;
    }

    let output = Command::new("git")
        .args(["worktree", "remove", "--force", &wt_path.to_string_lossy()])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            eprintln!(
                "warning: failed to remove rebase worktree for {task_id}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            let _ = fs::remove_dir_all(&wt_path);
        }
        Err(err) => {
            eprintln!("warning: failed to run git worktree remove for rebase-{task_id}: {err}");
            let _ = fs::remove_dir_all(&wt_path);
        }
    }

    let _ = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo_root)
        .output();
}

/// Remove a worktree for a task. Best-effort: logs warning on failure.
pub fn remove_worktree(repo_root: &Path, workspace_root: &Path, task_id: &str) {
    let wt_path = task_worktree_path(workspace_root, task_id);
    if !wt_path.exists() {
        return;
    }

    let output = Command::new("git")
        .args(["worktree", "remove", "--force", &wt_path.to_string_lossy()])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            eprintln!(
                "warning: failed to remove worktree for {task_id}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            // Fallback: try to remove the directory directly
            let _ = fs::remove_dir_all(&wt_path);
        }
        Err(err) => {
            eprintln!("warning: failed to run git worktree remove for {task_id}: {err}");
            let _ = fs::remove_dir_all(&wt_path);
        }
    }

    // Prune stale worktree entries
    let _ = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo_root)
        .output();
}

/// Reconcile orphaned and stale worktrees at startup.
///
/// Scans `.ralph/daemon/worktrees/` and removes directories that are either:
/// - Not associated with any known task ID (orphaned), or
/// - Associated with a task in a terminal state (stale).
///
/// `active_task_ids` should contain only IDs of non-terminal tasks that will
/// be re-adopted by the daemon.
pub fn reconcile_worktrees(repo_root: &Path, workspace_root: &Path, active_task_ids: &[String]) {
    let wt_dir = worktrees_dir(workspace_root);
    let entries = match fs::read_dir(&wt_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !active_task_ids.contains(&name) {
            eprintln!("reconcile: removing stale/orphaned worktree {name}");
            remove_worktree(repo_root, workspace_root, &name);
        }
    }

    // Also prune git's internal worktree list
    let _ = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo_root)
        .output();
}

fn checkout_branch_in_worktree(worktree_path: &Path, branch: &str) -> Result<()> {
    let checkout = Command::new("git")
        .args(["checkout", "--force", "--ignore-other-worktrees", branch])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to checkout {branch}: {err}")))?;

    if checkout.status.success() {
        return Ok(());
    }

    let remote_branch = format!("origin/{branch}");
    let fallback = Command::new("git")
        .args([
            "checkout",
            "--force",
            "--ignore-other-worktrees",
            "-B",
            branch,
            "--track",
            &remote_branch,
        ])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to checkout tracking branch {branch}: {err}"
            ))
        })?;

    if !fallback.status.success() {
        return Err(RalphError::Orchestration(format!(
            "git checkout failed for branch {branch}: {}",
            String::from_utf8_lossy(&fallback.stderr).trim()
        )));
    }

    Ok(())
}
