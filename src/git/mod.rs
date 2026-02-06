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
