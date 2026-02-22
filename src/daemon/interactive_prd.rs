//! State and helper utilities for the daemon's interactive PRD workflow.
//!
//! This module contains the state machine, persistence, transition logic, and
//! question-generation orchestration for the interactive PRD flow triggered by
//! `ralph:prd` issues.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::backend::{claude, codex, parse_backend_spec, Backend, CliBackend};
use crate::config::GlobalConfig;
use crate::daemon::github::{self, GhIssue};
use crate::error::RalphError;
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

// ---------------------------------------------------------------------------
// Runtime configuration for the interactive PRD poll phase
// ---------------------------------------------------------------------------

/// Runtime configuration for the interactive PRD poll phase.
#[derive(Debug, Clone)]
pub struct PrdPollConfig {
    pub owner: String,
    pub repo: String,
    pub data_dir: PathBuf,
    pub prd_enabled: bool,
    pub question_backends: Vec<String>,
    pub writer_backend: String,
    pub reviewer_backend: String,
    pub max_revisions: u32,
    pub backend_timeout_secs: u64,
    pub global_config: GlobalConfig,
    pub verbose: bool,
}

/// All PRD lifecycle label names.
pub const PRD_LABEL_NAMES: &[&str] = &[
    "ralph:prd",
    "ralph:prd-active",
    "ralph:prd-approved",
    "ralph:prd-done",
    "ralph:prd-failed",
];

/// Returns `true` if any PRD lifecycle label is present on the issue.
pub fn has_prd_label(labels: &[String]) -> bool {
    labels.iter().any(|l| PRD_LABEL_NAMES.contains(&l.as_str()))
}

// ---------------------------------------------------------------------------
// Backend helpers
// ---------------------------------------------------------------------------

/// Create a CLI backend from a backend spec string and global config.
fn create_backend(backend_spec: &str, global_config: &GlobalConfig) -> Result<CliBackend> {
    let spec = parse_backend_spec(backend_spec)?;
    let model = spec.model.as_deref();
    match spec.name.as_str() {
        "claude" => Ok(claude::backend_from_config(global_config, model, None)),
        "codex" => Ok(codex::backend_from_config(global_config, model, None)),
        _ => Err(RalphError::Validation(format!(
            "unknown PRD backend: {backend_spec}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Question-generation prompts
// ---------------------------------------------------------------------------

const QUESTION_GEN_PROMPT: &str = r#"You are an engineering specification analyst. Given the following GitHub issue that describes a feature idea, generate 3-5 clarifying questions that will help produce a complete engineering specification.

Focus on:
- Missing technical details (API design, data models, error handling)
- Scope ambiguities (what's in/out of scope)
- Non-functional requirements (performance, security, compatibility)
- User experience details (edge cases, error states)

Output ONLY a numbered list of questions, one per line. Do not include any preamble or explanation.

--- ISSUE ---
"#;

const SYNTHESIS_PROMPT: &str = r#"You are an engineering specification analyst. Given two sets of clarifying questions generated for the same feature, merge and deduplicate them into a single numbered list of 3-7 prioritized questions.

Rules:
- Remove exact or near-duplicate questions
- Combine related questions into a single more precise question
- Prioritize questions that address the most critical unknowns first
- Output ONLY a numbered list of questions, one per line
- Do not include preamble, explanation, or commentary

--- QUESTION SET A ---
{questions_a}

--- QUESTION SET B ---
{questions_b}
"#;

// ---------------------------------------------------------------------------
// Poll and advance: the main entry point called from the daemon runtime
// ---------------------------------------------------------------------------

/// Poll for `ralph:prd` issues and advance at most one transition per issue.
///
/// This function is called once per daemon poll tick when `prd_enabled` is true.
/// It runs synchronously (blocking) because the daemon wraps it in
/// `spawn_blocking`.
///
/// Enforces the spec invariant "at most one state transition per issue per tick"
/// by deduplicating issue numbers across both poll passes.
pub fn poll_and_advance_prd(config: &PrdPollConfig) -> Result<()> {
    let mut processed: std::collections::HashSet<u32> = std::collections::HashSet::new();

    let labels = vec!["ralph:prd".to_owned()];
    let (issues, _overflow) = github::poll_issues(&config.owner, &config.repo, &labels)?;

    for issue in &issues {
        if !processed.insert(issue.number) {
            continue;
        }
        if let Err(err) = advance_issue(config, issue) {
            eprintln!(
                "prd: failed to advance {}/{}#{}: {err}",
                config.owner, config.repo, issue.number
            );
        }
    }

    // Also process issues that are in prd-active state (already picked up).
    let active_labels = vec!["ralph:prd-active".to_owned()];
    let (active_issues, _) =
        github::poll_issues(&config.owner, &config.repo, &active_labels)?;

    for issue in &active_issues {
        if !processed.insert(issue.number) {
            continue; // already advanced in the ralph:prd pass
        }
        if let Err(err) = advance_issue(config, issue) {
            eprintln!(
                "prd: failed to advance active {}/{}#{}: {err}",
                config.owner, config.repo, issue.number
            );
        }
    }

    Ok(())
}

/// Advance a single issue by at most one state transition.
fn advance_issue(config: &PrdPollConfig, issue: &GhIssue) -> Result<()> {
    let mut state = InteractivePrdState::load(
        &config.data_dir,
        &config.owner,
        &config.repo,
        issue.number,
    )?
    .unwrap_or_else(|| {
        InteractivePrdState::new(&config.owner, &config.repo, issue.number)
    });

    if state.is_terminal() {
        return Ok(());
    }

    match state.state.clone() {
        PrdWorkflowState::Pending => {
            transition_pending_to_awaiting_answers(config, issue, &mut state)
        }
        // Future transitions (AwaitingAnswers, AwaitingFeedback) will be
        // implemented in subsequent loops.
        PrdWorkflowState::AwaitingAnswers
        | PrdWorkflowState::AwaitingFeedback => Ok(()),
        PrdWorkflowState::Done | PrdWorkflowState::Failed => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Pending -> AwaitingAnswers transition
// ---------------------------------------------------------------------------

/// Execute the Pending -> AwaitingAnswers transition:
/// 1. Swap labels: remove `ralph:prd`, add `ralph:prd-active`.
/// 2. If `ralph:ready` exists, remove it.
/// 3. Generate clarifying questions using two backends + synthesis.
/// 4. Post questions comment with idempotent marker.
/// 5. Persist updated state.
fn transition_pending_to_awaiting_answers(
    config: &PrdPollConfig,
    issue: &GhIssue,
    state: &mut InteractivePrdState,
) -> Result<()> {
    let issue_number = issue.number;
    let owner = &config.owner;
    let repo = &config.repo;

    if config.verbose {
        eprintln!(
            "prd: transition Pending->AwaitingAnswers for {owner}/{repo}#{issue_number}"
        );
    }

    let result = do_pending_to_awaiting(config, issue, state);

    match result {
        Ok(()) => {
            state.error_count = 0;
            state.last_error = None;
            state.save(&config.data_dir)?;
            Ok(())
        }
        Err(err) => {
            state.error_count += 1;
            state.last_error = Some(err.to_string());

            if state.error_count >= 3 {
                // Transition to Failed
                transition_to_failed(config, state)?;
            } else {
                // Persist incremented error_count for retry next tick
                state.save(&config.data_dir)?;
            }

            Err(err)
        }
    }
}

fn do_pending_to_awaiting(
    config: &PrdPollConfig,
    issue: &GhIssue,
    state: &mut InteractivePrdState,
) -> Result<()> {
    let issue_number = issue.number;
    let owner = &config.owner;
    let repo = &config.repo;

    // 1. Swap labels idempotently: only remove ralph:prd if still present,
    //    only add ralph:prd-active if not already present. This prevents
    //    failures on retry when labels were already swapped in a prior attempt.
    let has_prd = issue.labels.iter().any(|l| l == "ralph:prd");
    let has_active = issue.labels.iter().any(|l| l == "ralph:prd-active");

    if has_prd {
        let _ = github::remove_label_with_retry(owner, repo, issue_number, "ralph:prd");
    }
    if !has_active {
        github::add_label_with_retry(owner, repo, issue_number, "ralph:prd-active")
            .map_err(|err| {
                RalphError::InteractivePrdFailed(format!(
                    "failed to add ralph:prd-active for {owner}/{repo}#{issue_number}: {err}"
                ))
            })?;
    }

    // 2. Remove ralph:ready if present (prevent dual workflow ownership)
    if issue.labels.iter().any(|l| l == "ralph:ready") {
        let _ = github::remove_label_with_retry(owner, repo, issue_number, "ralph:ready");
    }

    // 3. Generate questions with timeout
    let issue_text = format!(
        "{}\n\n{}",
        issue.title,
        issue.body.as_deref().unwrap_or_default()
    );

    let questions = generate_questions_with_timeout(config, &issue_text)?;

    // 4. Post questions comment with idempotent marker
    let next_revision = state.question_revision + 1;
    let marker = prd_marker(issue_number, "questions", next_revision);

    let comment_body = format!(
        "## Clarifying Questions\n\n\
         Before generating the engineering specification, I need some clarification. \
         Please answer the following questions in a reply to this comment:\n\n\
         {questions}\n\n\
         *Reply to this comment with your answers and I'll generate a draft spec.*"
    );

    let comment_id =
        github::post_comment_with_marker(owner, repo, issue_number, &marker, &comment_body)
            .map_err(|err| {
                RalphError::InteractivePrdFailed(format!(
                    "failed to post questions comment for {owner}/{repo}#{issue_number}: {err}"
                ))
            })?;

    // 5. Update and persist state
    state.state = PrdWorkflowState::AwaitingAnswers;
    state.question_revision = next_revision;
    state.questions_comment_id = comment_id;
    state.questions_posted_at = Some(Utc::now());
    state.last_advanced_at = Some(Utc::now());

    Ok(())
}

/// Generate clarifying questions using two configured backends plus synthesis.
///
/// All backend work is bounded by `backend_timeout_secs` as total wall-clock.
fn generate_questions_with_timeout(
    config: &PrdPollConfig,
    issue_text: &str,
) -> Result<String> {
    if config.question_backends.len() != 2 {
        return Err(RalphError::InteractivePrdFailed(format!(
            "expected exactly 2 question backends, got {}",
            config.question_backends.len()
        )));
    }

    let timeout = Duration::from_secs(config.backend_timeout_secs);
    let deadline = std::time::Instant::now() + timeout;

    let prompt = format!("{QUESTION_GEN_PROMPT}{issue_text}");

    // Backend A
    let backend_a = create_backend(&config.question_backends[0], &config.global_config)?;
    let questions_a = run_backend_sync(&backend_a, &prompt, deadline)?;

    // Backend B
    let backend_b = create_backend(&config.question_backends[1], &config.global_config)?;
    let questions_b = run_backend_sync(&backend_b, &prompt, deadline)?;

    // Synthesis: merge/dedupe/prioritize
    let synthesis_prompt = SYNTHESIS_PROMPT
        .replace("{questions_a}", &questions_a)
        .replace("{questions_b}", &questions_b);

    // Use the first question backend for synthesis
    let synthesized = run_backend_sync(&backend_a, &synthesis_prompt, deadline)?;

    if synthesized.trim().is_empty() {
        return Err(RalphError::InteractivePrdFailed(
            "synthesis produced empty output".to_owned(),
        ));
    }

    Ok(synthesized)
}

/// Run a backend synchronously with a deadline, using tokio runtime.
fn run_backend_sync(
    backend: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
) -> Result<String> {
    let remaining = deadline
        .checked_duration_since(std::time::Instant::now())
        .unwrap_or(Duration::ZERO);

    if remaining.is_zero() {
        return Err(RalphError::InteractivePrdFailed(
            "PRD backend timeout exceeded".to_owned(),
        ));
    }

    // Create a runtime for blocking backend execution
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            RalphError::InteractivePrdFailed(format!("failed to create tokio runtime: {err}"))
        })?;

    let result = rt.block_on(async {
        tokio::time::timeout(remaining, backend.execute(prompt)).await
    });

    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(err)) => Err(RalphError::InteractivePrdFailed(format!(
            "backend execution failed: {err}"
        ))),
        Err(_) => Err(RalphError::InteractivePrdFailed(
            "PRD backend timeout exceeded".to_owned(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Failure transition
// ---------------------------------------------------------------------------

/// Transition an issue to the Failed state.
fn transition_to_failed(config: &PrdPollConfig, state: &mut InteractivePrdState) -> Result<()> {
    let owner = &config.owner;
    let repo = &config.repo;
    let issue_number = state.issue_number;

    // Post error comment marker
    let marker = format!("<!-- ralph:prd:{issue_number}:status-failed -->");
    let error_body = format!(
        "## PRD Workflow Failed\n\n\
         The interactive PRD workflow has failed after {} consecutive errors.\n\n\
         Last error: {}\n\n\
         *Apply the `ralph:prd` label again to retry.*",
        state.error_count,
        state.last_error.as_deref().unwrap_or("unknown")
    );

    let _ = github::post_comment_with_marker(owner, repo, issue_number, &marker, &error_body);

    // Swap labels: remove ralph:prd-active, add ralph:prd-failed
    // Best-effort: the label may not exist if we failed during Pending
    let _ = github::remove_label_with_retry(owner, repo, issue_number, "ralph:prd-active");
    let _ = github::remove_label_with_retry(owner, repo, issue_number, "ralph:prd");
    let _ = github::add_label_with_retry(owner, repo, issue_number, "ralph:prd-failed");

    state.state = PrdWorkflowState::Failed;
    state.last_advanced_at = Some(Utc::now());
    state.save(&config.data_dir)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Marker status-failed format (no version)
// ---------------------------------------------------------------------------

pub fn prd_status_failed_marker(issue_number: u32) -> String {
    format!("<!-- ralph:prd:{issue_number}:status-failed -->")
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
