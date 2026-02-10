//! Integration tests for PRD pipeline.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use ralph::backend::MockBackend;
use ralph::prd::{
    AnswerStore, CacheManager, MockInteraction, NonInteractiveInteraction, PrdOptions, PrdPipeline,
};
use ralph::RalphError;
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
async fn happy_path_produces_prd_and_cache_artifacts() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "smart onboarding system";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
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

    let pipeline = PrdPipeline::new(
        backend.clone(),
        interaction,
        cache.clone(),
        answer_store,
        options,
    )
    .expect("pipeline creation");

    let original_dir = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(workspace_root).expect("set current dir");

    let result = pipeline.run().await.expect("pipeline run");

    std::env::set_current_dir(original_dir).expect("restore current dir");

    // Verify PRD.md was written to current directory (workspace_root in this test).
    let prd_path = workspace_root.join("PRD.md");
    assert!(prd_path.exists());
    assert_eq!(result.prd_path, PathBuf::from("PRD.md"));

    // Verify metadata.
    assert_eq!(result.meta.question_rounds, 0);
    assert_eq!(result.meta.stage_timings.len(), 4);
    assert!(result.meta.completed_at.is_some());

    // Verify backend was called exactly 4 times (one per stage).
    assert_eq!(backend.call_count().await, 4);

    // Verify cache artifacts were written.
    assert!(cache
        .read_stage_output(ralph::prd::state::Stage::Ideation)
        .unwrap()
        .is_some());
    assert!(cache
        .read_stage_output(ralph::prd::state::Stage::Research)
        .unwrap()
        .is_some());
    assert!(cache
        .read_stage_output(ralph::prd::state::Stage::Synthesis)
        .unwrap()
        .is_some());
    assert!(cache
        .read_stage_output(ralph::prd::state::Stage::Prd)
        .unwrap()
        .is_some());

    // Verify meta.json was written.
    assert!(cache.read_meta().unwrap().is_some());
}

#[tokio::test]
async fn non_interactive_gap_path_writes_missing_info_report() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "incomplete ideation test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
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
    let interaction = Box::new(NonInteractiveInteraction::new());

    let options = PrdOptions {
        idea: idea.to_string(),
        backend_spec: "codex(gpt-5.3-codex)".to_string(),
        ask_max: 3,
        resume: false,
        dry_run: false,
    };

    let pipeline = PrdPipeline::new(backend, interaction, cache.clone(), answer_store, options)
        .expect("pipeline creation");

    let original_dir = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(workspace_root).expect("set current dir");

    let err = pipeline.run().await.expect_err("should fail");

    std::env::set_current_dir(original_dir).expect("restore current dir");

    // Verify error type.
    assert!(matches!(err, RalphError::PrdMissingInfo));

    // Verify missing_info_report.md was written.
    let report_path = cache.cache_dir().join("missing_info_report.md");
    assert!(report_path.exists());

    let report = std::fs::read_to_string(report_path).expect("read report");
    assert!(report.contains("Missing Information Report"));
    assert!(report.contains("Stage: Ideation"));
    assert!(report.contains("## Success Metrics"));
    assert!(report.contains("## Constraints & Assumptions"));
}

#[tokio::test]
async fn pipeline_acquires_lock_preventing_concurrent_runs() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "lock test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
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

    // Manually acquire lock to simulate concurrent run.
    let _lock = cache.acquire_lock().expect("acquire lock");

    let pipeline = PrdPipeline::new(backend, interaction, cache, answer_store, options)
        .expect("pipeline creation");

    let original_dir = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(workspace_root).expect("set current dir");

    let err = pipeline.run().await.expect_err("should fail due to lock");

    std::env::set_current_dir(original_dir).expect("restore current dir");

    // Verify error type.
    assert!(matches!(err, RalphError::PrdPipelineFailed(_)));
}

#[tokio::test]
async fn pipeline_writes_stage_artifacts_incrementally() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path();
    let idea = "incremental test";

    let cache = CacheManager::new(workspace_root, idea).expect("cache manager");
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

    let pipeline = PrdPipeline::new(backend, interaction, cache.clone(), answer_store, options)
        .expect("pipeline creation");

    let original_dir = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(workspace_root).expect("set current dir");

    let _result = pipeline.run().await.expect("pipeline run");

    std::env::set_current_dir(original_dir).expect("restore current dir");

    // Verify all stage artifact files exist.
    use ralph::prd::state::Stage;
    let ideation_file = cache.cache_dir().join(Stage::Ideation.artifact_filename());
    let research_file = cache.cache_dir().join(Stage::Research.artifact_filename());
    let synthesis_file = cache
        .cache_dir()
        .join(Stage::Synthesis.artifact_filename());
    let prd_file = cache.cache_dir().join(Stage::Prd.artifact_filename());

    assert!(ideation_file.exists(), "01_ideation.md should exist");
    assert!(research_file.exists(), "02_research.md should exist");
    assert!(synthesis_file.exists(), "03_synthesis.md should exist");
    assert!(prd_file.exists(), "04_prd.md should exist");
}
