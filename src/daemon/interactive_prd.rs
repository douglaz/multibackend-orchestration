//! State and helper utilities for the daemon's interactive PRD workflow.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::Result;

/// PRD workflow states persisted for daemon restart-safety.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum PrdWorkflowState {
    Pending,
    AwaitingAnswers,
    AwaitingFeedback,
    Done,
    Failed,
}

/// Persisted per-issue interactive PRD workflow state.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct InteractivePrdState {
    pub issue_number: u32,
    pub owner: String,
    pub repo: String,
    pub state: PrdWorkflowState,
    pub question_revision: u32,
    pub draft_revision: u32,
    pub questions_comment_id: Option<u64>,
    pub questions_posted_at: Option<DateTime<Utc>>,
    pub latest_draft_comment_id: Option<u64>,
    pub latest_draft_body: Option<String>,
    pub user_answers: Option<String>,
    pub last_processed_comment_id: Option<u64>,
    pub error_count: u32,
    pub last_error: Option<String>,
    pub last_advanced_at: Option<DateTime<Utc>>,
}

impl InteractivePrdState {
    pub fn new(owner: impl Into<String>, repo: impl Into<String>, issue_number: u32) -> Self {
        Self {
            issue_number,
            owner: owner.into(),
            repo: repo.into(),
            state: PrdWorkflowState::Pending,
            question_revision: 0,
            draft_revision: 0,
            questions_comment_id: None,
            questions_posted_at: None,
            latest_draft_comment_id: None,
            latest_draft_body: None,
            user_answers: None,
            last_processed_comment_id: None,
            error_count: 0,
            last_error: None,
            last_advanced_at: None,
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = state_path(data_dir, &self.owner, &self.repo, self.issue_number);
        let parent = path.parent().expect("state path should have a parent");
        fs::create_dir_all(parent)?;

        let payload = serde_json::to_vec_pretty(self)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(&payload)?;
        tmp.flush()?;
        tmp.as_file().sync_all()?;
        tmp.persist(&path)
            .map_err(|err| std::io::Error::new(err.error.kind(), err.error.to_string()))?;

        Ok(())
    }

    pub fn load(
        data_dir: &Path,
        owner: &str,
        repo: &str,
        issue_number: u32,
    ) -> Result<Option<Self>> {
        let path = state_path(data_dir, owner, repo, issue_number);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        let state = serde_json::from_str::<Self>(&raw)?;
        Ok(Some(state))
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            PrdWorkflowState::Done | PrdWorkflowState::Failed
        )
    }
}

pub fn detect_approval(text: &str) -> bool {
    let scrubbed = strip_code(text);

    let negative_patterns = [
        r"(?i)\bnot\s+approved\b",
        r"(?i)\bdon['’]t\s+approve\b",
        r"(?i)\bdo\s+not\s+approve\b",
        r"(?i)\bnot\s+lgtm\b",
    ];
    let positive_patterns = [
        r"(?i)\bapproved\b",
        r"(?i)\blgtm\b",
        r"(?i)\bship\s+it\b",
        r"(?i)\blooks\s+good\b",
    ];

    let has_negative = has_pattern_match(&scrubbed, &negative_patterns);
    let has_positive = has_pattern_match(&scrubbed, &positive_patterns);

    if has_negative && has_positive {
        return false;
    }
    if has_negative {
        return false;
    }

    has_positive
}

pub fn prd_marker(issue_number: u32, kind: &str, version: u32) -> String {
    format!("<!-- ralph:prd:{issue_number}:{kind}-v{version} -->")
}

pub const PRD_LABELS: &[(&str, &str, &str)] = &[
    (
        "ralph:prd",
        "#5319e7",
        "Issue is queued for the interactive PRD workflow",
    ),
    (
        "ralph:prd-active",
        "#fbca04",
        "Interactive PRD workflow is actively processing this issue",
    ),
    (
        "ralph:prd-approved",
        "#1d76db",
        "Interactive PRD draft has been approved",
    ),
    (
        "ralph:prd-done",
        "#0e8a16",
        "Interactive PRD workflow is complete",
    ),
    (
        "ralph:prd-failed",
        "#d93f0b",
        "Interactive PRD workflow failed and needs attention",
    ),
];

pub const PRD_LIFECYCLE_LABELS: &[(&str, &str, &str)] = PRD_LABELS;

fn state_path(data_dir: &Path, owner: &str, repo: &str, issue_number: u32) -> PathBuf {
    data_dir
        .join(owner)
        .join(repo)
        .join(".ralph")
        .join("interactive-prd")
        .join(format!("{issue_number}.json"))
}

fn strip_code(text: &str) -> String {
    let fenced = Regex::new(r"(?s)```.*?```").expect("fenced code regex should compile");
    let without_fences = fenced.replace_all(text, " ");

    let inline = Regex::new(r"`[^`\n]*`").expect("inline code regex should compile");
    inline.replace_all(&without_fences, " ").into_owned()
}

fn has_pattern_match(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| {
        Regex::new(pattern)
            .expect("approval regex should compile")
            .is_match(text)
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{
        detect_approval, prd_marker, InteractivePrdState, PrdWorkflowState, PRD_LABELS,
        PRD_LIFECYCLE_LABELS,
    };

    #[test]
    fn prd_workflow_state_serialization_roundtrip_for_all_variants() {
        let variants = [
            PrdWorkflowState::Pending,
            PrdWorkflowState::AwaitingAnswers,
            PrdWorkflowState::AwaitingFeedback,
            PrdWorkflowState::Done,
            PrdWorkflowState::Failed,
        ];

        for state in variants {
            let json = serde_json::to_string(&state).expect("serialize state");
            let parsed: PrdWorkflowState = serde_json::from_str(&json).expect("deserialize state");
            assert_eq!(parsed, state);
        }
    }

    #[test]
    fn detect_approval_positive_cases() {
        assert!(detect_approval("Approved."));
        assert!(detect_approval("LGTM, ship it"));
        assert!(detect_approval("Looks good to me"));
        assert!(detect_approval("This is approved!"));
    }

    #[test]
    fn detect_approval_negative_cases() {
        assert!(!detect_approval("not approved"));
        assert!(!detect_approval("do not approve this yet"));
        assert!(!detect_approval("don't approve until tests pass"));
        assert!(!detect_approval("not lgtm"));
    }

    #[test]
    fn detect_approval_strips_fenced_and_inline_code() {
        assert!(!detect_approval("```\napproved\n```"));
        assert!(!detect_approval("Please review: `lgtm`"));
        assert!(detect_approval("`not approved` but approved"));
        assert!(detect_approval("approved\n```\nnot approved\n```"));
    }

    #[test]
    fn detect_approval_mixed_signals_return_false() {
        assert!(!detect_approval("approved, but do not approve yet"));
        assert!(!detect_approval("looks good, not approved for merge"));
    }

    #[test]
    fn detect_approval_uses_word_boundaries() {
        assert!(!detect_approval("preapproved"));
        assert!(!detect_approval("thelgtmcheckfailed"));
        assert!(detect_approval("approved,"));
        assert!(detect_approval("ship it."));
    }

    #[test]
    fn marker_generation_matches_expected_format() {
        assert_eq!(prd_marker(42, "draft", 3), "<!-- ralph:prd:42:draft-v3 -->");
    }

    #[test]
    fn is_terminal_only_true_for_done_and_failed() {
        let mut state = InteractivePrdState::new("acme", "widgets", 7);

        state.state = PrdWorkflowState::Pending;
        assert!(!state.is_terminal());

        state.state = PrdWorkflowState::AwaitingAnswers;
        assert!(!state.is_terminal());

        state.state = PrdWorkflowState::AwaitingFeedback;
        assert!(!state.is_terminal());

        state.state = PrdWorkflowState::Done;
        assert!(state.is_terminal());

        state.state = PrdWorkflowState::Failed;
        assert!(state.is_terminal());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().expect("create tempdir");

        let mut state = InteractivePrdState::new("octo", "repo", 99);
        state.state = PrdWorkflowState::AwaitingFeedback;
        state.question_revision = 2;
        state.draft_revision = 3;
        state.questions_comment_id = Some(1234);
        state.questions_posted_at = Some(Utc::now());
        state.latest_draft_comment_id = Some(5678);
        state.latest_draft_body = Some("draft body".to_owned());
        state.user_answers = Some("answer block".to_owned());
        state.last_processed_comment_id = Some(9999);
        state.error_count = 1;
        state.last_error = Some("transient failure".to_owned());
        state.last_advanced_at = Some(Utc::now());

        state.save(tmp.path()).expect("save state");

        let loaded = InteractivePrdState::load(tmp.path(), "octo", "repo", 99)
            .expect("load state")
            .expect("state should exist");

        assert_eq!(loaded, state);
    }

    #[test]
    fn load_returns_none_when_state_file_missing() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let loaded = InteractivePrdState::load(tmp.path(), "octo", "repo", 404)
            .expect("load should succeed for missing file");
        assert!(loaded.is_none());
    }

    #[test]
    fn prd_labels_alias_matches_lifecycle_labels() {
        assert_eq!(PRD_LABELS, PRD_LIFECYCLE_LABELS);
        assert_eq!(PRD_LABELS.len(), 5);
    }
}
