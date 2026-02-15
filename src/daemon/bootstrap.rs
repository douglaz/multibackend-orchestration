use std::path::Path;
use std::process::{Command, Output};

use crate::error::RalphError;
use crate::Result;

const BOOTSTRAP_COMMIT_MESSAGE: &str = "ralph: bootstrap empty commit";
const FALLBACK_GIT_NAME: &str = "ralph-daemon";
const FALLBACK_GIT_EMAIL: &str = "ralph@localhost";

/// Ensure `repo_root` is ready for daemon dispatch/worktree operations.
///
/// Idempotent behavior:
/// - Initializes git for non-repo directories.
/// - Rejects bare repositories as unsupported.
/// - Creates one empty bootstrap commit for unborn HEAD.
/// - Initializes `.ralph/` workspace if missing.
pub async fn ensure_repo_ready(repo_root: &Path) -> Result<()> {
    let repo_root = repo_root.to_path_buf();
    tokio::task::spawn_blocking(move || ensure_repo_ready_sync(&repo_root))
        .await
        .map_err(|err| RalphError::Orchestration(format!("bootstrap task join failure: {err}")))?
}

pub fn ensure_repo_ready_sync(repo_root: &Path) -> Result<()> {
    if !repo_root.exists() {
        std::fs::create_dir_all(repo_root)?;
    }

    if !is_git_repo(repo_root)? {
        run_git_checked(repo_root, &["init"], "failed to initialize git repository")?;
    }

    if is_bare_repo(repo_root)? {
        return Err(RalphError::Unsupported(format!(
            "daemon bootstrap does not support bare repositories: {}",
            repo_root.display()
        )));
    }

    ensure_fallback_identity_if_missing(repo_root)?;

    if is_head_unborn(repo_root)? {
        run_git_checked(
            repo_root,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "--no-verify",
                "-m",
                BOOTSTRAP_COMMIT_MESSAGE,
            ],
            "failed to create bootstrap commit",
        )?;
    }

    ensure_workspace_initialized(repo_root)?;
    Ok(())
}

fn ensure_workspace_initialized(repo_root: &Path) -> Result<()> {
    let workspace_root = repo_root.join(".ralph");
    if workspace_root.exists() {
        return Ok(());
    }

    let _ = crate::cli::init::create_workspace(&workspace_root)?;
    Ok(())
}

fn ensure_fallback_identity_if_missing(repo_root: &Path) -> Result<()> {
    let has_name = has_git_config_value(repo_root, "user.name")?;
    let has_email = has_git_config_value(repo_root, "user.email")?;

    if !has_name {
        run_git_checked(
            repo_root,
            &["config", "--local", "user.name", FALLBACK_GIT_NAME],
            "failed to set fallback git user.name",
        )?;
    }

    if !has_email {
        run_git_checked(
            repo_root,
            &["config", "--local", "user.email", FALLBACK_GIT_EMAIL],
            "failed to set fallback git user.email",
        )?;
    }

    Ok(())
}

fn has_git_config_value(repo_root: &Path, key: &str) -> Result<bool> {
    let output = run_git_raw(
        repo_root,
        &["config", "--get", key],
        "failed to read git config",
    )?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

fn is_git_repo(repo_root: &Path) -> Result<bool> {
    let output = run_git_raw(
        repo_root,
        &["rev-parse", "--git-dir"],
        "failed to check git repository state",
    )?;
    Ok(output.status.success())
}

fn is_bare_repo(repo_root: &Path) -> Result<bool> {
    let stdout = run_git_checked(
        repo_root,
        &["rev-parse", "--is-bare-repository"],
        "failed to check bare repository state",
    )?;
    Ok(stdout.trim() == "true")
}

fn is_head_unborn(repo_root: &Path) -> Result<bool> {
    let output = run_git_raw(
        repo_root,
        &["rev-parse", "--verify", "--quiet", "HEAD"],
        "failed to check HEAD state",
    )?;
    Ok(!output.status.success())
}

fn run_git_checked(repo_root: &Path, args: &[&str], context: &str) -> Result<String> {
    let output = run_git_raw(repo_root, args, context)?;
    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "{context}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn run_git_raw(repo_root: &Path, args: &[&str], context: &str) -> Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|err| RalphError::Orchestration(format!("{context}: {err}")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::{ensure_repo_ready_sync, BOOTSTRAP_COMMIT_MESSAGE};
    use crate::error::RalphError;

    #[test]
    fn bootstrap_non_git_dir_creates_git_commit_and_workspace() {
        let temp = tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo dir");

        ensure_repo_ready_sync(&repo_root).expect("bootstrap succeeds");

        assert!(repo_root.join(".git").exists(), "git should be initialized");
        assert!(
            repo_root.join(".ralph").exists(),
            "workspace should be initialized"
        );

        let count = git_stdout(&repo_root, &["rev-list", "--count", "HEAD"]);
        assert_eq!(count, "1", "bootstrap should create exactly one commit");

        let message = git_stdout(&repo_root, &["log", "-1", "--pretty=%s"]);
        assert_eq!(
            message, BOOTSTRAP_COMMIT_MESSAGE,
            "bootstrap commit message should be stable"
        );
    }

    #[test]
    fn bootstrap_is_idempotent_for_unborn_repo() {
        let temp = tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo dir");

        ensure_repo_ready_sync(&repo_root).expect("first bootstrap succeeds");
        let head_before = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
        let count_before = git_stdout(&repo_root, &["rev-list", "--count", "HEAD"]);

        ensure_repo_ready_sync(&repo_root).expect("second bootstrap succeeds");

        let head_after = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
        let count_after = git_stdout(&repo_root, &["rev-list", "--count", "HEAD"]);

        assert_eq!(head_after, head_before, "HEAD should remain unchanged");
        assert_eq!(
            count_after, count_before,
            "second bootstrap should not add commits"
        );
    }

    #[test]
    fn bootstrap_existing_repo_is_noop() {
        let temp = tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo dir");

        git(&repo_root, &["init"]);
        git(&repo_root, &["add", "-A"]);
        git(
            &repo_root,
            &[
                "-c",
                "user.name=Tester",
                "-c",
                "user.email=tester@example.com",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );

        let head_before = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
        let count_before = git_stdout(&repo_root, &["rev-list", "--count", "HEAD"]);

        ensure_repo_ready_sync(&repo_root).expect("bootstrap succeeds");

        let head_after = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
        let count_after = git_stdout(&repo_root, &["rev-list", "--count", "HEAD"]);

        assert_eq!(head_after, head_before, "HEAD should not be modified");
        assert_eq!(
            count_after, count_before,
            "commit count should be unchanged"
        );
    }

    #[test]
    fn bootstrap_bare_repo_returns_unsupported_error() {
        let temp = tempdir().expect("temp dir");
        let bare_root = temp.path().join("bare.git");

        git(
            temp.path(),
            &["init", "--bare", bare_root.to_string_lossy().as_ref()],
        );

        let err = ensure_repo_ready_sync(&bare_root).expect_err("bare repo should be rejected");
        match err {
            RalphError::Unsupported(message) => {
                assert!(
                    message.contains("bare repositories"),
                    "unsupported message should mention bare repositories"
                );
            }
            other => panic!("expected unsupported error, got {other}"),
        }
    }

    fn git(repo_root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(repo_root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}
