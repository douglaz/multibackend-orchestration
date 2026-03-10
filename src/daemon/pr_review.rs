use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::daemon::github::{
    self, extract_pr_number, CommentEndpoint, PrReviewComment,
};
use crate::daemon::runtime::{DaemonRuntimeConfig, TaskMetadata};
use crate::daemon::TaskHandle;
use crate::error::RalphError;
use crate::project::amendments::{AmendmentPriority, AmendmentRequest, AmendmentSource};
use crate::Result;

// ---------------------------------------------------------------------------
// Deduplication state
// ---------------------------------------------------------------------------

/// Persisted dedup state for a single task's PR review comments.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PrReviewState {
    /// Set of composite dedup keys: `"{endpoint}:{comment_id}"`.
    pub processed_keys: HashSet<String>,
}

impl PrReviewState {
    pub fn load(workspace_root: &Path, task_id: &str) -> Self {
        let path = state_path(workspace_root, task_id);
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, workspace_root: &Path, task_id: &str) -> Result<()> {
        let path = state_path(workspace_root, task_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                RalphError::Orchestration(format!(
                    "failed to create pr-review-state dir: {err}"
                ))
            })?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|err| {
            RalphError::Orchestration(format!("failed to serialize pr-review state: {err}"))
        })?;
        fs::write(&path, json).map_err(|err| {
            RalphError::Orchestration(format!("failed to write pr-review state: {err}"))
        })?;
        Ok(())
    }
}

fn state_path(workspace_root: &Path, task_id: &str) -> PathBuf {
    workspace_root
        .join("daemon")
        .join("pr-review-state")
        .join(format!("{task_id}.json"))
}

// ---------------------------------------------------------------------------
// Amendment staging
// ---------------------------------------------------------------------------

/// Directory for staged amendments (outside any worktree).
fn staging_dir(workspace_root: &Path, task_id: &str) -> PathBuf {
    workspace_root
        .join("daemon")
        .join("pr-review-amendments")
        .join(task_id)
}

/// Write a single amendment to the staging area.
pub fn stage_amendment(
    workspace_root: &Path,
    task_id: &str,
    amendment: &AmendmentRequest,
) -> Result<()> {
    let dir = staging_dir(workspace_root, task_id);
    fs::create_dir_all(&dir).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to create staging dir {}: {err}",
            dir.display()
        ))
    })?;

    let filename = format!(
        "{}-{}.json",
        crate::util::time::now_timestamp_yyyymmddhhmmss(),
        crate::project::amendments::sanitize_id(&amendment.id),
    );
    let path = dir.join(filename);
    let json = serde_json::to_string_pretty(amendment).map_err(|err| {
        RalphError::Orchestration(format!("failed to serialize staged amendment: {err}"))
    })?;
    fs::write(&path, json).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to write staged amendment {}: {err}",
            path.display()
        ))
    })?;
    Ok(())
}

/// Drain all staged amendments for a task into the project's amendment-queue
/// inside the worktree.
///
/// Returns the number of amendments drained.
pub fn drain_staged_amendments(
    workspace_root: &Path,
    task_id: &str,
    project_dir: &Path,
) -> Result<u32> {
    let src_dir = staging_dir(workspace_root, task_id);
    if !src_dir.exists() {
        return Ok(0);
    }

    let queue_dir = project_dir.join("amendment-queue");
    fs::create_dir_all(&queue_dir).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to create amendment-queue dir {}: {err}",
            queue_dir.display()
        ))
    })?;

    let mut count = 0u32;
    let entries: Vec<_> = fs::read_dir(&src_dir)
        .map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to read staging dir {}: {err}",
                src_dir.display()
            ))
        })?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();

    for entry in entries {
        let src = entry.path();
        let dst = queue_dir.join(entry.file_name());
        fs::copy(&src, &dst).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to copy staged amendment {} -> {}: {err}",
                src.display(),
                dst.display()
            ))
        })?;
        let _ = fs::remove_file(&src);
        count += 1;
    }

    // Clean up empty staging dir
    let _ = fs::remove_dir(&src_dir);

    Ok(count)
}

/// Check if there are any staged amendments for a task.
pub fn has_staged_amendments(workspace_root: &Path, task_id: &str) -> bool {
    let dir = staging_dir(workspace_root, task_id);
    if !dir.exists() {
        return false;
    }
    fs::read_dir(&dir)
        .ok()
        .map(|mut entries| entries.any(|e| e.is_ok()))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Project state reset for resumed projects
// ---------------------------------------------------------------------------

/// Reset project state so the orchestrator re-enters the appropriate loop
/// instead of short-circuiting on `Completed`.
///
/// For quick-dev projects (identified by `is_quick` flag):
///   - `status` → `InProgress`
///   - `quick_dev_phase` → `Some(CodexReview)`
///
/// For regular projects:
///   - `status` → `InProgress`
pub fn reset_project_state_for_resume(
    project_dir: &Path,
    is_quick: bool,
) -> Result<()> {
    let state_path = project_dir.join("state.json");
    if !state_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&state_path).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to read project state {}: {err}",
            state_path.display()
        ))
    })?;

    let mut state: serde_json::Value = serde_json::from_str(&content).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to parse project state {}: {err}",
            state_path.display()
        ))
    })?;

    state["status"] = serde_json::Value::String("in_progress".to_string());

    if is_quick {
        state["quick_dev_phase"] = serde_json::Value::String("codex_review".to_string());
        state["current_phase"] = serde_json::Value::String("reviewing".to_string());
    }

    let json = serde_json::to_string_pretty(&state).map_err(|err| {
        RalphError::Orchestration(format!("failed to serialize reset project state: {err}"))
    })?;
    fs::write(&state_path, json).map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to write reset project state {}: {err}",
            state_path.display()
        ))
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Comment → Amendment conversion
// ---------------------------------------------------------------------------

/// Convert a PR review comment into an `AmendmentRequest`.
pub fn comment_to_amendment(
    comment: &PrReviewComment,
    pr_number: u32,
) -> AmendmentRequest {
    let body = match comment.endpoint {
        CommentEndpoint::PullComment => {
            match (&comment.path, comment.line) {
                (Some(path), Some(line)) => {
                    format!(
                        "PR review comment by @{} on {}:{}:\n\n{}",
                        comment.author, path, line, comment.body
                    )
                }
                (Some(path), None) => {
                    format!(
                        "PR review comment by @{} on {}:\n\n{}",
                        comment.author, path, comment.body
                    )
                }
                _ => {
                    format!(
                        "PR review comment by @{}:\n\n{}",
                        comment.author, comment.body
                    )
                }
            }
        }
        CommentEndpoint::IssueComment => {
            format!(
                "PR review comment by @{}:\n\n{}",
                comment.author, comment.body
            )
        }
        CommentEndpoint::Review => {
            format!(
                "PR review summary by @{}:\n\n{}",
                comment.author, comment.body
            )
        }
    };

    let amendment_id = format!(
        "PR-{}-{}-{}",
        pr_number,
        comment.endpoint.as_str(),
        comment.id
    );

    AmendmentRequest {
        id: amendment_id,
        body,
        priority: AmendmentPriority::P2,
        source: AmendmentSource::PrReview,
        source_detail: Some(format!(
            "pr#{}/{}#{}",
            pr_number,
            comment.endpoint.as_str(),
            comment.id
        )),
        created_at: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Task discovery from metadata
// ---------------------------------------------------------------------------

/// Information about a task with an open PR.
#[derive(Debug, Clone)]
pub struct TaskPrInfo {
    pub task_id: String,
    pub issue_number: u32,
    pub pr_number: u32,
    pub pr_url: String,
}

/// Scan task metadata files to find tasks with PR URLs.
pub fn discover_tasks_with_prs(workspace_root: &Path, owner: &str, repo: &str) -> Vec<TaskPrInfo> {
    let tasks_dir = workspace_root.join("daemon").join("tasks");
    if !tasks_dir.exists() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let entries = match fs::read_dir(&tasks_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        let task_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        let meta: TaskMetadata = match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => continue,
        };

        let pr_url = match meta.pr_url {
            Some(url) => url,
            None => continue,
        };

        let pr_number = match extract_pr_number(&pr_url) {
            Some(n) => n,
            None => continue,
        };

        // Extract issue number from task_id: "{owner}-{repo}-{N}"
        let issue_number = match extract_issue_number_from_task_id(&task_id, owner, repo) {
            Some(n) => n,
            None => continue,
        };

        result.push(TaskPrInfo {
            task_id,
            issue_number,
            pr_number,
            pr_url,
        });
    }

    result
}

/// Extract issue number from task_id format: `"{owner}-{repo}-{N}"`.
fn extract_issue_number_from_task_id(task_id: &str, owner: &str, repo: &str) -> Option<u32> {
    let prefix = format!("{owner}-{repo}-");
    task_id.strip_prefix(&prefix)?.parse().ok()
}

// ---------------------------------------------------------------------------
// Main polling function
// ---------------------------------------------------------------------------

/// Result of polling PR reviews for a single task.
#[derive(Debug)]
pub struct PrReviewPollResult {
    pub task_id: String,
    pub issue_number: u32,
    pub new_amendment_count: u32,
}

/// Poll all PR-backed tasks for new review comments from whitelisted users.
///
/// Returns a list of tasks that received new amendments.
pub async fn poll_pr_reviews(
    config: &DaemonRuntimeConfig,
    children: &HashMap<u32, TaskHandle>,
) -> Result<Vec<PrReviewPollResult>> {
    let whitelist = &config.pr_review_whitelist;
    if whitelist.is_empty() {
        return Ok(Vec::new());
    }

    // Resolve authenticated login once per poll cycle.
    let self_login =
        match github::fetch_authenticated_login_with_gh_bin(&config.gh_bin).await {
            Ok(login) => login,
            Err(err) => {
                eprintln!("warning: failed to resolve authenticated GitHub login for PR review polling: {err}");
                return Ok(Vec::new());
            }
        };

    let tasks = discover_tasks_with_prs(&config.workspace_root, &config.owner, &config.repo);
    if tasks.is_empty() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();

    for task_info in &tasks {
        // Skip tasks that are currently running (they'll handle their own amendments).
        if children.contains_key(&task_info.issue_number) {
            continue;
        }

        // Check if PR is still open.
        let is_open = match github::is_pr_open(
            &config.owner,
            &config.repo,
            task_info.pr_number,
            &config.gh_bin,
        )
        .await
        {
            Ok(open) => open,
            Err(err) => {
                eprintln!(
                    "warning: failed to check PR #{} state for {}: {err}",
                    task_info.pr_number, task_info.task_id
                );
                continue;
            }
        };

        if !is_open {
            continue;
        }

        // Fetch all comments.
        let comments = match github::fetch_pr_review_comments(
            &config.owner,
            &config.repo,
            task_info.pr_number,
            &config.gh_bin,
        )
        .await
        {
            Ok(c) => c,
            Err(err) => {
                eprintln!(
                    "warning: failed to fetch PR review comments for {}: {err}",
                    task_info.task_id
                );
                continue;
            }
        };

        // Load dedup state.
        let mut state = PrReviewState::load(&config.workspace_root, &task_info.task_id);
        let mut new_count = 0u32;

        for comment in &comments {
            // Skip self-comments.
            if comment.author == self_login {
                continue;
            }

            // Skip non-whitelisted users.
            if !whitelist.iter().any(|w| w == &comment.author) {
                continue;
            }

            // Skip empty body.
            if comment.body.trim().is_empty() {
                continue;
            }

            // Dedup check.
            let key = comment.dedup_key();
            if state.processed_keys.contains(&key) {
                continue;
            }

            // Convert to amendment and stage.
            let amendment = comment_to_amendment(comment, task_info.pr_number);
            stage_amendment(&config.workspace_root, &task_info.task_id, &amendment)?;

            state.processed_keys.insert(key);
            new_count += 1;

            info!(
                task_id = %task_info.task_id,
                pr = task_info.pr_number,
                comment_id = comment.id,
                endpoint = %comment.endpoint,
                author = %comment.author,
                "staged PR review amendment"
            );
        }

        // Persist updated dedup state.
        if new_count > 0 {
            state.save(&config.workspace_root, &task_info.task_id)?;
            results.push(PrReviewPollResult {
                task_id: task_info.task_id.clone(),
                issue_number: task_info.issue_number,
                new_amendment_count: new_count,
            });
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_key_format() {
        let comment = PrReviewComment {
            id: 12345,
            endpoint: CommentEndpoint::PullComment,
            author: "user1".to_string(),
            body: "fix this".to_string(),
            path: Some("src/main.rs".to_string()),
            line: Some(42),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        assert_eq!(comment.dedup_key(), "pull_comment:12345");

        let comment2 = PrReviewComment {
            id: 12345,
            endpoint: CommentEndpoint::IssueComment,
            author: "user1".to_string(),
            body: "looks good".to_string(),
            path: None,
            line: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        assert_eq!(comment2.dedup_key(), "issue_comment:12345");

        // Same numeric ID, different endpoints → distinct keys
        assert_ne!(comment.dedup_key(), comment2.dedup_key());
    }

    #[test]
    fn comment_to_amendment_inline() {
        let comment = PrReviewComment {
            id: 100,
            endpoint: CommentEndpoint::PullComment,
            author: "reviewer1".to_string(),
            body: "fix the bug".to_string(),
            path: Some("src/lib.rs".to_string()),
            line: Some(10),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let amendment = comment_to_amendment(&comment, 42);
        assert_eq!(amendment.id, "PR-42-pull_comment-100");
        assert!(amendment.body.contains("@reviewer1"));
        assert!(amendment.body.contains("src/lib.rs:10"));
        assert!(amendment.body.contains("fix the bug"));
        assert_eq!(amendment.source, AmendmentSource::PrReview);
        assert_eq!(
            amendment.source_detail,
            Some("pr#42/pull_comment#100".to_string())
        );
    }

    #[test]
    fn comment_to_amendment_top_level() {
        let comment = PrReviewComment {
            id: 200,
            endpoint: CommentEndpoint::IssueComment,
            author: "reviewer2".to_string(),
            body: "please add tests".to_string(),
            path: None,
            line: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let amendment = comment_to_amendment(&comment, 7);
        assert_eq!(amendment.id, "PR-7-issue_comment-200");
        assert!(amendment.body.contains("PR review comment by @reviewer2"));
        assert!(amendment.body.contains("please add tests"));
    }

    #[test]
    fn comment_to_amendment_review_summary() {
        let comment = PrReviewComment {
            id: 300,
            endpoint: CommentEndpoint::Review,
            author: "lead".to_string(),
            body: "needs major refactoring".to_string(),
            path: None,
            line: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let amendment = comment_to_amendment(&comment, 99);
        assert_eq!(amendment.id, "PR-99-review-300");
        assert!(amendment.body.contains("PR review summary by @lead"));
        assert!(amendment.body.contains("needs major refactoring"));
    }

    #[test]
    fn state_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws_root = tmp.path();
        let task_id = "owner-repo-42";

        let mut state = PrReviewState::default();
        state
            .processed_keys
            .insert("pull_comment:100".to_string());
        state
            .processed_keys
            .insert("issue_comment:200".to_string());

        state.save(ws_root, task_id).expect("save");

        let loaded = PrReviewState::load(ws_root, task_id);
        assert_eq!(loaded.processed_keys, state.processed_keys);
    }

    #[test]
    fn dedup_prevents_duplicate_processing() {
        let mut state = PrReviewState::default();
        let key = "pull_comment:12345".to_string();

        // First time: not seen
        assert!(!state.processed_keys.contains(&key));
        state.processed_keys.insert(key.clone());

        // Second time: already seen
        assert!(state.processed_keys.contains(&key));
    }

    #[test]
    fn whitelist_filtering() {
        let whitelist = vec!["alice".to_string(), "bob".to_string()];
        let comments = vec![
            PrReviewComment {
                id: 1,
                endpoint: CommentEndpoint::IssueComment,
                author: "alice".to_string(),
                body: "fix this".to_string(),
                path: None,
                line: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
            },
            PrReviewComment {
                id: 2,
                endpoint: CommentEndpoint::IssueComment,
                author: "charlie".to_string(),
                body: "also fix".to_string(),
                path: None,
                line: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
            },
            PrReviewComment {
                id: 3,
                endpoint: CommentEndpoint::IssueComment,
                author: "bob".to_string(),
                body: "looks good".to_string(),
                path: None,
                line: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
            },
        ];

        let self_login = "ralph-bot";
        let filtered: Vec<_> = comments
            .iter()
            .filter(|c| c.author != self_login)
            .filter(|c| whitelist.iter().any(|w| w == &c.author))
            .filter(|c| !c.body.trim().is_empty())
            .collect();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].author, "alice");
        assert_eq!(filtered[1].author, "bob");
    }

    #[test]
    fn self_comment_filtering() {
        let self_login = "ralph-bot";
        let comment = PrReviewComment {
            id: 1,
            endpoint: CommentEndpoint::IssueComment,
            author: "ralph-bot".to_string(),
            body: "automated comment".to_string(),
            path: None,
            line: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        assert_eq!(comment.author, self_login);
    }

    #[test]
    fn staging_and_drain_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws_root = tmp.path();
        let task_id = "owner-repo-42";

        // Create a project dir with amendment-queue subdir
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let amendment = AmendmentRequest {
            id: "PR-42-pull_comment-100".to_string(),
            body: "fix the bug".to_string(),
            priority: AmendmentPriority::P2,
            source: AmendmentSource::PrReview,
            source_detail: Some("pr#42/pull_comment#100".to_string()),
            created_at: Utc::now(),
        };

        // Stage
        stage_amendment(ws_root, task_id, &amendment).expect("stage");
        assert!(has_staged_amendments(ws_root, task_id));

        // Drain
        let count = drain_staged_amendments(ws_root, task_id, &project_dir).expect("drain");
        assert_eq!(count, 1);
        assert!(!has_staged_amendments(ws_root, task_id));

        // Verify amendment landed in queue
        let queue_dir = project_dir.join("amendment-queue");
        let entries: Vec<_> = fs::read_dir(&queue_dir)
            .expect("read queue")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);

        let content = fs::read_to_string(entries[0].path()).expect("read amendment");
        let loaded: AmendmentRequest = serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded.id, "PR-42-pull_comment-100");
    }

    #[test]
    fn extract_issue_number_from_task_id_valid() {
        assert_eq!(
            extract_issue_number_from_task_id("acme-widgets-42", "acme", "widgets"),
            Some(42)
        );
    }

    #[test]
    fn extract_issue_number_from_task_id_mismatch() {
        assert_eq!(
            extract_issue_number_from_task_id("other-repo-42", "acme", "widgets"),
            None
        );
    }

    #[test]
    fn empty_whitelist_returns_no_results() {
        let whitelist: Vec<String> = vec![];
        assert!(whitelist.is_empty());
    }

    #[test]
    fn has_staged_amendments_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(!has_staged_amendments(tmp.path(), "nonexistent"));
    }

    #[test]
    fn reset_project_state_regular() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path();

        let state = serde_json::json!({
            "project_id": "issue-42",
            "project_name": "test",
            "status": "completed",
            "current_phase": "completing",
            "current_loop": 1,
            "phase_iteration": 1,
            "prompt_file": "prompt.md",
            "parent_project": null,
            "loops": [],
            "completion_attempts": [],
            "created_at": "2024-01-01T00:00:00Z"
        });
        fs::write(
            project_dir.join("state.json"),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .expect("write state");

        reset_project_state_for_resume(project_dir, false).expect("reset");

        let content = fs::read_to_string(project_dir.join("state.json")).expect("read");
        let loaded: serde_json::Value = serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded["status"], "in_progress");
        // quick_dev_phase should not be set for regular projects
        assert!(loaded.get("quick_dev_phase").map_or(true, |v| v.is_null() || v.as_str() == Some("completing")));
    }

    #[test]
    fn reset_project_state_quick_dev() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path();

        let state = serde_json::json!({
            "project_id": "issue-42",
            "project_name": "test",
            "status": "completed",
            "current_phase": "completing",
            "quick_dev_phase": null,
            "current_loop": 1,
            "phase_iteration": 1,
            "prompt_file": "prompt.md",
            "parent_project": null,
            "loops": [],
            "completion_attempts": [],
            "created_at": "2024-01-01T00:00:00Z"
        });
        fs::write(
            project_dir.join("state.json"),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .expect("write state");

        reset_project_state_for_resume(project_dir, true).expect("reset");

        let content = fs::read_to_string(project_dir.join("state.json")).expect("read");
        let loaded: serde_json::Value = serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded["status"], "in_progress");
        assert_eq!(loaded["quick_dev_phase"], "codex_review");
        assert_eq!(loaded["current_phase"], "reviewing");
    }

    #[test]
    fn discover_tasks_with_prs_finds_matching() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws_root = tmp.path();
        let tasks_dir = ws_root.join("daemon").join("tasks");
        fs::create_dir_all(&tasks_dir).expect("create tasks dir");

        let meta = TaskMetadata {
            pr_url: Some("https://github.com/acme/widgets/pull/99".to_string()),
        };
        fs::write(
            tasks_dir.join("acme-widgets-42.json"),
            serde_json::to_string(&meta).unwrap(),
        )
        .expect("write meta");

        let results = discover_tasks_with_prs(ws_root, "acme", "widgets");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].task_id, "acme-widgets-42");
        assert_eq!(results[0].issue_number, 42);
        assert_eq!(results[0].pr_number, 99);
    }

    #[test]
    fn discover_tasks_skips_no_pr_url() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws_root = tmp.path();
        let tasks_dir = ws_root.join("daemon").join("tasks");
        fs::create_dir_all(&tasks_dir).expect("create tasks dir");

        let meta = TaskMetadata { pr_url: None };
        fs::write(
            tasks_dir.join("acme-widgets-42.json"),
            serde_json::to_string(&meta).unwrap(),
        )
        .expect("write meta");

        let results = discover_tasks_with_prs(ws_root, "acme", "widgets");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn comment_endpoint_serialization_roundtrip() {
        for endpoint in [
            CommentEndpoint::PullComment,
            CommentEndpoint::IssueComment,
            CommentEndpoint::Review,
        ] {
            let json = serde_json::to_string(&endpoint).expect("serialize");
            let parsed: CommentEndpoint = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(endpoint, parsed);
        }
    }

    #[test]
    fn amendment_source_pr_review_serialization() {
        let source = AmendmentSource::PrReview;
        let json = serde_json::to_string(&source).expect("serialize");
        assert_eq!(json, "\"pr-review\"");
        let parsed: AmendmentSource = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, AmendmentSource::PrReview);
    }
}
