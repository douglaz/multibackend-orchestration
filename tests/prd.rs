//! Integration tests for PRD pipeline gap analysis and interactive rerun flow.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ralph::backend::MockBackend;
use ralph::error::RalphError;
use ralph::prd::state::Stage;
use ralph::prd::{
    AnswerStore, CacheManager, MockInteraction, NonInteractiveInteraction, PrdOptions, PrdPipeline,
};
use tempfile::TempDir;

fn make_ideation_output(label: &str) -> String {
    format!(
        r#"
## Core Concept
{label} core concept.

## Target Users
Teams building internal software.

## Key Problems Solved
Manual process drift and poor handoffs.

## Proposed Features
- Guided setup
- Automated checks

## Success Metrics
- Time saved
- Fewer onboarding errors

## Constraints & Assumptions
- Existing auth systems remain unchanged.
"#
    )
}

fn make_research_output(label: &str) -> String {
    format!(
        r#"
## Market Context
{label} market context.

## Technical Landscape
Rust + web stack.

## Comparable Solutions
Internal tooling platforms.

## Technical Feasibility
Feasible with moderate effort.

## Risk Assessment
Moderate adoption risk.
"#
    )
}

fn make_synthesis_output(label: &str) -> String {
    format!(
        r#"
## Product Vision
{label} product vision.

## User Stories
- As a team lead, I want predictable onboarding.

## Feature Prioritization
1. Guided flow
2. Automation hooks

## Architecture Overview
Service + worker architecture.

## MVP Scope
Core flow and metrics.

## Open Questions
- Integration timeline?
"#
    )
}

fn make_prd_output(label: &str) -> String {
    format!(
        r#"
## Executive Summary
{label} PRD summary.

## Goals & Non-Goals
Goals: consistency. Non-goals: org redesign.

## User Stories
- As an operator, I want visibility into status.

## Functional Requirements
- Create and manage onboarding workflows.

## Non-Functional Requirements
- 99.9% availability.

## Technical Architecture
API and background jobs.

## Data Model
Workflow, step, assignment.

## API Design
REST API with JSON.

## Security Considerations
RBAC and audit logs.

## Testing Strategy
Unit and integration coverage.

## Rollout Plan
Pilot with one team.

## Success Metrics
- Completion rate and latency.

## Open Questions
- Long-term ownership model?
"#
    )
}

fn empty_gap_report() -> String {
    r#"
```json
{
  "missing_fields": [],
  "ambiguities": [],
  "questions": [],
  "suggested_defaults": []
}
```
"#
    .to_string()
}

fn gap_report_with_question(key: &str, prompt: &str, impact_stage: Stage) -> String {
    format!(
        r#"
```json
{{
  "missing_fields": [{{"field": "target_market", "description": "target market is unclear"}}],
  "ambiguities": [{{"area": "scope", "description": "MVP scope is ambiguous"}}],
  "questions": [
    {{
      "key": "{key}",
      "prompt": "{prompt}",
      "kind": "FreeText",
      "suggested_default": null,
      "impact_stage": "{impact_stage:?}"
    }}
  ],
  "suggested_defaults": []
}}
```
"#
    )
}

fn invalid_gap_response(body: &str) -> String {
    format!("not parseable as fenced json: {body}")
}

fn base_options(idea: &str, ask_max: u32) -> PrdOptions {
    PrdOptions {
        idea: idea.to_string(),
        backend_spec: "codex(gpt-5.3-codex)".to_string(),
        ask_max,
        resume: false,
        dry_run: false,
    }
}

fn cwd_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct CwdGuard {
    original: PathBuf,
}

impl CwdGuard {
    fn enter(path: &Path) -> Self {
        let original = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(path).expect("set current dir");
        Self { original }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

#[tokio::test]
async fn llm_gap_detected_questions_answered_reruns_from_correct_stage_then_succeeds() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "interactive rerun test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("initial"),
            empty_gap_report(),
            make_research_output("initial"),
            gap_report_with_question(
                "target_platform",
                "Which platform should be prioritized?",
                Stage::Research,
            ),
            make_research_output("rerun"),
            empty_gap_report(),
            make_synthesis_output("rerun"),
            empty_gap_report(),
            make_prd_output("rerun"),
            empty_gap_report(),
        ],
    ));

    let mut answers = BTreeMap::new();
    answers.insert("target_platform".to_string(), "web".to_string());
    let interaction = Box::new(MockInteraction::new(vec![Some(answers)]));

    let pipeline = PrdPipeline::new(
        backend.clone(),
        interaction,
        cache.clone(),
        answer_store,
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let _cwd_guard = cwd_lock().lock().expect("cwd lock");
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    assert_eq!(backend.call_count().await, 10);
    assert_eq!(result.meta.question_rounds, 1);
    assert_eq!(result.meta.rerun_stages, vec![Stage::Research]);

    let cached_ideation = cache
        .read_stage_output(Stage::Ideation)
        .expect("read ideation")
        .expect("missing ideation");
    let cached_research = cache
        .read_stage_output(Stage::Research)
        .expect("read research")
        .expect("missing research");
    assert!(cached_ideation.contains("initial core concept"));
    assert!(cached_research.contains("rerun market context"));

    assert!(workspace_root.join("PRD.md").exists());
}

#[tokio::test]
async fn max_question_rounds_exceeded_returns_prd_missing_info_exit_12() {
    let temp = TempDir::new().expect("temp dir");
    let idea = "ask max exceeded";
    let cache = CacheManager::new(temp.path(), idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("initial"),
            gap_report_with_question(
                "target_users",
                "Who exactly are the primary users?",
                Stage::Ideation,
            ),
            make_ideation_output("rerun"),
            gap_report_with_question(
                "target_users",
                "Who exactly are the primary users?",
                Stage::Ideation,
            ),
        ],
    ));

    let mut answers = BTreeMap::new();
    answers.insert(
        "target_users".to_string(),
        "engineering managers".to_string(),
    );
    let interaction = Box::new(MockInteraction::new(vec![Some(answers)]));

    let pipeline = PrdPipeline::new(
        backend.clone(),
        interaction,
        cache.clone(),
        answer_store,
        base_options(idea, 1),
    )
    .expect("pipeline creation");

    let err = pipeline.run().await.expect_err("should fail");
    assert!(matches!(err, RalphError::PrdMissingInfo));
    assert_eq!(err.exit_code(), 12);
    assert_eq!(backend.call_count().await, 4);

    let report = std::fs::read_to_string(cache.cache_dir().join("missing_info_report.md"))
        .expect("read report");
    assert!(report.contains("Maximum question rounds reached"));
    assert!(report.contains("Who exactly are the primary users?"));
    assert!(report.contains("Missing Fields"));
    assert!(report.contains("Ambiguities"));
}

#[tokio::test]
async fn non_interactive_mode_with_llm_gaps_returns_prd_missing_info_exit_12() {
    let temp = TempDir::new().expect("temp dir");
    let idea = "non-interactive llm gap";
    let cache = CacheManager::new(temp.path(), idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("initial"),
            gap_report_with_question(
                "timeline",
                "What is the expected launch timeline?",
                Stage::Ideation,
            ),
        ],
    ));

    let interaction = Box::new(NonInteractiveInteraction::new());

    let pipeline = PrdPipeline::new(
        backend.clone(),
        interaction,
        cache.clone(),
        answer_store,
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let err = pipeline.run().await.expect_err("should fail");
    assert!(matches!(err, RalphError::PrdMissingInfo));
    assert_eq!(err.exit_code(), 12);
    assert_eq!(backend.call_count().await, 2);

    let report = std::fs::read_to_string(cache.cache_dir().join("missing_info_report.md"))
        .expect("read report");
    assert!(report.contains("non-interactive mode"));
    assert!(report.contains("What is the expected launch timeline?"));
}

#[tokio::test]
async fn user_quit_during_questions_returns_prd_pipeline_failed_exit_10() {
    let temp = TempDir::new().expect("temp dir");
    let idea = "user quit";
    let cache = CacheManager::new(temp.path(), idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("initial"),
            gap_report_with_question(
                "scope",
                "Should this include admin workflows?",
                Stage::Ideation,
            ),
        ],
    ));

    let interaction = Box::new(MockInteraction::new(vec![None]));

    let pipeline = PrdPipeline::new(
        backend.clone(),
        interaction,
        cache,
        answer_store,
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let err = pipeline.run().await.expect_err("should fail");
    assert!(matches!(err, RalphError::PrdPipelineFailed(_)));
    assert_eq!(err.exit_code(), 10);
    assert_eq!(backend.call_count().await, 2);
}

#[tokio::test]
async fn gap_analysis_json_parse_failure_retried_three_times_then_falls_back() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "parse retry fallback";
    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("initial"),
            invalid_gap_response("attempt 1"),
            invalid_gap_response("attempt 2"),
            invalid_gap_response("attempt 3"),
            make_research_output("initial"),
            empty_gap_report(),
            make_synthesis_output("initial"),
            empty_gap_report(),
            make_prd_output("initial"),
            empty_gap_report(),
        ],
    ));

    let interaction = Box::new(MockInteraction::new(vec![]));

    let pipeline = PrdPipeline::new(
        backend.clone(),
        interaction,
        cache.clone(),
        answer_store,
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let _cwd_guard = cwd_lock().lock().expect("cwd lock");
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    assert_eq!(result.meta.question_rounds, 0);
    assert_eq!(backend.call_count().await, 10);
    assert!(workspace_root.join("PRD.md").exists());
    assert!(!cache.cache_dir().join("missing_info_report.md").exists());
}
