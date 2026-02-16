use std::path::Path;

use crate::error::RalphError;
use crate::git::{
    conflicting_files, ensure_git_repo, has_conflicts, read_porcelain_status, run_git,
    run_git_status,
};
use crate::Result;

pub const ORCHESTRATION_STATE_PATH_PREFIX: &str = ".ralph/";
pub const ORCHESTRATION_STATE_PATHSPEC: &str = ".ralph";

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

/// Returns the diff of both staged and unstaged changes against HEAD,
/// excluding selected pathspecs.
pub fn working_tree_diff_excluding(workdir: &Path, excluded_pathspecs: &[&str]) -> Result<String> {
    ensure_git_repo(workdir)?;

    if excluded_pathspecs.is_empty() {
        return working_tree_diff(workdir);
    }

    // In repositories without an initial commit, HEAD does not exist yet.
    // Build an equivalent working-tree diff by combining staged and unstaged diffs.
    if !ref_exists(workdir, "HEAD")? {
        let staged = staged_diff_excluding(workdir, excluded_pathspecs)?;
        let unstaged = unstaged_diff_excluding(workdir, excluded_pathspecs)?;
        return Ok(match (staged.is_empty(), unstaged.is_empty()) {
            (true, true) => String::new(),
            (false, true) => staged,
            (true, false) => unstaged,
            (false, false) => format!("{staged}\n\n{unstaged}"),
        });
    }

    run_git_with_exclusions(workdir, &["diff", "HEAD"], excluded_pathspecs)
}

/// Returns the working-tree diff while hiding Ralph runtime state artifacts.
pub fn working_tree_diff_excluding_orchestration_state(workdir: &Path) -> Result<String> {
    working_tree_diff_excluding(workdir, &[ORCHESTRATION_STATE_PATHSPEC])
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

/// Returns changed file paths from `git status --porcelain` (including untracked files).
pub fn changed_paths(workdir: &Path) -> Result<Vec<String>> {
    ensure_git_repo(workdir)?;
    let status = read_porcelain_status(workdir)?;
    Ok(status
        .lines()
        .filter_map(parse_porcelain_status_path)
        .collect())
}

/// Returns changed file paths, excluding entries under any provided path prefixes.
pub fn changed_paths_excluding_prefixes(
    workdir: &Path,
    excluded_prefixes: &[&str],
) -> Result<Vec<String>> {
    let mut paths = changed_paths(workdir)?;
    paths.retain(|path| {
        !excluded_prefixes
            .iter()
            .any(|prefix| path_matches_prefix(path, prefix))
    });
    Ok(paths)
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

/// Stage all non-orchestration changes so reviewer diff (`git diff HEAD`)
/// includes newly created files.
///
/// Uses `git add -A` followed by unstaging `.ralph/` because pathspec
/// exclusions (`:(exclude).ralph`) cause `git add` to error when `.ralph`
/// is gitignored.  The two-step approach works regardless of `.gitignore`
/// configuration.
pub fn stage_implementation_changes(workdir: &Path) -> Result<()> {
    ensure_git_repo(workdir)?;
    run_git(workdir, &["add", "-A"])?;
    // Unstage any .ralph/ entries that slipped in (e.g. repos where .ralph
    // is not gitignored).  --ignore-unmatch avoids errors when nothing was
    // staged.  Best-effort: errors are harmless.
    let _ = run_git(workdir, &["rm", "--cached", "-r", "--ignore-unmatch", ".ralph"]);
    Ok(())
}

/// Undo non-orchestration working-tree/index changes and remove non-orchestration
/// untracked files. Preserve `.ralph/**`.
pub fn reset_and_clean_working_tree(workdir: &Path) -> Result<()> {
    ensure_git_repo(workdir)?;

    if ref_exists(workdir, "HEAD")? {
        run_git_with_exclusions(
            workdir,
            &["checkout", "HEAD"],
            &[ORCHESTRATION_STATE_PATHSPEC],
        )?;
        let _ =
            run_git_with_exclusions(workdir, &["reset", "HEAD"], &[ORCHESTRATION_STATE_PATHSPEC]);
    } else {
        // Unborn branch: clear index entries if any.
        let _ = run_git(workdir, &["reset"]);
    }

    run_git(workdir, &["clean", "-fd", "--exclude", ".ralph"])?;
    Ok(())
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

fn staged_diff_excluding(workdir: &Path, excluded_pathspecs: &[&str]) -> Result<String> {
    run_git_with_exclusions(workdir, &["diff", "--cached"], excluded_pathspecs)
}

fn unstaged_diff_excluding(workdir: &Path, excluded_pathspecs: &[&str]) -> Result<String> {
    run_git_with_exclusions(workdir, &["diff"], excluded_pathspecs)
}

fn run_git_with_exclusions(
    workdir: &Path,
    args: &[&str],
    excluded_pathspecs: &[&str],
) -> Result<String> {
    if excluded_pathspecs.is_empty() {
        return run_git(workdir, args);
    }

    let mut argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    argv.push("--".to_owned());
    argv.push(".".to_owned());
    for pathspec in excluded_pathspecs {
        argv.push(format!(":(exclude){pathspec}"));
    }
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    run_git(workdir, &argv_refs)
}

fn parse_porcelain_status_path(line: &str) -> Option<String> {
    let trimmed = line.trim_end();
    if trimmed.is_empty() || trimmed.starts_with("!! ") {
        return None;
    }

    if let Some(path) = trimmed.strip_prefix("?? ") {
        return Some(path.to_owned());
    }

    let path_part = trimmed.get(3..)?.trim();
    if path_part.is_empty() {
        return None;
    }

    if let Some((_, new_path)) = path_part.rsplit_once(" -> ") {
        return Some(new_path.to_owned());
    }

    Some(path_part.to_owned())
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    let normalized = prefix.trim().trim_end_matches('/');
    !normalized.is_empty() && (path == normalized || path.starts_with(&format!("{normalized}/")))
}
