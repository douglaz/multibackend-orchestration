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

    // If the base ref doesn't exist (e.g. empty remote where the configured
    // base branch was never created), skip silently — there's nothing to merge.
    let base_exists = run_git_status(workdir, &["rev-parse", "--verify", base_ref])
        .map(|s| s.success())
        .unwrap_or(false);
    if !base_exists {
        return Ok(());
    }

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

/// Detect the remote's default branch when the configured base branch is missing.
///
/// Tries `origin/HEAD` symbolic-ref first, then falls back to common names.
/// Returns the **local** branch name (e.g. `"main"`), not the remote-tracking ref.
fn detect_remote_default_branch(repo_root: &Path) -> Option<String> {
    // Try symbolic-ref of origin/HEAD (set by `git clone` or `git remote set-head`).
    if let Ok(refname) = run_git(repo_root, &["symbolic-ref", "refs/remotes/origin/HEAD"]) {
        let refname = refname.trim().to_owned();
        if let Some(branch) = refname.strip_prefix("refs/remotes/origin/") {
            // Skip ralph/* branches — GitHub may set the default branch to a
            // ralph project/issue branch if it's the only branch on the remote.
            if !branch.is_empty() && !branch.starts_with("ralph/") {
                return Some(branch.to_owned());
            }
        }
    }

    // Fallback: probe common default branch names on the remote.
    for candidate in &["main", "master"] {
        let remote_ref = format!("origin/{candidate}");
        if remote_ref_exists(repo_root, &remote_ref).unwrap_or(false) {
            return Some(candidate.to_string());
        }
    }

    None
}

/// Remote-first project branch sync for daemon-managed worktrees.
///
/// 1. `git fetch origin` (best-effort)
/// 2. If remote base exists: `git branch -f <base_branch> origin/<base_branch>`
///    (auto-detects the actual remote default branch when configured base is missing)
/// 3. If `origin/ralph/issue-<n>` exists: `git checkout -B ralph/issue-<n> origin/ralph/issue-<n>`
/// 4. Else: `git checkout -B ralph/issue-<n> origin/<base_branch>` (or local base for empty remotes)
///
/// This function is intended **only** for daemon-managed worktree flows.
pub fn sync_project_branch(repo_root: &Path, issue_number: u32, base_branch: &str) -> Result<()> {
    ensure_git_repo(repo_root)?;

    let branch = format!("ralph/issue-{issue_number}");
    let remote_branch = format!("origin/ralph/issue-{issue_number}");
    let remote_base_branch = format!("origin/{base_branch}");

    // Step 1: fetch origin (best-effort — remote may be empty or unreachable
    // for bootstrapped repos).
    let fetch_ok = run_git(repo_root, &["fetch", "origin"]).is_ok();

    let mut has_remote_base = remote_ref_exists(repo_root, &remote_base_branch)?;

    // If the configured base branch doesn't exist on the remote but fetch
    // succeeded, auto-detect the actual default branch.  This handles the
    // common case where config says "master" but the remote uses "main" (or
    // vice-versa).
    let effective_base;
    let effective_remote_base;
    if !has_remote_base && fetch_ok {
        if let Some(detected) = detect_remote_default_branch(repo_root) {
            if detected != base_branch {
                eprintln!(
                    "sync_project_branch: configured base_branch '{base_branch}' not found on remote; \
                     using detected default branch '{detected}' for issue {issue_number}"
                );
                effective_base = detected;
                effective_remote_base = format!("origin/{effective_base}");
                has_remote_base = remote_ref_exists(repo_root, &effective_remote_base)?;
            } else {
                effective_base = base_branch.to_owned();
                effective_remote_base = remote_base_branch.clone();
            }
        } else {
            effective_base = base_branch.to_owned();
            effective_remote_base = remote_base_branch.clone();
        }
    } else {
        effective_base = base_branch.to_owned();
        effective_remote_base = remote_base_branch.clone();
    }

    let base_branch = effective_base.as_str();
    let remote_base_branch = &effective_remote_base;

    // Step 2: force-sync local base branch to the remote-tracking base.
    // Skip entirely when the remote has no branches (empty repo).
    if has_remote_base {
        // When the current worktree is checked out on the base branch, `git branch -f`
        // cannot move it. Detach first so the force-update can proceed.
        let active_branch = current_branch(repo_root).map_err(|err| {
            RalphError::Orchestration(format!(
                "sync_project_branch: failed to resolve current branch for issue {issue_number} \
                 (branch {branch}): {err}"
            ))
        })?;
        if active_branch == base_branch {
            run_git(repo_root, &["checkout", "--detach"]).map_err(|err| {
                RalphError::Orchestration(format!(
                    "sync_project_branch: git checkout --detach failed before base sync \
                     for issue {issue_number} (base {base_branch}, project branch {branch}): {err}"
                ))
            })?;
        }

        let branch_force_result = run_git(
            repo_root,
            &["branch", "-f", base_branch, &remote_base_branch],
        );
        if let Err(branch_force_err) = branch_force_result {
            let err_string = branch_force_err.to_string();
            if err_string.contains("cannot force update the branch") {
                let local_base_ref = format!("refs/heads/{base_branch}");
                let remote_base_ref = format!("refs/remotes/{remote_base_branch}");
                run_git(repo_root, &["update-ref", &local_base_ref, &remote_base_ref]).map_err(
                    |update_err| {
                        RalphError::Orchestration(format!(
                            "sync_project_branch: git branch -f {base_branch} {remote_base_branch} failed \
                             for issue {issue_number} (project branch {branch}): {branch_force_err}; \
                             fallback git update-ref {local_base_ref} {remote_base_ref} failed: {update_err}"
                        ))
                    },
                )?;
            } else {
                return Err(RalphError::Orchestration(format!(
                    "sync_project_branch: git branch -f {base_branch} {remote_base_branch} failed \
                     for issue {issue_number} (project branch {branch}): {branch_force_err}"
                )));
            }
        }
    } else if fetch_ok {
        // Fetch succeeded but no remote base branch exists — the remote is
        // truly empty (brand-new repo with no default branch).  Bootstrap a
        // local base branch from HEAD and push it so the remote gets a proper
        // default branch.
        let local_base_exists = run_git_status(
            repo_root,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{base_branch}"),
            ],
        )
        .map(|s| s.success())
        .unwrap_or(false);

        if !local_base_exists {
            // Create the local base branch from the current HEAD (bootstrap commit).
            eprintln!(
                "sync_project_branch: empty remote and no local '{base_branch}' branch; \
                 creating from HEAD for issue {issue_number}"
            );
            run_git(repo_root, &["branch", base_branch, "HEAD"]).map_err(|err| {
                RalphError::Orchestration(format!(
                    "sync_project_branch: git branch {base_branch} HEAD failed \
                     for issue {issue_number}: {err}"
                ))
            })?;
        }

        // Push the base branch to origin to establish the remote default branch.
        match run_git(repo_root, &["push", "-u", "origin", base_branch]) {
            Ok(_) => {
                eprintln!(
                    "sync_project_branch: pushed '{base_branch}' to origin for issue {issue_number}"
                );
                // Re-check: the remote base should now exist after push.
                has_remote_base = remote_ref_exists(repo_root, remote_base_branch)?;
            }
            Err(push_err) => {
                eprintln!(
                    "sync_project_branch: failed to push '{base_branch}' to origin \
                     for issue {issue_number}: {push_err}; continuing with local base"
                );
            }
        }
    } else {
        eprintln!(
            "sync_project_branch: no remote base branch and fetch failed; \
             using local {base_branch} for issue {issue_number}"
        );
    }

    // Step 3: check if remote project branch exists
    if remote_ref_exists(repo_root, &remote_branch)? {
        // Remote branch exists — force-reset local branch to match remote.
        // This discards any local-only diverged commits.
        run_git(
            repo_root,
            &["checkout", "--force", "-B", &branch, &remote_branch],
        )
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "sync_project_branch: git checkout -B {branch} {remote_branch} failed \
                 for issue {issue_number}: {err}"
            ))
        })?;
        return Ok(());
    }

    // Step 4: remote project branch missing — create from remote or local base
    let start_ref = if has_remote_base {
        remote_base_branch.clone()
    } else {
        // Empty remote: prefer local base branch (e.g. bootstrap commit).
        // If the local base branch doesn't exist either (e.g. repo was
        // bootstrapped with a different default branch name, or the remote
        // has no default branch at all), fall back to HEAD.
        let local_base_exists = run_git_status(
            repo_root,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{base_branch}"),
            ],
        )
        .map(|s| s.success())
        .unwrap_or(false);
        if local_base_exists {
            base_branch.to_owned()
        } else {
            eprintln!(
                "sync_project_branch: local base branch '{base_branch}' not found; \
                 falling back to HEAD for issue {issue_number}"
            );
            "HEAD".to_owned()
        }
    };

    run_git(
        repo_root,
        &["checkout", "--force", "-B", &branch, &start_ref],
    )
    .map_err(|err| {
        RalphError::Orchestration(format!(
            "sync_project_branch: git checkout -B {branch} {start_ref} failed \
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
        let base_branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);

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

        // Sync should reset to remote
        sync_project_branch(&clone_dir, 42, &base_branch).expect("sync should succeed");

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
        let base_branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let origin_base_ref = format!("origin/{base_branch}");

        // origin/<base_branch> should exist.
        let origin_base = git_output(&clone_dir, &["rev-parse", &origin_base_ref]);
        // Keep base branch unchecked out so `git branch -f` can move it.
        git_ok(&clone_dir, &["checkout", "-b", "scratch"]);

        // No remote ralph/issue-99 exists
        sync_project_branch(&clone_dir, 99, &base_branch).expect("sync should succeed");

        let after_head = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            after_head, origin_base,
            "new branch should be created from origin/<base_branch>"
        );

        let branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "ralph/issue-99");
    }

    #[test]
    fn sync_project_branch_falls_back_to_local_base_when_origin_missing() {
        let (_temp_dir, bare_dir, clone_dir) = init_test_repo_with_remote();
        let base_branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let origin_base_ref = format!("origin/{base_branch}");
        let local_base_sha = git_output(&clone_dir, &["rev-parse", &base_branch]);
        git_ok(&clone_dir, &["checkout", "-b", "scratch"]);

        // Delete remote base branch and point HEAD to a non-existent branch so
        // fetch won't restore origin/<base_branch>.
        git_ok(
            &bare_dir,
            &["symbolic-ref", "HEAD", "refs/heads/nonexistent"],
        );
        git_ok(&bare_dir, &["branch", "-D", &base_branch]);
        git_ok(
            &clone_dir,
            &[
                "update-ref",
                "-d",
                &format!("refs/remotes/{origin_base_ref}"),
            ],
        );

        let result = sync_project_branch(&clone_dir, 7, &base_branch);
        assert!(
            result.is_ok(),
            "should succeed via local base fallback: {:?}",
            result.err()
        );

        let branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "ralph/issue-7");
        let head_sha = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            head_sha, local_base_sha,
            "project branch should start from local base"
        );
    }

    #[test]
    fn sync_project_branch_discards_local_only_commit() {
        let (_temp_dir, _bare_dir, clone_dir) = init_test_repo_with_remote();
        let base_branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);

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

        // Sync should discard the local commit
        sync_project_branch(&clone_dir, 10, &base_branch).expect("sync should succeed");

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

    #[test]
    fn sync_project_branch_force_updates_local_base_branch() {
        let (_temp_dir, _bare_dir, clone_dir) = init_test_repo_with_remote();
        let base_branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);

        let stale_sha = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        fs::write(clone_dir.join("remote-base.txt"), "remote base advance\n")
            .expect("write remote base file");
        git_ok(&clone_dir, &["add", "-A"]);
        git_ok(&clone_dir, &["commit", "-m", "advance remote base"]);
        git_ok(&clone_dir, &["push", "origin", &base_branch]);
        let remote_base_sha =
            git_output(&clone_dir, &["rev-parse", &format!("origin/{base_branch}")]);

        git_ok(&clone_dir, &["reset", "--hard", &stale_sha]);
        let local_base_before = git_output(&clone_dir, &["rev-parse", &base_branch]);
        assert_ne!(
            local_base_before, remote_base_sha,
            "local base branch should be stale before sync"
        );

        git_ok(&clone_dir, &["checkout", "-b", "scratch"]);
        sync_project_branch(&clone_dir, 500, &base_branch).expect("sync should succeed");

        let local_base_after = git_output(&clone_dir, &["rev-parse", &base_branch]);
        let remote_base_after =
            git_output(&clone_dir, &["rev-parse", &format!("origin/{base_branch}")]);
        assert_eq!(
            local_base_after, remote_base_after,
            "sync should force-update local base branch to remote base"
        );
    }

    #[test]
    fn sync_project_branch_force_updates_custom_base_branch() {
        let (_temp_dir, _bare_dir, clone_dir) = init_test_repo_with_remote();
        let custom_base = "main";

        git_ok(&clone_dir, &["checkout", "-b", custom_base]);
        fs::write(clone_dir.join("main-base-1.txt"), "main base 1\n").expect("write main base 1");
        git_ok(&clone_dir, &["add", "-A"]);
        git_ok(&clone_dir, &["commit", "-m", "main base commit 1"]);
        git_ok(&clone_dir, &["push", "-u", "origin", custom_base]);

        fs::write(clone_dir.join("main-base-2.txt"), "main base 2\n").expect("write main base 2");
        git_ok(&clone_dir, &["add", "-A"]);
        git_ok(&clone_dir, &["commit", "-m", "main base commit 2"]);
        git_ok(&clone_dir, &["push", "origin", custom_base]);

        let stale_main_sha = git_output(&clone_dir, &["rev-parse", "HEAD~1"]);
        let remote_main_sha = git_output(&clone_dir, &["rev-parse", "origin/main"]);
        git_ok(&clone_dir, &["reset", "--hard", &stale_main_sha]);
        let local_main_before = git_output(&clone_dir, &["rev-parse", custom_base]);
        assert_ne!(
            local_main_before, remote_main_sha,
            "local custom base should be stale before sync"
        );

        git_ok(&clone_dir, &["checkout", "-"]);
        sync_project_branch(&clone_dir, 501, custom_base).expect("sync should succeed");

        let local_main_after = git_output(&clone_dir, &["rev-parse", custom_base]);
        let remote_main_after = git_output(&clone_dir, &["rev-parse", "origin/main"]);
        assert_eq!(
            local_main_after, remote_main_after,
            "sync should force-update local custom base branch to origin/main"
        );

        let head_after = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            head_after, remote_main_after,
            "new issue branch should be created from origin/main"
        );
    }

    #[test]
    fn sync_project_branch_autodetects_when_configured_base_missing() {
        // Simulate: config says "master" but the remote only has "main".
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let bare_dir = temp_dir.path().join("remote.git");
        let setup_dir = temp_dir.path().join("setup");
        let clone_dir = temp_dir.path().join("clone");

        // Create bare remote
        git_ok_abs(&bare_dir, &["init", "--bare", &bare_dir.to_string_lossy()]);

        // Create a working repo with "main" as the default branch, push to bare
        fs::create_dir_all(&setup_dir).expect("create setup dir");
        git_ok(&setup_dir, &["init", "-b", "main"]);
        git_ok(&setup_dir, &["config", "user.email", "test@example.com"]);
        git_ok(&setup_dir, &["config", "user.name", "Test User"]);
        fs::write(setup_dir.join("README.md"), "# test\n").expect("write README");
        git_ok(&setup_dir, &["add", "-A"]);
        git_ok(&setup_dir, &["commit", "-m", "initial on main"]);
        git_ok(
            &setup_dir,
            &["remote", "add", "origin", &bare_dir.to_string_lossy()],
        );
        git_ok(&setup_dir, &["push", "-u", "origin", "main"]);

        // Point bare remote's HEAD to "main" so clone works correctly
        git_ok(&bare_dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);

        // Clone from bare
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

        // Verify: no "master" branch exists locally or remotely
        let branch_list = git_output(&clone_dir, &["branch", "-a"]);
        assert!(
            !branch_list.contains("master"),
            "repo should have no master branch: {branch_list}"
        );

        let origin_main_sha = git_output(&clone_dir, &["rev-parse", "origin/main"]);

        // Call sync_project_branch with "master" as configured base —
        // should auto-detect "main" and succeed.
        sync_project_branch(&clone_dir, 77, "master")
            .expect("sync should succeed by auto-detecting 'main' when 'master' is missing");

        let branch = git_output(&clone_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "ralph/issue-77");

        let head_sha = git_output(&clone_dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            head_sha, origin_main_sha,
            "issue branch should be created from origin/main (auto-detected)"
        );
    }

    #[test]
    fn sync_project_branch_bootstraps_empty_remote() {
        // Simulate: brand-new repo with no branches on remote at all.
        // The local repo has a bootstrap commit (from ensure_repo_ready)
        // on whatever branch git init created, but no "master" branch.
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let bare_dir = temp_dir.path().join("remote.git");
        let work_dir = temp_dir.path().join("work");

        // Create empty bare remote
        git_ok_abs(&bare_dir, &["init", "--bare", &bare_dir.to_string_lossy()]);

        // Create a local repo that simulates the daemon bootstrap:
        // git init (creates branch with whatever default name),
        // then one empty bootstrap commit.
        fs::create_dir_all(&work_dir).expect("create work dir");
        git_ok(&work_dir, &["init"]);
        git_ok(&work_dir, &["config", "user.email", "test@example.com"]);
        git_ok(&work_dir, &["config", "user.name", "Test User"]);
        git_ok(
            &work_dir,
            &[
                "commit",
                "--allow-empty",
                "-m",
                "ralph: bootstrap empty commit",
            ],
        );
        git_ok(
            &work_dir,
            &["remote", "add", "origin", &bare_dir.to_string_lossy()],
        );

        let local_default = git_output(&work_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let head_sha = git_output(&work_dir, &["rev-parse", "HEAD"]);

        // The worktree would be checked out on a daemon branch.
        // Simulate that by creating a separate branch.
        git_ok(&work_dir, &["checkout", "-b", "ralph/daemon/test-task"]);

        // Call sync_project_branch with "master" as configured base.
        // The remote is empty — should create "master" locally from HEAD
        // and push it.
        sync_project_branch(&work_dir, 1, "master")
            .expect("sync should succeed by bootstrapping master on empty remote");

        let branch = git_output(&work_dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "ralph/issue-1");

        let issue_head = git_output(&work_dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            issue_head, head_sha,
            "issue branch should start from the bootstrap commit"
        );

        // Verify that "master" was pushed to the remote
        let remote_master_exists = Command::new("git")
            .args(["rev-parse", "--verify", "origin/master"])
            .current_dir(&work_dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(
            remote_master_exists,
            "master should have been pushed to origin"
        );

        // Verify the default branch was correctly named (may differ from
        // "master" if git defaults to something else — the sync should
        // create the configured base_branch name regardless).
        let local_master_exists = Command::new("git")
            .args(["rev-parse", "--verify", "refs/heads/master"])
            .current_dir(&work_dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(
            local_master_exists,
            "local 'master' branch should exist after bootstrap (default was '{local_default}')"
        );
    }
}
