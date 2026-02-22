//! State and helper utilities for the daemon's interactive PRD workflow.
//!
//! This module contains the state machine, persistence, transition logic, and
//! question-generation orchestration for the interactive PRD flow triggered by
//! `ralph:prd` issues.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::backend::{claude, codex, parse_backend_spec, Backend, CliBackend};
use crate::config::GlobalConfig;
use crate::daemon::github::{self, GhIssue};
use crate::error::RalphError;
use crate::prd::quick::{
    check_spec_sections, format_issues, render_prompt, run_review_with_retry, DRAFT_PROMPT,
    REVIEW_PROMPT, REVISION_PROMPT,
};
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

const INTERACTIVE_DRAFT_CONTEXT_TEMPLATE: &str = r#"Generate an implementation-ready engineering specification from the following interactive issue context.

## Original Issue
{issue}

## Clarifying Questions Asked
{questions}

## User Answers
{answers}
"#;

const DRAFT_SECTION_RETRIES: u8 = 2;
const REQUIRED_SPEC_SECTION_COUNT: usize = 6;

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
    let mut bot_login_cache: Option<String> = None;

    let labels = vec!["ralph:prd".to_owned()];
    let (issues, _overflow) = github::poll_issues(&config.owner, &config.repo, &labels)?;

    for issue in &issues {
        if !processed.insert(issue.number) {
            continue;
        }
        if let Err(err) = advance_issue(config, issue, &mut bot_login_cache) {
            eprintln!(
                "prd: failed to advance {}/{}#{}: {err}",
                config.owner, config.repo, issue.number
            );
        }
    }

    // Also process issues that are in prd-active state (already picked up).
    let active_labels = vec!["ralph:prd-active".to_owned()];
    let (active_issues, _) = github::poll_issues(&config.owner, &config.repo, &active_labels)?;

    for issue in &active_issues {
        if !processed.insert(issue.number) {
            continue; // already advanced in the ralph:prd pass
        }
        if let Err(err) = advance_issue(config, issue, &mut bot_login_cache) {
            eprintln!(
                "prd: failed to advance active {}/{}#{}: {err}",
                config.owner, config.repo, issue.number
            );
        }
    }

    Ok(())
}

/// Advance a single issue by at most one state transition.
fn advance_issue(
    config: &PrdPollConfig,
    issue: &GhIssue,
    bot_login_cache: &mut Option<String>,
) -> Result<()> {
    let mut state =
        InteractivePrdState::load(&config.data_dir, &config.owner, &config.repo, issue.number)?
            .unwrap_or_else(|| InteractivePrdState::new(&config.owner, &config.repo, issue.number));

    if state.is_terminal() {
        return Ok(());
    }

    match state.state.clone() {
        PrdWorkflowState::Pending => {
            transition_pending_to_awaiting_answers(config, issue, &mut state)
        }
        PrdWorkflowState::AwaitingAnswers => {
            let bot_login = get_or_fetch_bot_login(bot_login_cache)?;
            transition_awaiting_answers_to_awaiting_feedback(config, issue, &mut state, &bot_login)
        }
        PrdWorkflowState::AwaitingFeedback => {
            let bot_login = get_or_fetch_bot_login(bot_login_cache)?;
            transition_awaiting_feedback(config, issue, &mut state, &bot_login)
        }
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
        eprintln!("prd: transition Pending->AwaitingAnswers for {owner}/{repo}#{issue_number}");
    }

    let result = do_pending_to_awaiting(config, issue, state);
    finish_transition(config, state, result)
}

fn do_pending_to_awaiting(
    config: &PrdPollConfig,
    issue: &GhIssue,
    state: &mut InteractivePrdState,
) -> Result<()> {
    let issue_number = issue.number;
    let owner = &config.owner;
    let repo = &config.repo;

    // 1. Swap labels idempotently: add ralph:prd-active BEFORE removing
    //    ralph:prd so that on partial failure the issue remains visible to
    //    future polls (either label will be found). Only remove ralph:prd
    //    once active is confirmed present.
    let has_prd = issue.labels.iter().any(|l| l == "ralph:prd");
    let has_active = issue.labels.iter().any(|l| l == "ralph:prd-active");

    if !has_active {
        github::add_label_with_retry(owner, repo, issue_number, "ralph:prd-active").map_err(
            |err| {
                RalphError::InteractivePrdFailed(format!(
                    "failed to add ralph:prd-active for {owner}/{repo}#{issue_number}: {err}"
                ))
            },
        )?;
    }
    if has_prd {
        let _ = github::remove_label_with_retry(owner, repo, issue_number, "ralph:prd");
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

fn get_or_fetch_bot_login(bot_login_cache: &mut Option<String>) -> Result<String> {
    if let Some(login) = bot_login_cache.clone() {
        return Ok(login);
    }

    let login = github::fetch_authenticated_login().map_err(|err| {
        RalphError::InteractivePrdFailed(format!("failed to resolve authenticated gh login: {err}"))
    })?;
    *bot_login_cache = Some(login.clone());
    Ok(login)
}

/// Execute the AwaitingAnswers -> AwaitingFeedback transition:
/// 1. Find the first unprocessed non-bot answer comment after questions_posted_at.
/// 2. Generate a draft engineering spec from issue + questions + answers.
/// 3. Post draft comment idempotently with draft marker.
/// 4. Persist updated state fields and move to AwaitingFeedback.
fn transition_awaiting_answers_to_awaiting_feedback(
    config: &PrdPollConfig,
    issue: &GhIssue,
    state: &mut InteractivePrdState,
    bot_login: &str,
) -> Result<()> {
    let issue_number = issue.number;
    if config.verbose {
        eprintln!(
            "prd: transition AwaitingAnswers->AwaitingFeedback for {}/{}#{issue_number}",
            config.owner, config.repo
        );
    }

    let result = do_awaiting_answers_to_awaiting_feedback(config, issue, state, bot_login);
    finish_transition(config, state, result)
}

fn do_awaiting_answers_to_awaiting_feedback(
    config: &PrdPollConfig,
    issue: &GhIssue,
    state: &mut InteractivePrdState,
    bot_login: &str,
) -> Result<()> {
    let issue_number = issue.number;
    let owner = &config.owner;
    let repo = &config.repo;

    let questions_posted_at = state.questions_posted_at.ok_or_else(|| {
        RalphError::InteractivePrdFailed(format!(
            "missing questions_posted_at for {owner}/{repo}#{issue_number}"
        ))
    })?;

    let comments = github::fetch_issue_comments(owner, repo, issue_number).map_err(|err| {
        RalphError::InteractivePrdFailed(format!(
            "failed to fetch comments for {owner}/{repo}#{issue_number}: {err}"
        ))
    })?;

    let Some(answer_comment) = find_first_answer_comment(
        &comments,
        questions_posted_at,
        bot_login,
        state.last_processed_comment_id,
    ) else {
        // No user answers yet; remain AwaitingAnswers with no state mutation.
        return Ok(());
    };

    let user_answers = answer_comment.body.trim().to_owned();
    let questions_text = extract_questions_text(
        &comments,
        state.questions_comment_id,
        issue_number,
        state.question_revision,
    );

    let issue_text = format!(
        "{}\n\n{}",
        issue.title,
        issue.body.as_deref().unwrap_or_default()
    );

    let draft_spec = generate_draft_from_answers_with_timeout(
        config,
        &issue_text,
        &questions_text,
        &user_answers,
    )?;

    let next_revision = state.draft_revision + 1;
    let marker = prd_marker(issue_number, "draft", next_revision);
    let draft_comment_body = format!(
        "## Draft Engineering Specification (Revision {next_revision})\n\n{draft_spec}\n\n\
         *Reply with feedback. Reply with \"approved\" or \"lgtm\" when this draft is ready.*"
    );
    let comment_id =
        github::post_comment_with_marker(owner, repo, issue_number, &marker, &draft_comment_body)
            .map_err(|err| {
            RalphError::InteractivePrdFailed(format!(
                "failed to post draft comment for {owner}/{repo}#{issue_number}: {err}"
            ))
        })?;

    state.state = PrdWorkflowState::AwaitingFeedback;
    state.draft_revision = next_revision;
    state.latest_draft_comment_id = comment_id;
    state.latest_draft_body = Some(draft_spec);
    state.user_answers = Some(user_answers);
    state.last_processed_comment_id = Some(answer_comment.id);
    state.last_advanced_at = Some(Utc::now());

    Ok(())
}

// ---------------------------------------------------------------------------
// AwaitingFeedback transition (approval path + revision loop)
// ---------------------------------------------------------------------------

const FEEDBACK_REVISION_PROMPT: &str = r#"You are a senior software engineer revising an engineering specification based on user feedback.

**Current Spec:**
{{spec}}

**User Feedback:**
{{feedback}}

**Task:**
Address each piece of feedback and produce an updated specification. You MUST preserve the same 6 required section headings:
## Summary, ## Acceptance Criteria, ## Technical Approach, ## Files & Modules, ## Testing Strategy, ## Out of Scope
"#;

/// Execute the AwaitingFeedback transition:
/// - If approval detected (label or comment), transition to Done.
/// - If new non-approval feedback exists, generate a revised draft.
/// - If no new feedback and no approval, no-op.
fn transition_awaiting_feedback(
    config: &PrdPollConfig,
    issue: &GhIssue,
    state: &mut InteractivePrdState,
    bot_login: &str,
) -> Result<()> {
    let issue_number = issue.number;
    if config.verbose {
        eprintln!(
            "prd: transition AwaitingFeedback for {}/{}#{issue_number}",
            config.owner, config.repo
        );
    }

    let result = do_awaiting_feedback(config, issue, state, bot_login);
    finish_transition(config, state, result)
}

fn do_awaiting_feedback(
    config: &PrdPollConfig,
    issue: &GhIssue,
    state: &mut InteractivePrdState,
    bot_login: &str,
) -> Result<()> {
    let issue_number = issue.number;
    let owner = &config.owner;
    let repo = &config.repo;

    // Fetch current labels and comments
    let labels = github::fetch_issue_labels(owner, repo, issue_number).map_err(|err| {
        RalphError::InteractivePrdFailed(format!(
            "failed to fetch labels for {owner}/{repo}#{issue_number}: {err}"
        ))
    })?;

    let comments = github::fetch_issue_comments(owner, repo, issue_number).map_err(|err| {
        RalphError::InteractivePrdFailed(format!(
            "failed to fetch comments for {owner}/{repo}#{issue_number}: {err}"
        ))
    })?;

    // Check approval by label
    if labels.iter().any(|l| l == "ralph:prd-approved") {
        return do_approval_transition(config, state, issue_number);
    }

    // Find new unprocessed non-bot comments
    let new_comments = find_new_feedback_comments(
        &comments,
        bot_login,
        state.last_processed_comment_id,
    );

    if new_comments.is_empty() {
        // No new feedback and no approval signal — no-op
        return Ok(());
    }

    // Check if any new comment is an approval
    let has_approval = new_comments.iter().any(|c| detect_approval(&c.body));

    // If any new comment passes approval detection, transition to Done
    if has_approval {
        // Update last_processed_comment_id to the latest comment
        let last_id = new_comments.last().map(|c| c.id);
        if let Some(id) = last_id {
            state.last_processed_comment_id = Some(id);
        }
        return do_approval_transition(config, state, issue_number);
    }

    // Aggregate feedback text for revision
    let aggregated_feedback: String = new_comments
        .iter()
        .map(|c| format!("**@{}:**\n{}", c.author_login, c.body))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    // Update cursor to latest comment
    let last_id = new_comments.last().map(|c| c.id);

    // Generate revised draft
    let current_draft = state
        .latest_draft_body
        .as_deref()
        .unwrap_or("(no previous draft)");

    let revised_spec = generate_revision_from_feedback_with_timeout(
        config,
        current_draft,
        &aggregated_feedback,
    )?;

    // Post new draft comment with incremented marker
    let next_revision = state.draft_revision + 1;
    let marker = prd_marker(issue_number, "draft", next_revision);
    let draft_comment_body = format!(
        "## Draft Engineering Specification (Revision {next_revision})\n\n{revised_spec}\n\n\
         *Reply with feedback. Reply with \"approved\" or \"lgtm\" when this draft is ready.*"
    );
    let comment_id =
        github::post_comment_with_marker(owner, repo, issue_number, &marker, &draft_comment_body)
            .map_err(|err| {
                RalphError::InteractivePrdFailed(format!(
                    "failed to post revision comment for {owner}/{repo}#{issue_number}: {err}"
                ))
            })?;

    // Update state fields
    state.draft_revision = next_revision;
    state.latest_draft_comment_id = comment_id;
    state.latest_draft_body = Some(revised_spec);
    if let Some(id) = last_id {
        state.last_processed_comment_id = Some(id);
    }
    state.last_advanced_at = Some(Utc::now());

    Ok(())
}

/// Transition to Done: post approval marker, swap labels, persist terminal state.
fn do_approval_transition(
    config: &PrdPollConfig,
    state: &mut InteractivePrdState,
    issue_number: u32,
) -> Result<()> {
    let owner = &config.owner;
    let repo = &config.repo;

    // Post idempotent status-approved marker referencing latest draft
    let marker = prd_marker(issue_number, "status-approved", state.draft_revision);
    let approval_body = format!(
        "## PRD Approved\n\n\
         Draft revision {} has been approved.\n\n\
         *The interactive PRD workflow is now complete.*",
        state.draft_revision
    );
    github::post_comment_with_marker(owner, repo, issue_number, &marker, &approval_body)
        .map_err(|err| {
            RalphError::InteractivePrdFailed(format!(
                "failed to post approval comment for {owner}/{repo}#{issue_number}: {err}"
            ))
        })?;

    // Swap labels: remove ralph:prd-active, add ralph:prd-done
    // Keep ralph:prd-approved if already present
    github::remove_label_with_retry(owner, repo, issue_number, "ralph:prd-active").map_err(
        |err| {
            RalphError::InteractivePrdFailed(format!(
                "failed to remove ralph:prd-active for {owner}/{repo}#{issue_number}: {err}"
            ))
        },
    )?;
    github::add_label_with_retry(owner, repo, issue_number, "ralph:prd-done").map_err(|err| {
        RalphError::InteractivePrdFailed(format!(
            "failed to add ralph:prd-done for {owner}/{repo}#{issue_number}: {err}"
        ))
    })?;

    // Persist terminal Done state
    state.state = PrdWorkflowState::Done;
    state.last_advanced_at = Some(Utc::now());

    Ok(())
}

/// Find all new non-bot comments after `last_processed_comment_id`.
fn find_new_feedback_comments<'a>(
    comments: &'a [github::IssueComment],
    bot_login: &str,
    last_processed_comment_id: Option<u64>,
) -> Vec<&'a github::IssueComment> {
    comments
        .iter()
        .filter(|comment| {
            if comment.author_login == bot_login {
                return false;
            }
            if let Some(last) = last_processed_comment_id {
                if comment.id <= last {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Generate a revised draft from current draft + aggregated feedback.
fn generate_revision_from_feedback_with_timeout(
    config: &PrdPollConfig,
    current_draft: &str,
    aggregated_feedback: &str,
) -> Result<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(config.backend_timeout_secs);
    let writer = create_backend(&config.writer_backend, &config.global_config)?;
    let reviewer = create_backend(&config.reviewer_backend, &config.global_config)?;

    // Build feedback revision prompt
    let revision_prompt = render_prompt(
        FEEDBACK_REVISION_PROMPT,
        &[
            ("{{spec}}", current_draft),
            ("{{feedback}}", aggregated_feedback),
        ],
    );

    let mut current_spec = run_draft_with_section_retry_sync(&writer, &revision_prompt, deadline)?;

    // Run reviewer/section validation loop (same as initial draft generation)
    let idea_context = format!(
        "Revision based on user feedback:\n{aggregated_feedback}"
    );
    for _iteration in 1..=config.max_revisions {
        let review_prompt = render_prompt(
            REVIEW_PROMPT,
            &[("{{idea}}", &idea_context), ("{{spec}}", &current_spec)],
        );
        let feedback =
            run_review_with_retry_sync(&reviewer, review_prompt, deadline).map_err(|err| {
                RalphError::InteractivePrdFailed(format!(
                    "review/retry failed during feedback revision: {err}"
                ))
            })?;

        if feedback.approved || feedback.issues.is_empty() {
            return Ok(current_spec);
        }

        let formatted_issues = format_issues(&feedback.issues);
        let rev_prompt = render_prompt(
            REVISION_PROMPT,
            &[
                ("{{idea}}", &idea_context),
                ("{{spec}}", &current_spec),
                ("{{issues}}", &formatted_issues),
            ],
        );
        let revised = run_backend_sync(&writer, &rev_prompt, deadline)?;
        let (cleaned, missing) = check_spec_sections(&revised);
        if missing.len() < REQUIRED_SPEC_SECTION_COUNT {
            current_spec = cleaned;
        }
    }

    Ok(current_spec)
}

/// Public helper to create a `status-approved` marker.
pub fn prd_status_approved_marker(issue_number: u32, draft_revision: u32) -> String {
    prd_marker(issue_number, "status-approved", draft_revision)
}

fn finish_transition(
    config: &PrdPollConfig,
    state: &mut InteractivePrdState,
    result: Result<()>,
) -> Result<()> {
    let should_fail = apply_transition_result(state, &result);

    if should_fail {
        transition_to_failed(config, state)?;
    } else {
        state.save(&config.data_dir)?;
    }

    result
}

fn apply_transition_result(state: &mut InteractivePrdState, result: &Result<()>) -> bool {
    match result {
        Ok(()) => {
            state.error_count = 0;
            state.last_error = None;
            false
        }
        Err(err) => {
            state.error_count += 1;
            state.last_error = Some(err.to_string());
            state.error_count >= 3
        }
    }
}

fn find_first_answer_comment<'a>(
    comments: &'a [github::IssueComment],
    questions_posted_at: DateTime<Utc>,
    bot_login: &str,
    last_processed_comment_id: Option<u64>,
) -> Option<&'a github::IssueComment> {
    comments.iter().find(|comment| {
        if comment.author_login == bot_login {
            return false;
        }
        if comment.created_at <= questions_posted_at {
            return false;
        }
        if let Some(last) = last_processed_comment_id {
            if comment.id <= last {
                return false;
            }
        }
        true
    })
}

fn extract_questions_text(
    comments: &[github::IssueComment],
    questions_comment_id: Option<u64>,
    issue_number: u32,
    question_revision: u32,
) -> String {
    if let Some(id) = questions_comment_id {
        if let Some(comment) = comments.iter().find(|comment| comment.id == id) {
            return strip_prd_marker_lines(&comment.body);
        }
    }

    if question_revision > 0 {
        let marker = prd_marker(issue_number, "questions", question_revision);
        if let Some(comment) = comments
            .iter()
            .find(|comment| comment.body.contains(&marker))
        {
            return strip_prd_marker_lines(&comment.body);
        }
    }

    "(questions unavailable)".to_owned()
}

fn strip_prd_marker_lines(body: &str) -> String {
    body.lines()
        .filter(|line| !line.trim_start().starts_with("<!-- ralph:prd:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn render_answer_to_draft_prompt(
    issue_text: &str,
    questions_text: &str,
    user_answers: &str,
) -> String {
    let idea = INTERACTIVE_DRAFT_CONTEXT_TEMPLATE
        .replace("{issue}", issue_text)
        .replace("{questions}", questions_text)
        .replace("{answers}", user_answers);
    render_prompt(DRAFT_PROMPT, &[("{{idea}}", &idea)])
}

fn generate_draft_from_answers_with_timeout(
    config: &PrdPollConfig,
    issue_text: &str,
    questions_text: &str,
    user_answers: &str,
) -> Result<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(config.backend_timeout_secs);
    let writer = create_backend(&config.writer_backend, &config.global_config)?;
    let reviewer = create_backend(&config.reviewer_backend, &config.global_config)?;

    let idea_context = INTERACTIVE_DRAFT_CONTEXT_TEMPLATE
        .replace("{issue}", issue_text)
        .replace("{questions}", questions_text)
        .replace("{answers}", user_answers);
    let draft_prompt = render_answer_to_draft_prompt(issue_text, questions_text, user_answers);

    let mut current_spec = run_draft_with_section_retry_sync(&writer, &draft_prompt, deadline)?;

    for _iteration in 1..=config.max_revisions {
        let review_prompt = render_prompt(
            REVIEW_PROMPT,
            &[("{{idea}}", &idea_context), ("{{spec}}", &current_spec)],
        );
        let feedback =
            run_review_with_retry_sync(&reviewer, review_prompt, deadline).map_err(|err| {
                RalphError::InteractivePrdFailed(format!(
                    "review/retry failed while generating draft: {err}"
                ))
            })?;

        if feedback.approved || feedback.issues.is_empty() {
            return Ok(current_spec);
        }

        let formatted_issues = format_issues(&feedback.issues);
        let revision_prompt = render_prompt(
            REVISION_PROMPT,
            &[
                ("{{idea}}", &idea_context),
                ("{{spec}}", &current_spec),
                ("{{issues}}", &formatted_issues),
            ],
        );
        let revised = run_backend_sync(&writer, &revision_prompt, deadline)?;
        let (cleaned, missing) = check_spec_sections(&revised);
        if missing.len() < REQUIRED_SPEC_SECTION_COUNT {
            current_spec = cleaned;
        }
    }

    Ok(current_spec)
}

fn run_draft_with_section_retry_sync(
    writer: &CliBackend,
    prompt: &str,
    deadline: std::time::Instant,
) -> Result<String> {
    for attempt in 0..=DRAFT_SECTION_RETRIES {
        let raw = run_backend_sync(writer, prompt, deadline)?;
        let (cleaned, missing) = check_spec_sections(&raw);
        if missing.is_empty() || attempt == DRAFT_SECTION_RETRIES {
            return Ok(cleaned);
        }
    }
    unreachable!("draft section retry loop should return")
}

fn run_review_with_retry_sync(
    reviewer: &CliBackend,
    prompt: String,
    deadline: std::time::Instant,
) -> Result<crate::prd::quick::ReviewFeedback> {
    let remaining = deadline
        .checked_duration_since(std::time::Instant::now())
        .unwrap_or(Duration::ZERO);
    if remaining.is_zero() {
        return Err(RalphError::InteractivePrdFailed(
            "PRD backend timeout exceeded".to_owned(),
        ));
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            RalphError::InteractivePrdFailed(format!("failed to create tokio runtime: {err}"))
        })?;

    let backend: Arc<dyn Backend> = Arc::new(reviewer.clone());
    let result = rt.block_on(async {
        tokio::time::timeout(remaining, run_review_with_retry(backend, prompt)).await
    });

    match result {
        Ok(Ok(feedback)) => Ok(feedback),
        Ok(Err(err)) => Err(RalphError::InteractivePrdFailed(format!(
            "review execution failed: {err}"
        ))),
        Err(_) => Err(RalphError::InteractivePrdFailed(
            "PRD backend timeout exceeded".to_owned(),
        )),
    }
}

/// Generate clarifying questions using two configured backends plus synthesis.
///
/// All backend work is bounded by `backend_timeout_secs` as total wall-clock.
fn generate_questions_with_timeout(config: &PrdPollConfig, issue_text: &str) -> Result<String> {
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

    let result =
        rt.block_on(async { tokio::time::timeout(remaining, backend.execute(prompt)).await });

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
        apply_transition_result, detect_approval, extract_questions_text,
        find_first_answer_comment, find_new_feedback_comments, prd_marker,
        prd_status_approved_marker, render_answer_to_draft_prompt, InteractivePrdState,
        PrdWorkflowState, FEEDBACK_REVISION_PROMPT, PRD_LABELS, PRD_LIFECYCLE_LABELS,
    };
    use crate::daemon::github::IssueComment;
    use crate::error::RalphError;

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

    #[test]
    fn answer_comment_detection_skips_bot_and_selects_first_valid_user_comment() {
        let comments = vec![
            test_comment(10, "ralph-bot", "bot question", "2026-01-01T00:00:10Z"),
            test_comment(11, "alice", "before question", "2025-12-31T23:59:59Z"),
            test_comment(12, "bob", "first answer", "2026-01-01T00:00:20Z"),
            test_comment(13, "carol", "later answer", "2026-01-01T00:00:30Z"),
        ];

        let found =
            find_first_answer_comment(&comments, ts("2026-01-01T00:00:15Z"), "ralph-bot", None)
                .expect("should find answer");

        assert_eq!(found.id, 12);
        assert_eq!(found.author_login, "bob");
    }

    #[test]
    fn answer_comment_detection_returns_none_when_no_qualifying_comment() {
        let comments = vec![
            test_comment(1, "ralph-bot", "bot response", "2026-01-01T00:00:20Z"),
            test_comment(2, "alice", "old answer", "2026-01-01T00:00:05Z"),
        ];

        let found =
            find_first_answer_comment(&comments, ts("2026-01-01T00:00:10Z"), "ralph-bot", None);
        assert!(found.is_none());
    }

    #[test]
    fn answer_comment_detection_respects_last_processed_comment_id() {
        let comments = vec![
            test_comment(20, "alice", "first", "2026-01-01T00:00:20Z"),
            test_comment(21, "bob", "second", "2026-01-01T00:00:30Z"),
        ];

        let found =
            find_first_answer_comment(&comments, ts("2026-01-01T00:00:10Z"), "ralph-bot", Some(20))
                .expect("should find unprocessed answer");
        assert_eq!(found.id, 21);
    }

    #[test]
    fn extract_questions_text_prefers_comment_id_and_strips_marker() {
        let comments = vec![
            test_comment(
                100,
                "ralph-bot",
                "<!-- ralph:prd:7:questions-v1 -->\n## Clarifying Questions\n1. Q1",
                "2026-01-01T00:00:10Z",
            ),
            test_comment(101, "alice", "answer", "2026-01-01T00:00:20Z"),
        ];

        let extracted = extract_questions_text(&comments, Some(100), 7, 1);
        assert!(!extracted.contains("<!-- ralph:prd:"));
        assert!(extracted.contains("Clarifying Questions"));
        assert!(extracted.contains("1. Q1"));
    }

    #[test]
    fn draft_prompt_contains_issue_questions_and_answers() {
        let prompt = render_answer_to_draft_prompt(
            "Issue title\nIssue body",
            "1. What is the API?",
            "Use REST endpoints.",
        );
        assert!(prompt.contains("Issue title"));
        assert!(prompt.contains("What is the API"));
        assert!(prompt.contains("Use REST endpoints."));
        assert!(prompt.contains("## Summary"));
    }

    #[test]
    fn transition_error_accumulation_triggers_failure_on_third_error() {
        let mut state = InteractivePrdState::new("acme", "widgets", 42);

        let err = Err(RalphError::InteractivePrdFailed("boom".to_owned()));
        assert!(!apply_transition_result(&mut state, &err));
        assert_eq!(state.error_count, 1);
        assert!(!apply_transition_result(&mut state, &err));
        assert_eq!(state.error_count, 2);
        assert!(apply_transition_result(&mut state, &err));
        assert_eq!(state.error_count, 3);
        assert!(state
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("interactive PRD failed: boom"));
    }

    #[test]
    fn successful_transition_result_clears_previous_error_state() {
        let mut state = InteractivePrdState::new("acme", "widgets", 42);
        state.error_count = 2;
        state.last_error = Some("previous".to_owned());
        let ok: crate::Result<()> = Ok(());
        assert!(!apply_transition_result(&mut state, &ok));
        assert_eq!(state.error_count, 0);
        assert!(state.last_error.is_none());
    }

    fn ts(raw: &str) -> chrono::DateTime<Utc> {
        chrono::DateTime::parse_from_rfc3339(raw)
            .expect("timestamp should parse")
            .with_timezone(&Utc)
    }

    fn test_comment(id: u64, author: &str, body: &str, created_at: &str) -> IssueComment {
        IssueComment {
            id,
            author_login: author.to_owned(),
            body: body.to_owned(),
            created_at: ts(created_at),
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests for AwaitingFeedback transition helpers
    // -----------------------------------------------------------------------

    #[test]
    fn find_new_feedback_comments_filters_bot_and_old() {
        let comments = vec![
            test_comment(100, "ralph-bot", "draft v1", "2026-01-01T00:00:10Z"),
            test_comment(101, "alice", "old comment", "2026-01-01T00:00:20Z"),
            test_comment(102, "bob", "new feedback", "2026-01-01T00:00:30Z"),
            test_comment(103, "ralph-bot", "bot reply", "2026-01-01T00:00:35Z"),
            test_comment(104, "carol", "more feedback", "2026-01-01T00:00:40Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(101));
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].id, 102);
        assert_eq!(new[1].id, 104);
    }

    #[test]
    fn find_new_feedback_comments_returns_all_non_bot_when_no_cursor() {
        let comments = vec![
            test_comment(50, "ralph-bot", "draft", "2026-01-01T00:00:10Z"),
            test_comment(51, "alice", "feedback 1", "2026-01-01T00:00:20Z"),
            test_comment(52, "bob", "feedback 2", "2026-01-01T00:00:30Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", None);
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].id, 51);
        assert_eq!(new[1].id, 52);
    }

    #[test]
    fn find_new_feedback_comments_returns_empty_when_all_bot() {
        let comments = vec![
            test_comment(60, "ralph-bot", "draft", "2026-01-01T00:00:10Z"),
            test_comment(61, "ralph-bot", "followup", "2026-01-01T00:00:20Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", None);
        assert!(new.is_empty());
    }

    #[test]
    fn find_new_feedback_comments_returns_empty_when_all_processed() {
        let comments = vec![
            test_comment(70, "alice", "old", "2026-01-01T00:00:10Z"),
            test_comment(71, "bob", "also old", "2026-01-01T00:00:20Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(71));
        assert!(new.is_empty());
    }

    #[test]
    fn status_approved_marker_format() {
        assert_eq!(
            prd_status_approved_marker(42, 3),
            "<!-- ralph:prd:42:status-approved-v3 -->"
        );
    }

    #[test]
    fn feedback_revision_prompt_contains_placeholders() {
        assert!(FEEDBACK_REVISION_PROMPT.contains("{{spec}}"));
        assert!(FEEDBACK_REVISION_PROMPT.contains("{{feedback}}"));
        assert!(FEEDBACK_REVISION_PROMPT.contains("## Summary"));
    }

    #[test]
    fn detect_approval_plain_feedback_returns_false() {
        assert!(!detect_approval("Please add more detail to the testing section."));
        assert!(!detect_approval("The acceptance criteria are incomplete."));
        assert!(!detect_approval("Can you add error handling?"));
    }

    #[test]
    fn detect_approval_approval_with_feedback_returns_true() {
        assert!(detect_approval("Looks good! Ship it when ready."));
        assert!(detect_approval("I've reviewed the spec. Approved."));
    }

    #[test]
    fn detect_approval_question_about_lgtm_still_matches() {
        // Per spec: `\blgtm\b` matches standalone "lgtm" even in questions
        assert!(detect_approval("is this lgtm?"));
    }

    /// Mixed new comments: one LGTM and one non-approval feedback.
    /// Per spec, any approval comment triggers Done. This test verifies that
    /// `has_approval` is true when at least one comment passes `detect_approval()`,
    /// even when other comments are plain feedback.
    #[test]
    fn mixed_comments_approval_plus_feedback_triggers_approval() {
        let comments = vec![
            test_comment(200, "ralph-bot", "draft v1", "2026-01-01T00:00:10Z"),
            test_comment(201, "alice", "old answer", "2026-01-01T00:00:15Z"),
            // New comments after cursor (id > 201):
            test_comment(202, "bob", "Please add error handling.", "2026-01-01T00:00:30Z"),
            test_comment(203, "alice", "LGTM, ship it!", "2026-01-01T00:00:35Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(201));
        assert_eq!(new.len(), 2, "should find 2 new non-bot comments");

        let has_approval = new.iter().any(|c| detect_approval(&c.body));
        assert!(
            has_approval,
            "has_approval should be true when any comment passes detect_approval()"
        );
    }

    /// All new comments are plain feedback with no approval signals.
    /// In this case has_approval is false, triggering the revision path.
    #[test]
    fn all_feedback_comments_without_approval_triggers_revision() {
        let comments = vec![
            test_comment(300, "ralph-bot", "draft v1", "2026-01-01T00:00:10Z"),
            test_comment(301, "alice", "answers", "2026-01-01T00:00:15Z"),
            // New feedback:
            test_comment(302, "bob", "Please fix the testing section.", "2026-01-01T00:00:30Z"),
            test_comment(303, "carol", "Add more acceptance criteria.", "2026-01-01T00:00:35Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(301));
        assert_eq!(new.len(), 2);

        let has_approval = new.iter().any(|c| detect_approval(&c.body));
        assert!(
            !has_approval,
            "has_approval should be false when no comment passes detect_approval()"
        );
    }
}
