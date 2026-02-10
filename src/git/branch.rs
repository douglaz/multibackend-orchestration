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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::merge_base_branch;

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
}
