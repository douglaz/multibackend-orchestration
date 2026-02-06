use std::path::Path;

use crate::git::{ensure_git_repo, run_git};
use crate::Result;

pub fn resolve_branch_name(format: &str, project_id: &str) -> String {
    format.replace("{project_id}", project_id)
}

pub fn branch_exists(workdir: &Path, branch: &str) -> Result<bool> {
    ensure_git_repo(workdir)?;
    let output = run_git(workdir, &["branch", "--list", branch])?;
    Ok(!output.trim().is_empty())
}

pub fn create_branch(workdir: &Path, branch: &str, from_ref: &str) -> Result<()> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["branch", branch, from_ref])?;
    Ok(())
}

pub fn checkout_branch(workdir: &Path, branch: &str) -> Result<()> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["checkout", branch])?;
    Ok(())
}

pub fn ensure_project_branch(workdir: &Path, branch: &str, base_ref: &str) -> Result<()> {
    ensure_git_repo(workdir)?;
    if !branch_exists(workdir, branch)? {
        create_branch(workdir, branch, base_ref)?;
    }
    checkout_branch(workdir, branch)?;
    Ok(())
}

pub fn current_branch(workdir: &Path) -> Result<String> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["rev-parse", "--abbrev-ref", "HEAD"])
}
