use super::*;

use std::fs;

use chrono::Utc;

use crate::daemon::github::CommentEndpoint;
use crate::daemon::pr_review::{
    self, drain_staged_amendments, has_staged_amendments, reset_project_state_for_resume,
    stage_amendment, PrReviewState,
};
use crate::project::amendments::{AmendmentPriority, AmendmentRequest, AmendmentSource};
use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "pr_review::whitelist_filters_comments",
            func: whitelist_filters_comments,
        },
        ConformanceTest {
            name: "pr_review::completed_project_resumes_with_state_reset",
            func: completed_project_resumes_with_state_reset,
        },
        ConformanceTest {
            name: "pr_review::dedup_across_restart",
            func: dedup_across_restart,
        },
        ConformanceTest {
            name: "pr_review::capacity_deferral_preserves_staged",
            func: capacity_deferral_preserves_staged,
        },
        ConformanceTest {
            name: "pr_review::quick_dev_resume_resets_phase",
            func: quick_dev_resume_resets_phase,
        },
    ]
}

/// Verify that only whitelisted comments produce staged amendments, and
/// non-whitelisted comments are silently ignored.
fn whitelist_filters_comments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let ws_root = h.data_dir();
        let task_id = "acme-widgets-42";
        let whitelist = vec!["alice".to_string(), "bob".to_string()];
        let self_login = "ralph-bot";

        let comments = vec![
            make_comment(1, CommentEndpoint::IssueComment, "alice", "fix auth bug"),
            make_comment(2, CommentEndpoint::IssueComment, "charlie", "also fix this"),
            make_comment(3, CommentEndpoint::PullComment, "bob", "typo on line 10"),
            make_comment(4, CommentEndpoint::IssueComment, "ralph-bot", "status update"),
            make_comment(5, CommentEndpoint::Review, "alice", "needs refactoring"),
        ];

        let mut state = PrReviewState::default();

        for comment in &comments {
            if comment.author == self_login {
                continue;
            }
            if !whitelist.iter().any(|w| w == &comment.author) {
                continue;
            }
            if comment.body.trim().is_empty() {
                continue;
            }
            let key = comment.dedup_key();
            if state.processed_keys.contains(&key) {
                continue;
            }

            let amendment = pr_review::comment_to_amendment(comment, 99);
            stage_amendment(ws_root, task_id, &amendment).expect("stage");
            state.processed_keys.insert(key);
        }

        state.save(ws_root, task_id).expect("save state");

        // Verify: only alice (2 comments) and bob (1 comment) produced amendments.
        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join(task_id);
        let count = fs::read_dir(&staging_dir)
            .expect("read staging")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .count();
        assert_eq!(count, 3, "expected 3 staged amendments (alice x2, bob x1)");

        // Verify dedup state has 3 keys.
        let loaded = PrReviewState::load(ws_root, task_id);
        assert_eq!(loaded.processed_keys.len(), 3);
    })
}

/// Verify that a completed project's state is correctly reset for resume:
/// status → in_progress for regular projects.
fn completed_project_resumes_with_state_reset(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_dir = h.data_dir().join("project-regular");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let state = serde_json::json!({
            "project_id": "issue-42",
            "project_name": "test",
            "status": "completed",
            "current_phase": "completing",
            "current_loop": 3,
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

        // Also stage an amendment to verify drain works.
        let ws_root = h.data_dir();
        let task_id = "acme-widgets-42";
        let amendment = AmendmentRequest {
            id: "PR-99-issue_comment-1".to_string(),
            body: "fix the auth bug".to_string(),
            priority: AmendmentPriority::P2,
            source: AmendmentSource::PrReview,
            source_detail: Some("pr#99/issue_comment#1".to_string()),
            created_at: Utc::now(),
        };
        stage_amendment(ws_root, task_id, &amendment).expect("stage");
        assert!(has_staged_amendments(ws_root, task_id));

        // Reset state for regular (non-quick-dev) project.
        reset_project_state_for_resume(&project_dir, false).expect("reset");

        let content = fs::read_to_string(project_dir.join("state.json")).expect("read state");
        let loaded: serde_json::Value = serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded["status"], "in_progress");
        // current_phase should not be changed for regular projects.
        assert_eq!(loaded["current_phase"], "completing");

        // Drain staged amendments into the project dir.
        let count = drain_staged_amendments(ws_root, task_id, &project_dir).expect("drain");
        assert_eq!(count, 1);
        assert!(!has_staged_amendments(ws_root, task_id));

        // Verify the amendment file exists in the queue.
        let queue_dir = project_dir.join("amendment-queue");
        let entries: Vec<_> = fs::read_dir(&queue_dir)
            .expect("read queue")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);
    })
}

/// Verify that dedup state persists across simulated restarts: a comment
/// processed in one cycle is not re-processed in the next.
fn dedup_across_restart(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let ws_root = h.data_dir();
        let task_id = "acme-widgets-10";

        // Cycle 1: process comment #500.
        let mut state = PrReviewState::default();
        let key = "issue_comment:500".to_string();
        state.processed_keys.insert(key.clone());
        state.save(ws_root, task_id).expect("save cycle 1");

        // Simulate restart: load fresh from disk.
        let reloaded = PrReviewState::load(ws_root, task_id);
        assert!(
            reloaded.processed_keys.contains(&key),
            "dedup key should survive restart"
        );

        // Cycle 2: same comment should be skipped.
        let comment = make_comment(500, CommentEndpoint::IssueComment, "alice", "fix this");
        let dup_key = comment.dedup_key();
        assert_eq!(dup_key, key);
        assert!(
            reloaded.processed_keys.contains(&dup_key),
            "duplicate comment should be detected"
        );
    })
}

/// Verify that staged amendments survive when capacity is exhausted:
/// amendments remain in the staging directory without being drained.
fn capacity_deferral_preserves_staged(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let ws_root = h.data_dir();
        let task_id = "acme-widgets-77";

        // Stage two amendments.
        for i in 1..=2 {
            let amendment = AmendmentRequest {
                id: format!("PR-50-pull_comment-{i}"),
                body: format!("fix item {i}"),
                priority: AmendmentPriority::P2,
                source: AmendmentSource::PrReview,
                source_detail: Some(format!("pr#50/pull_comment#{i}")),
                created_at: Utc::now(),
            };
            stage_amendment(ws_root, task_id, &amendment).expect("stage");
        }

        // Simulate capacity full: do NOT drain. Just verify amendments persist.
        assert!(has_staged_amendments(ws_root, task_id));

        let staging_dir = ws_root
            .join("daemon")
            .join("pr-review-amendments")
            .join(task_id);
        let count = fs::read_dir(&staging_dir)
            .expect("read staging")
            .filter_map(|e| e.ok())
            .count();
        assert_eq!(count, 2, "both staged amendments should persist");

        // On a later cycle (capacity freed), drain should work.
        let project_dir = h.data_dir().join("project-deferred");
        fs::create_dir_all(&project_dir).expect("create project dir");
        let drained = drain_staged_amendments(ws_root, task_id, &project_dir).expect("drain");
        assert_eq!(drained, 2);
        assert!(!has_staged_amendments(ws_root, task_id));
    })
}

/// Verify that quick-dev project resume resets status to in_progress AND
/// sets quick_dev_phase to codex_review to avoid the short-circuit.
fn quick_dev_resume_resets_phase(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_dir = h.data_dir().join("project-quick");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let state = serde_json::json!({
            "project_id": "issue-55",
            "project_name": "quick-test",
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

        // Reset for quick-dev project.
        reset_project_state_for_resume(&project_dir, true).expect("reset");

        let content = fs::read_to_string(project_dir.join("state.json")).expect("read state");
        let loaded: serde_json::Value = serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded["status"], "in_progress");
        assert_eq!(
            loaded["quick_dev_phase"], "codex_review",
            "quick_dev_phase should be reset to codex_review"
        );
        assert_eq!(
            loaded["current_phase"], "reviewing",
            "current_phase should be reset to reviewing"
        );
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_comment(
    id: u64,
    endpoint: CommentEndpoint,
    author: &str,
    body: &str,
) -> crate::daemon::github::PrReviewComment {
    crate::daemon::github::PrReviewComment {
        id,
        endpoint,
        author: author.to_string(),
        body: body.to_string(),
        path: None,
        line: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
