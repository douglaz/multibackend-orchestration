//! PRD pipeline state types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::gaps::Question;

/// PRD generation stages in canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Stage {
    Ideation,
    Research,
    Synthesis,
    Prd,
}

impl Stage {
    /// Returns all stages in canonical order.
    pub fn all() -> &'static [Stage] {
        &[Stage::Ideation, Stage::Research, Stage::Synthesis, Stage::Prd]
    }

    /// Returns the zero-based index of this stage in the canonical order.
    pub fn index(&self) -> usize {
        match self {
            Stage::Ideation => 0,
            Stage::Research => 1,
            Stage::Synthesis => 2,
            Stage::Prd => 3,
        }
    }

    /// Returns the artifact filename for this stage.
    pub fn artifact_filename(&self) -> &str {
        match self {
            Stage::Ideation => "01_ideation.md",
            Stage::Research => "02_research.md",
            Stage::Synthesis => "03_synthesis.md",
            Stage::Prd => "04_prd.md",
        }
    }
}

/// PRD pipeline state machine phases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrdPhase {
    RunStage(Stage),
    CheckGaps(Stage),
    AskUser(Vec<Question>),
    ApplyAnswers,
    MaybeRerun(Stage),
    ValidatePrd,
    Done,
}

/// PRD pipeline execution context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineContext {
    /// The initial idea/product description.
    pub idea: String,
    /// User-provided answers to questions (key -> answer).
    pub answers: BTreeMap<String, String>,
    /// Generated outputs per stage.
    pub stage_outputs: BTreeMap<Stage, String>,
    /// Input hashes per stage (for cache invalidation).
    pub stage_input_hashes: BTreeMap<Stage, String>,
    /// Hash of current answers state.
    pub answers_hash: String,
    /// Number of question rounds completed.
    pub question_rounds: u32,
}

/// PRD pipeline metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrdMeta {
    /// The initial idea/product description.
    pub idea: String,
    /// Hash of the idea (first 12 chars of SHA256).
    pub idea_hash: String,
    /// Backend specification used.
    pub backend: String,
    /// ISO8601 timestamp when pipeline started.
    pub started_at: String,
    /// ISO8601 timestamp when pipeline completed (if finished).
    pub completed_at: Option<String>,
    /// Time taken per stage in seconds.
    pub stage_timings: BTreeMap<Stage, f64>,
    /// Number of question rounds completed.
    pub question_rounds: u32,
    /// Stages that were rerun due to new answers.
    pub rerun_stages: Vec<Stage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stage_ordering() {
        let stages = Stage::all();
        assert_eq!(stages.len(), 4);
        assert_eq!(stages[0], Stage::Ideation);
        assert_eq!(stages[1], Stage::Research);
        assert_eq!(stages[2], Stage::Synthesis);
        assert_eq!(stages[3], Stage::Prd);

        // Verify PartialOrd derives work correctly
        assert!(Stage::Ideation < Stage::Research);
        assert!(Stage::Research < Stage::Synthesis);
        assert!(Stage::Synthesis < Stage::Prd);
    }

    #[test]
    fn test_stage_index() {
        assert_eq!(Stage::Ideation.index(), 0);
        assert_eq!(Stage::Research.index(), 1);
        assert_eq!(Stage::Synthesis.index(), 2);
        assert_eq!(Stage::Prd.index(), 3);
    }

    #[test]
    fn test_stage_artifact_filename() {
        assert_eq!(Stage::Ideation.artifact_filename(), "01_ideation.md");
        assert_eq!(Stage::Research.artifact_filename(), "02_research.md");
        assert_eq!(Stage::Synthesis.artifact_filename(), "03_synthesis.md");
        assert_eq!(Stage::Prd.artifact_filename(), "04_prd.md");
    }

    #[test]
    fn test_stage_serde_roundtrip() {
        for stage in Stage::all() {
            let json = serde_json::to_string(stage).unwrap();
            let deserialized: Stage = serde_json::from_str(&json).unwrap();
            assert_eq!(*stage, deserialized);
        }
    }

    #[test]
    fn test_pipeline_context_serde() {
        let mut stage_outputs = BTreeMap::new();
        stage_outputs.insert(Stage::Ideation, "ideation content".to_string());
        stage_outputs.insert(Stage::Research, "research content".to_string());

        let mut stage_input_hashes = BTreeMap::new();
        stage_input_hashes.insert(Stage::Ideation, "hash1".to_string());
        stage_input_hashes.insert(Stage::Research, "hash2".to_string());

        let mut answers = BTreeMap::new();
        answers.insert("q1".to_string(), "answer1".to_string());

        let ctx = PipelineContext {
            idea: "test idea".to_string(),
            answers,
            stage_outputs,
            stage_input_hashes,
            answers_hash: "answers_hash".to_string(),
            question_rounds: 2,
        };

        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: PipelineContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, deserialized);
    }

    #[test]
    fn test_prd_meta_serde() {
        let mut stage_timings = BTreeMap::new();
        stage_timings.insert(Stage::Ideation, 1.5);
        stage_timings.insert(Stage::Research, 2.3);

        let meta = PrdMeta {
            idea: "test idea".to_string(),
            idea_hash: "abc123".to_string(),
            backend: "codex(gpt-5.3-codex)".to_string(),
            started_at: "2026-02-10T20:00:00Z".to_string(),
            completed_at: Some("2026-02-10T20:10:00Z".to_string()),
            stage_timings,
            question_rounds: 1,
            rerun_stages: vec![Stage::Research, Stage::Synthesis],
        };

        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: PrdMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, deserialized);
    }
}
