//! Integration tests for PRD pipeline gap analysis and interactive rerun flow.
#![allow(clippy::await_holding_lock)]

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

fn validation_pass() -> String {
    r#"
```json
{
  "valid": true,
  "issues": []
}
```
"#
    .to_string()
}

fn validation_fail(field: &str, description: &str) -> String {
    format!(
        r#"
```json
{{
  "valid": false,
  "issues": [
    {{"field": "{field}", "description": "{description}"}}
  ]
}}
```
"#
    )
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

fn gap_report_with_defaults(
    key: &str,
    prompt: &str,
    suggested_default: &str,
    impact_stage: Stage,
) -> String {
    format!(
        r#"
```json
{{
  "missing_fields": [],
  "ambiguities": [],
  "questions": [
    {{
      "key": "{key}",
      "prompt": "{prompt}",
      "kind": "FreeText",
      "suggested_default": "{suggested_default}",
      "impact_stage": "{impact_stage:?}"
    }}
  ],
  "suggested_defaults": [
    {{
      "key": "extra_default",
      "value": "auto_value",
      "rationale": "reasonable default"
    }}
  ]
}}
```
"#
    )
}

fn invalid_gap_response(body: &str) -> String {
    format!("not parseable as fenced json: {body}")
}

fn malformed_stage_output(label: &str) -> String {
    format!("Malformed stage output ({label}) with no required markdown headings.")
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

fn acquire_cwd_lock() -> std::sync::MutexGuard<'static, ()> {
    match cwd_lock().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
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
            validation_pass(),
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

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    assert_eq!(backend.call_count().await, 11);
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
async fn max_question_rounds_exceeded_auto_applies_defaults_and_continues() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "ask max exceeded";
    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    // Flow: ideation → gap (ask user) → rerun ideation → gap (max reached, auto-apply)
    //       → rerun ideation → clean gap → research → ... → PRD → validate
    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("initial"),
            gap_report_with_question(
                "target_users",
                "Who exactly are the primary users?",
                Stage::Ideation,
            ),
            // After user answers round 1, rerun ideation
            make_ideation_output("rerun1"),
            gap_report_with_question(
                "target_users",
                "Who exactly are the primary users?",
                Stage::Ideation,
            ),
            // Max rounds reached → auto-apply (no defaults) → rerun ideation
            make_ideation_output("rerun2"),
            empty_gap_report(),
            make_research_output("rerun2"),
            empty_gap_report(),
            make_synthesis_output("rerun2"),
            empty_gap_report(),
            make_prd_output("rerun2"),
            empty_gap_report(),
            validation_pass(),
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

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    // 4 calls for initial + gap + rerun + gap, then 9 for final full run
    assert_eq!(backend.call_count().await, 13);
    assert_eq!(result.meta.question_rounds, 1);
    assert!(workspace_root.join("PRD.md").exists());
}

#[tokio::test]
async fn non_interactive_mode_with_llm_gaps_auto_applies_defaults_and_continues() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "non-interactive llm gap";
    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    // After the gap report, the pipeline auto-applies defaults and reruns from
    // the impact stage (Ideation). We need mock responses for the full rerun.
    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("initial"),
            gap_report_with_question(
                "timeline",
                "What is the expected launch timeline?",
                Stage::Ideation,
            ),
            // Rerun from Ideation after auto-apply (no defaults to apply, best-effort)
            make_ideation_output("rerun"),
            empty_gap_report(),
            make_research_output("rerun"),
            empty_gap_report(),
            make_synthesis_output("rerun"),
            empty_gap_report(),
            make_prd_output("rerun"),
            empty_gap_report(),
            validation_pass(),
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

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    // 2 calls for initial ideation + gap, then 9 for full rerun
    assert_eq!(backend.call_count().await, 11);
    assert_eq!(result.meta.question_rounds, 0);
    assert!(workspace_root.join("PRD.md").exists());
}

#[tokio::test]
async fn non_interactive_auto_applies_suggested_defaults_into_answers() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "non-interactive with defaults";
    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("initial"),
            gap_report_with_defaults(
                "timeline",
                "What is the expected launch timeline?",
                "Q3 2026",
                Stage::Ideation,
            ),
            // Rerun from Ideation after auto-applying defaults
            make_ideation_output("rerun"),
            empty_gap_report(),
            make_research_output("rerun"),
            empty_gap_report(),
            make_synthesis_output("rerun"),
            empty_gap_report(),
            make_prd_output("rerun"),
            empty_gap_report(),
            validation_pass(),
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

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    assert_eq!(backend.call_count().await, 11);
    assert!(workspace_root.join("PRD.md").exists());

    // Verify that suggested defaults were persisted in the answers file
    let answers_content = std::fs::read_to_string(&answers_path).expect("read answers");
    assert!(answers_content.contains("timeline"));
    assert!(answers_content.contains("Q3 2026"));
    assert!(answers_content.contains("extra_default"));
    assert!(answers_content.contains("auto_value"));

    // Ideation was rerun (rerun stage tracked)
    assert_eq!(result.meta.rerun_stages, vec![Stage::Ideation]);
}

#[tokio::test]
async fn forward_impact_stage_advances_without_skipping_stages() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "forward impact stage";
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
                "launch_scope",
                "Which scope should the launch prioritize?",
                Stage::Prd,
            ),
            make_synthesis_output("continued"),
            empty_gap_report(),
            make_prd_output("continued"),
            empty_gap_report(),
            validation_pass(),
        ],
    ));

    let interaction = Box::new(NonInteractiveInteraction::new());
    let pipeline = PrdPipeline::new(
        backend.clone(),
        interaction,
        cache.clone(),
        answer_store,
        base_options(idea, 0),
    )
    .expect("pipeline creation");

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    assert_eq!(backend.call_count().await, 9);
    assert!(result.meta.rerun_stages.is_empty());
    let synthesis = cache
        .read_stage_output(Stage::Synthesis)
        .expect("read synthesis")
        .expect("missing synthesis");
    assert!(synthesis.contains("continued product vision"));
}

#[tokio::test]
async fn apply_answers_caps_rerun_to_current_stage() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "apply answers rerun cap";
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
                "platform_focus",
                "Which platform should be prioritized first?",
                Stage::Prd,
            ),
            make_research_output("rerun"),
            empty_gap_report(),
            make_synthesis_output("rerun"),
            empty_gap_report(),
            make_prd_output("rerun"),
            empty_gap_report(),
            validation_pass(),
        ],
    ));

    let mut answers = BTreeMap::new();
    answers.insert("platform_focus".to_string(), "web".to_string());
    let interaction = Box::new(MockInteraction::new(vec![Some(answers)]));

    let pipeline = PrdPipeline::new(
        backend.clone(),
        interaction,
        cache.clone(),
        answer_store,
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    assert_eq!(backend.call_count().await, 11);
    assert_eq!(result.meta.rerun_stages, vec![Stage::Research]);
    let synthesis = cache
        .read_stage_output(Stage::Synthesis)
        .expect("read synthesis")
        .expect("missing synthesis");
    assert!(synthesis.contains("rerun product vision"));
}

#[tokio::test]
async fn missing_sections_retried_then_succeeds() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "missing sections retried";
    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            malformed_stage_output("ideation attempt 1"),
            make_ideation_output("ideation attempt 2"),
            empty_gap_report(),
            make_research_output("steady"),
            empty_gap_report(),
            make_synthesis_output("steady"),
            empty_gap_report(),
            make_prd_output("steady"),
            empty_gap_report(),
            validation_pass(),
        ],
    ));

    let interaction = MockInteraction::new(vec![]);
    let interaction_handle = interaction.clone();
    let pipeline = PrdPipeline::new(
        backend.clone(),
        Box::new(interaction),
        cache,
        answer_store,
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    pipeline.run().await.expect("pipeline run");

    assert_eq!(backend.call_count().await, 10);
    assert!(interaction_handle
        .status_messages()
        .iter()
        .any(|msg| msg.contains("retry 1/2")));
}

#[tokio::test]
async fn missing_sections_retry_exhaustion_continues_best_effort() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "missing sections retry exhaustion";
    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            malformed_stage_output("ideation attempt 1"),
            malformed_stage_output("ideation attempt 2"),
            malformed_stage_output("ideation attempt 3"),
            empty_gap_report(),
            make_research_output("steady"),
            empty_gap_report(),
            make_synthesis_output("steady"),
            empty_gap_report(),
            make_prd_output("steady"),
            empty_gap_report(),
            validation_pass(),
        ],
    ));

    let interaction = MockInteraction::new(vec![]);
    let interaction_handle = interaction.clone();
    let pipeline = PrdPipeline::new(
        backend.clone(),
        Box::new(interaction),
        cache,
        answer_store,
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    pipeline.run().await.expect("pipeline run");

    assert_eq!(backend.call_count().await, 11);
    assert!(interaction_handle
        .status_messages()
        .iter()
        .any(|msg| msg.contains("continuing best-effort")));
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
            validation_pass(),
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

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    assert_eq!(result.meta.question_rounds, 0);
    assert_eq!(backend.call_count().await, 11);
    assert!(workspace_root.join("PRD.md").exists());
    assert!(!cache.cache_dir().join("missing_info_report.md").exists());
}

#[tokio::test]
async fn validation_pass_writes_final_prd() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "validation pass test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("v1"),
            empty_gap_report(),
            make_research_output("v1"),
            empty_gap_report(),
            make_synthesis_output("v1"),
            empty_gap_report(),
            make_prd_output("v1"),
            empty_gap_report(),
            validation_pass(),
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

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let result = pipeline.run().await.expect("pipeline run");

    assert_eq!(backend.call_count().await, 9);
    assert_eq!(result.meta.question_rounds, 0);
    assert!(workspace_root.join("PRD.md").exists());

    let prd_content = std::fs::read_to_string(workspace_root.join("PRD.md")).expect("read PRD");
    assert!(prd_content.contains("v1 PRD summary"));
}

#[tokio::test]
async fn validation_fail_returns_exit_11_with_report() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "validation fail test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("v1"),
            empty_gap_report(),
            make_research_output("v1"),
            empty_gap_report(),
            make_synthesis_output("v1"),
            empty_gap_report(),
            make_prd_output("v1"),
            empty_gap_report(),
            validation_fail("api_design", "API endpoints not fully specified"),
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

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let err = pipeline.run().await.expect_err("should fail");

    assert!(matches!(err, RalphError::PrdValidationFailed(_)));
    assert_eq!(err.exit_code(), 11);
    assert_eq!(backend.call_count().await, 9);

    let report_path = cache.cache_dir().join("validation_report.md");
    assert!(report_path.exists());
    let report = std::fs::read_to_string(report_path).expect("read validation report");
    assert!(report.contains("PRD Validation Failed"));
    assert!(report.contains("api_design"));
    assert!(report.contains("API endpoints not fully specified"));

    assert!(!workspace_root.join("PRD.md").exists());
}

#[tokio::test]
async fn resume_skips_cached_stages_with_matching_hashes() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "resume skip test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    // First run: complete pipeline
    let backend_first = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("first"),
            empty_gap_report(),
            make_research_output("first"),
            empty_gap_report(),
            make_synthesis_output("first"),
            empty_gap_report(),
            make_prd_output("first"),
            empty_gap_report(),
            validation_pass(),
        ],
    ));

    let interaction_first = Box::new(MockInteraction::new(vec![]));

    let pipeline_first = PrdPipeline::new(
        backend_first.clone(),
        interaction_first,
        cache.clone(),
        answer_store.clone(),
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    pipeline_first.run().await.expect("first run");
    assert_eq!(backend_first.call_count().await, 9);

    // Second run: resume with no changes (all stages should skip)
    let backend_second = Arc::new(MockBackend::new("mock", vec![validation_pass()]));

    let interaction_second = Box::new(MockInteraction::new(vec![]));

    let mut options_second = base_options(idea, 3);
    options_second.resume = true;

    let pipeline_second = PrdPipeline::new(
        backend_second.clone(),
        interaction_second,
        cache.clone(),
        answer_store,
        options_second,
    )
    .expect("pipeline creation");

    pipeline_second.run().await.expect("second run");

    // All 4 stages should be skipped, only validation runs
    assert_eq!(backend_second.call_count().await, 1);
}

#[tokio::test]
async fn resume_invalidates_stages_when_answers_change() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "resume invalidation test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let mut answer_store = AnswerStore::new(&answers_path);

    // First run: complete pipeline
    let backend_first = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("first"),
            empty_gap_report(),
            make_research_output("first"),
            empty_gap_report(),
            make_synthesis_output("first"),
            empty_gap_report(),
            make_prd_output("first"),
            empty_gap_report(),
            validation_pass(),
        ],
    ));

    let interaction_first = Box::new(MockInteraction::new(vec![]));

    let pipeline_first = PrdPipeline::new(
        backend_first.clone(),
        interaction_first,
        cache.clone(),
        answer_store.clone(),
        base_options(idea, 3),
    )
    .expect("pipeline creation");

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    pipeline_first.run().await.expect("first run");
    assert_eq!(backend_first.call_count().await, 9);

    // Modify answers file
    let mut new_answers = std::collections::BTreeMap::new();
    new_answers.insert("platform".to_string(), "mobile".to_string());
    answer_store.merge(new_answers);
    answer_store.save().expect("save modified answers");

    // Second run: resume with changed answers (stages should regenerate)
    let backend_second = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("second"),
            empty_gap_report(),
            make_research_output("second"),
            empty_gap_report(),
            make_synthesis_output("second"),
            empty_gap_report(),
            make_prd_output("second"),
            empty_gap_report(),
            validation_pass(),
        ],
    ));

    let interaction_second = Box::new(MockInteraction::new(vec![]));

    let mut options_second = base_options(idea, 3);
    options_second.resume = true;

    let pipeline_second = PrdPipeline::new(
        backend_second.clone(),
        interaction_second,
        cache.clone(),
        AnswerStore::new(&answers_path),
        options_second,
    )
    .expect("pipeline creation");

    pipeline_second.run().await.expect("second run");

    // All stages should rerun due to changed answers affecting input hashes
    assert_eq!(backend_second.call_count().await, 9);
}

#[tokio::test]
async fn resume_idea_mismatch_returns_cache_mismatch_error() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea_first = "first idea";

    let cache_first = CacheManager::new(workspace_root, idea_first).expect("cache manager");
    let answers_path_first = cache_first.cache_dir().join("answers.yaml");
    let answer_store_first = AnswerStore::new(&answers_path_first);

    // First run with idea_first
    let backend_first = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("first"),
            empty_gap_report(),
            make_research_output("first"),
            empty_gap_report(),
            make_synthesis_output("first"),
            empty_gap_report(),
            make_prd_output("first"),
            empty_gap_report(),
            validation_pass(),
        ],
    ));

    let interaction_first = Box::new(MockInteraction::new(vec![]));

    let pipeline_first = PrdPipeline::new(
        backend_first.clone(),
        interaction_first,
        cache_first.clone(),
        answer_store_first,
        base_options(idea_first, 3),
    )
    .expect("pipeline creation");

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    pipeline_first.run().await.expect("first run");

    // Second run with different idea (same cache hash would be unlikely, but we'll use same cache)
    let idea_second = "different idea";
    let cache_second = CacheManager::new(workspace_root, idea_second).expect("cache manager");

    // Manually copy meta.json from first run to simulate collision
    std::fs::copy(
        cache_first.cache_dir().join("meta.json"),
        cache_second.cache_dir().join("meta.json"),
    )
    .expect("copy meta");

    let answers_path_second = cache_second.cache_dir().join("answers.yaml");
    let answer_store_second = AnswerStore::new(&answers_path_second);

    let backend_second = Arc::new(MockBackend::new("mock", vec![]));
    let interaction_second = Box::new(MockInteraction::new(vec![]));

    let mut options_second = base_options(idea_second, 3);
    options_second.resume = true;

    let pipeline_second = PrdPipeline::new(
        backend_second,
        interaction_second,
        cache_second,
        answer_store_second,
        options_second,
    )
    .expect("pipeline creation");

    let err = pipeline_second.run().await.expect_err("should fail");
    assert!(matches!(err, RalphError::PrdCacheMismatch(_)));
    assert_eq!(err.exit_code(), 2);
}

#[tokio::test]
async fn validation_parse_retry_exhaustion_fails_closed() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "validation parse retry test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
    let answers_path = cache.cache_dir().join("answers.yaml");
    let answer_store = AnswerStore::new(&answers_path);

    // All stages succeed, but validation JSON is malformed for 3 attempts
    let backend = Arc::new(MockBackend::new(
        "mock",
        vec![
            make_ideation_output("v1"),
            empty_gap_report(),
            make_research_output("v1"),
            empty_gap_report(),
            make_synthesis_output("v1"),
            empty_gap_report(),
            make_prd_output("v1"),
            empty_gap_report(),
            "Invalid JSON response 1".to_string(),
            "Invalid JSON response 2".to_string(),
            "Invalid JSON response 3".to_string(),
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

    let _cwd_guard = acquire_cwd_lock();
    let _cwd = CwdGuard::enter(workspace_root);
    let err = pipeline.run().await.expect_err("should fail");

    assert!(matches!(err, RalphError::PrdValidationFailed(_)));
    assert_eq!(err.exit_code(), 11);
    assert_eq!(backend.call_count().await, 11);
    assert!(!workspace_root.join("PRD.md").exists());
}
