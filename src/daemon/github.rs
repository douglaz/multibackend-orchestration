use std::process::Command;
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

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

/// Poll open issues matching all supplied labels.
///
/// Uses `gh issue list --repo <owner/repo> --limit 100 --json number,title,labels,body`
/// with repeated `--label` arguments (AND semantics).
///
/// Returns `(issues, overflow)` where overflow is true when exactly 100 issues
/// were returned, indicating possible truncation.
pub fn poll_issues(owner: &str, repo: &str, labels: &[String]) -> Result<(Vec<GhIssue>, bool)> {
    poll_issues_with_gh_bin(DEFAULT_GH_BIN, owner, repo, labels)
}

pub fn poll_issues_with_gh_bin(
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

/// Release a previously-claimed issue by removing the `ralph:in-progress`
/// label.
pub fn release_claim(owner: &str, repo: &str, issue_number: u32) -> Result<()> {
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

/// Update the body of a GitHub issue.
pub fn update_issue_body(owner: &str, repo: &str, issue_number: u32, body: &str) -> Result<()> {
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

/// Post a raw comment on an issue without any idempotency check.
///
/// This is used when bot identity is unavailable and body-only marker lookup
/// would be vulnerable to user spoofing.  Callers should include any marker
/// text in `body` themselves.
pub fn post_raw_issue_comment(
    owner: &str,
    repo: &str,
    issue_number: u32,
    body: &str,
) -> Result<()> {
    post_raw_issue_comment_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number, body)
}

pub fn post_raw_issue_comment_with_gh_bin(
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
pub fn create_pr(
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
pub fn has_commits_ahead_of_base(
    worktree_path: &std::path::Path,
    base_branch: &str,
) -> Result<bool> {
    let base = resolve_ahead_base(worktree_path, base_branch)?;
    let range = format!("{base}..HEAD");
    let output = Command::new("git")
        .args(["rev-list", "--count", &range])
        .current_dir(worktree_path)
        .output()
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
fn resolve_ahead_base(worktree_path: &std::path::Path, configured_base: &str) -> Result<String> {
    // Try the configured base as a remote ref first.
    let candidate = format!("origin/{configured_base}");
    let check = Command::new("git")
        .args(["rev-parse", "--verify", &candidate])
        .current_dir(worktree_path)
        .output();
    if check.map(|o| o.status.success()).unwrap_or(false) {
        return Ok(candidate);
    }

    // Configured base not found as remote ref — fall back to auto-detection.
    let detected = detect_base_branch(worktree_path);

    // Verify the detected ref actually resolves.
    let verify = Command::new("git")
        .args(["rev-parse", "--verify", &detected])
        .current_dir(worktree_path)
        .output();
    if verify.map(|o| o.status.success()).unwrap_or(false) {
        return Ok(detected);
    }

    Err(RalphError::Orchestration(format!(
        "cannot resolve base branch for ahead-of-base check: configured '{configured_base}' \
         (tried origin/{configured_base}) and auto-detected '{detected}' are both unresolvable"
    )))
}

/// Mark a draft pull request as ready for review.
pub fn mark_pr_ready(owner: &str, repo: &str, pr_number: u32) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let pr_number_str = pr_number.to_string();
    let output = Command::new("gh")
        .args(["pr", "ready", &pr_number_str, "--repo", &full_repo])
        .output()
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
pub fn is_pr_draft(owner: &str, repo: &str, pr_number: u32) -> Result<bool> {
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
pub fn close_pr(owner: &str, repo: &str, pr_number: u32) -> Result<()> {
    let full_repo = format!("{owner}/{repo}");
    let pr_number_str = pr_number.to_string();
    let output = Command::new("gh")
        .args(["pr", "close", &pr_number_str, "--repo", &full_repo])
        .output()
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
    has_diff_with_base(worktree_path, None)
}

/// Check whether the task branch has diverged from the given base branch
/// (or an auto-detected default if `base_branch` is `None`).
pub fn has_diff_with_base(
    worktree_path: &std::path::Path,
    base_branch: Option<&str>,
) -> Result<bool> {
    // 1. Check uncommitted changes (working tree + index vs HEAD)
    let wt_status = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .current_dir(worktree_path)
        .status()
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
                .output();
            if check.map(|o| o.status.success()).unwrap_or(false) {
                candidate
            } else {
                detect_base_branch(worktree_path)
            }
        }
        None => detect_base_branch(worktree_path),
    };

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

/// Ensure required lifecycle labels exist in the repository.
///
/// This is best-effort and intentionally non-failing: startup must continue
/// even when label creation fails.
pub fn ensure_labels_best_effort(owner: &str, repo: &str) {
    ensure_labels_best_effort_with_gh_bin(DEFAULT_GH_BIN, owner, repo);
}

pub fn ensure_labels_best_effort_with_gh_bin(gh_bin: &str, owner: &str, repo: &str) {
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
            .output();

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
pub fn normalize_multi_lifecycle_labels(
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
        remove_label_with_retry(owner, repo, issue_number, label)?;
    }

    // Ensure ralph:failed is present
    if !lifecycle_labels.iter().any(|l| l == "ralph:failed") {
        add_label_with_retry(owner, repo, issue_number, "ralph:failed")?;
    }

    Ok(true)
}

/// Swap lifecycle labels atomically with retry-on-conflict and retry-on-transient.
///
/// Removes `from_label` and adds `to_label`. Both operations are retried
/// individually with bounded attempts and exponential backoff.
pub fn swap_lifecycle_label(
    owner: &str,
    repo: &str,
    issue_number: u32,
    from_label: &str,
    to_label: &str,
) -> Result<()> {
    remove_label_with_retry(owner, repo, issue_number, from_label)?;
    add_label_with_retry(owner, repo, issue_number, to_label)?;
    Ok(())
}

/// Add a label with retry-on-conflict/transient-failure behavior.
pub fn add_label_with_retry(owner: &str, repo: &str, issue_number: u32, label: &str) -> Result<()> {
    add_label_with_retry_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number, label)
}

pub fn add_label_with_retry_with_gh_bin(
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
            thread::sleep(delay);
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
pub fn remove_label_with_retry(
    owner: &str,
    repo: &str,
    issue_number: u32,
    label: &str,
) -> Result<()> {
    remove_label_with_retry_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number, label)
}

pub fn remove_label_with_retry_with_gh_bin(
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
            thread::sleep(delay);
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
pub fn fetch_issue_labels(owner: &str, repo: &str, issue_number: u32) -> Result<Vec<String>> {
    fetch_issue_labels_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number)
}

pub fn fetch_issue_labels_with_gh_bin(
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

/// A structured issue comment returned by [`fetch_issue_comments`].
#[derive(Debug, Clone)]
pub struct IssueComment {
    pub id: u64,
    pub author_login: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

/// Fetch all comments on an issue as structured data.
///
/// Returns a list of [`IssueComment`] in chronological order.
pub fn fetch_issue_comments(
    owner: &str,
    repo: &str,
    issue_number: u32,
) -> Result<Vec<IssueComment>> {
    fetch_issue_comments_with_gh_bin(DEFAULT_GH_BIN, owner, repo, issue_number)
}

pub fn fetch_issue_comments_with_gh_bin(
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
pub fn fetch_authenticated_login() -> Result<String> {
    fetch_authenticated_login_with_gh_bin(DEFAULT_GH_BIN)
}

pub fn fetch_authenticated_login_with_gh_bin(gh_bin: &str) -> Result<String> {
    let output = Command::new(gh_bin)
        .args(["api", "user", "-q", ".login"])
        .output()
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
pub fn find_comment_with_marker(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
) -> Result<Option<IssueComment>> {
    let comments = fetch_issue_comments(owner, repo, issue_number)?;
    Ok(comments.into_iter().find(|c| c.body.contains(marker)))
}

/// Post a comment on an issue with a marker prefix. If a comment with the same
/// marker already exists, skip posting and return the existing comment's ID.
///
/// Returns the comment ID of the posted (or existing) comment.
pub fn post_comment_with_marker(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
) -> Result<Option<u64>> {
    let meta = post_comment_with_marker_metadata(owner, repo, issue_number, marker, body_text)?;
    Ok(meta.map(|c| c.id))
}

/// Post a comment on an issue with a marker prefix and return full structured
/// metadata (id, created_at, etc.). If a comment with the same marker already
/// exists, skip posting and return the existing comment's metadata.
pub fn post_comment_with_marker_metadata(
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    body_text: &str,
) -> Result<Option<IssueComment>> {
    if let Some(existing) = find_comment_with_marker(owner, repo, issue_number, marker)? {
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
    find_comment_with_marker(owner, repo, issue_number, marker)
}

/// Find a comment with the given marker string authored by the specified bot login.
///
/// Bot-scoped lookup: only matches comments where `author_login == bot_login`
/// AND the body contains the marker string.  User-authored comments with the
/// same marker text are ignored, preventing marker spoofing.
pub fn find_bot_comment_with_marker(
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
}

pub fn find_bot_comment_with_marker_with_gh_bin(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    marker: &str,
    bot_login: &str,
) -> Result<Option<IssueComment>> {
    let comments = fetch_issue_comments_with_gh_bin(gh_bin, owner, repo, issue_number)?;
    Ok(comments
        .into_iter()
        .find(|c| c.author_login == bot_login && c.body.contains(marker)))
}

/// Post a comment on an issue with a marker prefix, using bot-scoped idempotency.
///
/// Only considers existing bot-authored comments when checking for duplicate
/// markers.  User-authored spoof markers are ignored.  Returns `Some(id)` of
/// the posted (or existing bot) comment.
pub fn post_bot_comment_with_marker(
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
}

pub fn post_bot_comment_with_marker_with_gh_bin(
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
    )?;
    Ok(meta.map(|c| c.id))
}

/// Post a comment on an issue with a marker prefix and return full structured
/// metadata, using bot-scoped idempotency.
///
/// Only considers existing bot-authored comments when checking for duplicate
/// markers.  User-authored spoof markers are ignored.
pub fn post_bot_comment_with_marker_metadata(
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
}

pub fn post_bot_comment_with_marker_metadata_with_gh_bin(
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
    )? {
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
}

/// Ensure PRD lifecycle labels exist in the repository (idempotent, best-effort).
pub fn ensure_prd_labels_best_effort(owner: &str, repo: &str) {
    ensure_prd_labels_best_effort_with_gh_bin(DEFAULT_GH_BIN, owner, repo);
}

pub fn ensure_prd_labels_best_effort_with_gh_bin(gh_bin: &str, owner: &str, repo: &str) {
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
            .output();

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

fn parse_authenticated_login(raw: &str) -> Result<String> {
    let login = raw.trim();
    if login.is_empty() {
        return Err(RalphError::Orchestration(
            "gh api user returned empty login".to_owned(),
        ));
    }
    Ok(login.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::process::Command;

    use tempfile::tempdir;

    use super::{
        has_diff, is_invalid_revision_error, parse_issue_list, parse_pr_is_draft,
        parse_pr_merge_info, GhIssue, PrMergeStatus, REQUIRED_LABELS,
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
}
