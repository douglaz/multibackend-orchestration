use std::path::Path;

use crate::error::RalphError;
use crate::git::{conflicting_files, ensure_git_repo, has_conflicts, run_git, run_git_status};
use crate::Result;

/// Returns the diff of both staged and unstaged changes against HEAD.
pub fn working_tree_diff(workdir: &Path) -> Result<String> {
    ensure_git_repo(workdir)?;

    // In repositories without an initial commit, HEAD does not exist yet.
    // Build an equivalent working-tree diff by combining staged and unstaged diffs.
    if !ref_exists(workdir, "HEAD")? {
        let staged = staged_diff(workdir)?;
        let unstaged = unstaged_diff(workdir)?;
        return Ok(match (staged.is_empty(), unstaged.is_empty()) {
            (true, true) => String::new(),
            (false, true) => staged,
            (true, false) => unstaged,
            (false, false) => format!("{staged}\n\n{unstaged}"),
        });
    }

    run_git(workdir, &["diff", "HEAD"])
}

/// Returns only the diff of staged changes.
pub fn staged_diff(workdir: &Path) -> Result<String> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["diff", "--cached"])
}

/// Returns only the diff of unstaged changes.
pub fn unstaged_diff(workdir: &Path) -> Result<String> {
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

    // Check for merge conflicts before attempting to commit
    if has_conflicts(workdir)? {
        let files = conflicting_files(workdir)?;
        return Err(RalphError::GitConflict {
            details: format!(
                "Merge conflicts detected in {} file(s): {}",
                files.len(),
                files.join(", ")
            ),
        });
    }

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
