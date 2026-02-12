//! Quick PRD foundational types, prompts, and helper functions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::prd::gaps::extract_fenced_json;
use crate::workflow::parser::strip_frontmatter;
use crate::Result;

const REQUIRED_SECTIONS: [&str; 6] = [
    "## Summary",
    "## Acceptance Criteria",
    "## Technical Approach",
    "## Files & Modules",
    "## Testing Strategy",
    "## Out of Scope",
];

/// Options for running quick PRD generation.
#[derive(Debug, Clone)]
pub struct QuickPrdOptions {
    pub idea: String,
    pub writer_spec: String,
    pub reviewer_spec: String,
    pub max_revisions: u32,
    pub dry_run: bool,
}

/// Result of a completed quick PRD run.
#[derive(Debug, Clone)]
pub struct QuickPrdResult {
    pub spec_path: PathBuf,
    pub cache_dir: PathBuf,
    pub revision_count: u32,
    pub approved: bool,
    pub summary: String,
}

/// Metadata persisted for a quick PRD run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuickPrdMeta {
    pub idea: String,
    pub idea_hash: String,
    pub writer_backend: String,
    pub reviewer_backend: String,
    pub started_at: String,
    pub completed_at: String,
    pub revision_count: u32,
    pub approved: bool,
    pub draft_time_secs: f64,
    pub review_times_secs: Vec<f64>,
    pub revision_times_secs: Vec<f64>,
}

/// Structured reviewer response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFeedback {
    pub approved: bool,
    pub issues: Vec<ReviewIssue>,
}

/// A single reviewer issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub area: String,
    pub feedback: String,
}

/// Draft prompt used by the writer backend.
pub const DRAFT_PROMPT: &str = r#"You are a senior software engineer writing a focused engineering specification.

**Feature Idea:**
{{idea}}

**Required Output Format:**
Your response must be a markdown document with the following exact section headings:

## Summary
## Acceptance Criteria
## Technical Approach
## Files & Modules
## Testing Strategy
## Out of Scope

Each section should be concise, specific, and implementation-ready.
"#;

/// Review prompt used by the reviewer backend.
pub const REVIEW_PROMPT: &str = r#"You are a senior engineer reviewing an engineering specification for completeness and feasibility.

**Feature Idea:**
{{idea}}

**Engineering Spec:**
{{spec}}

**Task:**
Review the spec for: technical feasibility, missing edge cases, completeness of acceptance criteria, testing coverage, and clarity.

**Required Output Format:**
Your response MUST be a single fenced JSON block:

```json
{"approved": true, "issues": []}
```

If issues found:

```json
{"approved": false, "issues": [{"area": "...", "feedback": "..."}]}
```
"#;

/// Revision prompt used by the writer backend.
pub const REVISION_PROMPT: &str = r#"You are a senior software engineer revising an engineering specification based on review feedback.

**Feature Idea:**
{{idea}}

**Current Spec:**
{{spec}}

**Review Issues:**
{{issues}}

**Task:**
Address each review issue and produce an updated specification. You MUST preserve the same 6 required section headings:
## Summary, ## Acceptance Criteria, ## Technical Approach, ## Files & Modules, ## Testing Strategy, ## Out of Scope
"#;

/// Simple inline placeholder replacement.
pub fn render_prompt(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (placeholder, value) in replacements {
        result = result.replace(placeholder, value);
    }
    result
}

/// Checks quick PRD spec output for required sections after frontmatter removal.
pub fn check_spec_sections(raw: &str) -> (String, Vec<String>) {
    let cleaned = strip_frontmatter(raw);
    let mut missing_sections = Vec::new();

    for section in REQUIRED_SECTIONS {
        if !cleaned.lines().any(|line| line.trim() == section) {
            missing_sections.push(section.to_string());
        }
    }

    (cleaned, missing_sections)
}

/// Parses reviewer feedback from a fenced JSON payload.
pub fn parse_review_feedback(raw: &str) -> Result<ReviewFeedback> {
    let fenced_json = extract_fenced_json(raw)?;
    let feedback = serde_json::from_str::<ReviewFeedback>(fenced_json)?;
    Ok(feedback)
}

/// Formats issues as a numbered list for revision prompts.
pub fn format_issues(issues: &[ReviewIssue]) -> String {
    if issues.is_empty() {
        return "(none)".to_string();
    }

    issues
        .iter()
        .enumerate()
        .map(|(index, issue)| format!("{}. {}: {}", index + 1, issue.area, issue.feedback))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec() -> &'static str {
        "## Summary\nBody\n## Acceptance Criteria\nBody\n## Technical Approach\nBody\n## Files & Modules\nBody\n## Testing Strategy\nBody\n## Out of Scope\nBody"
    }

    #[test]
    fn test_render_prompt() {
        let template = "Hello {{name}}, role={{role}}";
        let rendered = render_prompt(template, &[("{{name}}", "Alice"), ("{{role}}", "writer")]);
        assert_eq!(rendered, "Hello Alice, role=writer");
    }

    #[test]
    fn test_check_spec_sections_all_present() {
        let (cleaned, missing) = check_spec_sections(valid_spec());
        assert_eq!(cleaned, valid_spec());
        assert!(missing.is_empty());
    }

    #[test]
    fn test_check_spec_sections_some_missing() {
        let raw = "## Summary\nBody\n## Acceptance Criteria\nBody\n## Testing Strategy\nBody";
        let (_, missing) = check_spec_sections(raw);
        assert_eq!(
            missing,
            vec![
                "## Technical Approach".to_string(),
                "## Files & Modules".to_string(),
                "## Out of Scope".to_string(),
            ]
        );
    }

    #[test]
    fn test_check_spec_sections_with_frontmatter() {
        let raw = format!("---\nartifact: spec\n---\n{}", valid_spec());
        let (cleaned, missing) = check_spec_sections(&raw);
        assert_eq!(cleaned, valid_spec());
        assert!(missing.is_empty());
    }

    #[test]
    fn test_parse_review_feedback_approved() {
        let raw = "```json\n{\"approved\": true, \"issues\": []}\n```";
        let parsed = parse_review_feedback(raw).expect("feedback should parse");
        assert!(parsed.approved);
        assert!(parsed.issues.is_empty());
    }

    #[test]
    fn test_parse_review_feedback_rejected() {
        let raw = "prefix\n```json\n{\"approved\": false, \"issues\": [{\"area\": \"testing\", \"feedback\": \"add integration test\"}]}\n```\nsuffix";
        let parsed = parse_review_feedback(raw).expect("feedback should parse");
        assert!(!parsed.approved);
        assert_eq!(parsed.issues.len(), 1);
        assert_eq!(parsed.issues[0].area, "testing");
        assert_eq!(parsed.issues[0].feedback, "add integration test");
    }

    #[test]
    fn test_parse_review_feedback_malformed() {
        let raw = "{\"approved\": true, \"issues\": []}";
        assert!(parse_review_feedback(raw).is_err());
    }

    #[test]
    fn test_review_feedback_serde_roundtrip() {
        let feedback = ReviewFeedback {
            approved: false,
            issues: vec![
                ReviewIssue {
                    area: "feasibility".to_string(),
                    feedback: "clarify migration strategy".to_string(),
                },
                ReviewIssue {
                    area: "testing".to_string(),
                    feedback: "define failure-mode tests".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&feedback).unwrap();
        let roundtrip: ReviewFeedback = serde_json::from_str(&json).unwrap();
        assert_eq!(feedback, roundtrip);
    }

    #[test]
    fn test_format_issues() {
        let issues = vec![
            ReviewIssue {
                area: "acceptance criteria".to_string(),
                feedback: "add timeout behavior".to_string(),
            },
            ReviewIssue {
                area: "technical approach".to_string(),
                feedback: "include rollback strategy".to_string(),
            },
        ];

        let formatted = format_issues(&issues);
        assert_eq!(
            formatted,
            "1. acceptance criteria: add timeout behavior\n2. technical approach: include rollback strategy"
        );
    }
}
