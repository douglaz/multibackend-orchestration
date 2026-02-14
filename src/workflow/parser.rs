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
}
