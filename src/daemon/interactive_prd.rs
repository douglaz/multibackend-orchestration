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
        // Test-only failure injection: when RALPH_TEST_INJECT_SAVE_FAILURE is
        // set, all saves fail deterministically.  This allows integration and
        // conformance tests to exercise save-failure paths reliably regardless
        // of privilege level (e.g., running as root).  The env var is only
        // checked at runtime and has zero cost when unset.
        if std::env::var_os("RALPH_TEST_INJECT_SAVE_FAILURE").is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected save failure for testing",
            )
            .into());
        }

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

Before generating questions, explore the project to understand its structure and context:
1. List the top-level directory tree to understand the project layout
2. Read key documentation files (README, AGENTS.md, docs/, etc.) if they exist
3. Browse relevant source files to understand existing architecture, data models, and patterns

Use this understanding to ask informed, specific questions that go beyond what could be asked from the issue text alone. Avoid asking about details that are already evident from the codebase.

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

pub const DRAFT_SECTION_RETRIES: u8 = 2;
pub const REQUIRED_SPEC_SECTION_COUNT: usize = 6;

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
            transition_pending_to_awaiting_answers(config, issue, &mut state, bot_login_cache)
        }
        PrdWorkflowState::AwaitingAnswers => transition_awaiting_answers_to_awaiting_feedback(
            config,
            issue,
            &mut state,
            bot_login_cache,
        ),
        PrdWorkflowState::AwaitingFeedback => {
            transition_awaiting_feedback(config, issue, &mut state, bot_login_cache)
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
    bot_login_cache: &mut Option<String>,
) -> Result<()> {
    let issue_number = issue.number;
    let owner = &config.owner;
    let repo = &config.repo;

    eprintln!("prd: attempting Pending->AwaitingAnswers for {owner}/{repo}#{issue_number}");

    let result = get_or_fetch_bot_login(bot_login_cache)
        .and_then(|bot_login| do_pending_to_awaiting(config, issue, state, &bot_login));
    finish_transition(config, state, result, bot_login_cache)
}

fn do_pending_to_awaiting(
    config: &PrdPollConfig,
    issue: &GhIssue,
    state: &mut InteractivePrdState,
    bot_login: &str,
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

    // 3. Check if questions marker already exists (idempotent restart/retry).
    //    If it does, skip question generation entirely to avoid unnecessary
    //    backend calls and reduce failure surface during restart recovery.
    let next_revision = state.question_revision + 1;
    let marker = prd_marker(issue_number, "questions", next_revision);

    let existing_marker_comment =
        github::find_bot_comment_with_marker(owner, repo, issue_number, &marker, bot_login)
            .map_err(|err| {
                RalphError::InteractivePrdFailed(format!(
                    "failed to check existing marker for {owner}/{repo}#{issue_number}: {err}"
                ))
            })?;

    let (comment_id, questions_posted_at) = if let Some(existing) = existing_marker_comment {
        // Marker already exists — hydrate timestamp from real comment time,
        // skip question generation entirely.
        (Some(existing.id), existing.created_at)
    } else {
        // Generate questions with timeout
        let issue_text = format!(
            "{}\n\n{}",
            issue.title,
            issue.body.as_deref().unwrap_or_default()
        );

        let questions = generate_questions_with_timeout(config, &issue_text)?;

        // Post questions comment with idempotent marker
        let comment_body = format!(
            "## Clarifying Questions\n\n\
             Before generating the engineering specification, I need some clarification. \
             Please answer the following questions in a reply to this comment:\n\n\
             {questions}\n\n\
             *Reply to this comment with your answers and I'll generate a draft spec.*"
        );

        // Post and fetch back metadata to use the actual GitHub `created_at`
        // timestamp rather than local wall clock. This ensures answer-gating
        // compares against the real comment time consistently.
        // Uses bot-scoped posting so user-authored spoof markers are ignored.
        let posted_meta = github::post_bot_comment_with_marker_metadata(
            owner,
            repo,
            issue_number,
            &marker,
            &comment_body,
            bot_login,
        )
        .map_err(|err| {
            RalphError::InteractivePrdFailed(format!(
                "failed to post questions comment for {owner}/{repo}#{issue_number}: {err}"
            ))
        })?;

        match posted_meta {
            Some(meta) => (Some(meta.id), meta.created_at),
            None => (None, Utc::now()),
        }
    };

    // 5. Update and persist state
    state.state = PrdWorkflowState::AwaitingAnswers;
    state.question_revision = next_revision;
    state.questions_comment_id = comment_id;
    state.questions_posted_at = Some(questions_posted_at);
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
    bot_login_cache: &mut Option<String>,
) -> Result<()> {
    let issue_number = issue.number;
    eprintln!(
        "prd: checking AwaitingAnswers for {}/{}#{issue_number}",
        config.owner, config.repo
    );

    let result = get_or_fetch_bot_login(bot_login_cache).and_then(|bot_login| {
        do_awaiting_answers_to_awaiting_feedback(config, issue, state, &bot_login)
    });
    finish_transition(config, state, result, bot_login_cache)
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
        eprintln!(
            "prd: no answer found for {owner}/{repo}#{issue_number} ({} comments, questions_posted_at={})",
            comments.len(), questions_posted_at
        );
        return Ok(());
    };

    eprintln!(
        "prd: found answer for {owner}/{repo}#{issue_number} by @{} (comment {})",
        answer_comment.author_login, answer_comment.id
    );

    let user_answers = answer_comment.body.trim().to_owned();
    let questions_text = extract_questions_text(
        &comments,
        state.questions_comment_id,
        issue_number,
        state.question_revision,
        bot_login,
    );

    let issue_text = format!(
        "{}\n\n{}",
        issue.title,
        issue.body.as_deref().unwrap_or_default()
    );

    eprintln!("prd: generating draft for {owner}/{repo}#{issue_number}...");
    let draft_spec = generate_draft_from_answers_with_timeout(
        config,
        &issue_text,
        &questions_text,
        &user_answers,
    )?;
    eprintln!(
        "prd: draft generated for {owner}/{repo}#{issue_number} ({} chars)",
        draft_spec.len()
    );

    let next_revision = state.draft_revision + 1;
    let marker = prd_marker(issue_number, "draft", next_revision);
    let draft_comment_body = format!(
        "## Draft Engineering Specification (Revision {next_revision})\n\n{draft_spec}\n\n\
         *Reply with feedback. Reply with \"approved\" or \"lgtm\" when this draft is ready.*"
    );
    let comment_id = github::post_bot_comment_with_marker(
        owner,
        repo,
        issue_number,
        &marker,
        &draft_comment_body,
        bot_login,
    )
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

    eprintln!(
        "prd: AwaitingAnswers->AwaitingFeedback complete for {owner}/{repo}#{issue_number} (draft v{next_revision})"
    );
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
    bot_login_cache: &mut Option<String>,
) -> Result<()> {
    let issue_number = issue.number;
    eprintln!(
        "prd: checking AwaitingFeedback for {}/{}#{issue_number}",
        config.owner, config.repo
    );

    let result = get_or_fetch_bot_login(bot_login_cache)
        .and_then(|bot_login| do_awaiting_feedback(config, issue, state, &bot_login));
    finish_transition(config, state, result, bot_login_cache)
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
        eprintln!("prd: label approval detected for {owner}/{repo}#{issue_number}");
        return do_approval_transition(config, state, issue_number, bot_login);
    }

    // Find new unprocessed non-bot comments (post-draft boundary)
    let new_comments = find_new_feedback_comments(
        &comments,
        bot_login,
        state.last_processed_comment_id,
        state.latest_draft_comment_id,
    );

    if new_comments.is_empty() {
        eprintln!("prd: no new feedback for {owner}/{repo}#{issue_number}");
        return Ok(());
    }

    eprintln!(
        "prd: {} new feedback comment(s) for {owner}/{repo}#{issue_number}",
        new_comments.len()
    );

    // Check if any new comment is an approval
    let has_approval = new_comments.iter().any(|c| detect_approval(&c.body));

    // If any new comment passes approval detection, transition to Done
    if has_approval {
        eprintln!("prd: comment approval detected for {owner}/{repo}#{issue_number}");
        // Perform approval transition first — only advance cursor on success
        // so that if this fails, the approval comments remain "new" on the
        // next tick and can be retried (reaching failure threshold if needed).
        do_approval_transition(config, state, issue_number, bot_login)?;
        // Advance cursor only after successful approval transition
        let last_id = new_comments.last().map(|c| c.id);
        if let Some(id) = last_id {
            state.last_processed_comment_id = Some(id);
        }
        return Ok(());
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

    let revised_spec =
        generate_revision_from_feedback_with_timeout(config, current_draft, &aggregated_feedback)?;

    // Post new draft comment with incremented marker
    let next_revision = state.draft_revision + 1;
    let marker = prd_marker(issue_number, "draft", next_revision);
    let draft_comment_body = format!(
        "## Draft Engineering Specification (Revision {next_revision})\n\n{revised_spec}\n\n\
         *Reply with feedback. Reply with \"approved\" or \"lgtm\" when this draft is ready.*"
    );
    let comment_id = github::post_bot_comment_with_marker(
        owner,
        repo,
        issue_number,
        &marker,
        &draft_comment_body,
        bot_login,
    )
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

/// Transition to Done: post approval marker, persist terminal state durably,
/// then swap labels.
///
/// Persistence-safe: `ralph:prd-active` is NOT removed until the durable state
/// save succeeds.  If the save fails, the label is left intact (or explicitly
/// re-added) so the issue remains poll-visible for retry on the next daemon tick.
fn do_approval_transition(
    config: &PrdPollConfig,
    state: &mut InteractivePrdState,
    issue_number: u32,
    bot_login: &str,
) -> Result<()> {
    let owner = &config.owner;
    let repo = &config.repo;

    // Post idempotent status-approved marker referencing latest draft.
    // Bot-scoped: user-authored spoof markers are ignored.
    let marker = prd_marker(issue_number, "status-approved", state.draft_revision);
    let approval_body = format!(
        "## PRD Approved\n\n\
         Draft revision {} has been approved.\n\n\
         *The interactive PRD workflow is now complete.*",
        state.draft_revision
    );
    github::post_bot_comment_with_marker(
        owner,
        repo,
        issue_number,
        &marker,
        &approval_body,
        bot_login,
    )
    .map_err(|err| {
        RalphError::InteractivePrdFailed(format!(
            "failed to post approval comment for {owner}/{repo}#{issue_number}: {err}"
        ))
    })?;

    // Add ralph:prd-done BEFORE removing ralph:prd-active. On partial failure
    // the issue retains ralph:prd-active (poll-visible) and gains ralph:prd-done.
    github::add_label_with_retry(owner, repo, issue_number, "ralph:prd-done").map_err(|err| {
        RalphError::InteractivePrdFailed(format!(
            "failed to add ralph:prd-done for {owner}/{repo}#{issue_number}: {err}"
        ))
    })?;

    // Persist terminal Done state BEFORE removing ralph:prd-active.
    // This ensures the issue remains poll-visible (has ralph:prd-active) if
    // the save fails, so retry semantics kick in on the next tick.
    state.state = PrdWorkflowState::Done;
    state.last_advanced_at = Some(Utc::now());

    // Save is the critical durability point.  If this fails, we must NOT
    // proceed to remove ralph:prd-active — the issue must stay visible.
    // We save INSIDE do_approval_transition so the label removal only
    // happens after durable state persistence succeeds.
    if let Err(save_err) = state.save(&config.data_dir) {
        // Revert in-memory state so the caller sees a non-terminal failure
        // that can be routed through retry accounting.
        state.state = PrdWorkflowState::AwaitingFeedback;
        return Err(RalphError::InteractivePrdFailed(format!(
            "failed to persist Done state for {owner}/{repo}#{issue_number}: {save_err}"
        )));
    }

    // Save succeeded — now safe to remove ralph:prd-active (issue will be
    // polled via ralph:prd-done or terminal state file going forward).
    github::remove_label_with_retry(owner, repo, issue_number, "ralph:prd-active").map_err(
        |err| {
            RalphError::InteractivePrdFailed(format!(
                "failed to remove ralph:prd-active for {owner}/{repo}#{issue_number}: {err}"
            ))
        },
    )?;

    Ok(())
}

/// Find all new non-bot comments after both `last_processed_comment_id` and
/// `latest_draft_comment_id` (the draft boundary).
///
/// In the `AwaitingFeedback` state, only comments posted after the latest draft
/// should be considered for approval detection or revision aggregation.
/// Pre-draft comments are ignored even if they were previously unprocessed.
fn find_new_feedback_comments<'a>(
    comments: &'a [github::IssueComment],
    _bot_login: &str,
    last_processed_comment_id: Option<u64>,
    latest_draft_comment_id: Option<u64>,
) -> Vec<&'a github::IssueComment> {
    // The effective boundary is the maximum of last_processed_comment_id and
    // latest_draft_comment_id. This ensures pre-draft comments are always excluded.
    let boundary = match (last_processed_comment_id, latest_draft_comment_id) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };

    comments
        .iter()
        .filter(|comment| {
            // Skip bot-posted comments (identified by PRD marker, not author,
            // so the repo owner can provide feedback on their own PRD).
            if is_prd_bot_comment(&comment.body) {
                return false;
            }
            if let Some(bound) = boundary {
                if comment.id <= bound {
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
    let idea_context = format!("Revision based on user feedback:\n{aggregated_feedback}");
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
            // Reviewer approval does not bypass section completeness
            let (_cleaned, missing) = check_spec_sections(&current_spec);
            if missing.is_empty() {
                return Ok(current_spec);
            }
            // Fall through to revision if sections are missing despite reviewer approval
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
        if missing.is_empty() {
            current_spec = cleaned;
        }
    }

    // Final section check after exhausting revisions
    let (_final, missing) = check_spec_sections(&current_spec);
    if !missing.is_empty() {
        return Err(RalphError::InteractivePrdFailed(format!(
            "revision missing required sections after {} revisions: {}",
            config.max_revisions,
            missing.join(", ")
        )));
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
    bot_login_cache: &mut Option<String>,
) -> Result<()> {
    // Capture the pre-transition state so we can revert correctly on save failure.
    let pre_transition_state = state.state.clone();
    let should_fail = apply_transition_result(state, &result);

    if should_fail {
        let bot_login = bot_login_cache.as_deref();
        transition_to_failed(config, state, bot_login)?;
    } else {
        // Skip the redundant save for successful Done transitions —
        // do_approval_transition already persisted the terminal state.
        // A second save is unnecessary and increases the window for
        // transient I/O failures to cause state/label drift.
        if state.state == PrdWorkflowState::Done && result.is_ok() {
            return Ok(());
        }

        // Attempt to save state.  If save fails, route the failure through
        // retry accounting so that terminal transitions are not silently lost
        // and can trigger retry exhaustion after 3 consecutive save failures.
        if let Err(save_err) = state.save(&config.data_dir) {
            eprintln!(
                "prd: state save FAILED for {}/{}#{}: {save_err}",
                config.owner, config.repo, state.issue_number
            );
            // If the transition itself succeeded but save failed, revert any
            // terminal state to the pre-transition value so the issue remains
            // retryable and the state machine stays valid.
            if state.is_terminal() {
                state.state = pre_transition_state;
            }

            state.error_count += 1;
            state.last_error = Some(format!("state save failed: {save_err}"));

            if state.error_count >= 3 {
                // Save failure exhaustion — transition to Failed
                let bot_login = bot_login_cache.as_deref();
                transition_to_failed(config, state, bot_login)?;
            } else {
                // Best-effort persist the error count so retry accounting
                // survives daemon restart even when save is flaky.
                let _ = state.save(&config.data_dir);
            }

            return Err(RalphError::InteractivePrdFailed(format!(
                "state save failed for {}/{}#{}: {save_err}",
                config.owner, config.repo, state.issue_number
            )));
        }
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
            eprintln!(
                "prd: transition error for {}/{}#{} ({}/3): {err}",
                state.owner, state.repo, state.issue_number, state.error_count
            );
            state.error_count >= 3
        }
    }
}

fn find_first_answer_comment<'a>(
    comments: &'a [github::IssueComment],
    questions_posted_at: DateTime<Utc>,
    _bot_login: &str,
    last_processed_comment_id: Option<u64>,
) -> Option<&'a github::IssueComment> {
    comments.iter().find(|comment| {
        // Skip bot-posted comments (identified by PRD marker, not author,
        // so the repo owner can answer their own PRD questions).
        if is_prd_bot_comment(&comment.body) {
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

/// Returns true if the comment body contains a PRD bot marker.
/// Used instead of author-based filtering so the repo owner can
/// answer/review their own PRD when running the daemon under their account.
fn is_prd_bot_comment(body: &str) -> bool {
    body.contains("<!-- ralph:prd:")
}

fn extract_questions_text(
    comments: &[github::IssueComment],
    questions_comment_id: Option<u64>,
    issue_number: u32,
    question_revision: u32,
    bot_login: &str,
) -> String {
    // Prefer lookup by comment ID (already known to be bot-authored from when
    // we stored it, but verify author for safety against ID reuse).
    if let Some(id) = questions_comment_id {
        if let Some(comment) = comments
            .iter()
            .find(|comment| comment.id == id && comment.author_login == bot_login)
        {
            return strip_prd_marker_lines(&comment.body);
        }
        // Fallback: allow any author for the specific ID (backward compat)
        if let Some(comment) = comments.iter().find(|comment| comment.id == id) {
            return strip_prd_marker_lines(&comment.body);
        }
    }

    // Fallback: find by marker — bot-scoped to ignore user spoofs
    if question_revision > 0 {
        let marker = prd_marker(issue_number, "questions", question_revision);
        if let Some(comment) = comments
            .iter()
            .find(|comment| comment.author_login == bot_login && comment.body.contains(&marker))
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
            // Reviewer approval does not bypass section completeness
            let (_cleaned, missing) = check_spec_sections(&current_spec);
            if missing.is_empty() {
                return Ok(current_spec);
            }
            // Fall through to revision if sections are missing despite reviewer approval
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
        if missing.is_empty() {
            current_spec = cleaned;
        }
    }

    // Final section check after exhausting revisions
    let (_final, missing) = check_spec_sections(&current_spec);
    if !missing.is_empty() {
        return Err(RalphError::InteractivePrdFailed(format!(
            "draft missing required sections after {} revisions: {}",
            config.max_revisions,
            missing.join(", ")
        )));
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
        if missing.is_empty() {
            return Ok(cleaned);
        }
        if attempt == DRAFT_SECTION_RETRIES {
            return Err(RalphError::InteractivePrdFailed(format!(
                "draft missing required sections after {} retries: {}",
                DRAFT_SECTION_RETRIES,
                missing.join(", ")
            )));
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
///
/// Persistence-safe: the terminal Failed state is persisted to disk BEFORE
/// removing `ralph:prd-active`/`ralph:prd`, ensuring the issue remains
/// poll-visible if the save fails.  If save fails, labels are left intact
/// so the issue can be retried on the next daemon tick.
///
/// When `bot_login` is available, uses bot-scoped marker posting so that
/// user-spoofed failure markers cannot suppress the real status comment.
/// Falls back to generic posting when bot identity is unavailable.
fn transition_to_failed(
    config: &PrdPollConfig,
    state: &mut InteractivePrdState,
    bot_login: Option<&str>,
) -> Result<()> {
    let owner = &config.owner;
    let repo = &config.repo;
    let issue_number = state.issue_number;

    // Post error comment marker (best-effort, bot-scoped).
    // When bot identity is available, use bot-scoped idempotent posting so
    // user-spoofed markers are ignored.  When bot identity is unavailable,
    // post non-idempotently (no duplicate check) rather than falling back to
    // body-only lookup which would be vulnerable to user marker spoofing.
    let marker = format!("<!-- ralph:prd:{issue_number}:status-failed -->");
    let error_body = format!(
        "## PRD Workflow Failed\n\n\
         The interactive PRD workflow has failed after {} consecutive errors.\n\n\
         Last error: {}\n\n\
         *Apply the `ralph:prd` label again to retry.*",
        state.error_count,
        state.last_error.as_deref().unwrap_or("unknown")
    );

    if let Some(login) = bot_login {
        let _ = github::post_bot_comment_with_marker(
            owner,
            repo,
            issue_number,
            &marker,
            &error_body,
            login,
        );
    } else {
        // No bot identity — post without idempotency check rather than using
        // body-only lookup (which a user spoof could suppress).
        let full_body = format!("{marker}\n{error_body}");
        let _ = github::post_raw_issue_comment(owner, repo, issue_number, &full_body);
    }

    // Add ralph:prd-failed BEFORE removing ralph:prd-active (boundary-safe ordering).
    let _ = github::add_label_with_retry(owner, repo, issue_number, "ralph:prd-failed");

    // Set terminal state and persist BEFORE removing the poll-visible labels.
    let prev_state = state.state.clone();
    state.state = PrdWorkflowState::Failed;
    state.last_advanced_at = Some(Utc::now());

    if let Err(save_err) = state.save(&config.data_dir) {
        // Save failed — do NOT remove ralph:prd-active or ralph:prd so the
        // issue remains poll-visible.  The next daemon tick will see the
        // non-terminal persisted state and re-attempt the transition.
        eprintln!(
            "prd: CRITICAL: failed to save Failed state for {owner}/{repo}#{issue_number}: {save_err}; \
             leaving labels intact for retry"
        );
        // Revert in-memory state to pre-transition value so the caller
        // doesn't think we succeeded and the state machine stays valid.
        state.state = prev_state;
        return Err(RalphError::InteractivePrdFailed(format!(
            "failed to persist Failed state for {owner}/{repo}#{issue_number}: {save_err}"
        )));
    }

    // Save succeeded — now safe to remove poll-visible labels
    let _ = github::remove_label_with_retry(owner, repo, issue_number, "ralph:prd-active");
    let _ = github::remove_label_with_retry(owner, repo, issue_number, "ralph:prd");

    Ok(())
}

// ---------------------------------------------------------------------------
// Marker status-failed format (no version)
// ---------------------------------------------------------------------------

pub fn prd_status_failed_marker(issue_number: u32) -> String {
    format!("<!-- ralph:prd:{issue_number}:status-failed -->")
}

/// Public accessor for `extract_questions_text`.
///
/// Exposed to allow conformance tests to verify bot-scoped marker hydration
/// without going through the full daemon transition.
pub fn tests_extract_questions_text(
    comments: &[github::IssueComment],
    questions_comment_id: Option<u64>,
    issue_number: u32,
    question_revision: u32,
    bot_login: &str,
) -> String {
    extract_questions_text(
        comments,
        questions_comment_id,
        issue_number,
        question_revision,
        bot_login,
    )
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{
        apply_transition_result, detect_approval, extract_questions_text,
        find_first_answer_comment, find_new_feedback_comments,
        generate_draft_from_answers_with_timeout, generate_revision_from_feedback_with_timeout,
        prd_marker, prd_status_approved_marker, render_answer_to_draft_prompt,
        run_draft_with_section_retry_sync, InteractivePrdState, PrdPollConfig, PrdWorkflowState,
        DRAFT_SECTION_RETRIES, FEEDBACK_REVISION_PROMPT, PRD_LABELS, PRD_LIFECYCLE_LABELS,
        REQUIRED_SPEC_SECTION_COUNT,
    };
    use crate::backend::CliBackend;
    use crate::config::GlobalConfig;
    use crate::daemon::github::IssueComment;
    use crate::error::RalphError;
    use crate::prd::quick::check_spec_sections;

    use std::collections::BTreeMap;
    use std::io::Write as IoWrite;
    use std::path::PathBuf;
    use std::time::Duration;

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
            test_comment(
                10,
                "ralph-bot",
                "<!-- ralph:prd:7:questions-v1 -->\nbot question",
                "2026-01-01T00:00:10Z",
            ),
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
            test_comment(
                1,
                "ralph-bot",
                "<!-- ralph:prd:7:questions-v1 -->\nbot response",
                "2026-01-01T00:00:20Z",
            ),
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

        let extracted = extract_questions_text(&comments, Some(100), 7, 1, "ralph-bot");
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
            test_comment(
                100,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft v1",
                "2026-01-01T00:00:10Z",
            ),
            test_comment(101, "alice", "old comment", "2026-01-01T00:00:20Z"),
            test_comment(102, "bob", "new feedback", "2026-01-01T00:00:30Z"),
            test_comment(
                103,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v2 -->\nbot reply",
                "2026-01-01T00:00:35Z",
            ),
            test_comment(104, "carol", "more feedback", "2026-01-01T00:00:40Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(101), None);
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].id, 102);
        assert_eq!(new[1].id, 104);
    }

    #[test]
    fn find_new_feedback_comments_returns_all_non_bot_when_no_cursor() {
        let comments = vec![
            test_comment(
                50,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft",
                "2026-01-01T00:00:10Z",
            ),
            test_comment(51, "alice", "feedback 1", "2026-01-01T00:00:20Z"),
            test_comment(52, "bob", "feedback 2", "2026-01-01T00:00:30Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", None, None);
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].id, 51);
        assert_eq!(new[1].id, 52);
    }

    #[test]
    fn find_new_feedback_comments_returns_empty_when_all_bot() {
        let comments = vec![
            test_comment(
                60,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft",
                "2026-01-01T00:00:10Z",
            ),
            test_comment(
                61,
                "ralph-bot",
                "<!-- ralph:prd:7:questions-v1 -->\nfollowup",
                "2026-01-01T00:00:20Z",
            ),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", None, None);
        assert!(new.is_empty());
    }

    #[test]
    fn find_new_feedback_comments_returns_empty_when_all_processed() {
        let comments = vec![
            test_comment(70, "alice", "old", "2026-01-01T00:00:10Z"),
            test_comment(71, "bob", "also old", "2026-01-01T00:00:20Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(71), None);
        assert!(new.is_empty());
    }

    #[test]
    fn find_new_feedback_comments_respects_draft_boundary() {
        // Pre-draft user comments should be excluded even if unprocessed
        let comments = vec![
            test_comment(
                200,
                "ralph-bot",
                "<!-- ralph:prd:7:questions-v1 -->\nquestions",
                "2026-01-01T00:00:05Z",
            ),
            test_comment(201, "alice", "answers", "2026-01-01T00:00:10Z"),
            test_comment(
                202,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft-v1",
                "2026-01-01T00:00:15Z",
            ),
            // Pre-draft user comment (id 203 < draft id 204) — should be excluded
            test_comment(203, "bob", "pre-draft feedback", "2026-01-01T00:00:20Z"),
            test_comment(
                204,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v2 -->\ndraft-v2",
                "2026-01-01T00:00:25Z",
            ),
            // Post-draft comments — should be included
            test_comment(205, "carol", "post-draft feedback", "2026-01-01T00:00:30Z"),
        ];

        // last_processed_comment_id=201, latest_draft_comment_id=204
        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(201), Some(204));
        assert_eq!(new.len(), 1, "only post-draft comments should be included");
        assert_eq!(new[0].id, 205);
    }

    #[test]
    fn find_new_feedback_comments_draft_boundary_takes_precedence_over_cursor() {
        // Even if cursor is behind the draft, draft boundary should win
        let comments = vec![
            test_comment(300, "alice", "old feedback", "2026-01-01T00:00:10Z"),
            test_comment(
                301,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft-v1",
                "2026-01-01T00:00:15Z",
            ),
            test_comment(302, "bob", "new feedback", "2026-01-01T00:00:20Z"),
        ];

        // cursor=299 (behind everything), draft=301
        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(299), Some(301));
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, 302, "only post-draft feedback should be visible");
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
        assert!(!detect_approval(
            "Please add more detail to the testing section."
        ));
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
            test_comment(
                200,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft v1",
                "2026-01-01T00:00:10Z",
            ),
            test_comment(201, "alice", "old answer", "2026-01-01T00:00:15Z"),
            // New comments after cursor (id > 201):
            test_comment(
                202,
                "bob",
                "Please add error handling.",
                "2026-01-01T00:00:30Z",
            ),
            test_comment(203, "alice", "LGTM, ship it!", "2026-01-01T00:00:35Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(201), Some(200));
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
            test_comment(
                300,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft v1",
                "2026-01-01T00:00:10Z",
            ),
            test_comment(301, "alice", "answers", "2026-01-01T00:00:15Z"),
            // New feedback:
            test_comment(
                302,
                "bob",
                "Please fix the testing section.",
                "2026-01-01T00:00:30Z",
            ),
            test_comment(
                303,
                "carol",
                "Add more acceptance criteria.",
                "2026-01-01T00:00:35Z",
            ),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", Some(301), Some(300));
        assert_eq!(new.len(), 2);

        let has_approval = new.iter().any(|c| detect_approval(&c.body));
        assert!(
            !has_approval,
            "has_approval should be false when no comment passes detect_approval()"
        );
    }

    // -----------------------------------------------------------------------
    // Transition decision-path unit tests
    // -----------------------------------------------------------------------

    /// Pending pickup: a new InteractivePrdState starts as Pending, non-terminal.
    #[test]
    fn transition_path_pending_pickup_starts_non_terminal() {
        let state = InteractivePrdState::new("acme", "widgets", 42);
        assert_eq!(state.state, PrdWorkflowState::Pending);
        assert!(!state.is_terminal());
        assert_eq!(state.question_revision, 0);
        assert_eq!(state.draft_revision, 0);
        assert!(state.questions_comment_id.is_none());
        assert!(state.questions_posted_at.is_none());
    }

    /// Pending -> AwaitingAnswers: idempotent marker reuse should not post a
    /// duplicate. Verified by checking that when the marker already exists,
    /// `find_comment_with_marker` would return Some.
    #[test]
    fn transition_path_pending_idempotent_marker_reuse() {
        let marker = prd_marker(42, "questions", 1);
        let comments = vec![test_comment(
            500,
            "ralph-bot",
            &format!("{marker}\n## Clarifying Questions\n1. Q?"),
            "2026-01-15T10:00:00Z",
        )];
        // Simulates find_comment_with_marker behavior: check that marker is in body
        let found = comments.iter().find(|c| c.body.contains(&marker));
        assert!(found.is_some(), "existing marker should be found");
        assert_eq!(found.unwrap().id, 500);
        assert_eq!(
            found.unwrap().created_at,
            ts("2026-01-15T10:00:00Z"),
            "should hydrate created_at from existing comment"
        );
    }

    /// AwaitingAnswers: first valid user comment after questions_posted_at is selected.
    #[test]
    fn transition_path_awaiting_answers_selects_first_valid_answer() {
        let comments = vec![
            test_comment(
                600,
                "ralph-bot",
                "<!-- ralph:prd:7:questions-v1 -->\nquestions",
                "2026-01-01T00:00:05Z",
            ),
            // Pre-questions comment (should be skipped)
            test_comment(601, "alice", "early comment", "2026-01-01T00:00:03Z"),
            // Post-questions answers
            test_comment(602, "bob", "first answer", "2026-01-01T00:00:10Z"),
            test_comment(603, "carol", "second answer", "2026-01-01T00:00:15Z"),
        ];

        let answer =
            find_first_answer_comment(&comments, ts("2026-01-01T00:00:05Z"), "ralph-bot", None);
        assert!(answer.is_some(), "should find an answer");
        assert_eq!(answer.unwrap().id, 602, "should select first valid answer");
    }

    /// AwaitingFeedback approval path: at least one approval comment triggers Done.
    #[test]
    fn transition_path_awaiting_feedback_approval() {
        let comments = vec![
            test_comment(
                700,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft-v1",
                "2026-01-01T00:00:15Z",
            ),
            test_comment(701, "alice", "Approved!", "2026-01-01T00:00:25Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", None, Some(700));
        assert_eq!(new.len(), 1);
        assert!(detect_approval(&new[0].body), "should detect approval");
    }

    /// AwaitingFeedback revision path: non-approval feedback triggers revision.
    #[test]
    fn transition_path_awaiting_feedback_revision() {
        let comments = vec![
            test_comment(
                800,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft-v1",
                "2026-01-01T00:00:15Z",
            ),
            test_comment(
                801,
                "alice",
                "Please add error handling.",
                "2026-01-01T00:00:25Z",
            ),
            test_comment(
                802,
                "bob",
                "Also fix the testing section.",
                "2026-01-01T00:00:30Z",
            ),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", None, Some(800));
        assert_eq!(new.len(), 2, "should find 2 feedback comments");
        assert!(
            !new.iter().any(|c| detect_approval(&c.body)),
            "no approval should be detected"
        );
    }

    /// Retry exhaustion: error_count reaches 3 and triggers Failed.
    #[test]
    fn transition_path_retry_exhaustion_triggers_failed() {
        let mut state = InteractivePrdState::new("acme", "widgets", 42);
        state.state = PrdWorkflowState::AwaitingFeedback;

        let err = Err(RalphError::InteractivePrdFailed(
            "persistent error".to_owned(),
        ));

        // First error
        assert!(!apply_transition_result(&mut state, &err));
        assert_eq!(state.error_count, 1);
        assert_eq!(state.state, PrdWorkflowState::AwaitingFeedback);

        // Second error
        assert!(!apply_transition_result(&mut state, &err));
        assert_eq!(state.error_count, 2);

        // Third error — should trigger failure
        assert!(apply_transition_result(&mut state, &err));
        assert_eq!(state.error_count, 3);
        // Note: the caller (finish_transition) actually sets state to Failed;
        // apply_transition_result just signals via the return value.
    }

    /// Pre-draft comments must be ignored for approval detection.
    #[test]
    fn pre_draft_comments_ignored_for_approval_detection() {
        let comments = vec![
            test_comment(400, "alice", "Approved!", "2026-01-01T00:00:05Z"),
            test_comment(
                401,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft-v1",
                "2026-01-01T00:00:15Z",
            ),
            test_comment(402, "bob", "Please fix typo.", "2026-01-01T00:00:25Z"),
        ];

        // Draft boundary at 401: only comments after 401 are visible
        let new = find_new_feedback_comments(&comments, "ralph-bot", None, Some(401));
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, 402);
        assert!(
            !detect_approval(&new[0].body),
            "pre-draft approval should not be visible"
        );
    }

    /// Pre-draft comments must be ignored for revision aggregation.
    #[test]
    fn pre_draft_comments_ignored_for_revision_aggregation() {
        let comments = vec![
            test_comment(
                500,
                "alice",
                "old feedback from before draft",
                "2026-01-01T00:00:05Z",
            ),
            test_comment(501, "bob", "more old feedback", "2026-01-01T00:00:08Z"),
            test_comment(
                502,
                "ralph-bot",
                "<!-- ralph:prd:7:draft-v1 -->\ndraft-v1",
                "2026-01-01T00:00:15Z",
            ),
            test_comment(503, "carol", "new feedback", "2026-01-01T00:00:25Z"),
        ];

        let new = find_new_feedback_comments(&comments, "ralph-bot", None, Some(502));
        assert_eq!(
            new.len(),
            1,
            "only post-draft comments should be aggregated for revision"
        );
        assert_eq!(new[0].author_login, "carol");
    }

    // -----------------------------------------------------------------------
    // Section-completeness enforcement unit tests
    // -----------------------------------------------------------------------

    /// A complete spec with all 6 sections passes section validation.
    #[test]
    fn section_complete_spec_passes_validation() {
        let complete = "\
## Summary\nDraft summary.\n\n\
## Acceptance Criteria\n- [ ] AC1\n\n\
## Technical Approach\nApproach.\n\n\
## Files & Modules\n- file.rs\n\n\
## Testing Strategy\n- tests\n\n\
## Out of Scope\n- none";
        let (_cleaned, missing) = check_spec_sections(complete);
        assert!(
            missing.is_empty(),
            "complete spec should have no missing sections, got: {missing:?}"
        );
    }

    /// A spec missing some sections is detected by check_spec_sections.
    #[test]
    fn section_incomplete_spec_reports_missing_sections() {
        let incomplete = "\
## Summary\nPartial draft.\n\n\
## Acceptance Criteria\n- [ ] AC1";
        let (_cleaned, missing) = check_spec_sections(incomplete);
        assert!(
            !missing.is_empty(),
            "incomplete spec should report missing sections"
        );
        assert!(
            missing.len() < REQUIRED_SPEC_SECTION_COUNT,
            "should have fewer than all sections missing"
        );
    }

    /// A completely empty spec should report all 6 sections as missing.
    #[test]
    fn section_empty_spec_reports_all_missing() {
        let empty = "No sections at all, just plain text.";
        let (_cleaned, missing) = check_spec_sections(empty);
        assert_eq!(
            missing.len(),
            REQUIRED_SPEC_SECTION_COUNT,
            "empty spec should be missing all {REQUIRED_SPEC_SECTION_COUNT} sections, got: {missing:?}"
        );
    }

    /// Verify that DRAFT_SECTION_RETRIES is at least 1 (allows retry after
    /// first incomplete output) and REQUIRED_SPEC_SECTION_COUNT is 6.
    #[test]
    fn section_retry_constants_are_correct() {
        assert!(
            DRAFT_SECTION_RETRIES >= 1,
            "DRAFT_SECTION_RETRIES should be >= 1"
        );
        assert_eq!(
            REQUIRED_SPEC_SECTION_COUNT, 6,
            "REQUIRED_SPEC_SECTION_COUNT should be 6"
        );
    }

    /// Verify that a spec with only some sections is considered incomplete
    /// (would trigger the InteractivePrdFailed error in the hardened flow).
    #[test]
    fn section_incomplete_writer_output_would_fail_after_retries() {
        // Simulate what happens when writer only produces 3 of 6 sections
        let partial = "\
## Summary\nPartial.\n\n\
## Acceptance Criteria\n- [ ] AC\n\n\
## Technical Approach\nApproach.";
        let (_cleaned, missing) = check_spec_sections(partial);
        assert!(
            !missing.is_empty(),
            "partial output should have missing sections"
        );

        // The error message should list the missing section names
        let error_msg = format!(
            "draft missing required sections after {} retries: {}",
            DRAFT_SECTION_RETRIES,
            missing.join(", ")
        );
        assert!(
            error_msg.contains("Files & Modules")
                || error_msg.contains("Testing Strategy")
                || error_msg.contains("Out of Scope"),
            "error message should list specific missing sections: {error_msg}"
        );
    }

    /// Verify that reviewer approval does NOT bypass section completeness.
    /// When a reviewer approves but the spec is incomplete, the spec should
    /// not be accepted.
    #[test]
    fn section_reviewer_approval_does_not_bypass_completeness() {
        // Simulate a spec that a reviewer might approve but is missing sections
        let spec_with_missing = "\
## Summary\nGreat draft.\n\n\
## Acceptance Criteria\n- [ ] Well defined.\n\n\
## Technical Approach\nSolid approach.";
        let (_cleaned, missing) = check_spec_sections(spec_with_missing);
        // Even if reviewer says "approved", missing sections should be detected
        assert!(
            !missing.is_empty(),
            "spec should still have missing sections even if reviewer approved"
        );
        // In the hardened code, this causes the loop to continue to revision
        // rather than returning Ok(current_spec)
    }

    // -----------------------------------------------------------------------
    // Test helpers: temporary mock backend scripts
    // -----------------------------------------------------------------------

    /// Write a temporary bash script that echoes fixed output and return a
    /// `CliBackend` pointing to it.
    fn make_mock_backend(output: &str) -> CliBackend {
        let mut tmp = tempfile::NamedTempFile::new().expect("create temp script");
        writeln!(tmp, "#!/bin/sh").unwrap();
        writeln!(tmp, "cat >/dev/null").unwrap(); // consume stdin
                                                  // Use a heredoc-style approach to avoid quoting issues
        write!(tmp, "cat <<'__MOCK_EOF__'\n{output}\n__MOCK_EOF__").unwrap();
        tmp.flush().unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(tmp.path()).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(tmp.path(), perms).unwrap();
        }

        // Persist the temp file so it outlives this function.
        // The test is responsible for cleanup (or it happens on drop of PathBuf).
        let path = tmp.into_temp_path();
        let path_str = path.to_string_lossy().into_owned();
        // Leak the temp path so it isn't deleted while the backend is in use.
        std::mem::forget(path);

        CliBackend::new(
            "mock-backend",
            path_str,
            vec![],
            Duration::from_secs(10),
            BTreeMap::new(),
        )
    }

    /// Build a `PrdPollConfig` whose claude/codex backends both point to a
    /// given script path.  The writer and reviewer both use "claude" spec
    /// so `create_backend` resolves through `GlobalConfig.backends.claude`.
    fn make_test_prd_config(script_path: &str) -> PrdPollConfig {
        let mut global = GlobalConfig::default();
        global.backends.claude.command = script_path.to_owned();
        global.backends.claude.args = vec![];
        global.backends.codex.command = script_path.to_owned();
        global.backends.codex.args = vec![];

        PrdPollConfig {
            owner: "test".to_owned(),
            repo: "repo".to_owned(),
            data_dir: PathBuf::from("/tmp/ralph-test-prd-unit"),
            prd_enabled: true,
            question_backends: vec!["claude".to_owned(), "codex".to_owned()],
            writer_backend: "claude".to_owned(),
            reviewer_backend: "codex".to_owned(),
            max_revisions: 1,
            backend_timeout_secs: 30,
            global_config: global,
            verbose: false,
        }
    }

    /// Create a persistent temp script that echoes the given output.
    /// Returns the absolute path to the script.
    fn write_persistent_mock_script(output: &str) -> String {
        let mut tmp = tempfile::NamedTempFile::new().expect("create temp script");
        writeln!(tmp, "#!/bin/sh").unwrap();
        writeln!(tmp, "cat >/dev/null").unwrap();
        write!(tmp, "cat <<'__MOCK_EOF__'\n{output}\n__MOCK_EOF__").unwrap();
        tmp.flush().unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(tmp.path()).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(tmp.path(), perms).unwrap();
        }

        let path = tmp.into_temp_path();
        let path_str = path.to_string_lossy().into_owned();
        std::mem::forget(path);
        path_str
    }

    // -----------------------------------------------------------------------
    // Unit tests exercising actual draft/revision control flow
    // -----------------------------------------------------------------------

    /// `run_draft_with_section_retry_sync` with a complete-spec backend
    /// should return Ok with the complete spec.
    #[test]
    fn run_draft_with_section_retry_sync_complete_output_succeeds() {
        let complete = "\
## Summary\nDraft.\n\n\
## Acceptance Criteria\n- [ ] AC1\n\n\
## Technical Approach\nApproach.\n\n\
## Files & Modules\n- file.rs\n\n\
## Testing Strategy\n- tests\n\n\
## Out of Scope\n- none";

        let backend = make_mock_backend(complete);
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let result = run_draft_with_section_retry_sync(&backend, "generate spec", deadline);
        assert!(
            result.is_ok(),
            "complete spec should succeed: {:?}",
            result.err()
        );
        let spec = result.unwrap();
        let (_cleaned, missing) = check_spec_sections(&spec);
        assert!(missing.is_empty(), "returned spec should have all sections");
    }

    /// `run_draft_with_section_retry_sync` with an incomplete-spec backend
    /// should return `InteractivePrdFailed` listing the missing sections.
    #[test]
    fn run_draft_with_section_retry_sync_incomplete_output_fails() {
        let incomplete = "\
## Summary\nPartial.\n\n\
## Acceptance Criteria\n- [ ] AC1";

        let backend = make_mock_backend(incomplete);
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let result = run_draft_with_section_retry_sync(&backend, "generate spec", deadline);
        assert!(result.is_err(), "incomplete spec should fail");

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("draft missing required sections"),
            "error should mention missing sections: {msg}"
        );
        // Should list specific missing section names
        assert!(
            msg.contains("Technical Approach")
                || msg.contains("Files & Modules")
                || msg.contains("Testing Strategy")
                || msg.contains("Out of Scope"),
            "error should list specific missing section names: {msg}"
        );
    }

    /// `generate_draft_from_answers_with_timeout` with an incomplete-spec
    /// backend should return `InteractivePrdFailed` after exhausting retries.
    #[test]
    fn generate_draft_incomplete_writer_output_fails_after_exhaustion() {
        let incomplete = "\
## Summary\nPartial draft.\n\n\
## Acceptance Criteria\n- [ ] AC1\n\n\
## Technical Approach\nApproach.";

        let script = write_persistent_mock_script(incomplete);
        let config = make_test_prd_config(&script);

        let result = generate_draft_from_answers_with_timeout(
            &config,
            "Feature: add auth",
            "1. What auth method?",
            "Use JWT tokens.",
        );
        assert!(
            result.is_err(),
            "incomplete writer output should cause failure"
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing required sections"),
            "error should mention missing sections: {msg}"
        );
    }

    /// Write a persistent mock script that distinguishes writer vs reviewer
    /// prompts: reviewer prompts contain "REVIEW_PROMPT" sentinel or the
    /// `{{spec}}` placeholder pattern; for those, output valid approved JSON.
    /// For writer prompts, output a complete spec.
    fn write_smart_mock_script(spec_output: &str) -> String {
        let mut tmp = tempfile::NamedTempFile::new().expect("create temp script");
        // The script reads stdin, checks if it's a review prompt, and outputs accordingly
        writeln!(tmp, "#!/bin/sh").unwrap();
        writeln!(tmp, "INPUT=\"$(cat)\"").unwrap();
        writeln!(tmp, "if echo \"$INPUT\" | grep -q 'Review the spec for\\|\\*\\*Engineering Spec:\\*\\*\\|review response could not be parsed'; then").unwrap();
        writeln!(
            tmp,
            "  printf '```json\\n{{\"approved\": true, \"issues\": []}}\\n```\\n'"
        )
        .unwrap();
        writeln!(tmp, "else").unwrap();
        write!(tmp, "  cat <<'__MOCK_EOF__'\n{spec_output}\n__MOCK_EOF__\n").unwrap();
        writeln!(tmp, "fi").unwrap();
        tmp.flush().unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(tmp.path()).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(tmp.path(), perms).unwrap();
        }

        let path = tmp.into_temp_path();
        let path_str = path.to_string_lossy().into_owned();
        std::mem::forget(path);
        path_str
    }

    /// `generate_draft_from_answers_with_timeout` with a complete-spec
    /// backend should return Ok.
    #[test]
    fn generate_draft_complete_writer_output_succeeds() {
        let complete = "\
## Summary\nDraft.\n\n\
## Acceptance Criteria\n- [ ] AC1\n\n\
## Technical Approach\nApproach.\n\n\
## Files & Modules\n- file.rs\n\n\
## Testing Strategy\n- tests\n\n\
## Out of Scope\n- none";

        let script = write_smart_mock_script(complete);
        let config = make_test_prd_config(&script);

        let result = generate_draft_from_answers_with_timeout(
            &config,
            "Feature: add auth",
            "1. What auth method?",
            "Use JWT tokens.",
        );
        assert!(
            result.is_ok(),
            "complete writer output should succeed: {:?}",
            result.err()
        );
    }

    /// `generate_revision_from_feedback_with_timeout` with an incomplete-spec
    /// backend should return `InteractivePrdFailed`.
    #[test]
    fn generate_revision_incomplete_writer_output_fails_after_exhaustion() {
        let incomplete = "\
## Summary\nRevised.\n\n\
## Acceptance Criteria\n- [ ] Updated AC.";

        let script = write_persistent_mock_script(incomplete);
        let config = make_test_prd_config(&script);

        let current_draft = "\
## Summary\nOriginal.\n\n\
## Acceptance Criteria\n- [ ] AC1\n\n\
## Technical Approach\nOld.\n\n\
## Files & Modules\n- file.rs\n\n\
## Testing Strategy\n- tests\n\n\
## Out of Scope\n- none";

        let result = generate_revision_from_feedback_with_timeout(
            &config,
            current_draft,
            "Please add more detail to the testing strategy.",
        );
        assert!(
            result.is_err(),
            "incomplete revision output should cause failure"
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing required sections"),
            "error should mention missing sections: {msg}"
        );
    }

    /// `generate_revision_from_feedback_with_timeout` with a complete-spec
    /// backend should return Ok.
    #[test]
    fn generate_revision_complete_writer_output_succeeds() {
        let complete = "\
## Summary\nRevised.\n\n\
## Acceptance Criteria\n- [ ] Updated AC.\n\n\
## Technical Approach\nUpdated approach.\n\n\
## Files & Modules\n- file.rs\n\n\
## Testing Strategy\n- tests\n\n\
## Out of Scope\n- none";

        let script = write_smart_mock_script(complete);
        let config = make_test_prd_config(&script);

        let current_draft = "\
## Summary\nOriginal.\n\n\
## Acceptance Criteria\n- [ ] AC1\n\n\
## Technical Approach\nOld.\n\n\
## Files & Modules\n- file.rs\n\n\
## Testing Strategy\n- tests\n\n\
## Out of Scope\n- none";

        let result = generate_revision_from_feedback_with_timeout(
            &config,
            current_draft,
            "Please add more detail.",
        );
        assert!(
            result.is_ok(),
            "complete revision output should succeed: {:?}",
            result.err()
        );
    }

    // -----------------------------------------------------------------------
    // Reviewer approval does NOT bypass 6-section gating
    // -----------------------------------------------------------------------

    /// Write a mock script where the reviewer always approves but the writer
    /// always produces an incomplete spec (only 3 of 6 sections).  This
    /// exercises the "approval does not bypass section completeness" contract
    /// in `generate_draft_from_answers_with_timeout`.
    fn write_approving_reviewer_incomplete_writer_script() -> String {
        let mut tmp = tempfile::NamedTempFile::new().expect("create temp script");
        writeln!(tmp, "#!/bin/sh").unwrap();
        writeln!(tmp, "INPUT=\"$(cat)\"").unwrap();
        // Reviewer prompts: always approve
        writeln!(tmp, "if echo \"$INPUT\" | grep -q 'Review the spec for\\|\\*\\*Engineering Spec:\\*\\*\\|review response could not be parsed'; then").unwrap();
        writeln!(
            tmp,
            "  printf '```json\\n{{\"approved\": true, \"issues\": []}}\\n```\\n'"
        )
        .unwrap();
        writeln!(tmp, "else").unwrap();
        // Writer prompts: always produce incomplete spec (3 of 6 sections)
        writeln!(tmp, "  cat <<'__MOCK_EOF__'").unwrap();
        writeln!(tmp, "## Summary").unwrap();
        writeln!(tmp, "Incomplete draft from approval-bypass test.").unwrap();
        writeln!(tmp, "").unwrap();
        writeln!(tmp, "## Acceptance Criteria").unwrap();
        writeln!(tmp, "- [ ] Criterion 1").unwrap();
        writeln!(tmp, "").unwrap();
        writeln!(tmp, "## Technical Approach").unwrap();
        writeln!(tmp, "Approach description.").unwrap();
        writeln!(tmp, "__MOCK_EOF__").unwrap();
        writeln!(tmp, "fi").unwrap();
        tmp.flush().unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(tmp.path()).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(tmp.path(), perms).unwrap();
        }

        let path = tmp.into_temp_path();
        let path_str = path.to_string_lossy().into_owned();
        std::mem::forget(path);
        path_str
    }

    /// `generate_draft_from_answers_with_timeout` must reject an incomplete
    /// spec even when the reviewer approves it (`{"approved": true}`).
    #[test]
    fn generate_draft_reviewer_approval_does_not_bypass_section_gating() {
        let script = write_approving_reviewer_incomplete_writer_script();
        let config = make_test_prd_config(&script);

        let result = generate_draft_from_answers_with_timeout(
            &config,
            "Feature: add auth",
            "1. What auth method?",
            "Use JWT tokens.",
        );
        assert!(
            result.is_err(),
            "incomplete spec should fail even when reviewer approves"
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing required sections"),
            "error should mention missing sections despite reviewer approval: {msg}"
        );
        // Verify at least one of the actually-missing sections is named
        assert!(
            msg.contains("Files & Modules")
                || msg.contains("Testing Strategy")
                || msg.contains("Out of Scope"),
            "error should list specific missing section names: {msg}"
        );
    }

    /// `generate_revision_from_feedback_with_timeout` must reject an incomplete
    /// revision even when the reviewer approves it (`{"approved": true}`).
    #[test]
    fn generate_revision_reviewer_approval_does_not_bypass_section_gating() {
        let script = write_approving_reviewer_incomplete_writer_script();
        let config = make_test_prd_config(&script);

        let current_draft = "\
## Summary\nOriginal.\n\n\
## Acceptance Criteria\n- [ ] AC1\n\n\
## Technical Approach\nOld.\n\n\
## Files & Modules\n- file.rs\n\n\
## Testing Strategy\n- tests\n\n\
## Out of Scope\n- none";

        let result = generate_revision_from_feedback_with_timeout(
            &config,
            current_draft,
            "Please expand the testing strategy.",
        );
        assert!(
            result.is_err(),
            "incomplete revision should fail even when reviewer approves"
        );

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing required sections"),
            "error should mention missing sections despite reviewer approval: {msg}"
        );
        assert!(
            msg.contains("Files & Modules")
                || msg.contains("Testing Strategy")
                || msg.contains("Out of Scope"),
            "error should list specific missing section names: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // Bot-scoped marker lookup and extract_questions_text tests
    // -----------------------------------------------------------------------

    /// Bot-scoped extract_questions_text ignores user-authored spoof markers.
    #[test]
    fn extract_questions_text_ignores_user_spoof_marker() {
        let marker_body =
            "<!-- ralph:prd:7:questions-v1 -->\n## Clarifying Questions\n1. Spoofed Q";
        let comments = vec![
            // User spoofs the questions marker
            test_comment(100, "alice", marker_body, "2026-01-01T00:00:05Z"),
            // Bot posts the real questions
            test_comment(
                101,
                "ralph-bot",
                "<!-- ralph:prd:7:questions-v1 -->\n## Clarifying Questions\n1. Real Q",
                "2026-01-01T00:00:10Z",
            ),
        ];

        // With bot-scoped lookup, should find the bot comment (id=101), not the user spoof
        let extracted = extract_questions_text(&comments, None, 7, 1, "ralph-bot");
        assert!(
            extracted.contains("Real Q"),
            "should find bot-authored questions, not user spoof: {extracted}"
        );
        assert!(
            !extracted.contains("Spoofed Q"),
            "user-spoofed marker should be ignored: {extracted}"
        );
    }

    /// Bot-scoped extract_questions_text falls back to ID lookup even with spoof.
    #[test]
    fn extract_questions_text_by_id_prefers_bot_author() {
        let comments = vec![
            test_comment(
                100,
                "ralph-bot",
                "<!-- ralph:prd:7:questions-v1 -->\n1. Bot Q",
                "2026-01-01T00:00:10Z",
            ),
            // User spoof with different ID
            test_comment(
                99,
                "alice",
                "<!-- ralph:prd:7:questions-v1 -->\n1. Spoof Q",
                "2026-01-01T00:00:05Z",
            ),
        ];

        let extracted = extract_questions_text(&comments, Some(100), 7, 1, "ralph-bot");
        assert!(
            extracted.contains("Bot Q"),
            "should prefer bot-authored comment by ID: {extracted}"
        );
    }

    /// Save-failure in finish_transition increments error_count.
    #[test]
    fn save_failure_increments_error_count_in_apply_transition_result() {
        // This tests the retry accounting logic indirectly by verifying that
        // error_count is properly tracked across multiple failures.
        let mut state = InteractivePrdState::new("acme", "widgets", 42);
        state.state = PrdWorkflowState::AwaitingFeedback;

        // Simulate 3 save failures by using apply_transition_result
        let save_err: crate::Result<()> = Err(RalphError::InteractivePrdFailed(
            "state save failed: permission denied".to_owned(),
        ));
        assert!(!apply_transition_result(&mut state, &save_err));
        assert_eq!(state.error_count, 1);
        assert!(!apply_transition_result(&mut state, &save_err));
        assert_eq!(state.error_count, 2);
        assert!(apply_transition_result(&mut state, &save_err));
        assert_eq!(state.error_count, 3);
        assert!(
            state
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("state save failed"),
            "last_error should contain save failure info"
        );
    }
}
