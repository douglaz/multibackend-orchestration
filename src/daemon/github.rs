use std::process::Command;

use serde::Deserialize;

use crate::error::RalphError;
use crate::Result;

/// Represents a single issue returned from `gh issue list`.
#[derive(Debug, Clone)]
pub struct GhIssue {
    pub number: u32,
    pub title: String,
    pub labels: Vec<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrMergeStatus {
    Conflicting,
    Mergeable,
    Unknown,
}

impl PrMergeStatus {
    fn from_gh_mergeable(raw: &str) -> Result<Self> {
        match raw {
            "CONFLICTING" => Ok(Self::Conflicting),
            "MERGEABLE" => Ok(Self::Mergeable),
            "UNKNOWN" => Ok(Self::Unknown),
            _ => Err(RalphError::Orchestration(format!(
                "unexpected gh pr mergeable value: {raw}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrMergeInfo {
    pub merge_status: PrMergeStatus,
    pub state: String,
    pub base_branch: String,
    pub head_oid: String,
}

/// Poll open issues matching all supplied labels.
///
/// Uses `gh issue list --repo <owner/repo> --limit 100 --json number,title,labels,body`
/// with repeated `--label` arguments (AND semantics).
///
/// Returns `(issues, overflow)` where overflow is true when exactly 100 issues
/// were returned, indicating possible truncation.
pub fn poll_issues(owner: &str, repo: &str, labels: &[String]) -> Result<(Vec<GhIssue>, bool)> {
    let full_repo = format!("{owner}/{repo}");
    let mut args: Vec<String> = vec![
        "issue".into(),
        "list".into(),
        "--repo".into(),
        full_repo,
        "--limit".into(),
        "100".into(),
        "--state".into(),
        "open".into(),
        "--json".into(),
        "number,title,labels,body".into(),
    ];

    for label in labels {
        args.push("--label".into());
        args.push(label.clone());
    }

    let output = Command::new("gh")
        .args(&args)
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to run gh issue list: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let raw_trimmed = raw.trim();
    if raw_trimmed.is_empty() || raw_trimmed == "[]" {
        return Ok((Vec::new(), false));
    }

    let items = parse_issue_list(raw_trimmed)?;

    let overflow = items.len() == 100;

    let issues = items
        .into_iter()
        .map(|item| {
            let labels = item.labels.into_iter().map(|label| label.name).collect();
            GhIssue {
                number: item.number,
                title: item.title,
                labels,
                body: item.body,
            }
        })
        .collect();

    Ok((issues, overflow))
}

/// Fetch an issue's title/body for restart recovery of legacy daemon tasks.
pub fn fetch_issue_body(
    owner: &str,
    repo: &str,
    issue_number: u32,
) -> Result<(String, Option<String>)> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "issue",
            "view",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--json",
            "title,body",
        ])
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run gh issue view for title/body: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue view (title/body) failed for {}#{}: {}",
            full_repo,
            issue_number,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let raw_trimmed = raw.trim();
    let parsed: RawIssueBody = serde_json::from_str(raw_trimmed).map_err(|err| {
        RalphError::Orchestration(format!("failed to parse gh issue view output: {err}"))
    })?;
    Ok((parsed.title, parsed.body))
}

/// Query PR mergeability/status metadata needed by daemon rebase logic.
pub fn query_pr_merge_info(owner: &str, repo: &str, pr_number: u32) -> Result<PrMergeInfo> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--repo",
            &full_repo,
            "--json",
            "mergeable,state,baseRefName,headRefOid",
        ])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to run gh pr view: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr view failed for {}#{}: {}",
            full_repo,
            pr_number,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    parse_pr_merge_info(raw.trim())
}

/// Filter out issues that already have any `ralph:*` label.
/// Filter issues to only those that are claimable.
///
/// An issue is NOT claimable if it has any `ralph:` label other than
/// `ralph:ready` (the trigger label). Labels like `ralph:in-progress`,
/// `ralph:completed`, `ralph:failed` indicate the daemon already owns it.
pub fn filter_claimable(issues: Vec<GhIssue>) -> Vec<GhIssue> {
    const TRIGGER_LABELS: &[&str] = &["ralph:ready"];

    issues
        .into_iter()
        .filter(|issue| {
            !issue.labels.iter().any(|l| {
                l.starts_with("ralph:") && !TRIGGER_LABELS.iter().any(|trigger| l == trigger)
            })
        })
        .collect()
}

/// Claim an issue by adding the `ralph:in-progress` label.
pub fn claim_issue(owner: &str, repo: &str, issue_number: u32) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "issue",
            "edit",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--add-label",
            "ralph:in-progress",
        ])
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run gh issue edit for claiming: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue edit (claim) failed for {}#{}: {}",
            full_repo,
            issue_number,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Update the title of a GitHub issue.
pub fn update_issue_title(owner: &str, repo: &str, issue_number: u32, title: &str) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "issue",
            "edit",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--title",
            title,
        ])
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run gh issue edit --title: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue edit --title failed for {}#{}: {}",
            full_repo,
            issue_number,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Check whether a comment with the given marker already exists on the issue.
pub fn comment_marker_exists(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
) -> Result<bool> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "issue",
            "view",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--json",
            "comments",
            "-q",
            ".comments[].body",
        ])
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to check issue comments: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue view (comments) failed for {}#{}: {}",
            full_repo,
            issue_number,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let body = String::from_utf8_lossy(&output.stdout);
    Ok(body.contains(marker))
}

/// Post an idempotent comment on the issue. If a comment with the given marker
/// already exists, skip posting.
pub fn post_idempotent_comment(
    owner: &str,
    repo: &str,
    issue_number: u32,
    task_id: &str,
    phase: &str,
    body_text: &str,
) -> Result<()> {
    let marker = format!("<!-- ralph:task:{task_id}:{phase} -->");
    if comment_marker_exists(owner, repo, issue_number, &marker)? {
        return Ok(());
    }

    let full_body = format!("{marker}\n{body_text}");
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "issue",
            "comment",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--body",
            &full_body,
        ])
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to post comment on {}#{}: {err}",
                full_repo, issue_number
            ))
        })?;

    if !output.status.success() {
        eprintln!(
            "warning: failed to post comment on {}#{}: {}",
            full_repo,
            issue_number,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(())
}

/// Post a comment on a pull request.
pub fn post_pr_comment(owner: &str, repo: &str, pr_number: u32, body: &str) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "pr",
            "comment",
            &pr_number.to_string(),
            "--repo",
            &full_repo,
            "--body",
            body,
        ])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to run gh pr comment: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr comment failed for {}#{}: {}",
            full_repo,
            pr_number,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Check for an existing PR with the given head branch.
/// Returns `Some(url)` if found, `None` otherwise.
pub fn find_existing_pr(owner: &str, repo: &str, branch: &str) -> Result<Option<String>> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "pr", "list", "--repo", &full_repo, "--head", branch, "--json", "url", "-q", ".[0].url",
        ])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to check existing PRs: {err}")))?;

    if !output.status.success() {
        return Ok(None);
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if url.is_empty() {
        Ok(None)
    } else {
        Ok(Some(url))
    }
}

/// Create a pull request. Returns the PR URL on success, or an error.
pub fn create_pr(owner: &str, repo: &str, branch: &str, title: &str, body: &str) -> Result<String> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "pr", "create", "--repo", &full_repo, "--head", branch, "--title", title, "--body",
            body,
        ])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to create PR: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr create failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Get a diff stat summary (committed changes vs merge-base of default branch).
///
/// Returns `Ok(Some(stat))` on success, `Ok(None)` if the diff stat cannot be
/// determined (e.g. no merge-base), or `Err` on execution failure.
pub fn diff_stat(worktree_path: &std::path::Path) -> Result<Option<String>> {
    let base = detect_base_branch(worktree_path);
    let output = Command::new("git")
        .args(["diff", "--stat", &format!("{base}...HEAD")])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run git diff --stat: {err}"))
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    let stat = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if stat.is_empty() {
        Ok(None)
    } else {
        Ok(Some(stat))
    }
}

/// Create a pull request using `--body-file` for large body content.
/// Returns the PR URL on success, or an error.
pub fn create_pr_with_body_file(
    owner: &str,
    repo: &str,
    branch: &str,
    title: &str,
    body_file: &std::path::Path,
) -> Result<String> {
    let full_repo = format!("{owner}/{repo}");
    let body_file_str = body_file.to_string_lossy();
    let output = Command::new("gh")
        .args([
            "pr",
            "create",
            "--repo",
            &full_repo,
            "--head",
            branch,
            "--title",
            title,
            "--body-file",
            &body_file_str,
        ])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to create PR: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr create failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Edit an existing PR by URL using `--body-file` for large body content.
pub fn edit_pr(pr_url: &str, title: &str, body_file: &std::path::Path) -> Result<()> {
    let body_file_str = body_file.to_string_lossy();
    let output = Command::new("gh")
        .args([
            "pr",
            "edit",
            pr_url,
            "--title",
            title,
            "--body-file",
            &body_file_str,
        ])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to edit PR: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr edit failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Extract a PR number from a GitHub PR URL.
///
/// Accepts URLs like `https://github.com/owner/repo/pull/123`.
/// Returns `None` if the URL cannot be parsed.
pub fn extract_pr_number(pr_url: &str) -> Option<u32> {
    let parts: Vec<&str> = pr_url.trim_end_matches('/').rsplit('/').collect();
    if parts.len() >= 2 && parts[1] == "pull" {
        parts[0].parse().ok()
    } else {
        None
    }
}

/// Push with `--force-with-lease` from a worktree. Returns `Ok(())` on
/// success, or an error. The caller can inspect the error message for
/// lease rejection (see `is_lease_rejection`).
pub fn push_force_with_lease(worktree_path: &std::path::Path, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["push", "--force-with-lease", "origin", branch])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run git push --force-with-lease: {err}"))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(RalphError::Orchestration(format!(
            "git push --force-with-lease failed for branch {branch}: {stderr}"
        )));
    }

    Ok(())
}

/// Check whether a push error message indicates a lease mismatch
/// (`--force-with-lease` rejection).
pub fn is_lease_rejection(error_msg: &str) -> bool {
    error_msg.contains("stale info")
        || error_msg.contains("[rejected]")
        || error_msg.contains("failed to push")
        || error_msg.contains("fetch first")
}

/// Read the current branch name from a worktree.
///
/// The orchestrator may switch the worktree to a project-specific branch
/// (e.g. `ralph/{project_id}`) during `ralph auto`, so the branch may differ
/// from the one the daemon originally created (`ralph/daemon/{task_id}`).
pub fn current_branch(worktree_path: &std::path::Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to read current branch: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "git rev-parse --abbrev-ref HEAD failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Push the current branch to the remote from a worktree.
pub fn push_branch(worktree_path: &std::path::Path, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["push", "-u", "origin", branch])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to run git push: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "git push failed for branch {}: {}",
            branch,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Returns true when the worktree has an `origin` remote configured.
pub fn has_origin_remote(worktree_path: &std::path::Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to check origin remote: {err}"))
        })?;

    Ok(output.status.success())
}

/// Check whether the task branch has diverged from the base branch.
///
/// First checks for uncommitted working-tree/index changes against HEAD,
/// then checks for committed changes by comparing the merge-base of the
/// default branch with the current HEAD. This ensures that committed changes
/// on the task branch are detected even when the working tree is clean.
pub fn has_diff(worktree_path: &std::path::Path) -> Result<bool> {
    // 1. Check uncommitted changes (working tree + index vs HEAD)
    let wt_status = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .current_dir(worktree_path)
        .status()
        .map_err(|err| RalphError::Orchestration(format!("failed to run git diff: {err}")))?;

    if !wt_status.success() {
        return Ok(true);
    }

    // 2. Detect the default/base branch via symbolic-ref of origin/HEAD,
    //    falling back to common names.
    let base = detect_base_branch(worktree_path);

    // 3. Compare committed changes: merge-base of base..HEAD
    let diff_output = Command::new("git")
        .args(["diff", "--quiet", &format!("{base}...HEAD")])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run git diff against base: {err}"))
        })?;

    if diff_output.status.success() {
        return Ok(false);
    }

    if diff_output.status.code() == Some(1) {
        return Ok(true);
    }

    let stderr = String::from_utf8_lossy(&diff_output.stderr).to_lowercase();
    if is_invalid_revision_error(&stderr) {
        eprintln!(
            "warning: git diff base comparison used invalid/missing revision ({base}...HEAD); treating as no diff"
        );
        return Ok(false);
    }

    Err(RalphError::Orchestration(format!(
        "git diff against base failed for {base}...HEAD: {}",
        String::from_utf8_lossy(&diff_output.stderr).trim()
    )))
}

/// Try to detect the base/default branch for diff comparison.
fn detect_base_branch(worktree_path: &std::path::Path) -> String {
    // Try symbolic-ref of origin/HEAD
    if let Ok(output) = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .current_dir(worktree_path)
        .output()
    {
        if output.status.success() {
            let refname = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !refname.is_empty() {
                return refname;
            }
        }
    }

    // Fallback: try common default branch names
    for candidate in &["origin/main", "origin/master", "main", "master"] {
        let check = Command::new("git")
            .args(["rev-parse", "--verify", candidate])
            .current_dir(worktree_path)
            .output();
        if let Ok(output) = check {
            if output.status.success() {
                return candidate.to_string();
            }
        }
    }

    // Last resort: use HEAD~1 (will show last commit as diff)
    "HEAD~1".to_string()
}

fn is_invalid_revision_error(stderr_lower: &str) -> bool {
    stderr_lower.contains("ambiguous argument")
        || stderr_lower.contains("unknown revision")
        || stderr_lower.contains("bad revision")
        || stderr_lower.contains("not a valid object name")
}

/// Update labels for task completion: remove `ralph:in-progress`, add the
/// given terminal label.
pub fn update_terminal_labels_best_effort(
    owner: &str,
    repo: &str,
    issue_number: u32,
    terminal_label: &str,
) {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "issue",
            "edit",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--remove-label",
            "ralph:in-progress",
            "--add-label",
            terminal_label,
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            eprintln!(
                "warning: failed to update labels for {}#{}: {}",
                full_repo,
                issue_number,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Err(err) => {
            eprintln!(
                "warning: failed to run gh for {}#{} label update: {}",
                full_repo, issue_number, err
            );
        }
    }
}

#[derive(Deserialize)]
struct RawGhIssue {
    number: u32,
    title: String,
    labels: Vec<RawLabel>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}

#[derive(Deserialize)]
struct RawIssueBody {
    title: String,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Deserialize)]
struct RawPrMergeInfo {
    mergeable: String,
    state: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
}

fn parse_issue_list(raw: &str) -> Result<Vec<RawGhIssue>> {
    serde_json::from_str(raw).map_err(|err| {
        RalphError::Orchestration(format!("failed to parse gh issue list output: {err}"))
    })
}

fn parse_pr_merge_info(raw: &str) -> Result<PrMergeInfo> {
    let parsed: RawPrMergeInfo = serde_json::from_str(raw).map_err(|err| {
        RalphError::Orchestration(format!("failed to parse gh pr view output: {err}"))
    })?;
    Ok(PrMergeInfo {
        merge_status: PrMergeStatus::from_gh_mergeable(&parsed.mergeable)?,
        state: parsed.state,
        base_branch: parsed.base_ref_name,
        head_oid: parsed.head_ref_oid,
    })
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::tempdir;

    use super::{
        has_diff, is_invalid_revision_error, parse_issue_list, parse_pr_merge_info, GhIssue,
        PrMergeStatus,
    };

    #[test]
    fn gh_issue_deserialization_supports_body_present() {
        let raw = r#"[{"number":1,"title":"one","labels":[],"body":"details"}]"#;
        let items = parse_issue_list(raw).expect("should deserialize");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].body.as_deref(), Some("details"));
    }

    #[test]
    fn gh_issue_deserialization_supports_body_null() {
        let raw = r#"[{"number":1,"title":"one","labels":[],"body":null}]"#;
        let items = parse_issue_list(raw).expect("should deserialize");
        assert_eq!(items.len(), 1);
        assert!(items[0].body.is_none());
    }

    #[test]
    fn gh_issue_deserialization_supports_body_absent() {
        let raw = r#"[{"number":1,"title":"one","labels":[]}]"#;
        let items = parse_issue_list(raw).expect("should deserialize");
        assert_eq!(items.len(), 1);
        assert!(items[0].body.is_none());
    }

    #[test]
    fn gh_issue_struct_includes_body_field() {
        let issue = GhIssue {
            number: 1,
            title: "title".to_owned(),
            labels: Vec::new(),
            body: Some("body".to_owned()),
        };
        assert_eq!(issue.body.as_deref(), Some("body"));
    }

    #[test]
    fn invalid_revision_patterns_are_detected() {
        assert!(is_invalid_revision_error(
            "fatal: ambiguous argument 'HEAD~1...HEAD': unknown revision"
        ));
        assert!(is_invalid_revision_error("fatal: bad revision 'foo'"));
    }

    #[test]
    fn has_diff_returns_false_for_single_commit_head_tilde_base() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path();

        git(repo, &["init"]);
        git(repo, &["config", "user.email", "test@example.com"]);
        git(repo, &["config", "user.name", "Test"]);

        std::fs::write(repo.join("README.md"), "hello\n").expect("write readme");
        git(repo, &["add", "README.md"]);
        git(repo, &["commit", "-m", "initial"]);

        let diff = has_diff(repo).expect("has_diff should not error for invalid base revision");
        assert!(
            !diff,
            "single-commit fallback HEAD~1 should be treated as no diff"
        );
    }

    #[test]
    fn pr_merge_info_parses_expected_fields() {
        let raw = r#"{
            "mergeable":"MERGEABLE",
            "state":"OPEN",
            "baseRefName":"main",
            "headRefOid":"abc123"
        }"#;
        let info = parse_pr_merge_info(raw).expect("pr merge info should parse");
        assert_eq!(info.merge_status, PrMergeStatus::Mergeable);
        assert_eq!(info.state, "OPEN");
        assert_eq!(info.base_branch, "main");
        assert_eq!(info.head_oid, "abc123");
    }

    #[test]
    fn pr_merge_status_maps_conflicting() {
        let raw = r#"{
            "mergeable":"CONFLICTING",
            "state":"OPEN",
            "baseRefName":"main",
            "headRefOid":"abc123"
        }"#;
        let info = parse_pr_merge_info(raw).expect("pr merge info should parse");
        assert_eq!(info.merge_status, PrMergeStatus::Conflicting);
    }

    #[test]
    fn extract_pr_number_from_standard_url() {
        assert_eq!(
            super::extract_pr_number("https://github.com/acme/widgets/pull/42"),
            Some(42)
        );
    }

    #[test]
    fn extract_pr_number_from_trailing_slash() {
        assert_eq!(
            super::extract_pr_number("https://github.com/acme/widgets/pull/7/"),
            Some(7)
        );
    }

    #[test]
    fn extract_pr_number_returns_none_for_non_pr_url() {
        assert_eq!(
            super::extract_pr_number("https://github.com/acme/widgets/issues/5"),
            None
        );
    }

    #[test]
    fn is_lease_rejection_detects_stale_info() {
        assert!(super::is_lease_rejection("stale info"));
    }

    #[test]
    fn is_lease_rejection_detects_rejected() {
        assert!(super::is_lease_rejection("[rejected]"));
    }

    #[test]
    fn is_lease_rejection_returns_false_for_unrelated() {
        assert!(!super::is_lease_rejection("network timeout"));
    }

    #[test]
    fn pr_merge_status_maps_unknown() {
        let raw = r#"{
            "mergeable":"UNKNOWN",
            "state":"OPEN",
            "baseRefName":"main",
            "headRefOid":"abc123"
        }"#;
        let info = parse_pr_merge_info(raw).expect("pr merge info should parse");
        assert_eq!(info.merge_status, PrMergeStatus::Unknown);
    }

    fn git(repo_root: &std::path::Path, args: &[&str]) {
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
}
