use std::path::Path;

use crate::error::RalphError;
use crate::git::{ensure_git_repo, run_git, run_git_status};
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

/// Merges `base_ref` into the current branch if the base has commits not on `HEAD`.
pub fn merge_base_branch(workdir: &Path, base_ref: &str) -> Result<()> {
    ensure_git_repo(workdir)?;
    let output = run_git(
        workdir,
        &["rev-list", "--count", &format!("HEAD..{base_ref}")],
    )?;
    let count: u64 = output.trim().parse().unwrap_or(0);
    if count == 0 {
        return Ok(());
    }

    run_git(workdir, &["merge", base_ref, "--no-edit"])?;
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

/// Check whether a remote ref exists (e.g. `origin/ralph/issue-42`).
pub fn remote_ref_exists(workdir: &Path, remote_ref: &str) -> Result<bool> {
    let status = run_git_status(workdir, &["rev-parse", "--verify", remote_ref])?;
    Ok(status.success())
}

/// Remote-first project branch sync for daemon-managed worktrees.
///
/// 1. `git fetch origin`
/// 2. If `origin/ralph/issue-<n>` exists: `git checkout -B ralph/issue-<n> origin/ralph/issue-<n>`
/// 3. Else: `git checkout -b ralph/issue-<n> origin/HEAD`
/// 4. Never creates project branches from local refs.
///
/// This function is intended **only** for daemon-managed worktree flows.
pub fn sync_project_branch(repo_root: &Path, issue_number: u32) -> Result<()> {
    ensure_git_repo(repo_root)?;

    let branch = format!("ralph/issue-{issue_number}");
    let remote_branch = format!("origin/ralph/issue-{issue_number}");

    // Step 1: fetch origin
    run_git(repo_root, &["fetch", "origin"]).map_err(|err| {
        RalphError::Orchestration(format!(
            "sync_project_branch: git fetch origin failed for issue {issue_number} \
             (branch {branch}): {err}"
        ))
    })?;

    // Step 2: check if remote project branch exists
    if remote_ref_exists(repo_root, &remote_branch)? {
        // Remote branch exists — force-reset local branch to match remote.
        // This discards any local-only diverged commits.
        run_git(repo_root, &["checkout", "-B", &branch, &remote_branch]).map_err(|err| {
            RalphError::Orchestration(format!(
                "sync_project_branch: git checkout -B {branch} {remote_branch} failed \
                 for issue {issue_number}: {err}"
            ))
        })?;
        return Ok(());
    }

    // Step 3: remote project branch missing — create from origin/HEAD
    if !remote_ref_exists(repo_root, "origin/HEAD")? {
        return Err(RalphError::Orchestration(format!(
            "sync_project_branch: origin/HEAD is missing or invalid \
             (failed: git rev-parse --verify origin/HEAD); \
             cannot create branch {branch} for issue {issue_number}. \
             Ensure the remote has a default branch configured."
        )));
    }

    run_git(repo_root, &["checkout", "-B", &branch, "origin/HEAD"]).map_err(|err| {
        RalphError::Orchestration(format!(
            "sync_project_branch: git checkout -b {branch} origin/HEAD failed \
             for issue {issue_number}: {err}"
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::{merge_base_branch, sync_project_branch};

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

    fn init_test_repo() -> TempDir {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let repo = temp_dir.path();
        git_ok(repo, &["init"]);
        git_ok(repo, &["config", "user.email", "test@example.com"]);
        git_ok(repo, &["config", "user.name", "Test User"]);

        fs::write(repo.join("README.md"), "# test\n").expect("README should be written");
        git_ok(repo, &["add", "-A"]);
        git_ok(repo, &["commit", "-m", "initial"]);

        temp_dir
    }

    #[test]
    fn merge_base_branch_syncs_new_commits() {
        let temp_dir = init_test_repo();
        let repo = temp_dir.path();

        let base_branch = git_output(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
        git_ok(repo, &["branch", "ralph/test"]);

        fs::write(repo.join("project-state.txt"), "state\n")
            .expect("project state should be written");
        git_ok(repo, &["add", "-A"]);
        git_ok(repo, &["commit", "-m", "add project state"]);
        let base_head = git_output(repo, &["rev-parse", base_branch.as_str()]);

        git_ok(repo, &["checkout", "ralph/test"]);
        merge_base_branch(repo, &base_branch).expect("merge should succeed");

        let merge_base_status = Command::new("git")
            .args(["merge-base", "--is-ancestor", base_head.as_str(), "HEAD"])
            .current_dir(repo)
            .status()
            .expect("git merge-base should execute");
        assert!(
            merge_base_status.success(),
            "base branch head should become reachable from project branch head"
        );
        assert!(
            repo.join("project-state.txt").exists(),
            "project branch should include files from base branch"
        );
    }

    #[test]
    fn merge_base_branch_noop_when_up_to_date() {
        let temp_dir = init_test_repo();
        let repo = temp_dir.path();

        let base_branch = git_output(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
        git_ok(repo, &["checkout", "-b", "ralph/test"]);

        let before_head = git_output(repo, &["rev-parse", "HEAD"]);
        merge_base_branch(repo, &base_branch).expect("no-op merge should succeed");
        let after_head = git_output(repo, &["rev-parse", "HEAD"]);

        assert_eq!(
            before_head, after_head,
            "HEAD should remain unchanged when base branch has no new commits"
        );
    }

    /// Set up a local bare remote and a clone to simulate origin interactions.
    /// Returns (clone_dir, bare_dir, _temp_dir).
    fn init_test_repo_with_remote() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let bare_dir = temp_dir.path().join("remote.git");
        let clone_dir = temp_dir.path().join("clone");

        // Create bare remote
        git_ok_abs(&bare_dir, &["init", "--bare", &bare_dir.to_string_lossy()]);

        // Create a working repo, commit, push to bare remote
        let setup_dir = temp_dir.path().join("setup");
        fs::create_dir_all(&setup_dir).expect("create setup dir");
        git_ok(&setup_dir, &["init"]);
        git_ok(&setup_dir, &["config", "user.email", "test@example.com"]);
        git_ok(&setup_dir, &["config", "user.name", "Test User"]);
        fs::write(setup_dir.join("README.md"), "# test\n").expect("write README");
        git_ok(&setup_dir, &["add", "-A"]);
        git_ok(&setup_dir, &["commit", "-m", "initial"]);
        git_ok(
            &setup_dir,
            &["remote", "add", "origin", &bare_dir.to_string_lossy()],
        );
        git_ok(&setup_dir, &["push", "-u", "origin", "HEAD"]);

        // Clone from bare into clone_dir
        git_ok_abs(
            &clone_dir,
            &[
                "clone",
                &bare_dir.to_string_lossy(),
                &clone_dir.to_string_lossy(),
            ],
        );
        git_ok(&clone_dir, &["config", "user.email", "test@example.com"]);
        git_ok(&clone_dir, &["config", "user.name", "Test User"]);

        (temp_dir, bare_dir, clone_dir)
    }

    /// Run a git command with an absolute path (for init --bare etc.)
    fn git_ok_abs(dir: &Path, args: &[&str]) {
        let parent = dir.parent().unwrap_or(Path::new("/tmp"));
        fs::create_dir_all(parent).ok();
        let status = Command::new("git")
            .args(args)
            .current_dir(parent)
            .status()
            .expect("git command should execute");
        assert!(
            status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
    }

    #[test]
    fn sync_project_branch_resets_to_remote_when_exists() {
        let (_temp_dir, _bare_dir, clone_dir) = init_test_repo_with_remote();

        // Push a project branch to the remote
        git_ok(&clone_dir, &["checkout", "-b", "ralph/issue-42"]);
        fs::write(clone_dir.join("remote-file.txt"), "from remote\n").expect("write remote file");
        git_ok(&clone_dir, &["add", "-A"]);
        git_ok(&clone_dir, &["commit", "-m", "remote commit"]);
        git_ok(&clone_dir, &["push", "origin", "ralph/issue-42"]);
        let remote_head = git_output(&clone_dir, &["rev-parse", "HEAD"]);

        // Go back to default branch, add a local-only diverged commit on
        // ralph/issue-42
        git_ok(&clone_dir, &["checkout", "ralph/issue-42"]);
        fs::write(clone_dir.join("local-only.txt"), "local diverge\n").expect("write local file");
        git_ok(&clone_dir, &["add", "-A"]);
        git_ok(&clone_dir, &["commit", "-m", "local only commit"]);
        let local_head = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_ne!(remote_head, local_head, "local should have diverged");

        // Go back to default branch so sync can checkout
        git_ok(&clone_dir, &["checkout", "-"]);

        // Sync should reset to remote
        sync_project_branch(&clone_dir, 42).expect("sync should succeed");

        let after_head = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            after_head, remote_head,
            "local branch should be reset to remote HEAD, discarding local-only commit"
        );

        // Verify we are on the right branch
        let branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "ralph/issue-42");
    }

    #[test]
    fn sync_project_branch_creates_from_origin_head_when_missing() {
        let (_temp_dir, _bare_dir, clone_dir) = init_test_repo_with_remote();

        // origin/HEAD should exist pointing to default branch
        let origin_head = git_output(&clone_dir, &["rev-parse", "origin/HEAD"]);

        // No remote ralph/issue-99 exists
        sync_project_branch(&clone_dir, 99).expect("sync should succeed");

        let after_head = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            after_head, origin_head,
            "new branch should be created from origin/HEAD"
        );

        let branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "ralph/issue-99");
    }

    #[test]
    fn sync_project_branch_fails_when_origin_head_missing() {
        let (_temp_dir, _bare_dir, clone_dir) = init_test_repo_with_remote();

        // Delete origin/HEAD locally — simulates a remote that has no default
        // branch configured. The fetch inside sync_project_branch will not
        // restore it because we also remove the remote HEAD symref.
        git_ok(&clone_dir, &["remote", "set-head", "origin", "-d"]);

        // Also point the bare remote's HEAD to a non-existent branch so
        // that `git fetch` won't re-create origin/HEAD.
        git_ok(
            &_bare_dir,
            &["symbolic-ref", "HEAD", "refs/heads/nonexistent"],
        );

        let result = sync_project_branch(&clone_dir, 7);
        assert!(result.is_err(), "should fail when origin/HEAD is missing");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("origin/HEAD"),
            "error should mention origin/HEAD: {err_msg}"
        );
        assert!(
            err_msg.contains("issue 7") || err_msg.contains("issue-7"),
            "error should mention issue number: {err_msg}"
        );
        assert!(
            err_msg.contains("ralph/issue-7"),
            "error should mention branch name: {err_msg}"
        );
        assert!(
            err_msg.contains("git rev-parse --verify origin/HEAD"),
            "error should mention the failed git operation: {err_msg}"
        );
    }

    #[test]
    fn sync_project_branch_discards_local_only_commit() {
        let (_temp_dir, _bare_dir, clone_dir) = init_test_repo_with_remote();

        // Push a project branch to the remote
        git_ok(&clone_dir, &["checkout", "-b", "ralph/issue-10"]);
        fs::write(clone_dir.join("base.txt"), "base content\n").expect("write base file");
        git_ok(&clone_dir, &["add", "-A"]);
        git_ok(&clone_dir, &["commit", "-m", "base commit"]);
        git_ok(&clone_dir, &["push", "origin", "ralph/issue-10"]);
        let remote_sha = git_output(&clone_dir, &["rev-parse", "HEAD"]);

        // Add a local-only commit (not pushed)
        fs::write(
            clone_dir.join("local-artifact.txt"),
            "should be discarded\n",
        )
        .expect("write local artifact");
        git_ok(&clone_dir, &["add", "-A"]);
        git_ok(&clone_dir, &["commit", "-m", "local only: should vanish"]);
        let local_sha = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_ne!(remote_sha, local_sha);

        // Switch away before sync
        git_ok(&clone_dir, &["checkout", "-"]);

        // Sync should discard the local commit
        sync_project_branch(&clone_dir, 10).expect("sync should succeed");

        let after_sha = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            after_sha, remote_sha,
            "local-only commit should be discarded after sync"
        );
        assert!(
            !clone_dir.join("local-artifact.txt").exists(),
            "local-only file should not exist after sync"
        );
    }
}
