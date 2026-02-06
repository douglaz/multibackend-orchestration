pub mod branch;
pub mod commit;

use std::path::Path;
use std::process::{Command, ExitStatus};

use crate::error::RalphError;
use crate::Result;

pub(crate) fn run_git(workdir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workdir)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to execute git {:?}: {err}", args))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "git command failed: git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub(crate) fn run_git_status(workdir: &Path, args: &[&str]) -> Result<ExitStatus> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workdir)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to execute git {:?}: {err}", args))
        })?;
    Ok(output.status)
}

pub fn is_git_repo(workdir: &Path) -> bool {
    match run_git(workdir, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(out) => out.trim() == "true",
        Err(_) => false,
    }
}

pub fn ensure_git_repo(workdir: &Path) -> Result<()> {
    if !is_git_repo(workdir) {
        return Err(RalphError::Orchestration(
            "git repository not found for required operation".to_owned(),
        ));
    }
    Ok(())
}

/// Check if there are any unresolved merge conflicts in the working tree.
pub fn has_conflicts(workdir: &Path) -> Result<bool> {
    ensure_git_repo(workdir)?;
    let status = read_porcelain_status(workdir)?;
    // Look for conflict markers: UU (unmerged, both modified), AA (both added), etc.
    for line in status.lines() {
        let prefix = line.get(0..2).unwrap_or("");
        if matches!(prefix, "UU" | "AA" | "DD" | "AU" | "UA" | "DU" | "UD") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Returns the list of files with merge conflicts.
pub fn conflicting_files(workdir: &Path) -> Result<Vec<String>> {
    ensure_git_repo(workdir)?;
    let status = read_porcelain_status(workdir)?;
    let mut conflicts = Vec::new();
    for line in status.lines() {
        let prefix = line.get(0..2).unwrap_or("");
        if matches!(prefix, "UU" | "AA" | "DD" | "AU" | "UA" | "DU" | "UD") {
            if let Some(file) = line.get(3..) {
                conflicts.push(file.to_string());
            }
        }
    }
    Ok(conflicts)
}

fn read_porcelain_status(workdir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workdir)
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to check git status: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "git status --porcelain failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
