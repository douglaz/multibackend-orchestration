//! PRD gap analysis types and logic.

use std::sync::Arc;

use crate::backend::Backend;
use crate::error::RalphError;
use crate::Result;
use serde::{Deserialize, Serialize};

use super::stages::StagePromptBuilder;
use super::state::{PipelineContext, Stage};

/// Gap analysis report from a stage output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapReport {
    /// Fields that are required but missing from the output.
    pub missing_fields: Vec<MissingField>,
    /// Areas that are present but ambiguous or unclear.
    pub ambiguities: Vec<Ambiguity>,
    /// Questions to ask the user to resolve gaps.
    pub questions: Vec<Question>,
    /// Suggested default values for optional fields.
    pub suggested_defaults: Vec<SuggestedDefault>,
}

/// A required field that is missing from the stage output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingField {
    /// Name of the missing field.
    pub field: String,
    /// Description of what information is missing.
    pub description: String,
}

/// An area that is present but ambiguous or unclear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ambiguity {
    /// Area of ambiguity.
    pub area: String,
    /// Description of the ambiguity.
    pub description: String,
}

/// A suggested default value for an optional field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedDefault {
    /// Key for the default value.
    pub key: String,
    /// The suggested default value.
    pub value: String,
    /// Rationale for the suggested default.
    pub rationale: String,
}

/// A question to ask the user to resolve a gap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Unique key for this question (used to store answer).
    pub key: String,
    /// The question prompt to display to the user.
    pub prompt: String,
    /// The type of question (free text, choice, yes/no).
    pub kind: QuestionKind,
    /// Optional suggested default answer.
    pub suggested_default: Option<String>,
    /// Which stage this question impacts (typed, not stringly typed).
    pub impact_stage: Stage,
}

/// Type of question to ask the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionKind {
    /// Free-form text answer.
    FreeText,
    /// Multiple choice from a list of options.
    Choice(Vec<String>),
    /// Yes/No question.
    YesNo,
}

/// Parses a gap analysis response from an LLM, extracting the fenced JSON block.
pub fn parse_gap_report(raw: &str) -> Result<GapReport> {
    let fenced_json = extract_fenced_json(raw)?;
    let report = serde_json::from_str::<GapReport>(fenced_json)?;
    Ok(report)
}

/// Returns true when a gap report has at least one follow-up question.
pub fn gap_report_has_questions(report: &GapReport) -> bool {
    !report.questions.is_empty()
}

/// A validation issue found in the final PRD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Field name that has an issue.
    pub field: String,
    /// Description of what is missing or unclear.
    pub description: String,
}

/// Result of PRD validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether the PRD is valid and ready for implementation.
    pub valid: bool,
    /// List of issues found (empty if valid is true).
    pub issues: Vec<ValidationIssue>,
}

/// Runs LLM-based gap analysis with up to 3 parse attempts and parse-fallback behavior.
pub async fn run_llm_gap_analysis(
    backend: Arc<dyn Backend>,
    stage: Stage,
    stage_output: &str,
    context: &PipelineContext,
) -> Result<GapReport> {
    let prompt_builder = StagePromptBuilder::new(
        context.idea.clone(),
        context.answers.clone(),
        context.stage_outputs.clone(),
    );

    let mut prompt = prompt_builder.build_gap_analysis_prompt(stage, stage_output);

    for attempt in 1..=3_u8 {
        let raw = backend.execute(&prompt).await?;
        match parse_gap_report(&raw) {
            Ok(report) => return Ok(report),
            Err(parse_error) => {
                if attempt == 3 {
                    return Ok(GapReport::default());
                }
                prompt = build_reformat_prompt(stage, &parse_error, &raw);
            }
        }
    }

    Ok(GapReport::default())
}

/// Parses a validation response from an LLM, extracting the fenced JSON block.
pub fn parse_validation_result(raw: &str) -> Result<ValidationResult> {
    let fenced_json = extract_fenced_json(raw)?;
    let result = serde_json::from_str::<ValidationResult>(fenced_json)?;
    Ok(result)
}

/// Runs LLM-based PRD validation with up to 3 parse attempts and parse-fallback behavior.
pub async fn run_llm_validation(
    backend: Arc<dyn Backend>,
    prd: &str,
    context: &PipelineContext,
) -> Result<ValidationResult> {
    let prompt_builder = StagePromptBuilder::new(
        context.idea.clone(),
        context.answers.clone(),
        context.stage_outputs.clone(),
    );

    let mut prompt = prompt_builder.build_validation_prompt(prd);

    for attempt in 1..=3_u8 {
        let raw = backend.execute(&prompt).await?;
        match parse_validation_result(&raw) {
            Ok(result) => return Ok(result),
            Err(parse_error) => {
                if attempt == 3 {
                    return Err(RalphError::PrdValidationFailed(format!(
                        "Failed to parse validation result after 3 attempts. Last error: {}",
                        parse_error
                    )));
                }
                prompt = build_validation_reformat_prompt(&parse_error, &raw);
            }
        }
    }

    unreachable!("loop should return or error before reaching this point")
}

fn extract_fenced_json(raw: &str) -> Result<&str> {
    let fence_start = raw
        .find("```json")
        .ok_or_else(|| RalphError::ParseError("missing opening ```json fence".to_owned()))?;
    let json_start = fence_start + "```json".len();
    let tail = &raw[json_start..];
    let fence_end = tail
        .find("```")
        .ok_or_else(|| RalphError::ParseError("missing closing ``` fence".to_owned()))?;
    Ok(tail[..fence_end].trim())
}

fn build_reformat_prompt(stage: Stage, parse_error: &RalphError, previous_output: &str) -> String {
    format!(
        "CRITICAL: Your previous gap analysis for stage {stage:?} could not be parsed.\n\n\
Error: {parse_error}\n\n\
Return ONLY a single fenced JSON block with schema:\n\
`missing_fields`, `ambiguities`, `questions`, `suggested_defaults`.\n\
Use valid JSON, no prose before or after the fenced block.\n\n\
Previous response:\n---\n{previous_output}\n---\n"
    )
}

fn build_validation_reformat_prompt(parse_error: &RalphError, previous_output: &str) -> String {
    format!(
        "CRITICAL: Your previous validation response could not be parsed.\n\n\
Error: {parse_error}\n\n\
Return ONLY a single fenced JSON block with schema:\n\
{{\"valid\": bool, \"issues\": [{{\"field\": string, \"description\": string}}]}}\n\
Use valid JSON, no prose before or after the fenced block.\n\n\
Previous response:\n---\n{previous_output}\n---\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_question_kind_serde_roundtrip() {
        // Test FreeText variant
        let free_text = QuestionKind::FreeText;
        let json = serde_json::to_string(&free_text).unwrap();
        let deserialized: QuestionKind = serde_json::from_str(&json).unwrap();
        assert_eq!(free_text, deserialized);

        // Test Choice variant
        let choice = QuestionKind::Choice(vec!["Option A".to_string(), "Option B".to_string()]);
        let json = serde_json::to_string(&choice).unwrap();
        let deserialized: QuestionKind = serde_json::from_str(&json).unwrap();
        assert_eq!(choice, deserialized);

        // Test YesNo variant
        let yes_no = QuestionKind::YesNo;
        let json = serde_json::to_string(&yes_no).unwrap();
        let deserialized: QuestionKind = serde_json::from_str(&json).unwrap();
        assert_eq!(yes_no, deserialized);
    }

    #[test]
    fn test_gap_report_serde_roundtrip() {
        let report = GapReport {
            missing_fields: vec![MissingField {
                field: "target_audience".to_string(),
                description: "Target audience not specified".to_string(),
            }],
            ambiguities: vec![Ambiguity {
                area: "deployment".to_string(),
                description: "Deployment strategy is unclear".to_string(),
            }],
            questions: vec![Question {
                key: "q1".to_string(),
                prompt: "What is the target platform?".to_string(),
                kind: QuestionKind::Choice(vec!["Web".to_string(), "Mobile".to_string()]),
                suggested_default: Some("Web".to_string()),
                impact_stage: Stage::Research,
            }],
            suggested_defaults: vec![SuggestedDefault {
                key: "deployment_type".to_string(),
                value: "cloud".to_string(),
                rationale: "Most common deployment pattern".to_string(),
            }],
        };

        let json = serde_json::to_string(&report).unwrap();
        let deserialized: GapReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, deserialized);
    }

    #[test]
    fn test_question_typed_impact_stage() {
        let question = Question {
            key: "q_platform".to_string(),
            prompt: "Which platform?".to_string(),
            kind: QuestionKind::Choice(vec!["iOS".to_string(), "Android".to_string()]),
            suggested_default: None,
            impact_stage: Stage::Ideation,
        };

        // Verify impact_stage is typed as Stage enum
        assert_eq!(question.impact_stage, Stage::Ideation);
        assert!(question.impact_stage < Stage::Research);

        // Verify serde preserves typed stage
        let json = serde_json::to_string(&question).unwrap();
        let deserialized: Question = serde_json::from_str(&json).unwrap();
        assert_eq!(question.impact_stage, deserialized.impact_stage);
    }

    #[test]
    fn parse_gap_report_valid_fenced_json() {
        let raw = r#"
Some analysis text.

```json
{
  "missing_fields": [],
  "ambiguities": [],
  "questions": [
    {
      "key": "target_user",
      "prompt": "Who is the target user?",
      "kind": "FreeText",
      "suggested_default": null,
      "impact_stage": "Ideation"
    }
  ],
  "suggested_defaults": []
}
```
"#;

        let report = parse_gap_report(raw).expect("should parse");
        assert_eq!(report.questions.len(), 1);
        assert!(gap_report_has_questions(&report));
    }

    #[test]
    fn parse_gap_report_malformed_json_returns_error() {
        let raw = r#"
```json
{ "missing_fields": [ }
```
"#;

        assert!(parse_gap_report(raw).is_err());
    }

    #[test]
    fn parse_gap_report_missing_fence_markers_returns_error() {
        let raw =
            r#"{"missing_fields":[],"ambiguities":[],"questions":[],"suggested_defaults":[]}"#;
        assert!(parse_gap_report(raw).is_err());
    }

    #[test]
    fn parse_gap_report_empty_questions_list() {
        let raw = r#"
```json
{
  "missing_fields": [],
  "ambiguities": [],
  "questions": [],
  "suggested_defaults": []
}
```
"#;

        let report = parse_gap_report(raw).expect("should parse");
        assert!(report.questions.is_empty());
        assert!(!gap_report_has_questions(&report));
    }

    #[test]
    fn parse_validation_result_valid() {
        let raw = r#"
```json
{
  "valid": true,
  "issues": []
}
```
"#;

        let result = parse_validation_result(raw).expect("should parse");
        assert!(result.valid);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn parse_validation_result_invalid_with_issues() {
        let raw = r#"
```json
{
  "valid": false,
  "issues": [
    {"field": "api_design", "description": "API endpoints not specified"},
    {"field": "data_model", "description": "Schema details missing"}
  ]
}
```
"#;

        let result = parse_validation_result(raw).expect("should parse");
        assert!(!result.valid);
        assert_eq!(result.issues.len(), 2);
        assert_eq!(result.issues[0].field, "api_design");
        assert_eq!(result.issues[1].field, "data_model");
    }

    #[test]
    fn validation_result_serde_roundtrip() {
        let result = ValidationResult {
            valid: false,
            issues: vec![
                ValidationIssue {
                    field: "security".to_string(),
                    description: "Authentication strategy unclear".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: ValidationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }
}
