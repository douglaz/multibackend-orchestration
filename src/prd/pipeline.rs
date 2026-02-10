//! PRD pipeline driver.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::backend::Backend;
use crate::error::RalphError;
use crate::util::time::now_iso8601;
use crate::Result;

use super::answers::AnswerStore;
use super::cache::CacheManager;
use super::gaps::{
    gap_report_has_questions, run_llm_gap_analysis, GapReport, Question, QuestionKind,
};
use super::interaction::{InteractionContext, UserInteraction};
use super::stages::{check_stage_output, StagePromptBuilder};
use super::state::{PipelineContext, PrdMeta, PrdPhase, Stage};

/// Options for PRD pipeline execution.
#[derive(Debug, Clone)]
pub struct PrdOptions {
    /// The product idea to generate a PRD for.
    pub idea: String,
    /// The backend specification (e.g., "codex(gpt-5.3-codex)").
    pub backend_spec: String,
    /// Maximum number of question rounds.
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
    pending_questions: Vec<Question>,
    pending_answers: Option<BTreeMap<String, String>>,
    pending_gap_stage: Option<Stage>,
}

impl PrdPipeline {
    /// Creates a new PRD pipeline.
    pub fn new(
        backend: Arc<dyn Backend>,
        interaction: Box<dyn UserInteraction>,
        cache: CacheManager,
        answer_store: AnswerStore,
        options: PrdOptions,
    ) -> Result<Self> {
        let context = PipelineContext {
            idea: options.idea.clone(),
            answers: BTreeMap::new(),
            stage_outputs: BTreeMap::new(),
            stage_input_hashes: BTreeMap::new(),
            answers_hash: String::new(),
            question_rounds: 0,
        };

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
            pending_questions: Vec::new(),
            pending_answers: None,
            pending_gap_stage: None,
        })
    }

    /// Runs the PRD pipeline as a state machine.
    pub async fn run(mut self) -> Result<PrdResult> {
        let _lock = self.cache.acquire_lock()?;

        if self.options.resume {
            self.cache.validate_resume_idea()?;
        }

        self.answer_store.load()?;
        self.context.answers = self.answer_store.answers().clone();
        self.context.answers_hash = self.answer_store.hash()?;

        let mut phase = PrdPhase::RunStage(Stage::Ideation);
        loop {
            phase = match phase {
                PrdPhase::RunStage(stage) => {
                    self.run_stage(stage).await?;
                    PrdPhase::CheckGaps(stage)
                }
                PrdPhase::CheckGaps(stage) => self.check_gaps_phase(stage).await?,
                PrdPhase::AskUser(questions) => self.ask_user_phase(questions).await?,
                PrdPhase::ApplyAnswers => self.apply_answers_phase()?,
                PrdPhase::MaybeRerun(stage) => self.maybe_rerun_phase(stage),
                PrdPhase::ValidatePrd => PrdPhase::Done,
                PrdPhase::Done => break,
            };
        }

        self.finalize_result()
    }

    async fn run_stage(&mut self, stage: Stage) -> Result<()> {
        let start = Instant::now();
        self.interaction
            .status(&format!("Running {:?} stage...", stage));

        let builder = StagePromptBuilder::new(
            self.context.idea.clone(),
            self.context.answers.clone(),
            self.context.stage_outputs.clone(),
        );
        let prompt = builder.build_stage_prompt(stage);
        let raw_output = self.backend.execute(&prompt).await?;

        let check = check_stage_output(stage, &raw_output);
        self.cache
            .write_stage_output(stage, &check.cleaned_output)?;
        self.context
            .stage_outputs
            .insert(stage, check.cleaned_output.clone());

        let input_hash = self.cache.compute_stage_input_hash(stage, &self.context);
        self.context.stage_input_hashes.insert(stage, input_hash);

        let elapsed = start.elapsed().as_secs_f64();
        self.meta.stage_timings.insert(stage, elapsed);
        self.interaction
            .stage_complete(&stage, &format!("completed in {:.1}s", elapsed));

        Ok(())
    }

    async fn check_gaps_phase(&mut self, stage: Stage) -> Result<PrdPhase> {
        let stage_output = self
            .context
            .stage_outputs
            .get(&stage)
            .cloned()
            .ok_or_else(|| {
                RalphError::PrdPipelineFailed(format!("{:?} stage output missing", stage))
            })?;

        let check = check_stage_output(stage, &stage_output);
        if !check.missing_sections.is_empty() {
            let report = format_deterministic_missing_info_report(stage, &check.missing_sections);
            self.cache.write_missing_info_report(&report)?;
            return Err(RalphError::PrdMissingInfo);
        }

        let gap_report =
            run_llm_gap_analysis(self.backend.clone(), stage, &stage_output, &self.context).await?;
        if gap_report_has_questions(&gap_report) {
            let max_rounds_reached = self.context.question_rounds >= self.options.ask_max;
            if !self.interaction.is_interactive() || max_rounds_reached {
                let reason = if !self.interaction.is_interactive() {
                    "Pipeline is running in non-interactive mode.".to_owned()
                } else {
                    format!(
                        "Maximum question rounds reached ({}/{}).",
                        self.context.question_rounds, self.options.ask_max
                    )
                };
                let report = format_gap_missing_info_report(stage, &gap_report, &reason);
                self.cache.write_missing_info_report(&report)?;
                return Err(RalphError::PrdMissingInfo);
            }

            self.pending_gap_stage = Some(stage);
            self.pending_questions = gap_report.questions.clone();
            return Ok(PrdPhase::AskUser(gap_report.questions));
        }

        Ok(match next_stage(stage) {
            Some(next) => PrdPhase::RunStage(next),
            None => PrdPhase::ValidatePrd,
        })
    }

    async fn ask_user_phase(&mut self, questions: Vec<Question>) -> Result<PrdPhase> {
        self.context.question_rounds += 1;
        self.meta.question_rounds = self.context.question_rounds;

        self.pending_questions = questions.clone();
        let fallback_stage = min_question_impact_stage(&questions).unwrap_or(Stage::Ideation);
        let stage = self.pending_gap_stage.unwrap_or(fallback_stage);
        let interaction_ctx = InteractionContext {
            stage,
            question_round: self.context.question_rounds,
            max_rounds: self.options.ask_max,
        };

        let maybe_answers = self
            .interaction
            .ask_questions(&questions, &interaction_ctx)
            .await?;

        match maybe_answers {
            None => Err(RalphError::PrdPipelineFailed(
                "user exited during question flow".to_owned(),
            )),
            Some(answers) => {
                self.pending_answers = Some(answers);
                Ok(PrdPhase::ApplyAnswers)
            }
        }
    }

    fn apply_answers_phase(&mut self) -> Result<PrdPhase> {
        let answers = self.pending_answers.take().ok_or_else(|| {
            RalphError::PrdPipelineFailed("no answers available to apply".to_owned())
        })?;

        let fallback_stage = self
            .pending_gap_stage
            .take()
            .or_else(|| min_question_impact_stage(&self.pending_questions))
            .unwrap_or(Stage::Ideation);

        let rerun_stage = self
            .pending_questions
            .iter()
            .filter(|question| answers.contains_key(&question.key))
            .map(|question| question.impact_stage)
            .min()
            .or_else(|| min_question_impact_stage(&self.pending_questions))
            .unwrap_or(fallback_stage);

        self.answer_store.merge(answers);
        self.answer_store.save()?;
        self.context.answers = self.answer_store.answers().clone();
        self.context.answers_hash = self.answer_store.hash()?;

        self.pending_questions.clear();

        Ok(PrdPhase::MaybeRerun(rerun_stage))
    }

    fn maybe_rerun_phase(&mut self, rerun_stage: Stage) -> PrdPhase {
        for stage in Stage::all() {
            if *stage >= rerun_stage {
                self.context.stage_outputs.remove(stage);
                self.context.stage_input_hashes.remove(stage);
            }
        }

        self.meta.rerun_stages.push(rerun_stage);
        PrdPhase::RunStage(rerun_stage)
    }

    fn finalize_result(mut self) -> Result<PrdResult> {
        self.meta.completed_at = Some(now_iso8601());
        self.cache.write_meta(&self.meta)?;

        let final_prd =
            self.context.stage_outputs.get(&Stage::Prd).ok_or_else(|| {
                RalphError::PrdPipelineFailed("PRD stage output missing".to_owned())
            })?;

        let prd_path = PathBuf::from("PRD.md");
        std::fs::write(&prd_path, final_prd)?;

        Ok(PrdResult {
            prd_path,
            cache_dir: self.cache.cache_dir().to_path_buf(),
            meta: self.meta,
            summary: format!(
                "PRD generated successfully in {} question rounds",
                self.context.question_rounds
            ),
        })
    }
}

fn next_stage(stage: Stage) -> Option<Stage> {
    match stage {
        Stage::Ideation => Some(Stage::Research),
        Stage::Research => Some(Stage::Synthesis),
        Stage::Synthesis => Some(Stage::Prd),
        Stage::Prd => None,
    }
}

fn min_question_impact_stage(questions: &[Question]) -> Option<Stage> {
    questions.iter().map(|question| question.impact_stage).min()
}

fn format_deterministic_missing_info_report(stage: Stage, missing_sections: &[String]) -> String {
    let mut report = String::from("# Missing Information Report\n\n");
    report.push_str(&format!("## Stage\n`{stage:?}`\n\n"));
    report.push_str("## Missing Required Sections\n");
    for section in missing_sections {
        report.push_str(&format!("- {section}\n"));
    }
    report.push_str("\n## Next Step\n");
    report.push_str("Regenerate the stage output with all required headings.\n");
    report
}

fn format_gap_missing_info_report(stage: Stage, report: &GapReport, reason: &str) -> String {
    let mut markdown = String::from("# Missing Information Report\n\n");
    markdown.push_str(&format!("## Stage\n`{stage:?}`\n\n"));
    markdown.push_str(&format!("## Pipeline Stopped\n{reason}\n\n"));

    markdown.push_str("## Questions\n");
    if report.questions.is_empty() {
        markdown.push_str("- None\n");
    } else {
        for question in &report.questions {
            markdown.push_str(&format!(
                "- **{}** (`{:?}`): {}\n",
                question.key, question.impact_stage, question.prompt
            ));
            markdown.push_str(&format!(
                "  Type: {}\n",
                format_question_kind(&question.kind)
            ));
            if let Some(default) = &question.suggested_default {
                markdown.push_str(&format!("  Suggested default: {default}\n"));
            }
        }
    }
    markdown.push('\n');

    markdown.push_str("## Missing Fields\n");
    if report.missing_fields.is_empty() {
        markdown.push_str("- None\n");
    } else {
        for field in &report.missing_fields {
            markdown.push_str(&format!("- **{}**: {}\n", field.field, field.description));
        }
    }
    markdown.push('\n');

    markdown.push_str("## Ambiguities\n");
    if report.ambiguities.is_empty() {
        markdown.push_str("- None\n");
    } else {
        for ambiguity in &report.ambiguities {
            markdown.push_str(&format!(
                "- **{}**: {}\n",
                ambiguity.area, ambiguity.description
            ));
        }
    }
    markdown.push('\n');

    markdown.push_str("## Suggested Defaults\n");
    if report.suggested_defaults.is_empty() {
        markdown.push_str("- None\n");
    } else {
        for default in &report.suggested_defaults {
            markdown.push_str(&format!(
                "- **{}** = `{}` ({})\n",
                default.key, default.value, default.rationale
            ));
        }
    }

    markdown
}

fn format_question_kind(kind: &QuestionKind) -> String {
    match kind {
        QuestionKind::FreeText => "FreeText".to_owned(),
        QuestionKind::Choice(options) => format!("Choice [{}]", options.join(", ")),
        QuestionKind::YesNo => "YesNo".to_owned(),
    }
}
