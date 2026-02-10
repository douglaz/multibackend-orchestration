//! PRD gap analysis types and logic.

use serde::{Deserialize, Serialize};

use super::state::Stage;

/// Gap analysis report from a stage output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}
