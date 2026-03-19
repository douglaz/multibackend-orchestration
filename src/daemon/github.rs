use std::time::Duration;

use tokio::process::Command;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::RalphError;
use crate::Result;

/// Lifecycle labels recognized by the daemon runtime.
///
/// Spec-defined set: ready, in-progress, completed, failed.
pub const LIFECYCLE_LABELS: &[&str] = &[
    "ralph:ready",
    "ralph:in-progress",
    "ralph:completed",
    "ralph:failed",
];

/// Maximum retry attempts for label mutations (conflict/transient).
const LABEL_RETRY_MAX: u32 = 3;

/// Base delay between label mutation retries (doubles each attempt).
const LABEL_RETRY_BASE_DELAY_MS: u64 = 500;
const DEFAULT_GH_BIN: &str = "gh";

pub const REQUIRED_LABELS: &[(&str, &str, &str)] = &[
    (
        "ralph:ready",
        "#0e8a16",
        "Issue is ready for Ralph daemon pickup",
    ),
    (
        "ralph:in-progress",
        "#fbca04",
        "Ralph daemon is working on this issue",
    ),
    (
        "ralph:completed",
        "#1d76db",
        "Ralph daemon completed this issue",
    ),
    ("ralph:failed", "#d93f0b", "Ralph daemon task failed"),
    ("ralph:quick", "#5319e7", "Use quick-dev orchestration flow"),
];

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPrInfo {
    pub number: u32,
    pub head_sha: String,
    pub author: String,
}

/// Poll open issues matching all supplied labels.
///
/// Uses `gh issue list --repo <owner/repo> --limit 100 --json number,title,labels,body`
/// with repeated `--label` arguments (AND semantics).
///
/// Returns `(issues, overflow)` where overflow is true when exactly 100 issues
/// were returned, indicating possible truncation.
pub async fn poll_issues(
    owner: &str,
    repo: &str,
    labels: &[String],
) -> Result<(Vec<GhIssue>, bool)> {
    poll_issues_with_gh_bin(DEFAULT_GH_BIN, owner, repo, labels).await
}

pub async fn poll_issues_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    labels: &[String],
) -> Result<(Vec<GhIssue>, bool)> {
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

    let output = Command::new(gh_bin)
        .args(&args)
        .output()
        .await
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
pub async fn fetch_issue_body(
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
        .await
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
pub async fn query_pr_merge_info(owner: &str, repo: &str, pr_number: u32) -> Result<PrMergeInfo> {
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
        .await
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

pub async fn list_open_non_draft_prs(
    owner: &str,
    repo: &str,
    gh_bin: &str,
) -> Result<(Vec<OpenPrInfo>, bool)> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new(gh_bin)
        .args([
            "pr",
            "list",
            "--repo",
            &full_repo,
            "--state",
            "open",
            "--json",
            "number,headRefOid,isDraft,author",
            "--limit",
            "100",
        ])
        .output()
        .await
        .map_err(|err| RalphError::Orchestration(format!("failed to run gh pr list: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr list failed for {full_repo}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    parse_open_prs(String::from_utf8_lossy(&output.stdout).trim())
}

pub async fn fetch_pr_diff(
    owner: &str,
    repo: &str,
    pr_number: u32,
    gh_bin: &str,
) -> Result<String> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new(gh_bin)
        .args(["pr", "diff", &pr_number.to_string(), "--repo", &full_repo])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run gh pr diff for {full_repo}#{pr_number}: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr diff failed for {full_repo}#{pr_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
pub async fn claim_issue(owner: &str, repo: &str, issue_number: u32) -> Result<()> {
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
        .await
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

/// Release a previously-claimed issue by removing the `ralph:in-progress`
/// label.
pub async fn release_claim(owner: &str, repo: &str, issue_number: u32) -> Result<()> {
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
        ])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run gh issue edit for claim release: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue edit (claim release) failed for {}#{}: {}",
            full_repo,
            issue_number,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Update the title of a GitHub issue.
pub async fn update_issue_title(
    owner: &str,
    repo: &str,
    issue_number: u32,
    title: &str,
) -> Result<()> {
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
        .await
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

/// Update the body of a GitHub issue.
pub async fn update_issue_body(
    owner: &str,
    repo: &str,
    issue_number: u32,
    body: &str,
) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "issue",
            "edit",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--body",
            body,
        ])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run gh issue edit --body: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue edit --body failed for {}#{}: {}",
            full_repo,
            issue_number,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Check whether a comment with the given marker already exists on the issue.
pub async fn comment_marker_exists(
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
        .await
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
pub async fn post_idempotent_comment(
    owner: &str,
    repo: &str,
    issue_number: u32,
    task_id: &str,
    phase: &str,
    body_text: &str,
) -> Result<()> {
    let marker = format!("<!-- ralph:task:{task_id}:{phase} -->");
    if comment_marker_exists(owner, repo, issue_number, &marker).await? {
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
        .await
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
pub async fn post_pr_comment(owner: &str, repo: &str, pr_number: u32, body: &str) -> Result<()> {
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
        .await
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

/// Post a raw comment on an issue without any idempotency check.
///
/// This is used when bot identity is unavailable and body-only marker lookup
/// would be vulnerable to user spoofing.  Callers should include any marker
/// text in `body` themselves.
pub async fn post_raw_issue_comment(
    owner: &str,
    repo: &str,
    issue_number: u32,
    body: &str,
) -> Result<()> {
    post_raw_issue_comment_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number, body).await
}

pub async fn post_raw_issue_comment_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    body: &str,
) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new(gh_bin)
        .args([
            "issue",
            "comment",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--body",
            body,
        ])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to post raw comment on {full_repo}#{issue_number}: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue comment (raw) failed for {full_repo}#{issue_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Check for an existing PR with the given head branch.
/// Returns `Some(url)` if found, `None` otherwise.
pub async fn find_existing_pr(owner: &str, repo: &str, branch: &str) -> Result<Option<String>> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new("gh")
        .args([
            "pr", "list", "--repo", &full_repo, "--head", branch, "--json", "url", "-q", ".[0].url",
        ])
        .output()
        .await
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
pub async fn create_pr(
    owner: &str,
    repo: &str,
    branch: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<String> {
    let full_repo = format!("{owner}/{repo}");
    let mut args = vec![
        "pr", "create", "--repo", &full_repo, "--head", branch, "--title", title, "--body", body,
    ];
    if draft {
        args.push("--draft");
    }
    let output = Command::new("gh")
        .args(&args)
        .output()
        .await
        .map_err(|err| RalphError::Orchestration(format!("failed to create PR: {err}")))?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr create failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Returns true when HEAD is ahead of `base_branch` by one or more commits.
///
/// Resolves the comparison ref robustly: prefers `origin/{base_branch}` if it
/// exists, otherwise falls back to `detect_base_branch` (which tries
/// `origin/HEAD`, common default branch names, etc.).  Returns a typed error
/// when no valid base ref can be resolved.
pub async fn has_commits_ahead_of_base(
    worktree_path: &std::path::Path,
    base_branch: &str,
) -> Result<bool> {
    let base = resolve_ahead_base(worktree_path, base_branch).await?;
    let range = format!("{base}..HEAD");
    let output = Command::new("git")
        .args(["rev-list", "--count", &range])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run git rev-list --count {range}: {err}"))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RalphError::Orchestration(format!(
            "git rev-list --count {range} failed (resolved base: {base}): {}",
            stderr.trim()
        )));
    }

    let count_raw = String::from_utf8_lossy(&output.stdout);
    let count = count_raw.trim().parse::<u64>().map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to parse git rev-list --count output '{value}' for {range}: {err}",
            value = count_raw.trim()
        ))
    })?;

    Ok(count > 0)
}

/// Resolve a valid base ref for ahead-of-base comparison.
///
/// Tries `origin/{base_branch}` first, then falls back to auto-detection via
/// `detect_base_branch`.  Returns an error only if no resolvable base ref can
/// be found at all.
async fn resolve_ahead_base(
    worktree_path: &std::path::Path,
    configured_base: &str,
) -> Result<String> {
    // Try the configured base as a remote ref first.
    let candidate = format!("origin/{configured_base}");
    let check = Command::new("git")
        .args(["rev-parse", "--verify", &candidate])
        .current_dir(worktree_path)
        .output()
        .await;
    if check.map(|o| o.status.success()).unwrap_or(false) {
        return Ok(candidate);
    }

    // Configured base not found as remote ref — fall back to auto-detection.
    let detected = detect_base_branch(worktree_path).await;

    // Verify the detected ref actually resolves.
    let verify = Command::new("git")
        .args(["rev-parse", "--verify", &detected])
        .current_dir(worktree_path)
        .output()
        .await;
    if verify.map(|o| o.status.success()).unwrap_or(false) {
        return Ok(detected);
    }

    Err(RalphError::Orchestration(format!(
        "cannot resolve base branch for ahead-of-base check: configured '{configured_base}' \
         (tried origin/{configured_base}) and auto-detected '{detected}' are both unresolvable"
    )))
}

/// Mark a draft pull request as ready for review.
pub async fn mark_pr_ready(owner: &str, repo: &str, pr_number: u32) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let pr_number_str = pr_number.to_string();
    let output = Command::new("gh")
        .args(["pr", "ready", &pr_number_str, "--repo", &full_repo])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run gh pr ready for {full_repo}#{pr_number}: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr ready failed for {full_repo}#{pr_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Return whether a pull request is currently a draft.
pub async fn is_pr_draft(owner: &str, repo: &str, pr_number: u32) -> Result<bool> {
    let full_repo = format!("{owner}/{repo}");
    let pr_number_str = pr_number.to_string();
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number_str,
            "--repo",
            &full_repo,
            "--json",
            "isDraft",
        ])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run gh pr view for draft state on {full_repo}#{pr_number}: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr view (isDraft) failed for {full_repo}#{pr_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    parse_pr_is_draft(raw.trim())
}

/// Close a pull request.
pub async fn close_pr(owner: &str, repo: &str, pr_number: u32) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let pr_number_str = pr_number.to_string();
    let output = Command::new("gh")
        .args(["pr", "close", &pr_number_str, "--repo", &full_repo])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run gh pr close for {full_repo}#{pr_number}: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr close failed for {full_repo}#{pr_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(())
}

/// Get a diff stat summary (committed changes vs merge-base of default branch).
///
/// Returns `Ok(Some(stat))` on success, `Ok(None)` if the diff stat cannot be
/// determined (e.g. no merge-base), or `Err` on execution failure.
pub async fn diff_stat(worktree_path: &std::path::Path) -> Result<Option<String>> {
    let base = detect_base_branch(worktree_path).await;
    let output = Command::new("git")
        .args(["diff", "--stat", &format!("{base}...HEAD")])
        .current_dir(worktree_path)
        .output()
        .await
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
pub async fn create_pr_with_body_file(
    owner: &str,
    repo: &str,
    branch: &str,
    title: &str,
    body_file: &std::path::Path,
    base_branch: Option<&str>,
    draft: bool,
) -> Result<String> {
    let full_repo = format!("{owner}/{repo}");
    let body_file_str = body_file.to_string_lossy();
    let mut args = vec![
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
    ];
    let base_owned;
    if let Some(base) = base_branch {
        base_owned = base.to_owned();
        args.push("--base");
        args.push(&base_owned);
    }
    if draft {
        args.push("--draft");
    }
    let output = Command::new("gh")
        .args(&args)
        .output()
        .await
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
pub async fn edit_pr(pr_url: &str, title: &str, body_file: &std::path::Path) -> Result<()> {
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
        .await
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
pub async fn push_force_with_lease(worktree_path: &std::path::Path, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["push", "--force-with-lease", "origin", branch])
        .current_dir(worktree_path)
        .output()
        .await
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
/// The worktree is created on the project branch (e.g. `ralph/issue-{N}`)
/// and `sync_project_branch` keeps it there throughout the task lifecycle.
pub async fn current_branch(worktree_path: &std::path::Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .await
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
pub async fn push_branch(worktree_path: &std::path::Path, branch: &str) -> Result<()> {
    push_branch_with_git_bin("git", worktree_path, branch)
        .await
        .map_err(|stderr| {
            RalphError::Orchestration(format!("git push failed for branch {branch}: {stderr}"))
        })
}

/// Determine whether raw `git push` stderr indicates a transient failure that
/// should be retried.
///
/// HTTP status codes are only recognized in explicit transport-error context
/// (e.g. `HTTP 503`, `returned error: 503`) so that numeric substrings inside
/// URLs or repository paths (like `/repo-403/`) do not influence classification.
pub fn is_retryable_push_stderr(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();

    // 1. Text-based permanent (non-retryable) patterns — no numeric collision risk.
    let permanent_text_patterns = [
        "permission denied",
        "authentication",
        "non-fast-forward",
        "protected branch",
        "denied",
        "forbidden",
        "gh013",
        "repository rule violation",
        "repository not found",
    ];
    if permanent_text_patterns
        .iter()
        .any(|pattern| lower.contains(pattern))
    {
        return false;
    }

    // 2. Extract HTTP status codes from bounded transport-error context only.
    let codes = extract_http_status_codes(&lower);

    // Non-retryable HTTP codes (auth/permission/policy).
    if codes.iter().any(|&c| c == 401 || c == 403) {
        return false;
    }

    // Retryable HTTP codes (server errors).
    if codes.iter().any(|&c| (500..600).contains(&c)) {
        return true;
    }

    // 3. Text-based transient (retryable) patterns.
    let transient_text_patterns = [
        "timeout",
        "timed out",
        "connection refused",
        "connection reset",
        "network",
        "dns",
        "resolve host",
        "could not resolve",
        "unable to access",
    ];
    if transient_text_patterns
        .iter()
        .any(|pattern| lower.contains(pattern))
    {
        return true;
    }

    // Unknown failures are treated as non-retryable so permanent failures do
    // not get delayed by retries.
    false
}

/// Extract HTTP status codes from explicit transport-error context in stderr.
///
/// Recognizes bounded patterns such as `HTTP 503` and `returned error: 503`
/// (the two forms used by git/curl transport errors).  Bare numeric substrings
/// inside URLs or paths are intentionally ignored.
fn extract_http_status_codes(lower_stderr: &str) -> Vec<u16> {
    let mut codes = Vec::new();
    for prefix in &["http ", "returned error: "] {
        let mut search_from = 0;
        while search_from < lower_stderr.len() {
            let haystack = &lower_stderr[search_from..];
            let Some(pos) = haystack.find(prefix) else {
                break;
            };
            let after = pos + prefix.len();
            if let Some(code) = parse_bounded_3digit_code(&haystack[after..]) {
                codes.push(code);
            }
            search_from += after;
        }
    }
    codes
}

/// Parse a 3-digit HTTP status code at the start of `s`, ensuring it is not
/// part of a longer numeric token (e.g. `5031` should not match as `503`).
fn parse_bounded_3digit_code(s: &str) -> Option<u16> {
    let bytes = s.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    if !bytes[0].is_ascii_digit() || !bytes[1].is_ascii_digit() || !bytes[2].is_ascii_digit() {
        return None;
    }
    // Reject if followed by another digit (not a bounded 3-digit code).
    if bytes.len() > 3 && bytes[3].is_ascii_digit() {
        return None;
    }
    let digits = &s[..3];
    digits.parse().ok()
}

/// Determine whether a [`RalphError`] from a git push operation represents a
/// transient failure that should be retried.
///
/// Extracts the git push stderr from the structured error message and
/// classifies it, ensuring branch names (which may contain numeric codes
/// like `403` or `500`) do not influence the decision.
pub fn is_retryable_push_error(err: &RalphError) -> bool {
    let message = err.to_string();
    // Extract stderr from the canonical "git push failed for branch <branch>: <stderr>" format.
    let Some(prefix_start) = message.find("git push failed for branch ") else {
        return false;
    };
    let after_prefix = &message[prefix_start + "git push failed for branch ".len()..];
    // Skip past the branch name to find the ": " delimiter before stderr.
    let Some(colon_pos) = after_prefix.find(": ") else {
        return false;
    };
    let stderr = &after_prefix[colon_pos + 2..];
    is_retryable_push_stderr(stderr)
}

/// Push the current branch with bounded retry for transient failures.
pub async fn push_branch_with_retry(worktree_path: &std::path::Path, branch: &str) -> Result<()> {
    push_branch_with_retry_impl("git", worktree_path, branch, &[10, 20, 40]).await
}

async fn push_branch_with_retry_impl(
    git_bin: &str,
    worktree_path: &std::path::Path,
    branch: &str,
    delays_secs: &[u64],
) -> Result<()> {
    let total_attempts = delays_secs.len() + 1;
    for (attempt_index, delay_secs) in delays_secs
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .enumerate()
    {
        match push_branch_with_git_bin(git_bin, worktree_path, branch).await {
            Ok(()) => return Ok(()),
            Err(stderr) => {
                let attempt = attempt_index + 1;
                let err = RalphError::Orchestration(format!(
                    "git push failed for branch {branch}: {stderr}"
                ));
                if !is_retryable_push_error(&err) || attempt == total_attempts {
                    return Err(err);
                }
                eprintln!(
                    "push-retry: push failed for branch {branch} in {} (attempt {attempt}/{total_attempts}), retrying in {delay_secs}s: {stderr}",
                    worktree_path.display()
                );
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            }
        }
    }

    unreachable!()
}

async fn push_branch_with_git_bin(
    git_bin: &str,
    worktree_path: &std::path::Path,
    branch: &str,
) -> std::result::Result<(), String> {
    let output = Command::new(git_bin)
        .args(["push", "-u", "origin", branch])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|err| format!("failed to run git push: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(stderr);
    }

    Ok(())
}

/// Returns true when the worktree has an `origin` remote configured.
pub async fn has_origin_remote(worktree_path: &std::path::Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(worktree_path)
        .output()
        .await
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
pub async fn has_diff(worktree_path: &std::path::Path) -> Result<bool> {
    has_diff_with_base(worktree_path, None).await
}

/// Check whether the task branch has diverged from the given base branch
/// (or an auto-detected default if `base_branch` is `None`).
pub async fn has_diff_with_base(
    worktree_path: &std::path::Path,
    base_branch: Option<&str>,
) -> Result<bool> {
    // 1. Check uncommitted changes (working tree + index vs HEAD)
    let wt_status = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .current_dir(worktree_path)
        .status()
        .await
        .map_err(|err| RalphError::Orchestration(format!("failed to run git diff: {err}")))?;

    if !wt_status.success() {
        return Ok(true);
    }

    // 2. Use the provided base branch, or auto-detect via symbolic-ref of
    //    origin/HEAD falling back to common names.
    // If the provided base doesn't exist as a remote ref, fall back to
    // auto-detection so we don't falsely report "no diff".
    let base = match base_branch {
        Some(b) => {
            let candidate = format!("origin/{b}");
            let check = Command::new("git")
                .args(["rev-parse", "--verify", &candidate])
                .current_dir(worktree_path)
                .output()
                .await;
            if check.map(|o| o.status.success()).unwrap_or(false) {
                candidate
            } else {
                detect_base_branch(worktree_path).await
            }
        }
        None => detect_base_branch(worktree_path).await,
    };

    // 3. Compare committed changes: merge-base of base..HEAD
    let diff_output = Command::new("git")
        .args(["diff", "--quiet", &format!("{base}...HEAD")])
        .current_dir(worktree_path)
        .output()
        .await
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
async fn detect_base_branch(worktree_path: &std::path::Path) -> String {
    // Try symbolic-ref of origin/HEAD
    if let Ok(output) = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .current_dir(worktree_path)
        .output()
        .await
    {
        if output.status.success() {
            let refname = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            // Skip ralph/* branches — on new repos GitHub may set the only
            // pushed branch (a project branch) as the default, which would
            // cause us to diff the project branch against itself.
            let branch_name = refname
                .strip_prefix("refs/remotes/origin/")
                .unwrap_or(&refname);
            if !refname.is_empty() && !branch_name.starts_with("ralph/") {
                return refname;
            }
        }
    }

    // Fallback: try common default branch names
    for candidate in &["origin/main", "origin/master", "main", "master"] {
        let check = Command::new("git")
            .args(["rev-parse", "--verify", candidate])
            .current_dir(worktree_path)
            .output()
            .await;
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
pub async fn update_terminal_labels_best_effort(
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
        .output()
        .await;

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

/// Ensure required lifecycle labels exist in the repository.
///
/// This is best-effort and intentionally non-failing: startup must continue
/// even when label creation fails.
pub async fn ensure_labels_best_effort(owner: &str, repo: &str) {
    ensure_labels_best_effort_with_gh_bin(DEFAULT_GH_BIN, owner, repo).await;
}

pub async fn ensure_labels_best_effort_with_gh_bin(gh_bin: &str, owner: &str, repo: &str) {
    let full_repo = format!("{owner}/{repo}");

    for (name, color, description) in REQUIRED_LABELS {
        let output = Command::new(gh_bin)
            .args([
                "label",
                "create",
                name,
                "--repo",
                &full_repo,
                "--color",
                color,
                "--description",
                description,
            ])
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}\n{stderr}");
                if combined.to_ascii_lowercase().contains("already exists") {
                    continue;
                }

                let detail = stderr.trim();
                let detail = if detail.is_empty() {
                    stdout.trim()
                } else {
                    detail
                };
                eprintln!(
                    "warning: failed to ensure label '{}' for {}: {}",
                    name, full_repo, detail
                );
            }
            Err(err) => {
                eprintln!(
                    "warning: failed to run gh label create for '{}' in {}: {}",
                    name, full_repo, err
                );
            }
        }
    }
}

// =============================================================================
// Lifecycle Label Classification and Normalization
// =============================================================================

/// Classify which lifecycle labels are present on an issue.
pub fn classify_lifecycle_labels(labels: &[String]) -> Vec<String> {
    labels
        .iter()
        .filter(|l| LIFECYCLE_LABELS.contains(&l.as_str()))
        .cloned()
        .collect()
}

/// Normalize an issue with multiple lifecycle labels: remove all lifecycle labels
/// and apply only `ralph:failed`. Returns `true` if normalization was performed.
///
/// Spec: "If issue has more than one lifecycle label, normalize to only
/// `ralph:failed` and skip processing this poll cycle."
pub async fn normalize_multi_lifecycle_labels(
    owner: &str,
    repo: &str,
    issue_number: u32,
    lifecycle_labels: &[String],
) -> Result<bool> {
    if lifecycle_labels.len() <= 1 {
        return Ok(false);
    }

    // Remove all lifecycle labels, then add ralph:failed
    for label in lifecycle_labels {
        if label == "ralph:failed" {
            continue;
        }
        remove_label_with_retry(owner, repo, issue_number, label).await?;
    }

    // Ensure ralph:failed is present
    if !lifecycle_labels.iter().any(|l| l == "ralph:failed") {
        add_label_with_retry(owner, repo, issue_number, "ralph:failed").await?;
    }

    Ok(true)
}

/// Error returned when a lifecycle label swap fails.
///
/// Provides context about whether the original label was restored after a
/// partial failure (remove succeeded but add failed).
#[derive(Debug)]
pub struct SwapLabelError {
    /// The underlying error that caused the swap to fail.
    pub error: RalphError,
    /// Whether the original `from_label` was restored after a partial failure.
    /// - `None`: the remove step failed (original label still present, no rollback needed).
    /// - `Some(true)`: the add step failed but the original label was successfully re-added.
    /// - `Some(false)`: the add step failed and the rollback also failed (label may be missing).
    pub from_label_restored: Option<bool>,
}

impl std::fmt::Display for SwapLabelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl From<SwapLabelError> for RalphError {
    fn from(e: SwapLabelError) -> Self {
        e.error
    }
}

/// Swap lifecycle labels atomically with retry-on-conflict and retry-on-transient.
///
/// Removes `from_label` and adds `to_label`. Both operations are retried
/// individually with bounded attempts and exponential backoff.
///
/// If the add step fails after a successful remove, a best-effort rollback
/// re-adds `from_label`. The returned [`SwapLabelError`] indicates whether
/// the rollback succeeded via `from_label_restored`.
pub async fn swap_lifecycle_label(
    owner: &str,
    repo: &str,
    issue_number: u32,
    from_label: &str,
    to_label: &str,
) -> std::result::Result<(), SwapLabelError> {
    if let Err(error) = remove_label_with_retry(owner, repo, issue_number, from_label).await {
        return Err(SwapLabelError {
            error,
            from_label_restored: None, // remove failed, original label still present
        });
    }
    if let Err(error) = add_label_with_retry(owner, repo, issue_number, to_label).await {
        // Best-effort rollback: try to re-add the original label.
        let restored = add_label_with_retry(owner, repo, issue_number, from_label)
            .await
            .is_ok();
        if !restored {
            eprintln!(
                "warning: swap_lifecycle_label rollback failed — \
                 issue {owner}/{repo}#{issue_number} may be missing lifecycle label {from_label}"
            );
        }
        return Err(SwapLabelError {
            error,
            from_label_restored: Some(restored),
        });
    }
    Ok(())
}

/// Add a label with retry-on-conflict/transient-failure behavior.
pub async fn add_label_with_retry(
    owner: &str,
    repo: &str,
    issue_number: u32,
    label: &str,
) -> Result<()> {
    add_label_with_retry_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number, label).await
}

pub async fn add_label_with_retry_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    label: &str,
) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    for attempt in 0..LABEL_RETRY_MAX {
        let output = Command::new(gh_bin)
            .args([
                "issue",
                "edit",
                &issue_number.to_string(),
                "--repo",
                &full_repo,
                "--add-label",
                label,
            ])
            .output()
            .await
            .map_err(|err| {
                RalphError::Orchestration(format!(
                    "failed to run gh issue edit --add-label {label}: {err}"
                ))
            })?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if attempt + 1 < LABEL_RETRY_MAX && is_retryable_gh_error(&stderr) {
            let delay = Duration::from_millis(LABEL_RETRY_BASE_DELAY_MS << attempt);
            eprintln!(
                "label-retry: add-label {label} on {full_repo}#{issue_number} failed (attempt {}), retrying in {}ms: {}",
                attempt + 1,
                delay.as_millis(),
                stderr.trim()
            );
            tokio::time::sleep(delay).await;
            continue;
        }

        return Err(RalphError::Orchestration(format!(
            "gh issue edit --add-label {label} failed for {full_repo}#{issue_number} after {} attempts: {}",
            attempt + 1,
            stderr.trim()
        )));
    }
    unreachable!()
}

/// Remove a label with retry-on-conflict/transient-failure behavior.
pub async fn remove_label_with_retry(
    owner: &str,
    repo: &str,
    issue_number: u32,
    label: &str,
) -> Result<()> {
    remove_label_with_retry_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number, label).await
}

pub async fn remove_label_with_retry_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    label: &str,
) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    for attempt in 0..LABEL_RETRY_MAX {
        let output = Command::new(gh_bin)
            .args([
                "issue",
                "edit",
                &issue_number.to_string(),
                "--repo",
                &full_repo,
                "--remove-label",
                label,
            ])
            .output()
            .await
            .map_err(|err| {
                RalphError::Orchestration(format!(
                    "failed to run gh issue edit --remove-label {label}: {err}"
                ))
            })?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if attempt + 1 < LABEL_RETRY_MAX && is_retryable_gh_error(&stderr) {
            let delay = Duration::from_millis(LABEL_RETRY_BASE_DELAY_MS << attempt);
            eprintln!(
                "label-retry: remove-label {label} on {full_repo}#{issue_number} failed (attempt {}), retrying in {}ms: {}",
                attempt + 1,
                delay.as_millis(),
                stderr.trim()
            );
            tokio::time::sleep(delay).await;
            continue;
        }

        return Err(RalphError::Orchestration(format!(
            "gh issue edit --remove-label {label} failed for {full_repo}#{issue_number} after {} attempts: {}",
            attempt + 1,
            stderr.trim()
        )));
    }
    unreachable!()
}

/// Fetch the current lifecycle labels for an issue from GitHub.
pub async fn fetch_issue_labels(owner: &str, repo: &str, issue_number: u32) -> Result<Vec<String>> {
    fetch_issue_labels_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number).await
}

pub async fn fetch_issue_labels_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
) -> Result<Vec<String>> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new(gh_bin)
        .args([
            "issue",
            "view",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--json",
            "labels",
        ])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run gh issue view for labels: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue view (labels) failed for {full_repo}#{issue_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let parsed: RawIssueLabels = serde_json::from_str(raw.trim()).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to parse gh issue view labels output: {err}"
        ))
    })?;
    Ok(parsed.labels.into_iter().map(|l| l.name).collect())
}

/// Determine if a `gh` CLI error is transient/retryable (rate limit, network,
/// server error, or conflict).
pub fn is_retryable_gh_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("rate limit")
        || lower.contains("api rate")
        || lower.contains("409")
        || lower.contains("conflict")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("timeout")
        || lower.contains("connection")
        || lower.contains("network")
        || lower.contains("could not resolve")
}

#[derive(Deserialize)]
struct RawIssueComments {
    #[serde(default)]
    comments: Vec<RawComment>,
}

#[derive(Deserialize)]
struct RawComment {
    #[serde(default, deserialize_with = "deserialize_comment_id")]
    id: Option<u64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    author: Option<RawCommentAuthor>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct RawCommentAuthor {
    login: String,
}

/// Extract a numeric comment ID from a GitHub comment URL.
///
/// Handles two URL formats:
/// - GraphQL: `https://github.com/…#issuecomment-3954637090`
/// - REST:    `https://api.github.com/repos/…/comments/3954637090`
fn extract_numeric_comment_id_from_url(url: Option<&str>) -> Option<u64> {
    let url = url?;
    // GraphQL URL: …#issuecomment-NNNNN
    if let Some(fragment) = url.rsplit_once('#') {
        if let Some(id_str) = fragment.1.strip_prefix("issuecomment-") {
            if let Ok(id) = id_str.parse::<u64>() {
                return Some(id);
            }
        }
    }
    // REST URL: …/comments/NNNNN
    if let Some(last_segment) = url.rsplit_once('/') {
        if let Ok(id) = last_segment.1.parse::<u64>() {
            return Some(id);
        }
    }
    None
}

/// Deserialize a comment ID that may be a numeric u64 (REST API / mocks)
/// or a string node ID (GraphQL API via `gh issue view --json`).
/// String node IDs are skipped (return None) — the caller should prefer
/// the numeric ID extracted from the `url` field instead.
fn deserialize_comment_id<'de, D>(deserializer: D) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    use std::fmt;

    struct CommentIdVisitor;

    impl<'de> de::Visitor<'de> for CommentIdVisitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a u64 or string comment ID")
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
            Ok(Some(v as u64))
        }

        fn visit_str<E: de::Error>(self, _v: &str) -> std::result::Result<Self::Value, E> {
            // GraphQL node IDs like "IC_kwDORMeVKs7q9rJD" are not numeric.
            // Return None — the caller extracts the numeric ID from the URL field.
            Ok(None)
        }

        fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
    }

    deserializer.deserialize_any(CommentIdVisitor)
}

#[derive(Deserialize)]
struct RawIssueLabels {
    labels: Vec<RawLabel>,
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

#[derive(Deserialize)]
struct RawPrDraftInfo {
    #[serde(rename = "isDraft")]
    is_draft: bool,
}

#[derive(Deserialize)]
struct RawOpenPrInfo {
    number: u32,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(default)]
    author: Option<RawAuthorLogin>,
}

#[derive(Deserialize)]
struct RawAuthorLogin {
    login: String,
}

/// A structured issue comment returned by [`fetch_issue_comments`].
#[derive(Debug, Clone)]
pub struct IssueComment {
    pub id: u64,
    pub author_login: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

/// Distinguishes whether a marker-based comment post was skipped, posted, or
/// failed before GitHub accepted the comment.
#[derive(Debug)]
pub enum PostCommentOutcome {
    AlreadyExists(IssueComment),
    Posted,
    PostFailed(RalphError),
}

/// Fetch all comments on an issue as structured data.
///
/// Returns a list of [`IssueComment`] in chronological order.
pub async fn fetch_issue_comments(
    owner: &str,
    repo: &str,
    issue_number: u32,
) -> Result<Vec<IssueComment>> {
    fetch_issue_comments_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number).await
}

pub async fn fetch_issue_comments_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
) -> Result<Vec<IssueComment>> {
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new(gh_bin)
        .args([
            "issue",
            "view",
            &issue_number.to_string(),
            "--repo",
            &full_repo,
            "--json",
            "comments",
        ])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to fetch issue comments for {full_repo}#{issue_number}: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue view (comments) failed for {full_repo}#{issue_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let raw_trimmed = raw.trim();
    if raw_trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let parsed: RawIssueComments = serde_json::from_str(raw_trimmed).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to parse issue comments for {full_repo}#{issue_number}: {err}"
        ))
    })?;

    let mut comments: Vec<IssueComment> = parsed
        .comments
        .into_iter()
        .filter_map(|raw_comment| {
            // Prefer numeric ID extracted from the URL field (works with
            // both GraphQL and REST responses).  The `url` field from
            // `gh issue view --json comments` looks like:
            //   https://github.com/…#issuecomment-3954637090
            // The REST API `url` looks like:
            //   https://api.github.com/repos/…/comments/3954637090
            let id = extract_numeric_comment_id_from_url(raw_comment.url.as_deref())
                .or(raw_comment.id)?;
            let author_login = raw_comment
                .author
                .as_ref()
                .map(|a| a.login.clone())
                .unwrap_or_default();
            let body = raw_comment.body.unwrap_or_default();
            let created_at = raw_comment.created_at?;
            Some(IssueComment {
                id,
                author_login,
                body,
                created_at,
            })
        })
        .collect();

    comments.sort_by_key(|c| c.created_at);
    Ok(comments)
}

/// Resolve the GitHub login of the currently authenticated `gh` user.
///
/// Uses `gh api user -q .login` and returns a non-empty login string.
pub async fn fetch_authenticated_login() -> Result<String> {
    fetch_authenticated_login_with_gh_bin(DEFAULT_GH_BIN).await
}

pub async fn fetch_authenticated_login_with_gh_bin(gh_bin: &str) -> Result<String> {
    let output = Command::new(gh_bin)
        .args(["api", "user", "-q", ".login"])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to run gh api user for authenticated login: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh api user failed while resolving authenticated login: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    parse_authenticated_login(&String::from_utf8_lossy(&output.stdout))
}

/// Check whether any comment on the issue contains the given marker string.
pub async fn find_comment_with_marker(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
) -> Result<Option<IssueComment>> {
    let comments = fetch_issue_comments(owner, repo, issue_number).await?;
    Ok(comments.into_iter().find(|c| c.body.contains(marker)))
}

/// Post a comment on an issue with a marker prefix. If a comment with the same
/// marker already exists, skip posting and return the existing comment's ID.
///
/// Returns the comment ID of the posted (or existing) comment.
pub async fn post_comment_with_marker(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
) -> Result<Option<u64>> {
    let meta =
        post_comment_with_marker_metadata(owner, repo, issue_number, marker, body_text).await?;
    Ok(meta.map(|c| c.id))
}

/// Post a comment on an issue with a marker prefix and return full structured
/// metadata (id, created_at, etc.). If a comment with the same marker already
/// exists, skip posting and return the existing comment's metadata.
pub async fn post_comment_with_marker_metadata(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
) -> Result<Option<IssueComment>> {
    if let Some(existing) = find_comment_with_marker(owner, repo, issue_number, marker).await? {
        return Ok(Some(existing));
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
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to post marker comment on {full_repo}#{issue_number}: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue comment failed for {full_repo}#{issue_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    // Fetch back to get the full metadata of the newly posted comment.
    find_comment_with_marker(owner, repo, issue_number, marker).await
}

/// Find a comment with the given marker string authored by the specified bot login.
///
/// Bot-scoped lookup: only matches comments where `author_login == bot_login`
/// AND the body contains the marker string.  User-authored comments with the
/// same marker text are ignored, preventing marker spoofing.
pub async fn find_bot_comment_with_marker(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    bot_login: &str,
) -> Result<Option<IssueComment>> {
    find_bot_comment_with_marker_with_gh_bin(
        DEFAULT_GH_BIN,
        owner,
        repo,
        issue_number,
        marker,
        bot_login,
    )
    .await
}

pub async fn find_bot_comment_with_marker_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    bot_login: &str,
) -> Result<Option<IssueComment>> {
    let comments = fetch_issue_comments_with_gh_bin(gh_bin, owner, repo, issue_number).await?;
    Ok(comments
        .into_iter()
        .find(|c| c.author_login == bot_login && c.body.contains(marker)))
}

/// Post a comment on an issue with a marker prefix, using bot-scoped idempotency.
///
/// Only considers existing bot-authored comments when checking for duplicate
/// markers.  User-authored spoof markers are ignored.  Returns `Some(id)` of
/// the posted (or existing bot) comment.
pub async fn post_bot_comment_with_marker(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
    bot_login: &str,
) -> Result<Option<u64>> {
    post_bot_comment_with_marker_with_gh_bin(
        DEFAULT_GH_BIN,
        owner,
        repo,
        issue_number,
        marker,
        body_text,
        bot_login,
    )
    .await
}

/// Post a comment on an issue with a marker prefix, using bot-scoped
/// idempotency, while distinguishing a true post failure from a metadata
/// readback failure after a successful `gh issue comment`.
pub async fn post_bot_comment_with_marker_outcome_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
    bot_login: &str,
) -> PostCommentOutcome {
    let existing = match find_bot_comment_with_marker_with_gh_bin(
        gh_bin,
        owner,
        repo,
        issue_number,
        marker,
        bot_login,
    )
    .await
    {
        Ok(existing) => existing,
        Err(err) => return PostCommentOutcome::PostFailed(err),
    };
    if let Some(existing) = existing {
        return PostCommentOutcome::AlreadyExists(existing);
    }

    let full_body = format!("{marker}\n{body_text}");
    let full_repo = format!("{owner}/{repo}");
    let output = match Command::new(gh_bin)
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
        .await
    {
        Ok(output) => output,
        Err(err) => {
            return PostCommentOutcome::PostFailed(RalphError::Orchestration(format!(
                "failed to post bot marker comment on {full_repo}#{issue_number}: {err}"
            )))
        }
    };

    if !output.status.success() {
        return PostCommentOutcome::PostFailed(RalphError::Orchestration(format!(
            "gh issue comment (bot-scoped) failed for {full_repo}#{issue_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    match find_bot_comment_with_marker_with_gh_bin(
        gh_bin,
        owner,
        repo,
        issue_number,
        marker,
        bot_login,
    )
    .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            eprintln!(
                "warning: github: bot marker comment readback missing after successful post for {full_repo}#{issue_number}"
            );
        }
        Err(err) => {
            eprintln!(
                "warning: github: bot marker comment readback failed after successful post for {full_repo}#{issue_number}: {err}"
            );
        }
    }

    PostCommentOutcome::Posted
}

pub async fn post_bot_comment_with_marker_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
    bot_login: &str,
) -> Result<Option<u64>> {
    let meta = post_bot_comment_with_marker_metadata_with_gh_bin(
        gh_bin,
        owner,
        repo,
        issue_number,
        marker,
        body_text,
        bot_login,
    )
    .await?;
    Ok(meta.map(|c| c.id))
}

/// Post a comment on an issue with a marker prefix and return full structured
/// metadata, using bot-scoped idempotency.
///
/// Only considers existing bot-authored comments when checking for duplicate
/// markers.  User-authored spoof markers are ignored.
pub async fn post_bot_comment_with_marker_metadata(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
    bot_login: &str,
) -> Result<Option<IssueComment>> {
    post_bot_comment_with_marker_metadata_with_gh_bin(
        DEFAULT_GH_BIN,
        owner,
        repo,
        issue_number,
        marker,
        body_text,
        bot_login,
    )
    .await
}

pub async fn post_bot_comment_with_marker_metadata_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
    bot_login: &str,
) -> Result<Option<IssueComment>> {
    if let Some(existing) = find_bot_comment_with_marker_with_gh_bin(
        gh_bin,
        owner,
        repo,
        issue_number,
        marker,
        bot_login,
    )
    .await?
    {
        return Ok(Some(existing));
    }

    let full_body = format!("{marker}\n{body_text}");
    let full_repo = format!("{owner}/{repo}");
    let output = Command::new(gh_bin)
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
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to post bot marker comment on {full_repo}#{issue_number}: {err}"
            ))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh issue comment (bot-scoped) failed for {full_repo}#{issue_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    // Fetch back to get the full metadata of the newly posted comment.
    find_bot_comment_with_marker_with_gh_bin(gh_bin, owner, repo, issue_number, marker, bot_login)
        .await
}

/// Ensure PRD lifecycle labels exist in the repository (idempotent, best-effort).
pub async fn ensure_prd_labels_best_effort(owner: &str, repo: &str) {
    ensure_prd_labels_best_effort_with_gh_bin(DEFAULT_GH_BIN, owner, repo).await;
}

pub async fn ensure_prd_labels_best_effort_with_gh_bin(gh_bin: &str, owner: &str, repo: &str) {
    use crate::daemon::interactive_prd::PRD_LABELS;
    let full_repo = format!("{owner}/{repo}");

    for (name, color, description) in PRD_LABELS {
        let output = Command::new(gh_bin)
            .args([
                "label",
                "create",
                name,
                "--repo",
                &full_repo,
                "--color",
                color,
                "--description",
                description,
            ])
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}\n{stderr}");
                if combined.to_ascii_lowercase().contains("already exists") {
                    continue;
                }

                let detail = stderr.trim();
                let detail = if detail.is_empty() {
                    stdout.trim()
                } else {
                    detail
                };
                eprintln!(
                    "warning: failed to ensure PRD label '{}' for {}: {}",
                    name, full_repo, detail
                );
            }
            Err(err) => {
                eprintln!(
                    "warning: failed to run gh label create for PRD label '{}' in {}: {}",
                    name, full_repo, err
                );
            }
        }
    }
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

fn parse_pr_is_draft(raw: &str) -> Result<bool> {
    let parsed: RawPrDraftInfo = serde_json::from_str(raw).map_err(|err| {
        RalphError::Orchestration(format!("failed to parse gh pr view isDraft output: {err}"))
    })?;
    Ok(parsed.is_draft)
}

pub fn parse_open_prs(raw: &str) -> Result<(Vec<OpenPrInfo>, bool)> {
    let parsed: Vec<RawOpenPrInfo> = serde_json::from_str(raw).map_err(|err| {
        RalphError::Orchestration(format!("failed to parse gh pr list output: {err}"))
    })?;
    let overflow = parsed.len() == 100;
    let prs = parsed
        .into_iter()
        .filter(|pr| !pr.is_draft)
        .map(|pr| OpenPrInfo {
            number: pr.number,
            head_sha: pr.head_ref_oid,
            author: pr.author.map(|author| author.login).unwrap_or_default(),
        })
        .collect();
    Ok((prs, overflow))
}

fn parse_authenticated_login(raw: &str) -> Result<String> {
    let login = raw.trim();
    if login.is_empty() {
        return Err(RalphError::Orchestration(
            "gh api user returned empty login".to_owned(),
        ));
    }
    Ok(login.to_owned())
}

// ---------------------------------------------------------------------------
// PR review comment fetching
// ---------------------------------------------------------------------------

/// Source endpoint for dedup key namespacing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CommentEndpoint {
    PullComment,
    IssueComment,
    Review,
}

impl CommentEndpoint {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PullComment => "pull_comment",
            Self::IssueComment => "issue_comment",
            Self::Review => "review",
        }
    }
}

impl std::fmt::Display for CommentEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single review comment from any of the three GitHub PR comment endpoints.
#[derive(Debug, Clone)]
pub struct PrReviewComment {
    pub id: u64,
    pub endpoint: CommentEndpoint,
    pub author: String,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub created_at: String,
}

impl PrReviewComment {
    /// Composite dedup key: `"{endpoint}:{id}"`.
    pub fn dedup_key(&self) -> String {
        format!("{}:{}", self.endpoint.as_str(), self.id)
    }
}

/// Raw JSON shape for `/pulls/{n}/comments` (inline review comments).
#[derive(Debug, Deserialize)]
struct RawPullComment {
    id: u64,
    #[serde(default)]
    user: Option<RawUser>,
    body: Option<String>,
    path: Option<String>,
    line: Option<u32>,
    created_at: String,
    /// Non-null when this comment is a reply to another inline comment.
    /// Replies are out of scope and should be skipped.
    in_reply_to_id: Option<u64>,
}

/// Raw JSON shape for `/issues/{n}/comments` (top-level PR comments).
#[derive(Debug, Deserialize)]
struct RawIssueComment {
    id: u64,
    #[serde(default)]
    user: Option<RawUser>,
    body: Option<String>,
    created_at: String,
}

/// Raw JSON shape for `/pulls/{n}/reviews` (review summaries).
#[derive(Debug, Deserialize)]
struct RawReview {
    id: u64,
    #[serde(default)]
    user: Option<RawUser>,
    body: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    state: Option<String>,
    submitted_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawUser {
    login: String,
}

/// Parse raw JSON from `/pulls/{n}/comments` into `PrReviewComment`s.
///
/// Entries with missing or empty `user.login` are skipped with a warning.
/// Returns an empty vec on parse failure (logged as warning).
fn parse_pull_comments(raw: &str, pr_number: u32) -> Vec<PrReviewComment> {
    let parsed: Vec<RawPullComment> = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("warning: failed to parse inline review comments for PR #{pr_number}: {err}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for c in parsed {
        if c.in_reply_to_id.is_some() {
            continue;
        }
        let login = match &c.user {
            Some(u) if !u.login.is_empty() => u.login.clone(),
            _ => {
                eprintln!(
                    "warning: skipping inline review comment {} for PR #{pr_number}: missing or empty user",
                    c.id
                );
                continue;
            }
        };
        out.push(PrReviewComment {
            id: c.id,
            endpoint: CommentEndpoint::PullComment,
            author: login,
            body: c.body.unwrap_or_default(),
            path: c.path,
            line: c.line,
            created_at: c.created_at,
        });
    }
    out
}

/// Parse raw JSON from `/issues/{n}/comments` into `PrReviewComment`s.
///
/// Entries with missing or empty `user.login` are skipped with a warning.
/// Returns an empty vec on parse failure (logged as warning).
fn parse_issue_comments(raw: &str, pr_number: u32) -> Vec<PrReviewComment> {
    let parsed: Vec<RawIssueComment> = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("warning: failed to parse top-level PR comments for PR #{pr_number}: {err}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for c in parsed {
        let login = match &c.user {
            Some(u) if !u.login.is_empty() => u.login.clone(),
            _ => {
                eprintln!(
                    "warning: skipping top-level PR comment {} for PR #{pr_number}: missing or empty user",
                    c.id
                );
                continue;
            }
        };
        out.push(PrReviewComment {
            id: c.id,
            endpoint: CommentEndpoint::IssueComment,
            author: login,
            body: c.body.unwrap_or_default(),
            path: None,
            line: None,
            created_at: c.created_at,
        });
    }
    out
}

/// Parse raw JSON from `/pulls/{n}/reviews` into `PrReviewComment`s.
///
/// Reviews with empty body and entries with missing or empty `user.login` are
/// skipped. Returns an empty vec on parse failure (logged as warning).
fn parse_review_summaries(raw: &str, pr_number: u32) -> Vec<PrReviewComment> {
    let parsed: Vec<RawReview> = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("warning: failed to parse review summaries for PR #{pr_number}: {err}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for r in parsed {
        let body = r.body.unwrap_or_default();
        if body.trim().is_empty() {
            continue;
        }
        let login = match &r.user {
            Some(u) if !u.login.is_empty() => u.login.clone(),
            _ => {
                eprintln!(
                    "warning: skipping review summary {} for PR #{pr_number}: missing or empty user",
                    r.id
                );
                continue;
            }
        };
        out.push(PrReviewComment {
            id: r.id,
            endpoint: CommentEndpoint::Review,
            author: login,
            body,
            path: None,
            line: None,
            created_at: r.submitted_at.unwrap_or_default(),
        });
    }
    out
}

/// Fetch PR review comments from all three GitHub endpoints.
///
/// Returns comments from:
/// 1. Inline review comments (`/pulls/{n}/comments`)
/// 2. Top-level PR comments (`/issues/{n}/comments`)
/// 3. Review summary comments (`/pulls/{n}/reviews`) — only those with non-empty body
pub async fn fetch_pr_review_comments(
    owner: &str,
    repo: &str,
    pr_number: u32,
    gh_bin: &str,
) -> Result<Vec<PrReviewComment>> {
    let mut comments = Vec::new();

    // 1. Inline review comments
    match fetch_endpoint_json(
        gh_bin,
        &format!("repos/{owner}/{repo}/pulls/{pr_number}/comments"),
    )
    .await
    {
        Ok(raw) => comments.extend(parse_pull_comments(&raw, pr_number)),
        Err(err) => {
            eprintln!("warning: failed to fetch inline review comments for PR #{pr_number}: {err}");
        }
    }

    // 2. Top-level PR comments (issue comments endpoint)
    match fetch_endpoint_json(
        gh_bin,
        &format!("repos/{owner}/{repo}/issues/{pr_number}/comments"),
    )
    .await
    {
        Ok(raw) => comments.extend(parse_issue_comments(&raw, pr_number)),
        Err(err) => {
            eprintln!("warning: failed to fetch top-level PR comments for PR #{pr_number}: {err}");
        }
    }

    // 3. Review summaries (only include those with non-empty body)
    match fetch_endpoint_json(
        gh_bin,
        &format!("repos/{owner}/{repo}/pulls/{pr_number}/reviews"),
    )
    .await
    {
        Ok(raw) => comments.extend(parse_review_summaries(&raw, pr_number)),
        Err(err) => {
            eprintln!("warning: failed to fetch review summaries for PR #{pr_number}: {err}");
        }
    }

    Ok(comments)
}

/// Check whether a PR is open via the GitHub API.
pub async fn is_pr_open(owner: &str, repo: &str, pr_number: u32, gh_bin: &str) -> Result<bool> {
    let output = Command::new(gh_bin)
        .args([
            "api",
            &format!("repos/{owner}/{repo}/pulls/{pr_number}"),
            "--jq",
            ".state",
        ])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to check PR #{pr_number} state: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh api failed checking PR #{pr_number} state: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(state == "open")
}

/// Fetch a paginated JSON array from a GitHub API endpoint.
async fn fetch_endpoint_json(gh_bin: &str, endpoint: &str) -> Result<String> {
    let output = Command::new(gh_bin)
        .args(["api", endpoint, "--paginate"])
        .output()
        .await
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to call gh api {endpoint}: {err}"))
        })?;

    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh api {endpoint} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    // `--paginate` concatenates JSON arrays, producing `[...][...]...` when
    // multiple pages exist. Merge them into a single array.
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    merge_paginated_json_arrays(&raw)
}

/// Merge adjacent JSON arrays (as produced by `gh api --paginate`) into one.
///
/// Input: `[{"a":1}][{"b":2}]` → Output: `[{"a":1},{"b":2}]`
/// Single array input is returned unchanged.
///
/// Uses `serde_json::Deserializer` streaming to correctly handle brackets
/// inside JSON string values (e.g. comment bodies containing `[` or `]`).
fn merge_paginated_json_arrays(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok("[]".to_string());
    }

    let mut merged = Vec::new();
    let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<serde_json::Value>();
    for value in stream {
        let value = value.map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to parse paginated JSON from GitHub API: {err}"
            ))
        })?;
        match value {
            serde_json::Value::Array(arr) => merged.extend(arr),
            other => merged.push(other),
        }
    }

    serde_json::to_string(&merged)
        .map_err(|err| RalphError::Orchestration(format!("failed to serialize merged JSON: {err}")))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use tempfile::tempdir;

    use super::{
        has_diff, is_invalid_revision_error, is_retryable_push_error, parse_issue_list,
        parse_open_prs, parse_pr_is_draft, parse_pr_merge_info,
        post_bot_comment_with_marker_metadata_with_gh_bin,
        post_bot_comment_with_marker_outcome_with_gh_bin, push_branch_with_retry_impl, GhIssue,
        PostCommentOutcome, PrMergeStatus, REQUIRED_LABELS,
    };
    use crate::error::RalphError;

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

    #[tokio::test]
    async fn has_diff_returns_false_for_single_commit_head_tilde_base() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path();

        git(repo, &["init"]);
        git(repo, &["config", "user.email", "test@example.com"]);
        git(repo, &["config", "user.name", "Test"]);

        std::fs::write(repo.join("README.md"), "hello\n").expect("write readme");
        git(repo, &["add", "README.md"]);
        git(repo, &["commit", "-m", "initial"]);

        let diff = has_diff(repo)
            .await
            .expect("has_diff should not error for invalid base revision");
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
    fn is_retryable_push_stderr_classifies_transient_and_permanent_errors() {
        assert!(super::is_retryable_push_stderr(
            "HTTP 503 Service Unavailable"
        ));
        assert!(super::is_retryable_push_stderr("operation timed out"));
        assert!(!super::is_retryable_push_stderr("permission denied"));
        assert!(!super::is_retryable_push_stderr("non-fast-forward"));
        assert!(!super::is_retryable_push_stderr(
            "GH013: Repository rule violations found"
        ));
        assert!(!super::is_retryable_push_stderr("HTTP 403"));
        assert!(!super::is_retryable_push_stderr(
            "protected branch hook declined"
        ));
        assert!(super::is_retryable_push_stderr("connection reset by peer"));
    }

    #[test]
    fn is_retryable_push_stderr_ignores_branch_name_collisions() {
        let stderr_403 = "permission denied";
        assert!(!super::is_retryable_push_stderr(stderr_403));

        let stderr_503 = "HTTP 503 Service Unavailable";
        assert!(super::is_retryable_push_stderr(stderr_503));
    }

    #[test]
    fn branch_name_with_numeric_code_does_not_affect_classification() {
        let transient_stderr = "HTTP 503 Service Unavailable";
        assert!(super::is_retryable_push_stderr(transient_stderr));

        let permanent_stderr = "permission denied";
        assert!(!super::is_retryable_push_stderr(permanent_stderr));
    }

    #[test]
    fn unknown_errors_are_not_retried() {
        assert!(!super::is_retryable_push_stderr("repository not found"));
        assert!(!super::is_retryable_push_stderr(
            "unexpected internal error xyz"
        ));
    }

    // ------------------------------------------------------------------
    // Regression tests for context-aware HTTP code extraction
    // (DAEMON-PUSH-RETRY-CLASSIFIER-001)
    // ------------------------------------------------------------------

    #[test]
    fn dns_error_with_url_containing_403_is_retryable() {
        // The "403" in the URL path must NOT be treated as an HTTP 403 code.
        let stderr = "fatal: unable to access 'https://github.com/org/repo-403/': \
                       Could not resolve host: github.com";
        assert!(
            super::is_retryable_push_stderr(stderr),
            "DNS error should be retryable even when URL contains '403'"
        );
    }

    #[test]
    fn explicit_http_401_is_non_retryable() {
        let stderr = "The requested URL returned error: 401";
        assert!(
            !super::is_retryable_push_stderr(stderr),
            "explicit HTTP 401 should be non-retryable"
        );
    }

    #[test]
    fn explicit_http_503_is_retryable() {
        let stderr = "HTTP 503 Service Unavailable";
        assert!(
            super::is_retryable_push_stderr(stderr),
            "explicit HTTP 503 should be retryable"
        );
    }

    #[test]
    fn explicit_http_403_is_non_retryable() {
        let stderr = "The requested URL returned error: 403";
        assert!(
            !super::is_retryable_push_stderr(stderr),
            "explicit HTTP 403 should be non-retryable"
        );
    }

    #[test]
    fn url_containing_500_plus_http_401_is_non_retryable() {
        // URL path contains "500" but actual HTTP code is 401 — non-retryable.
        let stderr = "fatal: unable to access 'https://github.com/org/error-500/': \
                       The requested URL returned error: 401";
        assert!(
            !super::is_retryable_push_stderr(stderr),
            "HTTP 401 should be non-retryable even when URL contains '500'"
        );
    }

    #[test]
    fn extract_http_status_codes_bounded_context() {
        // Only codes in "http NNN" or "returned error: NNN" context are extracted.
        let codes = super::extract_http_status_codes(
            "fatal: unable to access 'https://github.com/org/repo-403/': http 503 service unavailable"
        );
        assert_eq!(
            codes,
            vec![503],
            "should only extract 503 from 'http 503', not 403 from URL"
        );

        let codes = super::extract_http_status_codes("the requested url returned error: 401");
        assert_eq!(codes, vec![401]);

        // No codes from plain URLs.
        let codes = super::extract_http_status_codes(
            "fatal: unable to access 'https://github.com/org/error-500/': could not resolve host",
        );
        assert!(codes.is_empty(), "should not extract codes from URL paths");
    }

    #[test]
    fn is_retryable_push_error_classifies_transient_and_permanent() {
        let transient = RalphError::Orchestration(
            "git push failed for branch main: HTTP 503 Service Unavailable".to_owned(),
        );
        assert!(is_retryable_push_error(&transient));

        let timeout = RalphError::Orchestration(
            "git push failed for branch main: operation timed out".to_owned(),
        );
        assert!(is_retryable_push_error(&timeout));

        let connection = RalphError::Orchestration(
            "git push failed for branch main: connection reset by peer".to_owned(),
        );
        assert!(is_retryable_push_error(&connection));

        let permanent = RalphError::Orchestration(
            "git push failed for branch main: permission denied".to_owned(),
        );
        assert!(!is_retryable_push_error(&permanent));

        let nff = RalphError::Orchestration(
            "git push failed for branch main: non-fast-forward".to_owned(),
        );
        assert!(!is_retryable_push_error(&nff));

        let gh013 = RalphError::Orchestration(
            "git push failed for branch main: GH013: Repository rule violations found".to_owned(),
        );
        assert!(!is_retryable_push_error(&gh013));

        let forbidden =
            RalphError::Orchestration("git push failed for branch main: HTTP 403".to_owned());
        assert!(!is_retryable_push_error(&forbidden));
    }

    #[test]
    fn is_retryable_push_error_branch_name_collision_safety() {
        // Branch name contains "403" but stderr is transient — should be retryable.
        let err_403_branch = RalphError::Orchestration(
            "git push failed for branch fix/issue-403: HTTP 503 Service Unavailable".to_owned(),
        );
        assert!(is_retryable_push_error(&err_403_branch));

        // Branch name contains "500" but stderr is permanent — should not be retryable.
        let err_500_branch = RalphError::Orchestration(
            "git push failed for branch feature/500-errors: permission denied".to_owned(),
        );
        assert!(!is_retryable_push_error(&err_500_branch));
    }

    #[test]
    fn is_retryable_push_error_non_push_error_fallback() {
        // Non-push RalphError variants should be non-retryable.
        let io_err = RalphError::Orchestration("gh issue list failed: timeout".to_owned());
        assert!(!is_retryable_push_error(&io_err));

        let validation = RalphError::Validation("bad input".to_owned());
        assert!(!is_retryable_push_error(&validation));

        // Malformed push error (missing ": " separator) should be non-retryable.
        let malformed = RalphError::Orchestration("git push failed for branch main".to_owned());
        assert!(!is_retryable_push_error(&malformed));
    }

    #[test]
    fn is_retryable_push_error_unknown_stderr_is_non_retryable() {
        let unknown = RalphError::Orchestration(
            "git push failed for branch main: unexpected internal error xyz".to_owned(),
        );
        assert!(!is_retryable_push_error(&unknown));
    }

    #[tokio::test]
    async fn push_branch_with_retry_impl_retries_transient_then_succeeds() {
        let tmp = tempdir().expect("tempdir");
        let (git_bin, attempts_file) = write_mock_git_binary(tmp.path(), "transient_then_success");

        let result =
            push_branch_with_retry_impl(&git_bin, tmp.path(), "feature/test", &[0, 0, 0]).await;
        assert!(result.is_ok(), "expected retry flow to recover: {result:?}");
        assert_eq!(
            read_attempts(&attempts_file),
            3,
            "should retry twice then succeed"
        );
    }

    #[tokio::test]
    async fn push_branch_with_retry_impl_does_not_retry_permanent_failure() {
        let tmp = tempdir().expect("tempdir");
        let (git_bin, attempts_file) = write_mock_git_binary(tmp.path(), "permanent_failure");

        let result =
            push_branch_with_retry_impl(&git_bin, tmp.path(), "feature/test", &[0, 0, 0]).await;
        assert!(result.is_err(), "expected permanent failure");
        if attempts_file.exists() {
            assert_eq!(
                read_attempts(&attempts_file),
                1,
                "permanent failure should not retry"
            );
        }
        // If the attempts file does not exist, the mock was not invocable
        // (e.g. no /bin/sh in sandbox). The function still returns Err which
        // is the correct behavior — push failed, no retry.
    }

    #[tokio::test]
    async fn push_branch_with_retry_impl_exhausts_retries_for_transient_failure() {
        let tmp = tempdir().expect("tempdir");
        let (git_bin, attempts_file) = write_mock_git_binary(tmp.path(), "transient_exhaustion");

        let result =
            push_branch_with_retry_impl(&git_bin, tmp.path(), "feature/test", &[0, 0, 0]).await;
        assert!(result.is_err(), "expected transient retry exhaustion");
        assert_eq!(
            read_attempts(&attempts_file),
            4,
            "should attempt initial push plus three retries"
        );
    }

    #[tokio::test]
    async fn push_branch_with_retry_impl_classifies_on_stderr_not_branch_name() {
        let tmp = tempdir().expect("tempdir");
        let (git_bin, attempts_file) = write_mock_git_binary(tmp.path(), "transient_then_success");

        let result =
            push_branch_with_retry_impl(&git_bin, tmp.path(), "fix/issue-403", &[0, 0, 0]).await;
        assert!(
            result.is_ok(),
            "transient stderr should retry regardless of branch name collision: {result:?}"
        );
        assert_eq!(
            read_attempts(&attempts_file),
            3,
            "should retry transient failures and eventually succeed"
        );

        let tmp = tempdir().expect("tempdir");
        let (git_bin, attempts_file) = write_mock_git_binary(tmp.path(), "permanent_failure");
        let result =
            push_branch_with_retry_impl(&git_bin, tmp.path(), "feature/500-errors", &[0, 0, 0])
                .await;
        assert!(
            result.is_err(),
            "permanent stderr should fail even when branch name contains 500: {result:?}"
        );
        assert_eq!(
            read_attempts(&attempts_file),
            1,
            "permanent failures should not retry"
        );
    }

    #[tokio::test]
    async fn push_branch_with_retry_impl_does_not_retry_unknown_failure() {
        let tmp = tempdir().expect("tempdir");
        let (git_bin, attempts_file) = write_mock_git_binary(tmp.path(), "unknown_failure");

        let result =
            push_branch_with_retry_impl(&git_bin, tmp.path(), "feature/test", &[0, 0, 0]).await;
        assert!(result.is_err(), "expected unknown failure");
        assert_eq!(
            read_attempts(&attempts_file),
            1,
            "unknown errors should fail without retry"
        );
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

    #[test]
    fn parse_pr_is_draft_true() {
        let raw = r#"{"isDraft":true}"#;
        let is_draft = parse_pr_is_draft(raw).expect("draft state should parse");
        assert!(is_draft);
    }

    #[test]
    fn parse_pr_is_draft_false() {
        let raw = r#"{"isDraft":false}"#;
        let is_draft = parse_pr_is_draft(raw).expect("draft state should parse");
        assert!(!is_draft);
    }

    #[test]
    fn parse_open_prs_filters_drafts_and_tracks_overflow() {
        let raw = format!(
            "[{},{}]",
            r#"{"number":1,"headRefOid":"sha-1","isDraft":false,"author":{"login":"alice"}}"#,
            r#"{"number":2,"headRefOid":"sha-2","isDraft":true,"author":{"login":"bob"}}"#
        );
        let (prs, overflow) = parse_open_prs(&raw).expect("open PRs should parse");

        assert!(!overflow);
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 1);
        assert_eq!(prs[0].head_sha, "sha-1");
        assert_eq!(prs[0].author, "alice");
    }

    #[test]
    fn parse_open_prs_marks_exact_100_as_overflow() {
        let items: Vec<String> = (1..=100)
            .map(|number| {
                format!(
                    r#"{{"number":{number},"headRefOid":"sha-{number}","isDraft":false,"author":{{"login":"user-{number}"}}}}"#
                )
            })
            .collect();
        let raw = format!("[{}]", items.join(","));

        let (prs, overflow) = parse_open_prs(&raw).expect("open PRs should parse");
        assert!(overflow);
        assert_eq!(prs.len(), 100);
    }

    #[tokio::test]
    async fn post_bot_comment_outcome_treats_readback_failure_after_post_as_success() {
        let temp = tempdir().expect("tempdir");
        let gh_bin = write_mock_gh_comment_binary(temp.path());

        let outcome = post_bot_comment_with_marker_outcome_with_gh_bin(
            &gh_bin,
            "acme",
            "widgets",
            11,
            "<!-- marker -->",
            "body",
            "ralph-bot",
        )
        .await;

        assert!(matches!(outcome, PostCommentOutcome::Posted));
    }

    #[tokio::test]
    async fn post_bot_comment_metadata_still_errors_on_readback_failure() {
        let temp = tempdir().expect("tempdir");
        let gh_bin = write_mock_gh_comment_binary(temp.path());

        let err = post_bot_comment_with_marker_metadata_with_gh_bin(
            &gh_bin,
            "acme",
            "widgets",
            11,
            "<!-- marker -->",
            "body",
            "ralph-bot",
        )
        .await
        .expect_err("metadata helper should preserve readback failure semantics");

        assert!(err.to_string().contains("mock comment readback failure"));
    }

    #[test]
    fn required_labels_are_unique_and_include_lifecycle_labels() {
        let names: Vec<&str> = REQUIRED_LABELS.iter().map(|(name, _, _)| *name).collect();
        let unique: HashSet<&str> = names.iter().copied().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "REQUIRED_LABELS must not contain duplicate names"
        );

        for required in [
            "ralph:ready",
            "ralph:in-progress",
            "ralph:completed",
            "ralph:failed",
        ] {
            assert!(
                unique.contains(required),
                "REQUIRED_LABELS is missing required lifecycle label: {required}"
            );
        }
    }

    #[test]
    fn ralph_quick_is_in_required_labels() {
        let names: Vec<&str> = REQUIRED_LABELS.iter().map(|(name, _, _)| *name).collect();
        assert!(
            names.contains(&"ralph:quick"),
            "REQUIRED_LABELS must include ralph:quick"
        );
    }

    #[test]
    fn ralph_quick_is_not_a_lifecycle_label() {
        assert!(
            !super::LIFECYCLE_LABELS.contains(&"ralph:quick"),
            "ralph:quick must NOT be in LIFECYCLE_LABELS"
        );
    }

    #[test]
    fn classify_lifecycle_labels_excludes_ralph_quick() {
        let labels = vec![
            "ralph:ready".to_owned(),
            "ralph:quick".to_owned(),
            "bug".to_owned(),
        ];
        let lifecycle = super::classify_lifecycle_labels(&labels);
        assert_eq!(lifecycle.len(), 1);
        assert!(lifecycle.contains(&"ralph:ready".to_owned()));
        assert!(
            !lifecycle.contains(&"ralph:quick".to_owned()),
            "ralph:quick should not be classified as lifecycle"
        );
    }

    #[test]
    fn classify_lifecycle_labels_filters_correctly() {
        let labels = vec![
            "ralph:ready".to_owned(),
            "bug".to_owned(),
            "ralph:in-progress".to_owned(),
            "enhancement".to_owned(),
        ];
        let lifecycle = super::classify_lifecycle_labels(&labels);
        assert_eq!(lifecycle.len(), 2);
        assert!(lifecycle.contains(&"ralph:ready".to_owned()));
        assert!(lifecycle.contains(&"ralph:in-progress".to_owned()));
    }

    #[test]
    fn classify_lifecycle_labels_empty_for_no_ralph_labels() {
        let labels = vec!["bug".to_owned(), "enhancement".to_owned()];
        let lifecycle = super::classify_lifecycle_labels(&labels);
        assert!(lifecycle.is_empty());
    }

    #[test]
    fn is_retryable_gh_error_detects_rate_limit() {
        assert!(super::is_retryable_gh_error(
            "API rate limit exceeded for user"
        ));
    }

    #[test]
    fn is_retryable_gh_error_detects_503() {
        assert!(super::is_retryable_gh_error("HTTP 503 Service Unavailable"));
    }

    #[test]
    fn is_retryable_gh_error_detects_409_conflict() {
        assert!(super::is_retryable_gh_error("HTTP 409 Conflict"));
    }

    #[test]
    fn is_retryable_gh_error_detects_conflict_keyword() {
        assert!(super::is_retryable_gh_error(
            "GraphQL: was submitted too quickly (conflict)"
        ));
    }

    #[test]
    fn is_retryable_gh_error_returns_false_for_auth_errors() {
        assert!(!super::is_retryable_gh_error(
            "HTTP 401 Unauthorized: Bad credentials"
        ));
    }

    #[test]
    fn parse_authenticated_login_trims_and_returns_value() {
        let login = super::parse_authenticated_login("  ralph-bot\n").expect("login should parse");
        assert_eq!(login, "ralph-bot");
    }

    #[test]
    fn parse_authenticated_login_rejects_empty() {
        let err = super::parse_authenticated_login("   ").expect_err("empty login should fail");
        let message = err.to_string();
        assert!(message.contains("empty login"));
    }

    #[test]
    fn extract_numeric_comment_id_from_graphql_url() {
        let url = "https://github.com/douglaz/multibackend-orchestration/issues/92#issuecomment-3954637090";
        assert_eq!(
            super::extract_numeric_comment_id_from_url(Some(url)),
            Some(3954637090)
        );
    }

    #[test]
    fn extract_numeric_comment_id_from_rest_url() {
        let url = "https://api.github.com/repos/douglaz/multibackend-orchestration/issues/comments/3954637090";
        assert_eq!(
            super::extract_numeric_comment_id_from_url(Some(url)),
            Some(3954637090)
        );
    }

    #[test]
    fn extract_numeric_comment_id_from_none() {
        assert_eq!(super::extract_numeric_comment_id_from_url(None), None);
    }

    #[test]
    fn extract_numeric_comment_id_from_invalid_url() {
        assert_eq!(
            super::extract_numeric_comment_id_from_url(Some("https://example.com/no-id")),
            None
        );
    }

    #[test]
    fn deserialize_graphql_comment_with_string_id_and_url() {
        // Simulates the JSON returned by `gh issue view --json comments`
        // where `id` is a GraphQL node ID (string) and `url` contains the
        // numeric comment ID.
        let json = r#"{
            "comments": [{
                "id": "IC_kwDORMeVKs7rtvki",
                "url": "https://github.com/o/r/issues/1#issuecomment-3954637090",
                "author": {"login": "alice"},
                "body": "hello",
                "createdAt": "2026-01-01T00:00:00Z"
            }]
        }"#;
        let parsed: super::RawIssueComments = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.comments.len(), 1);
        let raw = &parsed.comments[0];
        // The string node ID should be None from the deserializer
        assert_eq!(raw.id, None);
        // The numeric ID should be extracted from the URL
        let numeric_id = super::extract_numeric_comment_id_from_url(raw.url.as_deref());
        assert_eq!(numeric_id, Some(3954637090));
    }

    #[test]
    fn deserialize_numeric_comment_id_without_url() {
        // Simulates mock/test JSON where `id` is already numeric
        let json = r#"{
            "comments": [{
                "id": 42001,
                "author": {"login": "bob"},
                "body": "test",
                "createdAt": "2026-01-01T00:00:00Z"
            }]
        }"#;
        let parsed: super::RawIssueComments = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.comments.len(), 1);
        let raw = &parsed.comments[0];
        assert_eq!(raw.id, Some(42001));
        // No URL field, so extraction returns None
        assert_eq!(
            super::extract_numeric_comment_id_from_url(raw.url.as_deref()),
            None
        );
        // The fallback to raw.id should yield 42001
        let id = super::extract_numeric_comment_id_from_url(raw.url.as_deref()).or(raw.id);
        assert_eq!(id, Some(42001));
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

    fn write_mock_git_binary(dir: &Path, behavior: &str) -> (String, PathBuf) {
        let script_path = dir.join("mock-git.sh");
        let attempts_file = dir.join("attempts.txt");
        let script = format!(
            r#"#!/bin/sh
set -eu
attempt_file="$(dirname "$0")/attempts.txt"
attempt=0
if [ -f "$attempt_file" ]; then
    attempt=$(cat "$attempt_file")
fi
attempt=$((attempt + 1))
echo "$attempt" > "$attempt_file"

if [ "$1" != "push" ]; then
    exit 0
fi

case "{behavior}" in
    transient_then_success)
        if [ "$attempt" -lt 3 ]; then
            echo "HTTP 503 Service Unavailable" >&2
            exit 1
        fi
        ;;
    permanent_failure)
        echo "permission denied" >&2
        exit 1
        ;;
    transient_exhaustion)
        echo "network timeout" >&2
        exit 1
        ;;
    unknown_failure)
        echo "repository not found" >&2
        exit 1
        ;;
    *)
        echo "unknown behavior" >&2
        exit 2
        ;;
esac

exit 0
"#
        );
        fs::write(&script_path, script).expect("write mock git script");
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&script_path)
                .expect("mock git metadata")
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).expect("set executable permissions");
        }
        (script_path.display().to_string(), attempts_file)
    }

    fn read_attempts(path: &Path) -> u32 {
        let raw = fs::read_to_string(path).expect("read attempts file");
        raw.trim().parse::<u32>().expect("parse attempts")
    }

    fn write_mock_gh_comment_binary(dir: &Path) -> String {
        let script_path = dir.join("mock-gh-comment.sh");
        let readback_flag = dir
            .join("readback.flag")
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script = format!(
            r#"#!/bin/sh
set -eu

readback_flag="{readback_flag}"

case "${{1:-}}" in
  issue)
    case "${{2:-}}" in
      view)
        if [ -f "$readback_flag" ]; then
          rm -f "$readback_flag"
          echo "mock comment readback failure" >&2
          exit 1
        fi
        printf '{{"comments":[]}}'
        exit 0
        ;;
      comment)
        : > "$readback_flag"
        exit 0
        ;;
    esac
    ;;
esac

echo "unexpected gh invocation: $*" >&2
exit 1
"#
        );
        fs::write(&script_path, script).expect("write mock gh script");
        let mut perms = fs::metadata(&script_path)
            .expect("mock gh metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("set mock gh permissions");
        script_path.display().to_string()
    }

    // --- PR review comment tests ---

    #[test]
    fn parse_pull_comments_json() {
        let json = r#"[
            {
                "id": 100,
                "user": {"login": "alice"},
                "body": "fix this line",
                "path": "src/main.rs",
                "line": 42,
                "created_at": "2024-01-01T00:00:00Z"
            }
        ]"#;
        let parsed: Vec<super::RawPullComment> = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, 100);
        assert_eq!(parsed[0].user.as_ref().unwrap().login, "alice");
        assert_eq!(parsed[0].body, Some("fix this line".to_string()));
        assert_eq!(parsed[0].path, Some("src/main.rs".to_string()));
        assert_eq!(parsed[0].line, Some(42));
    }

    #[test]
    fn parse_pull_comments_filters_replies() {
        // One top-level inline comment + one reply; call the production
        // parse_pull_comments function to verify reply filtering.
        let json = r#"[
            {
                "id": 100,
                "user": {"login": "alice"},
                "body": "fix this line",
                "path": "src/main.rs",
                "line": 42,
                "created_at": "2024-01-01T00:00:00Z"
            },
            {
                "id": 101,
                "user": {"login": "bob"},
                "body": "I agree with Alice",
                "path": "src/main.rs",
                "line": 42,
                "created_at": "2024-01-01T01:00:00Z",
                "in_reply_to_id": 100
            }
        ]"#;
        let comments = super::parse_pull_comments(json, 1);
        assert_eq!(comments.len(), 1, "only top-level comment should be kept");
        assert_eq!(comments[0].id, 100);
        assert_eq!(comments[0].author, "alice");
        assert_eq!(comments[0].endpoint, super::CommentEndpoint::PullComment);
    }

    #[test]
    fn parse_issue_comments_json() {
        let json = r#"[
            {
                "id": 200,
                "user": {"login": "bob"},
                "body": "please add tests",
                "created_at": "2024-01-01T00:00:00Z"
            }
        ]"#;
        let parsed: Vec<super::RawIssueComment> = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, 200);
        assert_eq!(parsed[0].user.as_ref().unwrap().login, "bob");
    }

    #[test]
    fn parse_reviews_json_filters_empty_body() {
        let json = r#"[
            {
                "id": 300,
                "user": {"login": "carol"},
                "body": "needs refactoring",
                "state": "CHANGES_REQUESTED",
                "submitted_at": "2024-01-01T00:00:00Z"
            },
            {
                "id": 301,
                "user": {"login": "dave"},
                "body": "",
                "state": "APPROVED",
                "submitted_at": "2024-01-01T00:00:00Z"
            },
            {
                "id": 302,
                "user": {"login": "eve"},
                "body": null,
                "state": "COMMENTED",
                "submitted_at": "2024-01-01T00:00:00Z"
            }
        ]"#;
        let parsed: Vec<super::RawReview> = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.len(), 3);

        // Only non-empty bodies should pass the filter
        let non_empty: Vec<_> = parsed
            .iter()
            .filter(|r| {
                r.body
                    .as_ref()
                    .map(|b| !b.trim().is_empty())
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(non_empty.len(), 1);
        assert_eq!(non_empty[0].user.as_ref().unwrap().login, "carol");
    }

    #[test]
    fn comment_endpoint_serialization_roundtrip() {
        for endpoint in [
            super::CommentEndpoint::PullComment,
            super::CommentEndpoint::IssueComment,
            super::CommentEndpoint::Review,
        ] {
            let json = serde_json::to_string(&endpoint).unwrap();
            let parsed: super::CommentEndpoint = serde_json::from_str(&json).unwrap();
            assert_eq!(endpoint, parsed);
        }
    }

    #[test]
    fn merge_paginated_json_arrays_single() {
        let input = r#"[{"a":1},{"b":2}]"#;
        let result = super::merge_paginated_json_arrays(input).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn merge_paginated_json_arrays_multi() {
        let input = r#"[{"a":1}][{"b":2}]"#;
        let result = super::merge_paginated_json_arrays(input).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn merge_paginated_json_arrays_empty() {
        let result = super::merge_paginated_json_arrays("").unwrap();
        assert_eq!(result, "[]");
    }

    #[test]
    fn merge_paginated_json_arrays_brackets_in_strings() {
        // Comment body contains brackets that would confuse naive bracket counting.
        let input = r#"[{"body":"fix [this] and ]that["}][{"body":"ok"}]"#;
        let result = super::merge_paginated_json_arrays(input).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["body"], "fix [this] and ]that[");
        assert_eq!(parsed[1]["body"], "ok");
    }

    #[test]
    fn merge_paginated_json_arrays_invalid_json_returns_error() {
        let input = r#"[{"a":1}][not valid json"#;
        let result = super::merge_paginated_json_arrays(input);
        assert!(result.is_err());
    }

    // ------------------------------------------------------------------
    // PR review comment parsing: null/missing user resilience
    // ------------------------------------------------------------------

    #[test]
    fn parse_pull_comments_skips_null_user_keeps_valid() {
        let json = r#"[
            {"id":1,"user":{"login":"alice"},"body":"fix this","path":"src/main.rs","line":10,"created_at":"2024-01-01T00:00:00Z","in_reply_to_id":null},
            {"id":2,"user":null,"body":"ghost comment","path":"src/lib.rs","line":5,"created_at":"2024-01-01T00:00:00Z","in_reply_to_id":null},
            {"id":3,"user":{"login":"bob"},"body":"also fix","path":null,"line":null,"created_at":"2024-01-01T00:00:00Z","in_reply_to_id":null}
        ]"#;
        let comments = super::parse_pull_comments(json, 42);
        assert_eq!(comments.len(), 2, "null-user entry should be skipped");
        assert_eq!(comments[0].author, "alice");
        assert_eq!(comments[0].id, 1);
        assert_eq!(comments[1].author, "bob");
        assert_eq!(comments[1].id, 3);
    }

    #[test]
    fn parse_pull_comments_skips_missing_user_field() {
        // user field entirely absent (serde(default) → None)
        let json = r#"[
            {"id":1,"body":"no user field","path":null,"line":null,"created_at":"2024-01-01T00:00:00Z","in_reply_to_id":null},
            {"id":2,"user":{"login":"valid"},"body":"ok","path":null,"line":null,"created_at":"2024-01-01T00:00:00Z","in_reply_to_id":null}
        ]"#;
        let comments = super::parse_pull_comments(json, 1);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author, "valid");
    }

    #[test]
    fn parse_pull_comments_skips_empty_login() {
        let json = r#"[
            {"id":1,"user":{"login":""},"body":"empty login","path":null,"line":null,"created_at":"2024-01-01T00:00:00Z","in_reply_to_id":null},
            {"id":2,"user":{"login":"real"},"body":"ok","path":null,"line":null,"created_at":"2024-01-01T00:00:00Z","in_reply_to_id":null}
        ]"#;
        let comments = super::parse_pull_comments(json, 1);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author, "real");
    }

    #[test]
    fn parse_pull_comments_returns_empty_on_malformed_json() {
        let json = r#"not valid json"#;
        let comments = super::parse_pull_comments(json, 42);
        assert!(
            comments.is_empty(),
            "malformed JSON should return empty vec, not error"
        );
    }

    #[test]
    fn parse_issue_comments_skips_null_user_keeps_valid() {
        let json = r#"[
            {"id":10,"user":{"login":"alice"},"body":"looks good","created_at":"2024-01-01T00:00:00Z"},
            {"id":11,"user":null,"body":"ghost","created_at":"2024-01-01T00:00:00Z"},
            {"id":12,"user":{"login":"bob"},"body":"please fix","created_at":"2024-01-01T00:00:00Z"}
        ]"#;
        let comments = super::parse_issue_comments(json, 7);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].author, "alice");
        assert_eq!(comments[1].author, "bob");
    }

    #[test]
    fn parse_issue_comments_returns_empty_on_malformed_json() {
        let comments = super::parse_issue_comments("}{bad", 7);
        assert!(comments.is_empty());
    }

    #[test]
    fn parse_review_summaries_skips_null_user_keeps_valid() {
        let json = r#"[
            {"id":100,"user":{"login":"reviewer"},"body":"needs work","state":"CHANGES_REQUESTED","submitted_at":"2024-01-01T00:00:00Z"},
            {"id":101,"user":null,"body":"ghost review","state":"COMMENTED","submitted_at":"2024-01-01T00:00:00Z"},
            {"id":102,"user":{"login":"lead"},"body":"approved","state":"APPROVED","submitted_at":"2024-01-01T00:00:00Z"}
        ]"#;
        let comments = super::parse_review_summaries(json, 99);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].author, "reviewer");
        assert_eq!(comments[1].author, "lead");
    }

    #[test]
    fn parse_review_summaries_returns_empty_on_malformed_json() {
        let comments = super::parse_review_summaries("[invalid", 99);
        assert!(comments.is_empty());
    }

    #[test]
    fn parse_review_summaries_skips_empty_body_even_with_valid_user() {
        let json = r#"[
            {"id":200,"user":{"login":"reviewer"},"body":"","state":"COMMENTED","submitted_at":"2024-01-01T00:00:00Z"},
            {"id":201,"user":{"login":"reviewer"},"body":"real feedback","state":"COMMENTED","submitted_at":"2024-01-01T00:00:00Z"}
        ]"#;
        let comments = super::parse_review_summaries(json, 5);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, 201);
    }
}
