use std::path::Path;

use crate::git::{ensure_git_repo, run_git, run_git_status};
use crate::Result;

pub fn working_tree_diff(workdir: &Path) -> Result<String> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["diff"])
}

pub fn commit_feature_loop(
    workdir: &Path,
    message: &str,
    tag_name: Option<&str>,
    sign_commits: bool,
) -> Result<String> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["add", "-A"])?;

    let mut commit_args = vec!["commit", "--allow-empty", "-m", message];
    if sign_commits {
        commit_args.insert(1, "-S");
    }
    run_git(workdir, &commit_args)?;

    let commit_hash = rev_parse(workdir, "HEAD")?;

    if let Some(tag) = tag_name {
        run_git(workdir, &["tag", tag, "HEAD"])?;
    }

    Ok(commit_hash)
}

pub fn has_uncommitted_changes(workdir: &Path) -> Result<bool> {
    ensure_git_repo(workdir)?;
    let status = run_git_status(workdir, &["diff", "--quiet"])?;
    Ok(!status.success())
}

pub fn reset_hard(workdir: &Path, reference: &str) -> Result<()> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["reset", "--hard", reference])?;
    Ok(())
}

pub fn ref_exists(workdir: &Path, reference: &str) -> Result<bool> {
    ensure_git_repo(workdir)?;
    let status = run_git_status(workdir, &["rev-parse", "--verify", "--quiet", reference])?;
    Ok(status.success())
}

pub fn rev_parse(workdir: &Path, reference: &str) -> Result<String> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["rev-parse", reference])
}

pub fn merge_base(workdir: &Path, left: &str, right: &str) -> Result<String> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["merge-base", left, right])
}
