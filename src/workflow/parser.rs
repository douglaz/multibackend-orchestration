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
        return Ok(PlannerDecision::Feature {
            name: trimmed.to_owned(),
            body,
        });
    }

    if first_h1.trim() == "# Project Completion Request" {
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
        "# Verdict: COMPLETE" => Ok(CompleterDecision {
            verdict: CompletionVerdict::Complete,
            body,
        }),
        "# Verdict: CONTINUE" => {
            validate_required_section(
                &body,
                "## Missing Requirements",
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

#[cfg(test)]
mod tests {
    use super::{
        extract_commit_message, parse_completer_output, parse_implementer_output,
        parse_planner_output, parse_reviewer_output, ImplementerDecision, PlannerDecision,
        ReviewerDecision,
    };
    use crate::project::state::CompletionVerdict;

    #[test]
    fn parses_feature_heading() {
        let text = "# Feature: User Authentication\n\n## Description\n...";
        let parsed = parse_planner_output(text).expect("expected parse success");
        match parsed {
            PlannerDecision::Feature { name, .. } => assert_eq!(name, "User Authentication"),
            _ => panic!("expected feature decision"),
        }
    }

    #[test]
    fn parses_completion_heading() {
        let text = "# Project Completion Request\n\n## Rationale\n...";
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
        let text = "# Verdict: CONTINUE\n\n## Missing Requirements\n1. x";
        let parsed = parse_completer_output(text).expect("parse should succeed");
        assert_eq!(parsed.verdict, CompletionVerdict::Continue);
    }

    #[test]
    fn extracts_commit_message_block() {
        let text = "# Review: APPROVED\n\n## Commit Message\nfeat: one\nmore\n\n## Notes\n...";
        assert_eq!(
            extract_commit_message(text),
            Some("feat: one more".to_owned())
        );
    }
}
