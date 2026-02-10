//! PRD pipeline driver with non-interactive happy path.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::backend::Backend;
use crate::error::RalphError;
use crate::util::time::now_iso8601;
use crate::Result;

use super::answers::AnswerStore;
use super::cache::{CacheManager, PrdLock};
use super::interaction::UserInteraction;
use super::stages::{check_stage_output, StagePromptBuilder};
use super::state::{PipelineContext, PrdMeta, Stage};

/// Options for PRD pipeline execution.
#[derive(Debug, Clone)]
pub struct PrdOptions {
    /// The product idea to generate a PRD for.
    pub idea: String,
    /// The backend specification (e.g., "codex(gpt-5.3-codex)").
    pub backend_spec: String,
    /// Maximum number of question rounds (deferred to loop 7).
    pub ask_max: u32,
    /// Whether to resume from cached outputs.
    pub resume: bool,
    /// Whether to run in dry-run mode (deferred).
    pub dry_run: bool,
}

/// Result of a successful PRD pipeline run.
#[derive(Debug, Clone)]
pub struct PrdResult {
    /// Path to the final PRD.md in the current working directory.
    pub prd_path: PathBuf,
    /// Path to the cache directory.
    pub cache_dir: PathBuf,
    /// Pipeline metadata.
    pub meta: PrdMeta,
    /// Human-readable summary of the result.
    pub summary: String,
}

/// The PRD pipeline driver.
pub struct PrdPipeline {
    backend: Arc<dyn Backend>,
    interaction: Box<dyn UserInteraction>,
    cache: CacheManager,
    answer_store: AnswerStore,
    context: PipelineContext,
    meta: PrdMeta,
    options: PrdOptions,
}

impl PrdPipeline {
    /// Creates a new PRD pipeline.
    ///
    /// # Arguments
    /// - `backend`: The backend to use for LLM calls.
    /// - `interaction`: The interaction layer for user communication.
    /// - `cache`: The cache manager for reading/writing artifacts.
    /// - `answer_store`: The answer store for loading/saving user answers.
    /// - `options`: Pipeline options.
    pub fn new(
        backend: Arc<dyn Backend>,
        interaction: Box<dyn UserInteraction>,
        cache: CacheManager,
        answer_store: AnswerStore,
        options: PrdOptions,
    ) -> Result<Self> {
        // Initialize context with the idea and empty state.
        let context = PipelineContext {
            idea: options.idea.clone(),
            answers: BTreeMap::new(),
            stage_outputs: BTreeMap::new(),
            stage_input_hashes: BTreeMap::new(),
            answers_hash: String::new(),
            question_rounds: 0,
        };

        // Initialize metadata.
        let meta = PrdMeta {
            idea: options.idea.clone(),
            idea_hash: cache.idea_hash().to_string(),
            backend: options.backend_spec.clone(),
            started_at: now_iso8601(),
            completed_at: None,
            stage_timings: BTreeMap::new(),
            question_rounds: 0,
            rerun_stages: Vec::new(),
        };

        Ok(Self {
            backend,
            interaction,
            cache,
            answer_store,
            context,
            meta,
            options,
        })
    }

    /// Runs the PRD pipeline, executing stages sequentially and handling gaps deterministically.
    ///
    /// This is the loop 6 implementation: sequential stage execution with deterministic gap checks.
    /// LLM gap analysis, interactive reruns, and validation are deferred to loops 7-8.
    pub async fn run(mut self) -> Result<PrdResult> {
        // Acquire lock for the duration of the run.
        let _lock = self.cache.acquire_lock()?;

        // Validate resume (if enabled) to ensure cache matches idea.
        if self.options.resume {
            self.cache.validate_resume_idea()?;
        }

        // Load pre-existing answers from answer store.
        self.answer_store.load()?;
        self.context.answers = self.answer_store.answers().clone();
        self.context.answers_hash = self.answer_store.hash()?;

        // Execute stages in order: Ideation -> Research -> Synthesis -> Prd.
        for &stage in Stage::all() {
            self.run_stage(stage).await?;
            self.check_stage_gaps(stage)?;
        }

        // Write final metadata and output PRD.md to current working directory.
        self.meta.completed_at = Some(now_iso8601());
        self.cache.write_meta(&self.meta)?;

        let final_prd = self
            .context
            .stage_outputs
            .get(&Stage::Prd)
            .ok_or_else(|| RalphError::PrdPipelineFailed("PRD stage output missing".to_string()))?;

        let prd_path = PathBuf::from("PRD.md");
        std::fs::write(&prd_path, final_prd)?;

        let summary = format!(
            "PRD generated successfully in {} question rounds",
            self.meta.question_rounds
        );

        Ok(PrdResult {
            prd_path,
            cache_dir: self.cache.cache_dir().to_path_buf(),
            meta: self.meta,
            summary,
        })
    }

    /// Runs a single stage: builds prompt, calls backend, persists output.
    async fn run_stage(&mut self, stage: Stage) -> Result<()> {
        let start = Instant::now();

        self.interaction
            .status(&format!("Running {:?} stage...", stage));

        // Build the stage prompt with current context.
        let builder =
            StagePromptBuilder::new(self.context.idea.clone(), self.context.answers.clone(), self.context.stage_outputs.clone());
        let prompt = builder.build_stage_prompt(stage);

        // Call the backend exactly once.
        let raw_output = self.backend.execute(&prompt).await?;

        // Parse and clean the output immediately after backend execution.
        let check = check_stage_output(stage, &raw_output);

        // Persist the cleaned output to cache (frontmatter stripped).
        self.cache.write_stage_output(stage, &check.cleaned_output)?;

        // Update context with cleaned output (used for downstream stages).
        self.context
            .stage_outputs
            .insert(stage, check.cleaned_output.clone());

        // Compute and store input hash for this stage.
        let input_hash = self.cache.compute_stage_input_hash(stage, &self.context);
        self.context.stage_input_hashes.insert(stage, input_hash);

        // Record timing.
        let elapsed = start.elapsed().as_secs_f64();
        self.meta.stage_timings.insert(stage, elapsed);

        self.interaction
            .stage_complete(&stage, &format!("completed in {:.1}s", elapsed));

        Ok(())
    }

    /// Checks stage output for deterministic missing sections.
    /// On missing sections, writes missing_info_report.md and returns PrdMissingInfo error.
    fn check_stage_gaps(&mut self, stage: Stage) -> Result<()> {
        let cleaned_output = self
            .context
            .stage_outputs
            .get(&stage)
            .ok_or_else(|| {
                RalphError::PrdPipelineFailed(format!("{:?} stage output missing", stage))
            })?;

        let check = check_stage_output(stage, cleaned_output);

        if !check.missing_sections.is_empty() {
            // Deterministic missing sections detected.
            let report = format_missing_info_report(stage, &check.missing_sections);
            self.cache.write_missing_info_report(&report)?;

            self.interaction
                .status(&format!("Missing sections in {:?} stage output", stage));

            return Err(RalphError::PrdMissingInfo);
        }

        Ok(())
    }
}

/// Formats a missing info report for deterministic missing sections.
fn format_missing_info_report(stage: Stage, missing_sections: &[String]) -> String {
    let mut report = format!("# Missing Information Report\n\n");
    report.push_str(&format!("Stage: {:?}\n\n", stage));
    report.push_str("Missing Required Sections:\n\n");
    for section in missing_sections {
        report.push_str(&format!("- {}\n", section));
    }
    report.push_str("\n");
    report.push_str("The LLM output for this stage is missing required sections.\n");
    report.push_str("Please ensure the backend is correctly configured and retry.\n");
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use crate::prd::interaction::MockInteraction;
    use tempfile::TempDir;

    fn make_ideation_output() -> String {
        r#"
## Core Concept
A smart onboarding system.

## Target Users
New hires in tech companies.

## Key Problems Solved
Manual onboarding is slow and error-prone.

## Proposed Features
- Automated task assignment
- Progress tracking

## Success Metrics
- Time to productivity
- User satisfaction

## Constraints & Assumptions
- Must integrate with existing HR systems.
"#
        .to_string()
    }

    fn make_research_output() -> String {
        r#"
## Market Context
Competitive market with several players.

## Technical Landscape
Cloud-based SaaS is standard.

## Comparable Solutions
BambooHR, Workday.

## Technical Feasibility
Highly feasible with modern tech stack.

## Risk Assessment
Low technical risk, moderate market risk.
"#
        .to_string()
    }

    fn make_synthesis_output() -> String {
        r#"
## Product Vision
Streamline employee onboarding.

## User Stories
- As an HR manager, I want to automate task assignment.

## Feature Prioritization
1. Core onboarding workflow
2. Integrations

## Architecture Overview
Microservices on AWS.

## MVP Scope
Basic task assignment and tracking.

## Open Questions
- Which HR systems to prioritize?
"#
        .to_string()
    }

    fn make_prd_output() -> String {
        r#"
## Executive Summary
A system to automate employee onboarding.

## Goals & Non-Goals
Goals: automate onboarding. Non-goals: payroll.

## User Stories
- As an HR manager, I want to assign tasks.

## Functional Requirements
- Task creation and assignment.

## Non-Functional Requirements
- 99.9% uptime.

## Technical Architecture
Microservices with API gateway.

## Data Model
User, Task, Assignment entities.

## API Design
RESTful API with JSON payloads.

## Security Considerations
OAuth 2.0 for authentication.

## Testing Strategy
Unit, integration, and E2E tests.

## Rollout Plan
Phased rollout starting with pilot team.

## Success Metrics
- Reduced onboarding time by 50%.

## Open Questions
- Timeline for pilot?
"#
        .to_string()
    }

    #[tokio::test]
    async fn happy_path_non_interactive_all_stages_succeed() {
        let temp = TempDir::new().expect("temp dir");
        let idea = "smart onboarding system";

        let cache = CacheManager::new(temp.path(), idea).expect("cache manager");
        let answers_path = cache.cache_dir().join("answers.yaml");
        let answer_store = AnswerStore::new(&answers_path);

        let backend = Arc::new(MockBackend::new(
            "mock",
            vec![
                make_ideation_output(),
                make_research_output(),
                make_synthesis_output(),
                make_prd_output(),
            ],
        ));

        let interaction = Box::new(MockInteraction::new(vec![]));

        let options = PrdOptions {
            idea: idea.to_string(),
            backend_spec: "codex(gpt-5.3-codex)".to_string(),
            ask_max: 3,
            resume: false,
            dry_run: false,
        };

        let pipeline = PrdPipeline::new(backend.clone(), interaction, cache, answer_store, options)
            .expect("pipeline creation");

        let result = pipeline.run().await.expect("pipeline run");

        // Verify PRD.md was written to current directory.
        assert_eq!(result.prd_path, PathBuf::from("PRD.md"));
        assert!(std::fs::metadata("PRD.md").is_ok());

        // Verify metadata was recorded.
        assert_eq!(result.meta.question_rounds, 0);
        assert_eq!(result.meta.stage_timings.len(), 4);
        assert!(result.meta.completed_at.is_some());

        // Verify backend was called exactly 4 times.
        assert_eq!(backend.call_count().await, 4);

        // Cleanup.
        let _ = std::fs::remove_file("PRD.md");
    }

    #[tokio::test]
    async fn missing_section_triggers_prd_missing_info() {
        let temp = TempDir::new().expect("temp dir");
        let idea = "incomplete ideation test";

        let cache = CacheManager::new(temp.path(), idea).expect("cache manager");
        let answers_path = cache.cache_dir().join("answers.yaml");
        let answer_store = AnswerStore::new(&answers_path);

        // Ideation output missing "## Success Metrics" and "## Constraints & Assumptions".
        let incomplete_ideation = r#"
## Core Concept
A thing.

## Target Users
Some users.

## Key Problems Solved
Some problems.

## Proposed Features
Some features.
"#
        .to_string();

        let backend = Arc::new(MockBackend::new("mock", vec![incomplete_ideation]));
        let interaction = Box::new(MockInteraction::new(vec![]));

        let options = PrdOptions {
            idea: idea.to_string(),
            backend_spec: "codex(gpt-5.3-codex)".to_string(),
            ask_max: 3,
            resume: false,
            dry_run: false,
        };

        let pipeline = PrdPipeline::new(backend, interaction, cache.clone(), answer_store, options)
            .expect("pipeline creation");

        let err = pipeline.run().await.expect_err("should fail");
        assert!(matches!(err, RalphError::PrdMissingInfo));

        // Verify missing_info_report.md was written.
        let report_path = cache.cache_dir().join("missing_info_report.md");
        let report = std::fs::read_to_string(report_path).expect("read report");
        assert!(report.contains("Missing Required Sections"));
        assert!(report.contains("## Success Metrics"));
        assert!(report.contains("## Constraints & Assumptions"));
    }
}
