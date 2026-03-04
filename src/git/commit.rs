use std::path::Path;

use tracing::info;

use crate::error::RalphError;
use crate::git::branch::current_branch;
use crate::git::ralph_commit::build_ralph_commit_message;
use crate::git::{
    conflicting_files, ensure_git_repo, has_conflicts, read_porcelain_status, run_git,
    run_git_status,
};
use crate::project::artifacts::{parse_artifact_filename_timestamp, ARTIFACT_TIMESTAMP_LEN};
use crate::project::state::Phase;
use crate::Result;

pub const ORCHESTRATION_STATE_PATH_PREFIX: &str = ".ralph/";
pub const ORCHESTRATION_STATE_PATHSPEC: &str = ".ralph";
const GENERATED_ARTIFACT_PATHS: &[&str] = &["SPEC.md"];

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
    _tag_name: Option<&str>,
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
    // Unstage runtime and generated artifacts to avoid git pollution.
    unstage_non_commit_artifacts(workdir);

    let mut commit_args = vec!["commit", "--allow-empty", "-m", message];
    if sign_commits {
        commit_args.insert(1, "-S");
    }
    run_git(workdir, &commit_args)?;

    let commit_hash = rev_parse(workdir, "HEAD")?;

    Ok(commit_hash)
}

pub fn commit_and_push_initial_prompt(
    repo_root: &Path,
    project_id: &str,
    expected_branch: &str,
    sign_commits: bool,
) -> Result<()> {
    ensure_git_repo(repo_root)?;

    let actual_branch = current_branch(repo_root)?;
    if actual_branch != expected_branch {
        return Err(RalphError::BranchMismatch {
            expected: expected_branch.to_owned(),
            actual: actual_branch,
        });
    }

    // Stage only prompt-input files created during project setup.
    let mut staged_any = false;
    for rel in [
        format!(".ralph/projects/{project_id}/prompt.md"),
        format!(".ralph/projects/{project_id}/project.toml"),
        format!(".ralph/projects/{project_id}/config.toml"),
    ] {
        if repo_root.join(&rel).exists() {
            run_git(repo_root, &["add", "--", &rel])?;
            staged_any = true;
        }
    }

    if !staged_any {
        return Ok(());
    }

    let has_staged_changes = {
        let status = run_git_status(repo_root, &["diff", "--cached", "--quiet"])?;
        !status.success()
    };

    if !has_staged_changes {
        return Ok(());
    }

    let message = format!("chore({project_id}): sync initial prompt inputs");
    let mut commit_args = vec!["commit", "-m", &message];
    if sign_commits {
        commit_args.insert(1, "-S");
    }
    run_git(repo_root, &commit_args)?;
    run_git(
        repo_root,
        &["push", "origin", &format!("HEAD:{expected_branch}")],
    )?;
    Ok(())
}

pub fn commit_and_push_phase_transition(
    repo_root: &Path,
    project_id: &str,
    loop_number: u32,
    from_phase: Phase,
    to_phase: Phase,
    branch: &str,
    sign_commits: bool,
) -> Result<()> {
    ensure_git_repo(repo_root)?;

    // Keep the failure behavior aligned with commit_feature_loop.
    if has_conflicts(repo_root)? {
        let files = conflicting_files(repo_root)?;
        return Err(RalphError::GitConflict {
            details: format!(
                "Merge conflicts detected in {} file(s): {}",
                files.len(),
                files.join(", ")
            ),
        });
    }

    run_git(repo_root, &["add", "-A"])?;
    // Only clean stray impl artifacts when transitioning out of Implementing —
    // that is the only phase where implementer backends leave root-level duplicates.
    if from_phase == Phase::Implementing {
        remove_stray_impl_artifacts(repo_root)?;
    }

    let message = build_ralph_commit_message(project_id, loop_number, from_phase, to_phase);
    let mut commit_args = vec!["commit", "--allow-empty", "-m", &message];
    if sign_commits {
        commit_args.insert(1, "-S");
    }
    run_git(repo_root, &commit_args)?;

    run_git(repo_root, &["push", "origin", &format!("HEAD:{branch}")])?;

    Ok(())
}

pub fn count_phase_transition_checkpoints(
    workdir: &Path,
    project_id: &str,
    from_phase: &str,
    to_phase: &str,
) -> Result<u32> {
    ensure_git_repo(workdir)?;
    let needle = format!("chore({project_id}): checkpoint {from_phase} -> {to_phase}");
    let log = run_git(
        workdir,
        &["log", "--format=%s", "--fixed-strings", "--grep", &needle],
    )?;

    if log.trim().is_empty() {
        return Ok(0);
    }
    Ok(log.lines().filter(|line| line.trim() == needle).count() as u32)
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
    remove_stray_impl_artifacts(workdir)?;
    // Unstage runtime and generated artifacts to avoid git pollution.
    // --ignore-unmatch avoids errors when nothing was staged.
    // Best-effort: errors are harmless.
    unstage_non_commit_artifacts(workdir);
    Ok(())
}

/// Returns `true` if `file_name` matches the canonical stray impl artifact
/// patterns: `YYYYMMDDHHMMSS-impl-notes.md` or `YYYYMMDDHHMMSS-impl-response-NNN.md`.
fn is_stray_impl_artifact(file_name: &str) -> bool {
    if parse_artifact_filename_timestamp(file_name).is_none() {
        return false;
    }
    // Skip past "YYYYMMDDHHMMSS-"
    let suffix = &file_name[ARTIFACT_TIMESTAMP_LEN + 1..];
    if suffix == "impl-notes.md" {
        return true;
    }
    // Exact match: "impl-response-NNN.md" where NNN is exactly 3 ASCII digits
    if let Some(rest) = suffix.strip_prefix("impl-response-") {
        if let Some(seq) = rest.strip_suffix(".md") {
            return seq.len() == 3 && seq.chars().all(|c| c.is_ascii_digit());
        }
    }
    false
}

/// Remove stray `*-impl-notes*.md` and `*-impl-response*.md` files from the
/// worktree root.  These are duplicates left behind by implementer backends
/// (canonical copies live in `.ralph/projects/<id>/loops/<NNN>/`).
///
/// Must be called **after** `git add -A` so that previously-untracked files are
/// in the index and `git rm --force` can remove them from both index and disk.
pub fn remove_stray_impl_artifacts(workdir: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(workdir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !is_stray_impl_artifact(name) {
            continue;
        }
        info!("removing stray impl artifact: {name}");
        // Try git rm first (handles tracked/staged files — removes from index + working tree).
        // --force is needed because after `git add -A` the file is staged but has no
        // HEAD entry, so git considers it "to be added" and plain `git rm` refuses.
        let _ = run_git(workdir, &["rm", "--force", "--ignore-unmatch", "--", name]);
        // git rm --ignore-unmatch exits 0 for untracked files without removing them.
        // Always fall back to filesystem deletion if the file still exists.
        let path = entry.path();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        // If the file was staged in the index (e.g. via a prior `git add -A`),
        // remove it from the index too so it won't be re-committed.
        let _ = run_git(workdir, &["rm", "--cached", "--ignore-unmatch", "--", name]);
    }
    Ok(())
}

fn unstage_non_commit_artifacts(workdir: &Path) {
    // Use `git reset HEAD -- .ralph` to unstage any .ralph/ entries that were
    // picked up by `git add -A`.  Unlike `git rm --cached -r`, this restores
    // the index entry to match HEAD rather than staging a deletion, so tracked
    // files under `.ralph/` (e.g. prompt inputs committed via early-prompt-push)
    // are preserved in the index without being re-staged or marked for removal.
    //
    // Best-effort: errors (e.g. unborn HEAD) are harmless — if HEAD doesn't
    // exist the entries simply stay staged, which only matters for the very
    // first commit where `.ralph` content is intentionally committed anyway.
    let _ = run_git(workdir, &["reset", "HEAD", "--", ".ralph"]);

    // Explicitly remove generated artifacts from the index.  These are files
    // that should never be committed (e.g. SPEC.md), so `git rm --cached` is
    // correct here — they are not tracked prompt inputs.
    for artifact in GENERATED_ARTIFACT_PATHS {
        let _ = run_git(
            workdir,
            &["rm", "--cached", "--ignore-unmatch", "--", artifact],
        );
    }
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use tempfile::{tempdir, TempDir};

    use super::{
        commit_and_push_initial_prompt, commit_and_push_phase_transition, commit_feature_loop,
        count_phase_transition_checkpoints,
    };
    use crate::error::RalphError;
    use crate::git::branch::sync_project_branch;
    use crate::git::ralph_commit::{build_ralph_commit_message, derive_position};
    use crate::project::state::Phase;

    fn git_ok(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git command should execute");
        assert!(
            status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
    }

    fn git_output(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git command should execute");
        assert!(
            output.status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn git_output_in_dir(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command should execute");
        assert!(
            output.status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn commit_empty_with_message(repo: &Path, message: &str) {
        let msg_file = repo.join(".git").join("RALPH_MSG");
        let msg_file_str = msg_file.to_string_lossy().into_owned();
        fs::write(&msg_file, message).expect("message file should be written");
        git_ok(
            repo,
            &["commit", "--allow-empty", "--file", msg_file_str.as_str()],
        );
        let _ = fs::remove_file(msg_file);
    }

    fn init_repo_with_remote() -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new().expect("temp dir should be created");
        let root = temp.path();
        let remote = root.join("remote.git");
        let work = root.join("work");
        let remote_str = remote.to_string_lossy().into_owned();

        git_ok(root, &["init", "--bare", remote_str.as_str()]);
        git_ok(root, &["clone", remote_str.as_str(), "work"]);
        git_ok(&work, &["config", "user.email", "test@example.com"]);
        git_ok(&work, &["config", "user.name", "Test User"]);

        fs::write(work.join("README.md"), "# test\n").expect("README should be written");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "-m", "initial"]);
        git_ok(&work, &["push", "-u", "origin", "HEAD:master"]);

        git_ok(&work, &["checkout", "-b", "ralph/issue-42"]);
        git_ok(&work, &["push", "-u", "origin", "ralph/issue-42"]);

        (temp, remote, work)
    }

    #[test]
    fn commit_and_push_phase_transition_pushes_structured_checkpoint() {
        let (_temp, _remote, work) = init_repo_with_remote();
        let before = git_output(&work, &["rev-parse", "origin/ralph/issue-42"]);

        fs::write(work.join(".ralph-checkpoint"), "checkpoint\n")
            .expect("checkpoint file should be written");
        commit_and_push_phase_transition(
            &work,
            "issue-42",
            3,
            Phase::Planning,
            Phase::Implementing,
            "ralph/issue-42",
            false,
        )
        .expect("checkpoint push should succeed");

        let after = git_output(&work, &["rev-parse", "origin/ralph/issue-42"]);
        let count = git_output(
            &work,
            &["rev-list", "--count", &format!("{before}..{after}")],
        );
        assert_eq!(count, "1", "expected exactly one remote checkpoint commit");

        let subject = git_output(&work, &["show", "-s", "--format=%s", &after]);
        assert_eq!(
            subject, "ralph(issue-42): loop 3 planning -> implementing",
            "unexpected commit subject"
        );

        let body = git_output(&work, &["show", "-s", "--format=%b", &after]);
        assert!(
            body.contains("Ralph-Project: issue-42"),
            "missing Ralph-Project trailer"
        );
        assert!(body.contains("Ralph-Loop: 3"), "missing Ralph-Loop trailer");
        assert!(
            body.contains("Ralph-Phase: implementing"),
            "missing Ralph-Phase trailer"
        );
    }

    #[test]
    fn commit_and_push_phase_transition_push_failure_keeps_local_commit_without_remote_advance() {
        let (_temp, remote, work) = init_repo_with_remote();
        let remote_str = remote.to_string_lossy().into_owned();
        let remote_before = git_output_in_dir(
            remote.parent().expect("remote parent should exist"),
            &[
                "--git-dir",
                remote_str.as_str(),
                "rev-parse",
                "refs/heads/ralph/issue-42",
            ],
        );
        let local_before = git_output(&work, &["rev-parse", "HEAD"]);

        git_ok(
            &work,
            &["remote", "set-url", "origin", "/definitely/missing/repo"],
        );
        fs::write(work.join("local-only.txt"), "local only\n").expect("write local file");

        let err = commit_and_push_phase_transition(
            &work,
            "issue-42",
            4,
            Phase::Implementing,
            Phase::Reviewing,
            "ralph/issue-42",
            false,
        )
        .expect_err("push should fail");
        let err_str = err.to_string();
        assert!(
            err_str.contains("push"),
            "push failure should mention push command: {err_str}"
        );

        let local_after = git_output(&work, &["rev-parse", "HEAD"]);
        assert_ne!(
            local_before, local_after,
            "local commit should exist even when push fails"
        );

        let remote_after = git_output_in_dir(
            remote.parent().expect("remote parent should exist"),
            &[
                "--git-dir",
                remote_str.as_str(),
                "rev-parse",
                "refs/heads/ralph/issue-42",
            ],
        );
        assert_eq!(
            remote_before, remote_after,
            "remote branch should not advance when push fails"
        );
    }

    #[test]
    fn sync_project_branch_discards_local_only_checkpoint_and_position_reverts() {
        let (_temp, _remote, work) = init_repo_with_remote();
        let base_branch = "master".to_owned();

        let remote_checkpoint =
            build_ralph_commit_message("issue-42", 1, Phase::Planning, Phase::Implementing);
        commit_empty_with_message(&work, &remote_checkpoint);
        git_ok(&work, &["push", "origin", "HEAD:ralph/issue-42"]);
        let remote_head = git_output(&work, &["rev-parse", "origin/ralph/issue-42"]);

        let local_only =
            build_ralph_commit_message("issue-42", 2, Phase::Implementing, Phase::Reviewing);
        commit_empty_with_message(&work, &local_only);
        let local_head = git_output(&work, &["rev-parse", "HEAD"]);
        assert_ne!(local_head, remote_head, "local should diverge from remote");

        let before_sync_position =
            derive_position(&work, "ralph/issue-42").expect("derive_position before sync");
        assert_eq!(
            before_sync_position,
            (2, Phase::Reviewing),
            "local-ahead position should reflect unpushed checkpoint"
        );

        sync_project_branch(&work, 42, &base_branch)
            .expect("sync should discard local-only checkpoint");
        let head_after_sync = git_output(&work, &["rev-parse", "HEAD"]);
        assert_eq!(
            head_after_sync, remote_head,
            "sync should reset local branch to remote checkpoint"
        );

        let after_sync_position =
            derive_position(&work, "ralph/issue-42").expect("derive_position after sync");
        assert_eq!(
            after_sync_position,
            (1, Phase::Implementing),
            "position should revert to last pushed checkpoint after sync"
        );
    }

    #[test]
    fn commit_and_push_initial_prompt_stages_only_prompt_inputs() {
        let (_temp, _remote, work) = init_repo_with_remote();

        git_ok(&work, &["checkout", "-b", "ralph/issue-99"]);
        git_ok(&work, &["push", "-u", "origin", "ralph/issue-99"]);

        let project_dir = work.join(".ralph/projects/issue-99");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(project_dir.join("prompt.md"), "initial prompt\n").expect("write prompt");
        fs::write(project_dir.join("project.toml"), "name = \"Issue 99\"\n")
            .expect("write project metadata");
        fs::write(project_dir.join("config.toml"), "[workflow]\n").expect("write config");
        fs::write(work.join("src-extra.txt"), "not prompt input\n").expect("write extra file");

        commit_and_push_initial_prompt(&work, "issue-99", "ralph/issue-99", false)
            .expect("initial prompt commit should succeed");

        let changed = git_output(&work, &["show", "--name-only", "--pretty=format:", "HEAD"]);
        let mut files: Vec<&str> = changed.lines().filter(|l| !l.trim().is_empty()).collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                ".ralph/projects/issue-99/config.toml",
                ".ralph/projects/issue-99/project.toml",
                ".ralph/projects/issue-99/prompt.md",
            ]
        );
    }

    #[test]
    fn commit_and_push_initial_prompt_fails_on_branch_mismatch() {
        let (_temp, _remote, work) = init_repo_with_remote();

        git_ok(&work, &["checkout", "-b", "ralph/issue-99"]);
        let project_dir = work.join(".ralph/projects/issue-99");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(project_dir.join("prompt.md"), "initial prompt\n").expect("write prompt");

        let before = git_output(&work, &["rev-parse", "HEAD"]);
        let err = commit_and_push_initial_prompt(&work, "issue-99", "ralph/issue-100", false)
            .expect_err("branch mismatch should fail");

        match err {
            RalphError::BranchMismatch { expected, actual } => {
                assert_eq!(expected, "ralph/issue-100");
                assert_eq!(actual, "ralph/issue-99");
            }
            other => panic!("expected BranchMismatch, got {other:?}"),
        }

        let after = git_output(&work, &["rev-parse", "HEAD"]);
        assert_eq!(before, after, "HEAD should not move on branch mismatch");
    }

    fn init_repo() -> tempfile::TempDir {
        let temp = tempdir().expect("temp dir");
        let repo = temp.path();
        git_ok(repo, &["init"]);
        git_ok(repo, &["config", "user.email", "test@example.com"]);
        git_ok(repo, &["config", "user.name", "Test User"]);
        fs::write(repo.join("README.md"), "# demo\n").expect("write readme");
        git_ok(repo, &["add", "-A"]);
        git_ok(repo, &["commit", "-m", "initial"]);
        temp
    }

    #[test]
    fn counts_only_matching_project_and_transition_messages() {
        let temp = init_repo();
        let repo = temp.path();

        commit_feature_loop(
            repo,
            "chore(proj-a): checkpoint final_review -> planning",
            None,
            false,
        )
        .expect("commit checkpoint 1");
        commit_feature_loop(
            repo,
            "chore(proj-a): checkpoint final_review -> planning",
            None,
            false,
        )
        .expect("commit checkpoint 2");
        commit_feature_loop(
            repo,
            "chore(proj-a): checkpoint final_review -> completing",
            None,
            false,
        )
        .expect("commit other transition");
        commit_feature_loop(
            repo,
            "chore(proj-b): checkpoint final_review -> planning",
            None,
            false,
        )
        .expect("commit other project");

        let count = count_phase_transition_checkpoints(repo, "proj-a", "final_review", "planning")
            .expect("count checkpoints");
        assert_eq!(count, 2);
    }

    #[test]
    fn is_stray_impl_artifact_canonical_notes() {
        assert!(super::is_stray_impl_artifact(
            "20260304123456-impl-notes.md"
        ));
    }

    #[test]
    fn is_stray_impl_artifact_canonical_response() {
        assert!(super::is_stray_impl_artifact(
            "20260304123456-impl-response-001.md"
        ));
    }

    #[test]
    fn is_stray_impl_artifact_response_999() {
        assert!(super::is_stray_impl_artifact(
            "20260304123456-impl-response-999.md"
        ));
    }

    #[test]
    fn is_stray_impl_artifact_no_timestamp() {
        assert!(!super::is_stray_impl_artifact("impl-notes.md"));
    }

    #[test]
    fn is_stray_impl_artifact_non_canonical_draft() {
        assert!(!super::is_stray_impl_artifact(
            "20260304123456-impl-response-draft.md"
        ));
    }

    #[test]
    fn is_stray_impl_artifact_four_digit_seq() {
        assert!(!super::is_stray_impl_artifact(
            "20260304123456-impl-response-0001.md"
        ));
    }

    #[test]
    fn is_stray_impl_artifact_extra_suffix() {
        assert!(!super::is_stray_impl_artifact(
            "20260304123456-impl-notes-custom.txt"
        ));
    }

    #[test]
    fn is_stray_impl_artifact_review_file() {
        assert!(!super::is_stray_impl_artifact(
            "20260304123456-review-001-feedback.md"
        ));
    }

    #[test]
    fn is_stray_impl_artifact_readme() {
        assert!(!super::is_stray_impl_artifact("README.md"));
    }

    #[test]
    fn is_stray_impl_artifact_spec() {
        assert!(!super::is_stray_impl_artifact("SPEC.md"));
    }

    #[test]
    fn is_stray_impl_artifact_short_timestamp() {
        assert!(!super::is_stray_impl_artifact(
            "2026030412345-impl-notes.md"
        ));
    }

    #[test]
    fn remove_stray_impl_artifacts_cleans_tracked_and_untracked() {
        let temp = init_repo();
        let repo = temp.path();

        // --- Truly tracked stray files: commit them first so they exist in HEAD ---
        fs::write(repo.join("20260304120000-impl-notes.md"), "stray notes\n")
            .expect("write stray notes");
        fs::write(
            repo.join("20260304120000-impl-response-001.md"),
            "stray response\n",
        )
        .expect("write stray response");
        git_ok(repo, &["add", "-A"]);
        git_ok(repo, &["commit", "-m", "add tracked stray files"]);

        // Verify they are truly tracked from HEAD
        let ls_files = git_output(repo, &["ls-files"]);
        assert!(
            ls_files.contains("20260304120000-impl-notes.md"),
            "impl-notes should be tracked in HEAD"
        );
        assert!(
            ls_files.contains("20260304120000-impl-response-001.md"),
            "impl-response should be tracked in HEAD"
        );

        // --- Untracked stray file (never committed or staged) ---
        fs::write(
            repo.join("20260304130000-impl-response-002.md"),
            "untracked stray\n",
        )
        .expect("write untracked stray");

        // Decoy files that should NOT be removed
        fs::write(repo.join("impl-notes.md"), "user notes\n").expect("write decoy notes");
        fs::write(
            repo.join("20260304120000-review-001-feedback.md"),
            "review\n",
        )
        .expect("write review artifact");

        // Simulate the flow: git add -A then cleanup
        git_ok(repo, &["add", "-A"]);
        super::remove_stray_impl_artifacts(repo).expect("cleanup should succeed");

        // Stray files should be gone from working tree
        assert!(
            !repo.join("20260304120000-impl-notes.md").exists(),
            "tracked stray notes should be removed from disk"
        );
        assert!(
            !repo.join("20260304120000-impl-response-001.md").exists(),
            "tracked stray response should be removed from disk"
        );
        assert!(
            !repo.join("20260304130000-impl-response-002.md").exists(),
            "untracked stray should be removed from disk"
        );

        // Decoy files should remain
        assert!(
            repo.join("impl-notes.md").exists(),
            "decoy notes should survive"
        );
        assert!(
            repo.join("20260304120000-review-001-feedback.md").exists(),
            "review artifact should survive"
        );

        // Verify stray files are also gone from the index
        let ls_after = git_output(repo, &["ls-files"]);
        assert!(
            !ls_after.contains("20260304120000-impl-notes.md"),
            "tracked stray notes should be removed from index"
        );
        assert!(
            !ls_after.contains("20260304120000-impl-response-001.md"),
            "tracked stray response should be removed from index"
        );
        assert!(
            !ls_after.contains("20260304130000-impl-response-002.md"),
            "untracked stray should not be in index"
        );
    }
}
