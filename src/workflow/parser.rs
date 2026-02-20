use regex::Regex;

use crate::error::RalphError;
use crate::project::state::CompletionVerdict;
use crate::Result;

#[derive(Debug, Clone)]
pub enum PlannerDecision {
    Feature { name: String, body: String },
    CompletionRequest { body: String },
}

#[derive(Debug, Clone)]
pub enum ImplementerDecision {
    Notes { body: String },
    Response { iteration: u32, body: String },
}

#[derive(Debug, Clone)]
pub enum ReviewerDecision {
    Approved {
        body: String,
        commit_message: Option<String>,
    },
    Suggestions {
        body: String,
    },
}

#[derive(Debug, Clone)]
pub struct CompleterDecision {
    pub verdict: CompletionVerdict,
    pub body: String,
}

#[derive(Debug, Clone)]
pub enum QaDecision {
    Pass { body: String },
    Fail { body: String },
}

#[derive(Debug, Clone)]
pub struct PromptReviewerDecision {
    pub body: String,
    pub refined_prompt: String,
}

pub fn parse_planner_output(raw: &str) -> Result<PlannerDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "planner output is missing a top-level H1".to_owned(),
        ));
    };

    if let Some(name) = first_h1.strip_prefix("# Feature: ") {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(RalphError::ParseError(
                "planner feature heading has empty feature name".to_owned(),
            ));
        }
        validate_required_section(&body, "## Description", "planner feature spec")?;
        validate_required_section(&body, "## Acceptance Criteria", "planner feature spec")?;
        validate_required_section(&body, "## Files to Modify/Create", "planner feature spec")?;
        validate_required_section(&body, "## Dependencies", "planner feature spec")?;
        return Ok(PlannerDecision::Feature {
            name: trimmed.to_owned(),
            body,
        });
    }

    if first_h1.trim() == "# Project Completion Request" {
        validate_required_section(&body, "## Rationale", "planner completion request")?;
        validate_required_section(&body, "## Summary of Work", "planner completion request")?;
        validate_required_section(&body, "## Remaining Items", "planner completion request")?;
        return Ok(PlannerDecision::CompletionRequest { body });
    }

    Err(RalphError::ParseError(format!(
        "unsupported planner H1: {first_h1}"
    )))
}

pub fn parse_implementer_output(
    raw: &str,
    expected_iteration: Option<u32>,
) -> Result<ImplementerDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "implementer output is missing a top-level H1".to_owned(),
        ));
    };

    if first_h1.trim() == "# Implementation Notes" {
        if expected_iteration.is_some() {
            return Err(RalphError::ParseError(
                "expected implementation response but got implementation notes".to_owned(),
            ));
        }
        validate_required_section(&body, "## Decisions Made", "implementer notes")?;
        validate_required_section(&body, "## Spec Deviations", "implementer notes")?;
        validate_required_section(&body, "## Testing", "implementer notes")?;
        return Ok(ImplementerDecision::Notes { body });
    }

    let re = Regex::new(r"^# Implementation Response \(Iteration (\d+)\)$")
        .map_err(|err| RalphError::ParseError(format!("regex compile error: {err}")))?;
    if let Some(captures) = re.captures(first_h1.trim()) {
        let iteration: u32 = captures
            .get(1)
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .ok_or_else(|| {
                RalphError::ParseError("invalid implementation response iteration".to_owned())
            })?;

        if let Some(expected) = expected_iteration {
            if iteration != expected {
                return Err(RalphError::ParseError(format!(
                    "implementation response iteration mismatch: expected {expected}, got {iteration}"
                )));
            }
        }

        validate_required_section(&body, "## Changes Made", "implementer response")?;
        validate_required_section(&body, "## Could Not Address", "implementer response")?;

        return Ok(ImplementerDecision::Response { iteration, body });
    }

    Err(RalphError::ParseError(format!(
        "unsupported implementer H1: {first_h1}"
    )))
}

pub fn parse_reviewer_output(raw: &str) -> Result<ReviewerDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "reviewer output is missing a top-level H1".to_owned(),
        ));
    };

    match first_h1.trim() {
        "# Review: APPROVED" => {
            validate_required_section(
                &body,
                "## Acceptance Criteria Checklist",
                "review approval",
            )?;
            let commit_message = extract_commit_message(&body);
            Ok(ReviewerDecision::Approved {
                body,
                commit_message,
            })
        }
        "# Review: SUGGESTIONS" => {
            validate_required_section(&body, "## Required Changes", "review suggestions")?;
            Ok(ReviewerDecision::Suggestions { body })
        }
        other => Err(RalphError::ParseError(format!(
            "unsupported reviewer H1: {other}"
        ))),
    }
}

pub fn parse_completer_output(raw: &str) -> Result<CompleterDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "completer output is missing a top-level H1".to_owned(),
        ));
    };

    match first_h1.trim() {
        "# Verdict: COMPLETE" => {
            validate_required_line(
                &body,
                "The project satisfies all requirements:",
                "completer complete verdict",
            )?;
            Ok(CompleterDecision {
                verdict: CompletionVerdict::Complete,
                body,
            })
        }
        "# Verdict: CONTINUE" => {
            validate_required_section(
                &body,
                "## Missing Requirements",
                "completer continue verdict",
            )?;
            validate_required_section(
                &body,
                "## Recommended Next Features",
                "completer continue verdict",
            )?;
            Ok(CompleterDecision {
                verdict: CompletionVerdict::Continue,
                body,
            })
        }
        other => Err(RalphError::ParseError(format!(
            "unsupported completer H1: {other}"
        ))),
    }
}

pub fn parse_qa_output(raw: &str) -> Result<QaDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "QA output is missing a top-level H1".to_owned(),
        ));
    };

    match first_h1.trim() {
        "# QA: PASS" => {
            validate_required_section(&body, "## Manual Testing", "QA pass report")?;
            validate_required_section(
                &body,
                "## Acceptance Criteria Verification",
                "QA pass report",
            )?;
            Ok(QaDecision::Pass { body })
        }
        "# QA: FAIL" => {
            validate_required_section(&body, "## Failures", "QA fail report")?;
            validate_required_section(&body, "## Suggested Fixes", "QA fail report")?;
            Ok(QaDecision::Fail { body })
        }
        other => Err(RalphError::ParseError(format!(
            "unsupported QA H1: {other}"
        ))),
    }
}

pub fn parse_prompt_reviewer_output(raw: &str) -> Result<PromptReviewerDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "prompt reviewer output is missing a top-level H1".to_owned(),
        ));
    };
    if first_h1.trim() != "# Prompt Review" {
        return Err(RalphError::ParseError(format!(
            "unsupported prompt reviewer H1: {}",
            first_h1.trim()
        )));
    }

    let issues_idx = body
        .lines()
        .position(|line| line.trim() == "## Issues Found")
        .ok_or_else(|| {
            RalphError::ParseError(
                "missing required section '## Issues Found' in prompt review".to_owned(),
            )
        })?;
    let refined_idx = body
        .lines()
        .position(|line| line.trim() == "## Refined Prompt")
        .ok_or_else(|| {
            RalphError::ParseError(
                "missing required section '## Refined Prompt' in prompt review".to_owned(),
            )
        })?;
    if issues_idx >= refined_idx {
        return Err(RalphError::ParseError(
            "invalid prompt review section order: '## Issues Found' must appear before '## Refined Prompt'"
                .to_owned(),
        ));
    }

    let mut capture_refined = false;
    let mut refined_lines = String::new();
    for line in body.lines() {
        if capture_refined {
            refined_lines.push_str(line);
            refined_lines.push('\n');
            continue;
        }

        if line.trim() == "## Refined Prompt" {
            capture_refined = true;
        }
    }

    let refined_prompt = refined_lines.trim().to_owned();
    if refined_prompt.chars().count() < 10 {
        return Err(RalphError::ParseError(
            "refined prompt is empty or too short".to_owned(),
        ));
    }

    Ok(PromptReviewerDecision {
        body,
        refined_prompt,
    })
}

// ---------------------------------------------------------------------------
// FinalReview parser types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amendment {
    pub id: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalReviewerDecision {
    NoAmendments { body: String },
    Amendments { body: String, amendments: Vec<Amendment> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendmentPosition {
    pub id: String,
    pub position: String, // "ACCEPT" or "REJECT"
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerPositionsDecision {
    pub body: String,
    pub positions: Vec<AmendmentPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendmentVote {
    pub id: String,
    pub vote: String, // "ACCEPT" or "REJECT"
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteDecision {
    pub body: String,
    pub votes: Vec<AmendmentVote>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendmentRuling {
    pub id: String,
    pub ruling: String, // "ACCEPT" or "REJECT"
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArbiterDecision {
    pub body: String,
    pub rulings: Vec<AmendmentRuling>,
}

// ---------------------------------------------------------------------------
// FinalReview parsers (fail-closed)
// ---------------------------------------------------------------------------

pub fn parse_final_reviewer_output(raw: &str) -> Result<FinalReviewerDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "final reviewer output is missing a top-level H1".to_owned(),
        ));
    };

    match first_h1.trim() {
        "# Final Review: NO AMENDMENTS" => {
            validate_required_section(&body, "## Summary", "final review (no amendments)")?;
            Ok(FinalReviewerDecision::NoAmendments { body })
        }
        "# Final Review: AMENDMENTS" => {
            let amendments = extract_amendment_blocks(&body)?;
            if amendments.is_empty() {
                return Err(RalphError::ParseError(
                    "final review declares AMENDMENTS but contains no ## Amendment: blocks"
                        .to_owned(),
                ));
            }
            validate_no_duplicate_ids(&amendments.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), "final reviewer")?;
            for amendment in &amendments {
                validate_amendment_subsections(
                    &amendment.body,
                    &["Problem", "Proposed Change", "Affected Files"],
                    &amendment.id,
                    "final reviewer",
                )?;
            }
            Ok(FinalReviewerDecision::Amendments { body, amendments })
        }
        other => Err(RalphError::ParseError(format!(
            "unsupported final reviewer H1: {other}"
        ))),
    }
}

pub fn parse_planner_position_output(
    raw: &str,
    required_ids: &[&str],
) -> Result<PlannerPositionsDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "planner position output is missing a top-level H1".to_owned(),
        ));
    };

    if first_h1.trim() != "# Planner Positions" {
        return Err(RalphError::ParseError(format!(
            "unsupported planner position H1: {}",
            first_h1.trim()
        )));
    }

    let positions = extract_amendment_value_blocks(&body, "Position", "planner position")?;
    validate_no_duplicate_ids(&positions.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), "planner position")?;
    validate_exact_id_coverage(
        &positions.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(),
        required_ids,
        "planner position",
    )?;

    let positions = positions
        .into_iter()
        .map(|(id, value, block_body)| {
            validate_accept_reject(&value, &id, "planner position")?;
            validate_amendment_subsections(&block_body, &["Rationale"], &id, "planner position")?;
            Ok(AmendmentPosition {
                id,
                position: value,
                body: block_body,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(PlannerPositionsDecision { body, positions })
}

pub fn parse_vote_output(raw: &str, required_ids: &[&str]) -> Result<VoteDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "vote output is missing a top-level H1".to_owned(),
        ));
    };

    if first_h1.trim() != "# Vote Results" {
        return Err(RalphError::ParseError(format!(
            "unsupported vote H1: {}",
            first_h1.trim()
        )));
    }

    let votes = extract_amendment_value_blocks(&body, "Vote", "vote")?;
    validate_no_duplicate_ids(&votes.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(), "vote")?;
    validate_exact_id_coverage(
        &votes.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(),
        required_ids,
        "vote",
    )?;

    let votes = votes
        .into_iter()
        .map(|(id, value, block_body)| {
            validate_accept_reject(&value, &id, "vote")?;
            validate_amendment_subsections(&block_body, &["Rationale"], &id, "vote")?;
            Ok(AmendmentVote {
                id,
                vote: value,
                body: block_body,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(VoteDecision { body, votes })
}

pub fn parse_arbiter_output(raw: &str, required_ids: &[&str]) -> Result<ArbiterDecision> {
    let body = strip_frontmatter(raw);
    let Some(first_h1) = first_h1_line(&body) else {
        return Err(RalphError::ParseError(
            "arbiter output is missing a top-level H1".to_owned(),
        ));
    };

    if first_h1.trim() != "# Arbiter Ruling" {
        return Err(RalphError::ParseError(format!(
            "unsupported arbiter H1: {}",
            first_h1.trim()
        )));
    }

    let rulings = extract_amendment_value_blocks(&body, "Ruling", "arbiter")?;
    validate_no_duplicate_ids(&rulings.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), "arbiter")?;
    validate_exact_id_coverage(
        &rulings.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        required_ids,
        "arbiter",
    )?;

    let rulings = rulings
        .into_iter()
        .map(|(id, value, block_body)| {
            validate_accept_reject(&value, &id, "arbiter")?;
            validate_amendment_subsections(&block_body, &["Rationale"], &id, "arbiter")?;
            Ok(AmendmentRuling {
                id,
                ruling: value,
                body: block_body,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(ArbiterDecision { body, rulings })
}

// ---------------------------------------------------------------------------
// FinalReview parser helpers
// ---------------------------------------------------------------------------

fn extract_amendment_blocks(body: &str) -> Result<Vec<Amendment>> {
    let mut amendments = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_lines = Vec::new();

    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("## Amendment: ") {
            if let Some(id) = current_id.take() {
                amendments.push(Amendment {
                    id,
                    body: current_lines.join("\n").trim().to_owned(),
                });
                current_lines.clear();
            }
            let id = rest.trim().to_owned();
            if id.is_empty() {
                return Err(RalphError::ParseError(
                    "amendment block has empty ID".to_owned(),
                ));
            }
            current_id = Some(id);
        } else if current_id.is_some() {
            current_lines.push(line);
        }
    }

    if let Some(id) = current_id {
        amendments.push(Amendment {
            id,
            body: current_lines.join("\n").trim().to_owned(),
        });
    }

    Ok(amendments)
}

/// Extract `## Amendment: <ID>` blocks and find the `### <value_section>` value inside each.
/// Returns `(id, value, block_body)` tuples.
fn extract_amendment_value_blocks(
    body: &str,
    value_section: &str,
    scope: &str,
) -> Result<Vec<(String, String, String)>> {
    let mut results = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_lines = Vec::new();

    let flush = |id: String, lines: &[&str], value_section: &str, scope: &str| -> Result<(String, String, String)> {
        let block = lines.join("\n");
        let value_heading = format!("### {value_section}");
        let mut in_value = false;
        let mut value_lines = Vec::new();
        for l in lines {
            let trimmed = l.trim();
            if trimmed == value_heading {
                in_value = true;
                continue;
            }
            if in_value {
                if trimmed.starts_with("### ") {
                    break;
                }
                if !trimmed.is_empty() {
                    value_lines.push(trimmed);
                }
            }
        }
        if value_lines.is_empty() {
            return Err(RalphError::ParseError(format!(
                "missing '### {value_section}' value in {scope} for amendment '{id}'"
            )));
        }
        let value = value_lines[0].to_owned();
        Ok((id, value, block.trim().to_owned()))
    };

    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("## Amendment: ") {
            if let Some(id) = current_id.take() {
                results.push(flush(id, &current_lines, value_section, scope)?);
                current_lines.clear();
            }
            let id = rest.trim().to_owned();
            if id.is_empty() {
                return Err(RalphError::ParseError(
                    "amendment block has empty ID".to_owned(),
                ));
            }
            current_id = Some(id);
        } else if current_id.is_some() {
            current_lines.push(line);
        }
    }

    if let Some(id) = current_id {
        results.push(flush(id, &current_lines, value_section, scope)?);
    }

    Ok(results)
}

fn validate_no_duplicate_ids(ids: &[&str], scope: &str) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            return Err(RalphError::ParseError(format!(
                "duplicate amendment ID '{id}' in {scope}"
            )));
        }
    }
    Ok(())
}

fn validate_exact_id_coverage(found_ids: &[&str], required_ids: &[&str], scope: &str) -> Result<()> {
    let found_set: std::collections::HashSet<&str> = found_ids.iter().copied().collect();
    let required_set: std::collections::HashSet<&str> = required_ids.iter().copied().collect();

    let missing: Vec<&&str> = required_set.difference(&found_set).collect();
    if !missing.is_empty() {
        let mut missing_sorted: Vec<&str> = missing.into_iter().copied().collect();
        missing_sorted.sort();
        return Err(RalphError::ParseError(format!(
            "missing amendment IDs in {scope}: {}",
            missing_sorted.join(", ")
        )));
    }

    let extra: Vec<&&str> = found_set.difference(&required_set).collect();
    if !extra.is_empty() {
        let mut extra_sorted: Vec<&str> = extra.into_iter().copied().collect();
        extra_sorted.sort();
        return Err(RalphError::ParseError(format!(
            "unexpected amendment IDs in {scope}: {}",
            extra_sorted.join(", ")
        )));
    }

    Ok(())
}

/// Shared helper that validates required subsection headings within an amendment block body.
/// For each required heading (e.g. `### Problem`), it checks that the heading appears on its
/// own line within `block_body`. Returns `ParseError` for the first missing heading.
fn validate_amendment_subsections(
    block_body: &str,
    required_subsections: &[&str],
    amendment_id: &str,
    scope: &str,
) -> Result<()> {
    for subsection in required_subsections {
        let heading = format!("### {subsection}");
        if !block_body.lines().any(|line| line.trim() == heading) {
            return Err(RalphError::ParseError(format!(
                "missing required subsection '### {subsection}' in {scope} for amendment '{amendment_id}'"
            )));
        }
    }
    Ok(())
}

fn validate_accept_reject(value: &str, id: &str, scope: &str) -> Result<()> {
    if value != "ACCEPT" && value != "REJECT" {
        return Err(RalphError::ParseError(format!(
            "invalid value '{value}' for amendment '{id}' in {scope}; expected ACCEPT or REJECT"
        )));
    }
    Ok(())
}

pub fn extract_commit_message(body: &str) -> Option<String> {
    let mut in_section = false;
    let mut lines = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "## Commit Message" {
            in_section = true;
            continue;
        }

        if in_section {
            if trimmed.starts_with("## ") {
                break;
            }
            if trimmed.is_empty() {
                continue;
            }
            lines.push(trimmed.to_owned());
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

pub fn strip_frontmatter(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with("---") {
        return trimmed.to_owned();
    }

    let mut lines = trimmed.lines();
    if lines.next() != Some("---") {
        return trimmed.to_owned();
    }

    let mut in_frontmatter = true;
    let mut out = String::new();
    for line in lines {
        if in_frontmatter {
            if line.trim() == "---" {
                in_frontmatter = false;
            }
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    if in_frontmatter {
        trimmed.to_owned()
    } else {
        out.trim().to_owned()
    }
}

pub fn first_h1_line(body: &str) -> Option<&str> {
    body.lines()
        .find(|line| line.trim_start().starts_with("# "))
}

fn validate_required_section(body: &str, section: &str, scope: &str) -> Result<()> {
    if !body.lines().any(|line| line.trim() == section) {
        return Err(RalphError::ParseError(format!(
            "missing required section '{section}' in {scope}"
        )));
    }

    Ok(())
}

fn validate_required_line(body: &str, line: &str, scope: &str) -> Result<()> {
    if !body.lines().any(|candidate| candidate.trim() == line) {
        return Err(RalphError::ParseError(format!(
            "missing required line '{line}' in {scope}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        extract_commit_message, parse_completer_output, parse_implementer_output,
        parse_planner_output, parse_prompt_reviewer_output, parse_qa_output, parse_reviewer_output,
        ImplementerDecision, PlannerDecision, QaDecision, ReviewerDecision,
    };
    use crate::project::state::CompletionVerdict;

    #[test]
    fn parses_feature_heading() {
        let text = "# Feature: User Authentication\n\n## Description\n...\n\n## Acceptance Criteria\n- [ ] one\n\n## Files to Modify/Create\n- `src/lib.rs` - update\n\n## Dependencies\n- Requires: none\n- Blocks: none";
        let parsed = parse_planner_output(text).expect("expected parse success");
        match parsed {
            PlannerDecision::Feature { name, .. } => assert_eq!(name, "User Authentication"),
            _ => panic!("expected feature decision"),
        }
    }

    #[test]
    fn parses_completion_heading() {
        let text = "# Project Completion Request\n\n## Rationale\n...\n\n## Summary of Work\n...\n\n## Remaining Items\n- None";
        let parsed = parse_planner_output(text).expect("expected parse success");
        match parsed {
            PlannerDecision::CompletionRequest { .. } => {}
            _ => panic!("expected completion decision"),
        }
    }

    #[test]
    fn parses_impl_notes() {
        let text = "# Implementation Notes\n\n## Decisions Made\n- a\n\n## Spec Deviations\n- none\n\n## Testing\n- cargo test";
        let parsed = parse_implementer_output(text, None).expect("parse should succeed");
        match parsed {
            ImplementerDecision::Notes { .. } => {}
            _ => panic!("expected notes"),
        }
    }

    #[test]
    fn parses_impl_response_iteration() {
        let text = "# Implementation Response (Iteration 2)\n\n## Changes Made\n- x\n\n## Could Not Address\n- none";
        let parsed = parse_implementer_output(text, Some(2)).expect("parse should succeed");
        match parsed {
            ImplementerDecision::Response { iteration, .. } => assert_eq!(iteration, 2),
            _ => panic!("expected response"),
        }
    }

    #[test]
    fn parses_review_approved() {
        let text = "# Review: APPROVED\n\n## Acceptance Criteria Checklist\n- [x] ok\n\n## Commit Message\nfeat: demo";
        let parsed = parse_reviewer_output(text).expect("parse should succeed");
        match parsed {
            ReviewerDecision::Approved { commit_message, .. } => {
                assert_eq!(commit_message.as_deref(), Some("feat: demo"));
            }
            _ => panic!("expected approved"),
        }
    }

    #[test]
    fn parses_review_suggestions() {
        let text = "# Review: SUGGESTIONS\n\n## Required Changes\n1. fix";
        let parsed = parse_reviewer_output(text).expect("parse should succeed");
        match parsed {
            ReviewerDecision::Suggestions { .. } => {}
            _ => panic!("expected suggestions"),
        }
    }

    #[test]
    fn parses_completer_verdict() {
        let text =
            "# Verdict: CONTINUE\n\n## Missing Requirements\n1. x\n\n## Recommended Next Features\n1. y";
        let parsed = parse_completer_output(text).expect("parse should succeed");
        assert_eq!(parsed.verdict, CompletionVerdict::Continue);
    }

    #[test]
    fn parses_completer_complete_verdict() {
        let text =
            "# Verdict: COMPLETE\n\nThe project satisfies all requirements:\n- requirement: done";
        let parsed = parse_completer_output(text).expect("parse should succeed");
        assert_eq!(parsed.verdict, CompletionVerdict::Complete);
    }

    #[test]
    fn extracts_commit_message_block() {
        let text = "# Review: APPROVED\n\n## Commit Message\nfeat: one\nmore\n\n## Notes\n...";
        assert_eq!(
            extract_commit_message(text),
            Some("feat: one more".to_owned())
        );
    }

    #[test]
    fn strip_frontmatter_removes_yaml_header() {
        let text = "---\ntitle: test\n---\n# Feature: Foo\n\n## Description\nbar";
        let body = super::strip_frontmatter(text);
        assert!(body.starts_with("# Feature: Foo"), "got: {body}");
    }

    #[test]
    fn strip_frontmatter_ignores_triple_dash_inside_content() {
        // Regression: embedded --- inside content (e.g. in a reformat prompt)
        // should NOT be treated as frontmatter boundaries
        let text = "# Implementation Response (Iteration 1)\n\n## Changes Made\n- fixed\n\n---\n\nsome separator\n\n## Could Not Address\n- none";
        let body = super::strip_frontmatter(text);
        assert!(
            body.starts_with("# Implementation Response"),
            "strip_frontmatter should not strip content when first line is not ---; got: {body}"
        );
    }

    #[test]
    fn strip_frontmatter_preserves_tilde_fences() {
        // ~~~ fences must not trigger frontmatter stripping
        let text = "# Implementation Response (Iteration 1)\n\n~~~\nembedded content\n~~~\n\n## Changes Made\n- x\n\n## Could Not Address\n- none";
        let body = super::strip_frontmatter(text);
        assert!(
            body.starts_with("# Implementation Response"),
            "tilde fences should not affect parsing; got: {body}"
        );
        assert!(body.contains("~~~"), "tilde fences should be preserved");
    }

    #[test]
    fn strip_frontmatter_with_dash_fences_around_raw_output_corrupts_parse() {
        // This demonstrates the bug that was fixed: if a reformat prompt embeds
        // raw output between --- fences, strip_frontmatter treats line 1 as
        // YAML frontmatter start and strips everything up to the closing ---
        let text = "---\nsome raw output here\n---\n# Implementation Response (Iteration 1)\n\n## Changes Made\n- x\n\n## Could Not Address\n- none";
        let body = super::strip_frontmatter(text);
        // After stripping, the H1 should be found
        assert!(
            body.starts_with("# Implementation Response"),
            "after stripping frontmatter, H1 should be first; got: {body}"
        );
    }

    #[test]
    fn parses_qa_pass() {
        let text = "# QA: PASS\n\n## Manual Testing\n- ran ralph init in temp dir\n- verified config created\n\n## Automated Tests\n- cargo test: passed\n\n## Acceptance Criteria Verification\n- [x] init creates config: verified manually";
        let parsed = parse_qa_output(text).expect("parse should succeed");
        match parsed {
            QaDecision::Pass { body } => {
                assert!(body.contains("## Manual Testing"));
                assert!(body.contains("## Acceptance Criteria Verification"));
            }
            _ => panic!("expected QA pass"),
        }
    }

    #[test]
    fn parses_qa_fail() {
        let text = "# QA: FAIL\n\n## Failures\n1. cargo test fails with 3 errors\n\n## Suggested Fixes\n1. Fix compilation error in src/lib.rs";
        let parsed = parse_qa_output(text).expect("parse should succeed");
        match parsed {
            QaDecision::Fail { body } => {
                assert!(body.contains("## Failures"));
                assert!(body.contains("## Suggested Fixes"));
            }
            _ => panic!("expected QA fail"),
        }
    }

    #[test]
    fn qa_parser_rejects_malformed_output() {
        let text = "# Some Random Heading\n\nno valid QA structure";
        let result = parse_qa_output(text);
        assert!(result.is_err(), "malformed QA output should fail parsing");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unsupported QA H1"),
            "error should mention unsupported H1: {err}"
        );
    }

    #[test]
    fn qa_pass_requires_all_sections() {
        // Missing "## Acceptance Criteria Verification"
        let text = "# QA: PASS\n\n## Manual Testing\n- ran the binary";
        let result = parse_qa_output(text);
        assert!(
            result.is_err(),
            "QA pass without Acceptance Criteria Verification should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Acceptance Criteria Verification"),
            "got: {err}"
        );
    }

    #[test]
    fn qa_fail_requires_all_sections() {
        // Missing "## Suggested Fixes"
        let text = "# QA: FAIL\n\n## Failures\n1. test broken";
        let result = parse_qa_output(text);
        assert!(
            result.is_err(),
            "QA fail without Suggested Fixes should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Suggested Fixes"), "got: {err}");
    }

    #[test]
    fn qa_parser_rejects_missing_h1() {
        let text = "no heading at all, just text";
        let result = parse_qa_output(text);
        assert!(result.is_err(), "QA output without H1 should fail");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing a top-level H1"), "got: {err}");
    }

    #[test]
    fn qa_parser_strips_frontmatter() {
        let text = "---\nartifact: qa\n---\n# QA: PASS\n\n## Manual Testing\n- ran binary ok\n\n## Automated Tests\n- cargo test passed\n\n## Acceptance Criteria Verification\nAll good.";
        let parsed = parse_qa_output(text).expect("should strip frontmatter and parse");
        assert!(matches!(parsed, QaDecision::Pass { .. }));
    }

    #[test]
    fn parses_prompt_reviewer_output() {
        let text = "# Prompt Review\n\n## Issues Found\n- clarify acceptance criteria\n\n## Refined Prompt\n# Feature: Better Prompt\n\n## Description\nA more specific prompt.";
        let parsed = parse_prompt_reviewer_output(text).expect("parse should succeed");
        assert!(parsed.body.contains("## Issues Found"));
        assert!(parsed
            .refined_prompt
            .starts_with("# Feature: Better Prompt"));
    }

    #[test]
    fn prompt_reviewer_output_requires_h1() {
        let text = "## Issues Found\n- x\n\n## Refined Prompt\nLong enough prompt body.";
        let result = parse_prompt_reviewer_output(text);
        assert!(result.is_err(), "missing h1 should fail");
        assert!(result
            .expect_err("expected error")
            .to_string()
            .contains("missing a top-level H1"));
    }

    #[test]
    fn prompt_reviewer_output_requires_issues_found() {
        let text = "# Prompt Review\n\n## Refined Prompt\nThis refined prompt is long enough.";
        let result = parse_prompt_reviewer_output(text);
        assert!(result.is_err(), "missing issues section should fail");
        assert!(result
            .expect_err("expected error")
            .to_string()
            .contains("## Issues Found"));
    }

    #[test]
    fn prompt_reviewer_output_requires_refined_prompt_section() {
        let text = "# Prompt Review\n\n## Issues Found\n- x";
        let result = parse_prompt_reviewer_output(text);
        assert!(result.is_err(), "missing refined section should fail");
        assert!(result
            .expect_err("expected error")
            .to_string()
            .contains("## Refined Prompt"));
    }

    #[test]
    fn prompt_reviewer_output_rejects_empty_refined_prompt() {
        let text = "# Prompt Review\n\n## Issues Found\n- x\n\n## Refined Prompt\n   \n\t";
        let result = parse_prompt_reviewer_output(text);
        assert!(result.is_err(), "empty refined prompt should fail");
        assert!(result
            .expect_err("expected error")
            .to_string()
            .contains("refined prompt is empty or too short"));
    }

    #[test]
    fn prompt_reviewer_output_rejects_too_short_refined_prompt() {
        let text = "# Prompt Review\n\n## Issues Found\n- x\n\n## Refined Prompt\nshort";
        let result = parse_prompt_reviewer_output(text);
        assert!(result.is_err(), "short refined prompt should fail");
        assert!(result
            .expect_err("expected error")
            .to_string()
            .contains("refined prompt is empty or too short"));
    }

    #[test]
    fn prompt_reviewer_output_extracts_refined_prompt_to_eof_with_nested_headings() {
        let text = "# Prompt Review\n\n## Issues Found\n- x\n\n## Refined Prompt\n# Feature: Prompt\n\n## Description\nNested heading remains.\n\n## Acceptance Criteria\n- [ ] one";
        let parsed = parse_prompt_reviewer_output(text).expect("parse should succeed");
        assert!(parsed.refined_prompt.contains("## Description"));
        assert!(parsed.refined_prompt.contains("## Acceptance Criteria"));
    }

    #[test]
    fn prompt_reviewer_output_rejects_wrong_section_order() {
        let text = "# Prompt Review\n\n## Refined Prompt\nThis refined prompt is long enough.\n\n## Issues Found\n- listed too late";
        let result = parse_prompt_reviewer_output(text);
        assert!(result.is_err(), "wrong section order should fail");
        assert!(result
            .expect_err("expected error")
            .to_string()
            .contains("must appear before"));
    }

    // -----------------------------------------------------------------------
    // FinalReview parser tests
    // -----------------------------------------------------------------------

    use super::{
        parse_arbiter_output, parse_final_reviewer_output, parse_planner_position_output,
        parse_vote_output, FinalReviewerDecision,
    };

    #[test]
    fn final_reviewer_no_amendments() {
        let text = "# Final Review: NO AMENDMENTS\n\n## Summary\nAll good.";
        let parsed = parse_final_reviewer_output(text).expect("should parse");
        assert!(matches!(parsed, FinalReviewerDecision::NoAmendments { .. }));
    }

    #[test]
    fn final_reviewer_with_amendments() {
        let text = "# Final Review: AMENDMENTS\n\n## Amendment: FIX-001\n\n### Problem\nbug\n\n### Proposed Change\nfix it\n\n### Affected Files\n- `src/lib.rs`\n\n## Amendment: FIX-002\n\n### Problem\ntypo\n\n### Proposed Change\ncorrect\n\n### Affected Files\n- `README.md`";
        let parsed = parse_final_reviewer_output(text).expect("should parse");
        match parsed {
            FinalReviewerDecision::Amendments { amendments, .. } => {
                assert_eq!(amendments.len(), 2);
                assert_eq!(amendments[0].id, "FIX-001");
                assert_eq!(amendments[1].id, "FIX-002");
            }
            _ => panic!("expected amendments"),
        }
    }

    #[test]
    fn final_reviewer_rejects_missing_h1() {
        let text = "no heading here";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing a top-level H1"));
    }

    #[test]
    fn final_reviewer_rejects_wrong_h1() {
        let text = "# Something Else\n\n## Summary\nstuff";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported final reviewer H1"));
    }

    #[test]
    fn final_reviewer_amendments_h1_but_no_blocks_fails() {
        let text = "# Final Review: AMENDMENTS\n\nno amendment blocks here";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no ## Amendment: blocks"));
    }

    #[test]
    fn final_reviewer_rejects_duplicate_amendment_ids() {
        let text = "# Final Review: AMENDMENTS\n\n## Amendment: DUP-1\n\n### Problem\nx\n\n### Proposed Change\nfix\n\n### Affected Files\n- `a.rs`\n\n## Amendment: DUP-1\n\n### Problem\ny\n\n### Proposed Change\nfix2\n\n### Affected Files\n- `b.rs`";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate amendment ID"));
    }

    #[test]
    fn planner_position_success() {
        let text = "# Planner Positions\n\n## Amendment: FIX-001\n\n### Position\nACCEPT\n\n### Rationale\ngood idea\n\n## Amendment: FIX-002\n\n### Position\nREJECT\n\n### Rationale\nnot needed";
        let parsed =
            parse_planner_position_output(text, &["FIX-001", "FIX-002"]).expect("should parse");
        assert_eq!(parsed.positions.len(), 2);
        assert_eq!(parsed.positions[0].id, "FIX-001");
        assert_eq!(parsed.positions[0].position, "ACCEPT");
        assert_eq!(parsed.positions[1].id, "FIX-002");
        assert_eq!(parsed.positions[1].position, "REJECT");
    }

    #[test]
    fn planner_position_rejects_missing_id() {
        let text = "# Planner Positions\n\n## Amendment: FIX-001\n\n### Position\nACCEPT\n\n### Rationale\nok";
        let result = parse_planner_position_output(text, &["FIX-001", "FIX-002"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing amendment IDs"));
    }

    #[test]
    fn planner_position_rejects_extra_id() {
        let text = "# Planner Positions\n\n## Amendment: FIX-001\n\n### Position\nACCEPT\n\n### Rationale\nok\n\n## Amendment: FIX-EXTRA\n\n### Position\nREJECT\n\n### Rationale\nnah";
        let result = parse_planner_position_output(text, &["FIX-001"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unexpected amendment IDs"));
    }

    #[test]
    fn planner_position_rejects_invalid_value() {
        let text = "# Planner Positions\n\n## Amendment: FIX-001\n\n### Position\nMAYBE\n\n### Rationale\nhmm";
        let result = parse_planner_position_output(text, &["FIX-001"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expected ACCEPT or REJECT"));
    }

    #[test]
    fn planner_position_rejects_duplicate_ids() {
        let text = "# Planner Positions\n\n## Amendment: FIX-001\n\n### Position\nACCEPT\n\n### Rationale\nok\n\n## Amendment: FIX-001\n\n### Position\nREJECT\n\n### Rationale\nnah";
        let result = parse_planner_position_output(text, &["FIX-001"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate amendment ID"));
    }

    #[test]
    fn vote_output_success() {
        let text = "# Vote Results\n\n## Amendment: A1\n\n### Vote\nACCEPT\n\n### Rationale\nyes\n\n## Amendment: A2\n\n### Vote\nREJECT\n\n### Rationale\nno";
        let parsed = parse_vote_output(text, &["A1", "A2"]).expect("should parse");
        assert_eq!(parsed.votes.len(), 2);
        assert_eq!(parsed.votes[0].vote, "ACCEPT");
        assert_eq!(parsed.votes[1].vote, "REJECT");
    }

    #[test]
    fn vote_output_rejects_missing_ids() {
        let text = "# Vote Results\n\n## Amendment: A1\n\n### Vote\nACCEPT\n\n### Rationale\nyes";
        let result = parse_vote_output(text, &["A1", "A2"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing amendment IDs"));
    }

    #[test]
    fn vote_output_rejects_duplicate_ids() {
        let text = "# Vote Results\n\n## Amendment: A1\n\n### Vote\nACCEPT\n\n### Rationale\nyes\n\n## Amendment: A1\n\n### Vote\nREJECT\n\n### Rationale\nno";
        let result = parse_vote_output(text, &["A1"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate amendment ID"));
    }

    #[test]
    fn arbiter_output_success() {
        let text = "# Arbiter Ruling\n\n## Amendment: D1\n\n### Ruling\nACCEPT\n\n### Rationale\ngood\n\n## Amendment: D2\n\n### Ruling\nREJECT\n\n### Rationale\nbad";
        let parsed = parse_arbiter_output(text, &["D1", "D2"]).expect("should parse");
        assert_eq!(parsed.rulings.len(), 2);
        assert_eq!(parsed.rulings[0].ruling, "ACCEPT");
        assert_eq!(parsed.rulings[1].ruling, "REJECT");
    }

    #[test]
    fn arbiter_output_rejects_missing_ids() {
        let text = "# Arbiter Ruling\n\n## Amendment: D1\n\n### Ruling\nACCEPT\n\n### Rationale\ngood";
        let result = parse_arbiter_output(text, &["D1", "D2"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing amendment IDs"));
    }

    #[test]
    fn arbiter_output_rejects_invalid_ruling() {
        let text = "# Arbiter Ruling\n\n## Amendment: D1\n\n### Ruling\nDEFER\n\n### Rationale\nhmm";
        let result = parse_arbiter_output(text, &["D1"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expected ACCEPT or REJECT"));
    }

    #[test]
    fn arbiter_output_rejects_wrong_h1() {
        let text = "# Wrong Heading\n\n## Amendment: D1\n\n### Ruling\nACCEPT\n\n### Rationale\nok";
        let result = parse_arbiter_output(text, &["D1"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported arbiter H1"));
    }

    #[test]
    fn final_reviewer_strips_frontmatter() {
        let text = "---\nartifact: final-review\n---\n# Final Review: NO AMENDMENTS\n\n## Summary\nAll good.";
        let parsed = parse_final_reviewer_output(text).expect("should strip frontmatter and parse");
        assert!(matches!(parsed, FinalReviewerDecision::NoAmendments { .. }));
    }

    #[test]
    fn vote_output_rejects_missing_h1() {
        let text = "no heading at all";
        let result = parse_vote_output(text, &["A1"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing a top-level H1"));
    }

    #[test]
    fn planner_position_rejects_missing_h1() {
        let text = "no heading at all";
        let result = parse_planner_position_output(text, &["A1"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing a top-level H1"));
    }

    #[test]
    fn final_reviewer_no_amendments_requires_summary() {
        let text = "# Final Review: NO AMENDMENTS\n\nno summary section";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("## Summary"));
    }

    // -----------------------------------------------------------------------
    // FinalReview fail-closed subsection enforcement tests
    // -----------------------------------------------------------------------

    #[test]
    fn final_reviewer_rejects_amendment_missing_problem() {
        let text = "# Final Review: AMENDMENTS\n\n## Amendment: FIX-001\n\n### Proposed Change\nfix it\n\n### Affected Files\n- `src/lib.rs`";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("### Problem"), "expected missing Problem error, got: {err}");
        assert!(err.contains("FIX-001"), "expected amendment ID in error, got: {err}");
    }

    #[test]
    fn final_reviewer_rejects_amendment_missing_proposed_change() {
        let text = "# Final Review: AMENDMENTS\n\n## Amendment: FIX-001\n\n### Problem\nbug\n\n### Affected Files\n- `src/lib.rs`";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("### Proposed Change"), "expected missing Proposed Change error, got: {err}");
    }

    #[test]
    fn final_reviewer_rejects_amendment_missing_affected_files() {
        let text = "# Final Review: AMENDMENTS\n\n## Amendment: FIX-001\n\n### Problem\nbug\n\n### Proposed Change\nfix it";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("### Affected Files"), "expected missing Affected Files error, got: {err}");
    }

    #[test]
    fn final_reviewer_rejects_second_amendment_missing_subsection() {
        // First amendment is valid, second is missing ### Proposed Change
        let text = "# Final Review: AMENDMENTS\n\n## Amendment: OK-1\n\n### Problem\nbug\n\n### Proposed Change\nfix\n\n### Affected Files\n- `a.rs`\n\n## Amendment: BAD-2\n\n### Problem\ntypo\n\n### Affected Files\n- `b.rs`";
        let result = parse_final_reviewer_output(text);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("### Proposed Change"), "got: {err}");
        assert!(err.contains("BAD-2"), "error should reference the failing amendment ID, got: {err}");
    }

    #[test]
    fn planner_position_rejects_missing_rationale() {
        let text = "# Planner Positions\n\n## Amendment: FIX-001\n\n### Position\nACCEPT";
        let result = parse_planner_position_output(text, &["FIX-001"]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("### Rationale"), "expected missing Rationale error, got: {err}");
        assert!(err.contains("FIX-001"), "expected amendment ID in error, got: {err}");
    }

    #[test]
    fn vote_output_rejects_missing_rationale() {
        let text = "# Vote Results\n\n## Amendment: A1\n\n### Vote\nACCEPT";
        let result = parse_vote_output(text, &["A1"]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("### Rationale"), "expected missing Rationale error, got: {err}");
        assert!(err.contains("A1"), "expected amendment ID in error, got: {err}");
    }

    #[test]
    fn arbiter_output_rejects_missing_rationale() {
        let text = "# Arbiter Ruling\n\n## Amendment: D1\n\n### Ruling\nACCEPT";
        let result = parse_arbiter_output(text, &["D1"]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("### Rationale"), "expected missing Rationale error, got: {err}");
        assert!(err.contains("D1"), "expected amendment ID in error, got: {err}");
    }

    #[test]
    fn planner_position_rejects_partial_rationale_coverage() {
        // First amendment has Rationale, second doesn't
        let text = "# Planner Positions\n\n## Amendment: A1\n\n### Position\nACCEPT\n\n### Rationale\ngood\n\n## Amendment: A2\n\n### Position\nREJECT";
        let result = parse_planner_position_output(text, &["A1", "A2"]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("### Rationale"), "got: {err}");
        assert!(err.contains("A2"), "error should reference the failing amendment, got: {err}");
    }
}
